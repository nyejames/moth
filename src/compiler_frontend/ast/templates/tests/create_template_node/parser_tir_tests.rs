use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    ConstRecordState, Expression, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, ReactiveSubscription, SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateControlFlowValidationMode;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopControlKind;
use crate::compiler_frontend::ast::templates::template_head_parser::{
    TemplateHeadParseRequest, parse_template_head,
};
use crate::compiler_frontend::ast::templates::template_render_units::install_formatted_tir_reference_for_linear_template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrSummary, TemplateTirPhase,
    TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::datatypes::datatype::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn parse_template(
    source: &str,
    string_table: &mut StringTable,
) -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let mut token_stream = template_tokens_from_source(source, string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());
    let template_ir_store = context.template_ir_store();

    let template = Template::new(&mut token_stream, &context, vec![], string_table)
        .expect("template should parse");

    (template, template_ir_store)
}

fn parse_const_required_template(
    source: &str,
    string_table: &mut StringTable,
) -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let mut token_stream = template_tokens_from_source(source, string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());
    let template_ir_store = context.template_ir_store();

    let template = Template::new_const_required(&mut token_stream, &context, vec![], string_table)
        .expect("const-required template should parse")
        .template;

    (template, template_ir_store)
}

fn tir_root_child_ids(template: &Template, store: &TemplateIrStore) -> Vec<TemplateIrNodeId> {
    let template_id = template.tir_reference.root;
    let template_ir = store
        .get_template(template_id)
        .expect("parser TIR template should exist");
    let root = store
        .get_node(template_ir.root)
        .expect("parser TIR root should exist");
    let children = match &root.kind {
        TemplateIrNodeKind::Sequence { children } => children,
        other => panic!("expected parser TIR sequence root, found {other:?}"),
    };

    children.to_owned()
}

fn parser_tir_texts(
    template: &Template,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> Vec<String> {
    tir_root_child_ids(template, store)
        .iter()
        .map(|child_id| parser_tir_text(*child_id, store, string_table))
        .collect()
}

fn parser_tir_text(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> String {
    let node = store
        .get_node(node_id)
        .expect("parser TIR child should exist");
    match &node.kind {
        TemplateIrNodeKind::Text { text, .. } => string_table.resolve(*text).to_owned(),
        other => panic!("expected parser TIR text node, found {other:?}"),
    }
}

fn parser_tir_root_child_origins(
    template: &Template,
    store: &TemplateIrStore,
) -> Vec<TemplateSegmentOrigin> {
    tir_root_child_ids(template, store)
        .iter()
        .map(|child_id| {
            let node = store
                .get_node(*child_id)
                .expect("parser TIR child should exist");
            match &node.kind {
                TemplateIrNodeKind::Text { origin, .. }
                | TemplateIrNodeKind::DynamicExpression { origin, .. } => *origin,
                other => panic!("expected origin-carrying node, found {other:?}"),
            }
        })
        .collect()
}

#[test]
fn parser_tir_owns_contiguous_literal_body_text() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[:\nalpha\nbeta]", &mut string_table);
    let store = store.borrow();

    assert_eq!(
        parser_tir_texts(&template, &store, &string_table),
        vec!["alpha\nbeta"],
        "simple finalized parser TIR should hold whitespace-normalized body text"
    );
}

#[test]
fn parser_tir_records_quoted_and_raw_markers_as_body_text() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[: \"quoted\" `raw` plain]", &mut string_table);
    let store = store.borrow();

    assert_eq!(
        parser_tir_texts(&template, &store, &string_table),
        vec![" \"quoted\" `raw` plain"]
    );
}

#[test]
fn parser_tir_records_suppressed_child_template_brackets_as_literal_text() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$doc:\n[: child]\n]", &mut string_table);
    let store = store.borrow();

    assert_eq!(
        parser_tir_texts(&template, &store, &string_table),
        vec!["<p>[: child]</p>"],
        "post-render parser TIR must reflect formatted output, with suppressed brackets preserved as literal text"
    );
}

#[test]
fn template_tir_folds_nested_child_as_child_template_boundary() {
    // A foldable nested child template now stays a `ChildTemplate` TIR boundary
    // because the TIR formatter is authoritative for child-template output. The
    // final folded output still matches the old linear-text result.
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[: before [: child] after]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(
        children.len(),
        3,
        "parser TIR root should keep leading text, child-template boundary, and trailing text"
    );
    assert_eq!(
        parser_tir_text(children[0], &store, &string_table),
        " before "
    );
    assert!(
        matches!(
            store
                .get_node(children[1])
                .expect("middle child node should exist")
                .kind,
            TemplateIrNodeKind::ChildTemplate { .. }
        ),
        "foldable nested child should remain a ChildTemplate TIR boundary"
    );
    assert_eq!(
        parser_tir_text(children[2], &store, &string_table),
        " after"
    );

    let rendered = folded_template_output("[: before [: child] after]");
    assert_eq!(
        rendered, " before  child after",
        "final folded output for a child-template boundary must match linear text"
    );
}

#[test]
fn parser_preserves_foldable_nested_child_as_template_boundary() {
    let mut string_table = StringTable::new();
    let (template, store) =
        parse_const_required_template("[:before[:child]after]", &mut string_table);
    let store = store.borrow();

    // The foldable nested child is now TIR-owned: it appears as a
    // `ChildTemplate` node in the parser TIR root, not as duplicated body content.
    let children = tir_root_child_ids(&template, &store);
    assert!(
        children.iter().any(|child_id| {
            matches!(
                store.get_node(*child_id).map(|node| &node.kind),
                Some(TemplateIrNodeKind::ChildTemplate { .. })
            )
        }),
        "foldable nested child should remain a ChildTemplate TIR boundary"
    );
}

/// Returns the root node kind of a template's finalized parser TIR entry.
fn parser_tir_root_kind<'store>(
    template: &Template,
    store: &'store TemplateIrStore,
) -> &'store TemplateIrNodeKind {
    let template_id = template.tir_reference.root;
    let template_ir = store
        .get_template(template_id)
        .expect("parser TIR template should exist");

    &store
        .get_node(template_ir.root)
        .expect("parser TIR root should exist")
        .kind
}

/// Returns the kind of the control-flow root node, whether the template root
/// is the control-flow node directly or a `Sequence` wrapping a single
/// control-flow child.
///
/// WHAT: after render-unit preparation, refreshed control-flow templates
///       finalize with the `BranchChain` or `Loop` node as the direct root.
///       Templates that still carry a prefix or were not refreshed keep a
///       `Sequence`-wrapped root. This helper abstracts over both shapes so
///       tests can assert on the control-flow node without knowing which
///       root shape the template took.
fn parser_tir_control_flow_root_kind<'store>(
    template: &Template,
    store: &'store TemplateIrStore,
) -> &'store TemplateIrNodeKind {
    match parser_tir_root_kind(template, store) {
        kind @ (TemplateIrNodeKind::BranchChain { .. }
        | TemplateIrNodeKind::Loop { .. }
        | TemplateIrNodeKind::LoopControl { .. }) => kind,
        TemplateIrNodeKind::Sequence { children } => {
            assert_eq!(
                children.len(),
                1,
                "expected parser TIR root to have exactly one child, found {children:?}"
            );
            &store
                .get_node(children[0])
                .expect("parser TIR control-flow node should exist")
                .kind
        }
        other => panic!("expected parser TIR control-flow root, found {other:?}"),
    }
}

/// Returns the child node IDs of a sequence node, panicking if the node is not
/// a sequence.
fn sequence_child_ids(node_id: TemplateIrNodeId, store: &TemplateIrStore) -> Vec<TemplateIrNodeId> {
    match &store.get_node(node_id).expect("node should exist").kind {
        TemplateIrNodeKind::Sequence { children } => children.to_owned(),
        other => panic!("expected sequence node, found {other:?}"),
    }
}

/// Returns true when the TIR subtree rooted at `node_id` contains an
/// `AggregateOutput` marker node.
///
/// WHAT: loop aggregate wrappers are sequences of wrapper content with a
///       single internal aggregate-output marker. This helper lets tests assert
///       that the installed wrapper subtree carries the marker.
fn tir_subtree_contains_aggregate_output(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
) -> bool {
    let node = store.get_node(node_id).expect("node should exist");
    match &node.kind {
        TemplateIrNodeKind::AggregateOutput => true,
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .any(|child_id| tir_subtree_contains_aggregate_output(*child_id, store)),
        TemplateIrNodeKind::Text { .. } | TemplateIrNodeKind::DynamicExpression { .. } => false,
        other => panic!("unexpected node in aggregate wrapper subtree: {other:?}"),
    }
}

#[test]
fn parser_tir_records_if_else_if_else_branch_chain() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template(
        "[if true:\nfirst\n[else if false]\nsecond\n[else]\nthird\n]",
        &mut string_table,
    );
    let store = store.borrow();

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert!(parent_template.summary.has_control_flow);

    let branch_chain = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::BranchChain { branches, fallback } => (branches, fallback),
        other => panic!("expected parser TIR BranchChain node, found {other:?}"),
    };

    let (branches, fallback) = branch_chain;
    assert_eq!(branches.len(), 2);
    assert!(fallback.is_some());

    let fallback_body = fallback
        .as_ref()
        .copied()
        .expect("branch-chain fallback body should exist");

    let first_branch_text = body_text(branches[0].body, &store, &string_table);
    let second_branch_text = body_text(branches[1].body, &store, &string_table);
    let fallback_text = body_text(fallback_body, &store, &string_table);

    assert_eq!(first_branch_text, "first");
    assert_eq!(second_branch_text, "second");
    assert_eq!(fallback_text, "third");
}

#[test]
fn template_tir_records_child_template_in_branch_body() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[if true:before [:child] after]", &mut string_table);
    let store = store.borrow();

    let branch_chain = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::BranchChain { branches, fallback } => (branches, fallback),
        other => panic!("expected parser TIR BranchChain node, found {other:?}"),
    };

    let (branches, fallback) = branch_chain;
    assert_eq!(branches.len(), 1);
    assert!(fallback.is_none());

    let body_children = sequence_child_ids(branches[0].body, &store);
    assert_eq!(body_children.len(), 3);
    assert!(matches!(
        store.get_node(body_children[0]).map(|n| &n.kind),
        Some(TemplateIrNodeKind::Text { .. })
    ));
    assert!(matches!(
        store.get_node(body_children[1]).map(|n| &n.kind),
        Some(TemplateIrNodeKind::ChildTemplate { .. })
    ));
    assert!(matches!(
        store.get_node(body_children[2]).map(|n| &n.kind),
        Some(TemplateIrNodeKind::Text { .. })
    ));
}

/// Finds the `BranchChain` node among the parser root children, returning its
/// branches and fallback. Needed when a shared head prefix precedes the
/// control-flow node so the root has more than one child.
fn branch_chain_from_root(
    template: &Template,
    store: &TemplateIrStore,
) -> (
    Vec<crate::compiler_frontend::ast::templates::tir::TemplateIrBranch>,
    Option<TemplateIrNodeId>,
) {
    let template_id = template.tir_reference.root;
    let template_ir = store
        .get_template(template_id)
        .expect("parser TIR template should exist");
    let root = store
        .get_node(template_ir.root)
        .expect("parser TIR root should exist");

    // After render-unit preparation, refreshed control-flow templates finalize
    // with the BranchChain as the direct root — whether or not a head prefix was
    // present. A Sequence-wrapped root only appears for multi-child or unrefreshed
    // control-flow templates.
    let branch_chain_node_id = match &root.kind {
        TemplateIrNodeKind::BranchChain { .. } => template_ir.root,
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .copied()
            .find(|id| {
                store
                    .get_node(*id)
                    .is_some_and(|node| matches!(node.kind, TemplateIrNodeKind::BranchChain { .. }))
            })
            .expect("parser root should contain a BranchChain node"),
        other => panic!("expected parser TIR BranchChain or Sequence root, found {other:?}"),
    };

    match &store
        .get_node(branch_chain_node_id)
        .expect("BranchChain node should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, fallback } => (branches.clone(), *fallback),
        other => panic!("expected BranchChain node, found {other:?}"),
    }
}

#[test]
fn branch_body_tir_root_derives_shared_head_prefix_from_parser_tir() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[\"prefix\", if true:body]", &mut string_table);
    let store = store.borrow();

    let (branches, fallback) = branch_chain_from_root(&template, &store);
    assert_eq!(branches.len(), 1);
    assert!(fallback.is_none());

    // The branch body root should carry the shared head prefix plus the body
    // text, proving the shared prefix still applies through the TIR-derived
    // body root.
    let body_children = sequence_child_ids(branches[0].body, &store);
    let branch_body_text = body_children
        .iter()
        .map(|child_id| parser_tir_text(*child_id, &store, &string_table))
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(branch_body_text, "prefixbody");

    // The head-prefix portion of the body root should retain the parser-emitted
    // TIR node. The owner root is now
    // the BranchChain itself, so the prefix lives only inside the selected
    // branch bodies.
    assert_eq!(
        parser_tir_text(body_children[0], &store, &string_table),
        "prefix",
        "branch body root should start with the shared head-prefix text"
    );
}

#[test]
fn fallback_body_tir_root_derives_shared_head_prefix_from_parser_tir() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template(
        "[\"prefix\", if false:\nbranch\n[else]\nfallback\n]",
        &mut string_table,
    );
    let store = store.borrow();

    let (branches, fallback) = branch_chain_from_root(&template, &store);
    assert_eq!(branches.len(), 1);
    let fallback_body = fallback.expect("branch chain should have a fallback body");

    // The fallback body root should carry the shared head prefix plus the
    // fallback body text, proving the shared prefix applies to the fallback
    // through the TIR-derived body root.
    let fallback_children = sequence_child_ids(fallback_body, &store);
    let fallback_body_text = fallback_children
        .iter()
        .map(|child_id| parser_tir_text(*child_id, &store, &string_table))
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(fallback_body_text, "prefixfallback");

    // The head-prefix portion of the fallback body root should retain the
    // parser-emitted TIR node. The owner root is now the BranchChain itself, so
    // the prefix lives only inside the branch/fallback bodies.
    assert_eq!(
        parser_tir_text(fallback_children[0], &store, &string_table),
        "prefix",
        "fallback body root should start with the shared head-prefix text"
    );
}

#[test]
fn parser_tir_trims_loop_control_boundary_whitespace() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template(
        "[loop true:\n    [continue]\n    visible\n]",
        &mut string_table,
    );
    let store = store.borrow();

    let loop_node = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::Loop { body, .. } => body,
        other => panic!("expected parser TIR Loop node, found {other:?}"),
    };

    let body_children = sequence_child_ids(*loop_node, &store);
    assert!(matches!(
        store
            .get_node(body_children[0])
            .expect("loop body child should exist")
            .kind,
        TemplateIrNodeKind::LoopControl {
            kind: TemplateLoopControlKind::Continue,
        }
    ));
    assert_eq!(
        parser_tir_text(body_children[1], &store, &string_table),
        "visible"
    );
}

#[test]
fn parser_tir_records_loop_node() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[loop true: body]", &mut string_table);
    let store = store.borrow();

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert!(parent_template.summary.has_control_flow);

    let (header, body, aggregate_wrapper) =
        match parser_tir_control_flow_root_kind(&template, &store) {
            TemplateIrNodeKind::Loop {
                header,
                body,
                aggregate_wrapper,
                ..
            } => (header, body, aggregate_wrapper),
            other => panic!("expected parser TIR Loop node, found {other:?}"),
        };

    assert!(matches!(
        header,
        crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader::Conditional { .. }
    ));
    assert!(
        aggregate_wrapper.is_some(),
        "loop aggregate wrapper should be installed during render-unit preparation"
    );
    assert!(
        tir_subtree_contains_aggregate_output(aggregate_wrapper.unwrap(), &store),
        "installed aggregate wrapper subtree should contain the AggregateOutput marker"
    );

    let body_children = sequence_child_ids(*body, &store);
    assert_eq!(body_children.len(), 1);
    assert_eq!(
        parser_tir_text(body_children[0], &store, &string_table),
        " body"
    );
}

#[test]
fn template_tir_records_child_template_in_loop_body() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[loop true:before [:child] after]", &mut string_table);
    let store = store.borrow();

    let loop_node = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::Loop { body, .. } => body,
        other => panic!("expected parser TIR Loop node, found {other:?}"),
    };

    let body_children = sequence_child_ids(*loop_node, &store);
    assert_eq!(body_children.len(), 3);
    assert!(matches!(
        store.get_node(body_children[0]).map(|n| &n.kind),
        Some(TemplateIrNodeKind::Text { .. })
    ));
    assert!(matches!(
        store.get_node(body_children[1]).map(|n| &n.kind),
        Some(TemplateIrNodeKind::ChildTemplate { .. })
    ));
    assert!(matches!(
        store.get_node(body_children[2]).map(|n| &n.kind),
        Some(TemplateIrNodeKind::Text { .. })
    ));
}

#[test]
fn parser_tir_records_loop_control_markers_inside_loop_body() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template(
        "[loop true:\n    before\n    [break]\n    after\n]",
        &mut string_table,
    );
    let store = store.borrow();

    let loop_node = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::Loop { body, .. } => body,
        other => panic!("expected parser TIR Loop node, found {other:?}"),
    };

    let body_children = sequence_child_ids(*loop_node, &store);
    let control_kinds: Vec<TemplateLoopControlKind> = body_children
        .iter()
        .filter_map(|child_id| {
            let node = store
                .get_node(*child_id)
                .expect("loop body child should exist");
            match &node.kind {
                TemplateIrNodeKind::LoopControl { kind } => Some(*kind),
                _ => None,
            }
        })
        .collect();

    assert_eq!(control_kinds, vec![TemplateLoopControlKind::Break]);
}

#[test]
fn parser_tir_records_continue_marker_inside_loop_body() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template(
        "[loop true:\n    before\n    [continue]\n    after\n]",
        &mut string_table,
    );
    let store = store.borrow();

    let loop_node = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::Loop { body, .. } => body,
        other => panic!("expected parser TIR Loop node, found {other:?}"),
    };

    let body_children = sequence_child_ids(*loop_node, &store);
    let control_kinds: Vec<TemplateLoopControlKind> = body_children
        .iter()
        .filter_map(|child_id| {
            let node = store
                .get_node(*child_id)
                .expect("loop body child should exist");
            match &node.kind {
                TemplateIrNodeKind::LoopControl { kind } => Some(*kind),
                _ => None,
            }
        })
        .collect();

    assert_eq!(control_kinds, vec![TemplateLoopControlKind::Continue]);
}

/// Returns the text content of a body root by reading its sequence text nodes.
/// Helper used for control-flow structure assertions only.
fn body_text(
    body: TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> String {
    sequence_child_ids(body, store)
        .iter()
        .map(|child_id| parser_tir_text(*child_id, store, string_table))
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn parser_tir_records_default_slot_placeholder() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[: before [$slot] after]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 3);
    assert_eq!(
        parser_tir_text(children[0], &store, &string_table),
        " before "
    );
    assert_eq!(
        parser_tir_text(children[2], &store, &string_table),
        " after"
    );

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert_eq!(parent_template.summary.slot_count, 1);
    assert!(parent_template.summary.has_slots());

    let slot = match &store
        .get_node(children[1])
        .expect("parser TIR slot node should exist")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => placeholder,
        other => panic!("expected parser TIR Slot node, found {other:?}"),
    };
    assert_eq!(slot.key, SlotKey::Default);
}

#[test]
fn parser_tir_records_named_slot_placeholder() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[: before [$slot(\"name\")] after]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 3);

    let slot = match &store
        .get_node(children[1])
        .expect("parser TIR slot node should exist")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => placeholder,
        other => panic!("expected parser TIR Slot node, found {other:?}"),
    };

    let expected_name = string_table.intern("name");
    assert_eq!(slot.key, SlotKey::Named(expected_name));
}

#[test]
fn parser_tir_records_positional_slot_placeholder() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[: before [$slot(1)] after]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 3);

    let slot = match &store
        .get_node(children[1])
        .expect("parser TIR slot node should exist")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => placeholder,
        other => panic!("expected parser TIR Slot node, found {other:?}"),
    };

    assert_eq!(slot.key, SlotKey::Positional(1));
}

#[test]
fn parser_tir_records_string_literal_head_before_body_with_head_origin() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[\"head\": body]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 2);
    assert_eq!(parser_tir_text(children[0], &store, &string_table), "head");
    assert_eq!(parser_tir_text(children[1], &store, &string_table), " body");
    assert_eq!(
        parser_tir_root_child_origins(&template, &store),
        vec![TemplateSegmentOrigin::Head, TemplateSegmentOrigin::Body]
    );
}

#[test]
fn parser_tir_records_numeric_head_as_dynamic_expression_with_head_origin() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[42: body]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 2);
    assert!(matches!(
        store
            .get_node(children[0])
            .expect("head child should exist")
            .kind,
        TemplateIrNodeKind::DynamicExpression {
            origin: TemplateSegmentOrigin::Head,
            reactive_subscription: None,
            ..
        }
    ));
    assert_eq!(parser_tir_text(children[1], &store, &string_table), " body");
}

#[test]
fn parser_tir_records_rendered_path_head_as_tir_only() {
    let mut string_table = StringTable::new();
    let resource = test_resource_file("parser_tir_rendered_path_head.txt");
    let (template, store) = parse_template(&format!("[@{resource}: body]"), &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 2);
    let head_text = parser_tir_text(children[0], &store, &string_table);
    assert!(!head_text.is_empty());
    assert_eq!(
        parser_tir_root_child_origins(&template, &store),
        vec![TemplateSegmentOrigin::Head, TemplateSegmentOrigin::Body]
    );
}

#[test]
fn parser_tir_preserves_reactive_head_and_nested_child_metadata() {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let source_name = string_table.intern("source");
    let source_path = scope.append(source_name);
    let source_location = SourceLocation {
        scope: scope.clone(),
        start_pos: CharPosition {
            line_number: 1,
            char_column: 0,
        },
        end_pos: CharPosition {
            line_number: 1,
            char_column: 120,
        },
    };
    let source = ReactiveSource {
        path: source_path.clone(),
        kind: ReactiveSourceKind::Declaration,
    };
    let source_expression = Expression::reference_with_type_id(
        source_path.clone(),
        DataType::StringSlice,
        builtin_type_ids::STRING,
        source_location.clone(),
        ValueMode::ImmutableOwned,
        ConstRecordState::ConstRecord,
    )
    .with_reactive_source(source);

    let declaration = Declaration {
        id: source_path.clone(),
        value: source_expression,
    };

    let mut token_stream = template_tokens_from_source("[$(source): body]", &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Template,
        token_stream.src_path.to_owned(),
        Rc::new(TopLevelDeclarationTable::new(vec![declaration])),
        Arc::new(crate::compiler_frontend::external_packages::ExternalPackageRegistry::default()),
        vec![],
        0,
    )
    .with_project_path_resolver(Some(test_project_path_resolver()))
    .with_source_file_scope(token_stream.src_path.to_owned())
    .with_path_format_config(PathStringFormatConfig::default());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("reactive head template should parse");
    let store = context.template_ir_store();
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 2);
    let head_node = store
        .get_node(children[0])
        .expect("head child should exist");
    let TemplateIrNodeKind::DynamicExpression {
        origin,
        reactive_subscription,
        ..
    } = &head_node.kind
    else {
        panic!("expected reactive head child to be a dynamic expression");
    };
    assert_eq!(*origin, TemplateSegmentOrigin::Head);
    assert_eq!(
        reactive_subscription
            .as_ref()
            .map(|subscription| &subscription.source.path),
        Some(&source_path),
        "parser TIR must preserve the explicit reactive subscription on the dynamic node"
    );
    assert_eq!(parser_tir_text(children[1], &store, &string_table), " body");

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert!(parent_template.summary.has_reactivity);
}

// -------------------------
//  Inline-code anchors
// -------------------------

/// Returns the concatenated text of all root children, treating non-text nodes
/// as empty strings. Used when the exact structural mix of text and opaque
/// anchors is the behavior under test.
fn root_text_excluding_opaque_anchors(
    template: &Template,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> String {
    tir_root_child_ids(template, store)
        .iter()
        .map(
            |child_id| match &store.get_node(*child_id).expect("child should exist").kind {
                TemplateIrNodeKind::Text { text, .. } => string_table.resolve(*text).to_owned(),
                _ => String::new(),
            },
        )
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn formatter_inline_code_literal_preserves_code_markup_in_parser_tir() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md: `literal code`]", &mut string_table);
    let store = store.borrow();

    let tir_reference = &template.tir_reference;
    assert_eq!(
        tir_reference.phase,
        TemplateTirPhase::Formatted,
        "explicit formatter inline-code bodies should install a Formatted TIR root"
    );

    let root_text = parser_tir_texts(&template, &store, &string_table).join("");
    assert_eq!(
        root_text, "<p> <code>literal code</code></p>",
        "literal inline-code spans must be preserved as formatted TIR text"
    );
}

#[test]
fn formatter_inline_code_preserves_span_for_authored_body_head_insert_anchor() {
    // Authored body `[value]` parses as a parser-TIR `ChildTemplate` boundary,
    // but the TIR formatter classifies head-expression inserts as
    // `DynamicExpression` anchors, so markdown inline code can pair across the
    // inserted scalar string through the TIR formatter path.
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let value_name = string_table.intern("value");
    let declarations = vec![Declaration {
        id: scope.append(value_name),
        value: Expression::string_slice(
            string_table.intern("ANCHOR"),
            SourceLocation {
                scope: InternedPath::new(),
                start_pos: CharPosition {
                    line_number: 1,
                    char_column: 0,
                },
                end_pos: CharPosition {
                    line_number: 1,
                    char_column: 120,
                },
            },
            ValueMode::ImmutableOwned,
        ),
    }];

    let mut token_stream =
        template_tokens_from_source("[$md: `before [value] after`]", &mut string_table);
    let context = constant_template_context(&token_stream.src_path, &declarations);
    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("markdown inline-code with body reference should parse");
    let store = context.template_ir_store();
    let store = store.borrow();

    let tir_reference = &template.tir_reference;
    assert_eq!(
        tir_reference.phase,
        TemplateTirPhase::Formatted,
        "explicit formatter inline-code bodies should install a Formatted TIR root"
    );

    let children = tir_root_child_ids(&template, &store);
    let child_template_position = children.iter().position(|child_id| {
        matches!(
            store.get_node(*child_id).expect("child should exist").kind,
            TemplateIrNodeKind::ChildTemplate { .. }
        )
    });
    assert!(
        child_template_position.is_some(),
        "authored body `[value]` must parse as a ChildTemplate boundary in parser TIR"
    );

    // Because the child-template anchor is a head-expression insert, the
    // inline-code span is preserved and the backticks become a paired `<code>`
    // wrapper around the inserted value.
    let root_text = root_text_excluding_opaque_anchors(&template, &store, &string_table);
    assert!(
        root_text.contains("<code>"),
        "head-insert inline-code span must emit <code> around the dynamic-expression boundary"
    );
    assert!(
        root_text.contains("before ") && root_text.contains(" after"),
        "inline-code wrapper must enclose the literal text around the head insert"
    );
}

#[test]
fn inline_code_head_insert_records_formatted_tir_phase() {
    // Markdown/head-insert inline-code surfaces install a module-local `Formatted`
    // TIR reference and the TIR formatter now classifies the head-expression
    // insert as a `DynamicExpression` anchor, so the formatted TIR root records
    // the inline-code span.
    let mut string_table = StringTable::new();
    let (template, store) =
        parse_template("[$md:\nLiteral syntax `[\"[slot]\"]`\n]", &mut string_table);
    let store = store.borrow();

    let tir_reference = &template.tir_reference;
    assert_eq!(
        tir_reference.phase,
        TemplateTirPhase::Formatted,
        "setup: inline-code markdown template must have a Formatted TIR reference"
    );
    // The head-expression insert remains a child-template boundary in the stored
    // TIR tree; the formatter only reclassifies it as an expression anchor for
    // the formatter pipeline, preserving inline-code span behavior.
    let parent_template = store
        .get_template(tir_reference.root)
        .expect("formatted TIR template should exist");
    assert!(
        parent_template.summary.child_template_count > 0,
        "setup: inline-code head insert must produce a child-template boundary in formatted TIR"
    );

    let rendered = folded_template_output("[$md:\nLiteral syntax `[\"[slot]\"]`\n]");
    assert_eq!(
        rendered, "<p>Literal syntax <code>[slot]</code></p>",
        "head-insert inline-code formatting must preserve the inline code span"
    );
}

#[test]
fn head_stringslice_records_formatted_tir_phase() {
    // Head-origin literal text is preserved unchanged by formatters, so the
    // module-local `Formatted` TIR root remains available.
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template("[\"prefix\", $md: body]", &mut string_table);

    let reference = &template.tir_reference;
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "setup: head-stringslice markdown template must have a Formatted TIR reference"
    );
    let rendered = folded_template_output("[\"prefix\", $md: body]");
    assert_eq!(
        rendered, "prefix<p> body</p>",
        "head-stringslice formatted TIR root must preserve the head prefix before the formatted body"
    );
}

#[test]
fn style_child_wrapper_no_children_records_formatted_tir_phase() {
    // A `$children(..)` style wrapper with no body child templates is a no-op
    // wrapper; the formatted TIR root remains available.
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template(
        "[$md, $children([:<b>[$slot]</b>]): body text]",
        &mut string_table,
    );

    let reference = &template.tir_reference;
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "setup: style-wrapper markdown template must have a Formatted TIR reference"
    );
    let rendered = folded_template_output("[$md, $children([:<b>[$slot]</b>]): body text]");
    assert_eq!(
        rendered, "<p> body text</p>",
        "style child-template wrappers with no body children must not change formatted output"
    );
}

#[test]
fn style_child_wrapper_with_children_records_formatted_tir_phase() {
    // When there are body child templates for the style wrapper to apply to, the
    // formatted TIR root remains available because the TIR formatter preserves
    // child-template boundaries for wrapper application.
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template(
        "[$md, $children([:<b>[$slot]</b>]): hello [:child] ]",
        &mut string_table,
    );

    let reference = &template.tir_reference;
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "setup: style-wrapper-with-children markdown template must have a Formatted TIR reference"
    );
    let rendered = folded_template_output("[$md, $children([:<b>[$slot]</b>]): hello [:child] ]");
    assert_eq!(
        rendered, "<p> hello <b>child</b> </p>",
        "style child-template wrappers with body children must still apply direct-child wrappers"
    );
}

#[test]
fn head_only_literal_text_records_formatted_tir_phase() {
    // A template whose only safe content is head-origin literal text has no body
    // for the formatter to contextually alter, so the module-local `Formatted`
    // TIR root remains available.
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template("[\"head\", $md:]", &mut string_table);

    let reference = &template.tir_reference;
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "setup: head-only literal markdown template must have a Formatted TIR reference"
    );
    assert_eq!(
        folded_template_output("[\"head\", $md:]"),
        "head",
        "head-only literal formatted TIR root must preserve the head prefix unchanged"
    );
}

/// Builds a `Template` handle whose TIR root is constructed directly in the
/// context's module store via `TemplateIrBuilder`, then attaches a `Parsed`
/// phase module-local TIR reference.
///
/// WHAT: lets formatter fixtures construct body-origin dynamic expressions and
///       reactive side-table subscriptions that source parsing cannot express
///       narrowly.
/// WHY: these tests protect TIR-owned payload facts directly.
fn build_template_with_direct_tir_root(
    context: &ScopeContext,
    kind: TemplateType,
    style: Style,
    location: SourceLocation,
    build_root: impl FnOnce(
        &mut TemplateIrStore,
        &mut StringTable,
    ) -> (TemplateIrNodeId, TemplateIrSummary),
    string_table: &mut StringTable,
) -> Template {
    let template_id = {
        let store = context.template_ir_store();
        let mut store = store.borrow_mut();
        let (root, summary) = build_root(&mut store, string_table);
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.finish_template(root, style.clone(), kind.clone(), summary, location.clone())
    };
    let context = TemplateViewContext::default();
    Template {
        location,
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
    }
}

#[test]
fn pure_direct_dynamic_formatter_template_records_formatted_tir_phase() {
    // A body with only an ordinary dynamic-expression anchor and an explicit
    // formatter has no formatter-context-sensitive text, so the module-local
    // `Formatted` TIR root remains available. Source parsing cannot express a
    // body-origin dynamic expression, so the body TIR is constructed directly
    // via `TemplateIrBuilder`.
    let mut string_table = StringTable::new();
    let context = new_constant_context(InternedPath::from_single_str(
        "main.moth",
        &mut string_table,
    ));
    let location = SourceLocation::default();

    let mut template = build_template_with_direct_tir_root(
        &context,
        TemplateType::String,
        Style {
            formatter: Some(markdown_formatter()),
            ..Style::default()
        },
        location.clone(),
        move |store, _string_table| {
            let mut builder = TemplateIrBuilder::new(store);
            let body_node = builder.push_dynamic_expression_node(
                Expression::int(42, location.clone(), ValueMode::ImmutableOwned),
                TemplateSegmentOrigin::Body,
                None,
                location.clone(),
            );
            let root = builder.push_sequence_node(vec![body_node], location.clone());
            let summary = TemplateIrSummary {
                dynamic_expression_count: 1,
                max_depth: 1,
                ..TemplateIrSummary::default()
            };
            (root, summary)
        },
        &mut string_table,
    );

    let style = effective_tir_style(&template, &context);
    let has_control_flow = {
        let store = context.template_ir_store.borrow();
        store
            .control_flow_node_id_for_template(template.tir_reference.root)
            .expect("control-flow lookup")
            .is_some()
    };
    let tir_reference = &mut template.tir_reference;
    install_formatted_tir_reference_for_linear_template(
        tir_reference,
        has_control_flow,
        &style,
        &context,
        &mut string_table,
    )
    .expect("formatted TIR reference installation should succeed");
}

#[test]
fn reactive_body_segment_records_formatted_tir_phase() {
    // A body segment carrying a reactive subscription whose expression is a safe
    // formatter anchor preserves the formatted TIR root because the subscription
    // metadata is preserved through the TIR formatter anchor and formatted
    // root. Source parsing only emits reactive head expressions, so
    // this body reactive dynamic-expression payload is constructed directly via
    // `TemplateIrBuilder`.
    let mut string_table = StringTable::new();
    let context = new_constant_context(InternedPath::from_single_str(
        "main.moth",
        &mut string_table,
    ));
    let location = SourceLocation::default();

    let source_path = InternedPath::from_single_str("main.moth/#reactive0", &mut string_table);
    let expected_source_path = source_path.clone();
    let source = ReactiveSource {
        path: source_path.clone(),
        kind: ReactiveSourceKind::Declaration,
    };
    let subscription = ReactiveSubscription {
        source: source.clone(),
        type_id: builtin_type_ids::STRING,
        location: location.clone(),
    };
    let expression = Expression::reference_with_type_id(
        source_path,
        DataType::StringSlice,
        builtin_type_ids::STRING,
        location.clone(),
        ValueMode::ImmutableOwned,
        ConstRecordState::ConstRecord,
    )
    .with_reactive_source(source);

    let mut template = build_template_with_direct_tir_root(
        &context,
        TemplateType::String,
        Style {
            formatter: Some(markdown_formatter()),
            ..Style::default()
        },
        location.clone(),
        move |store, _string_table| {
            let mut builder = TemplateIrBuilder::new(store);
            let body_node = builder.push_dynamic_expression_node(
                expression,
                TemplateSegmentOrigin::Body,
                Some(subscription),
                location.clone(),
            );
            let root = builder.push_sequence_node(vec![body_node], location.clone());
            let summary = TemplateIrSummary {
                dynamic_expression_count: 1,
                max_depth: 1,
                has_reactivity: true,
                ..TemplateIrSummary::default()
            };
            (root, summary)
        },
        &mut string_table,
    );

    let style = effective_tir_style(&template, &context);
    let has_control_flow = {
        let store = context.template_ir_store.borrow();
        store
            .control_flow_node_id_for_template(template.tir_reference.root)
            .expect("control-flow lookup")
            .is_some()
    };
    let tir_reference = &mut template.tir_reference;
    install_formatted_tir_reference_for_linear_template(
        tir_reference,
        has_control_flow,
        &style,
        &context,
        &mut string_table,
    )
    .expect("formatted TIR reference installation should succeed");

    let store = context.template_ir_store();
    let store = store.borrow();
    let children = tir_root_child_ids(&template, &store);
    let reactive_subscription = children
        .iter()
        .find_map(|child_id| {
            let node = store
                .get_node(*child_id)
                .expect("formatted root child should exist");
            match &node.kind {
                TemplateIrNodeKind::DynamicExpression {
                    reactive_subscription,
                    ..
                } => Some(reactive_subscription),
                _ => None,
            }
        })
        .expect("formatted root should preserve the dynamic-expression anchor");
    assert_eq!(
        reactive_subscription
            .as_ref()
            .map(|subscription| &subscription.source.path),
        Some(&expected_source_path),
        "formatting must preserve the dynamic anchor's reactive subscription"
    );
}

#[test]
fn reactive_literal_text_segment_records_formatted_tir_phase() {
    // A reactive subscription on literal body text is stored in the TIR store's
    // node-level reactive-subscription side-table, so the formatted TIR root can
    // be authoritative while preserving the dependency for reactive metadata.
    // Source parsing only emits reactive head text, so this body reactive text
    // payload is constructed directly via `TemplateIrBuilder`.
    let mut string_table = StringTable::new();
    let context = new_constant_context(InternedPath::from_single_str(
        "main.moth",
        &mut string_table,
    ));
    let location = SourceLocation::default();

    let source_path = InternedPath::from_single_str("main.moth/#reactive0", &mut string_table);
    let expected_source_path = source_path.clone();
    let subscription = ReactiveSubscription {
        source: ReactiveSource {
            path: source_path,
            kind: ReactiveSourceKind::Declaration,
        },
        type_id: builtin_type_ids::STRING,
        location: location.clone(),
    };

    let mut template = build_template_with_direct_tir_root(
        &context,
        TemplateType::String,
        Style {
            formatter: Some(markdown_formatter()),
            ..Style::default()
        },
        location.clone(),
        move |store, string_table| {
            let text = string_table.intern("reactive body");
            let byte_len = "reactive body".len();
            let mut builder = TemplateIrBuilder::new(store);
            let body_node = builder.push_text_node_with_subscription(
                text,
                byte_len,
                TemplateSegmentOrigin::Body,
                Some(subscription),
                location.clone(),
            );
            let root = builder.push_sequence_node(vec![body_node], location.clone());
            let summary = TemplateIrSummary {
                estimated_output_bytes: byte_len,
                text_node_count: 1,
                text_byte_count: byte_len,
                max_depth: 1,
                has_reactivity: true,
                ..TemplateIrSummary::default()
            };
            (root, summary)
        },
        &mut string_table,
    );

    let style = effective_tir_style(&template, &context);
    let has_control_flow = {
        let store = context.template_ir_store.borrow();
        store
            .control_flow_node_id_for_template(template.tir_reference.root)
            .expect("control-flow lookup")
            .is_some()
    };
    let tir_reference = &mut template.tir_reference;
    install_formatted_tir_reference_for_linear_template(
        tir_reference,
        has_control_flow,
        &style,
        &context,
        &mut string_table,
    )
    .expect("formatted TIR reference installation should succeed");

    let store = context.template_ir_store();
    let store = store.borrow();
    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 1);
    assert!(matches!(
        store
            .get_node(children[0])
            .expect("formatted reactive text node should exist")
            .kind,
        TemplateIrNodeKind::Text { .. }
    ));
    assert_eq!(
        store
            .node_reactive_subscription(children[0])
            .expect("reactive side-table lookup should succeed")
            .map(|subscription| &subscription.source.path),
        Some(&expected_source_path),
        "formatting must preserve the text node's reactive side-table entry"
    );
}

#[test]
fn head_expression_folds_through_tir_formatter() {
    // Head-origin non-literal expressions are formatter-visible opaque anchors.
    // The TIR formatter now owns that shape directly instead of rebuilding a
    // parser TIR.
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template("[42, $md:]", &mut string_table);

    let reference = &template.tir_reference;
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "setup: head-expression markdown template must have a Formatted TIR reference"
    );
    assert_eq!(
        folded_template_output("[42, $md:]"),
        "42",
        "head-expression output must fold correctly through the TIR formatter"
    );
}

// -------------------------
//  $raw formatter boundary
// -------------------------

#[test]
fn raw_directive_preserves_whitespace_and_advances_through_formatter_adapter() {
    // `$raw` disables the default body-whitespace pass and has no formatter of
    // its own, but the render-unit formatter adapter still runs over the body
    // and advances the TIR reference to `Formatted`. The important behavior is
    // that the authored whitespace survives unchanged.
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$raw:\n    Hello\n    World\n]", &mut string_table);
    let store = store.borrow();

    let tir_reference = &template.tir_reference;
    assert_eq!(
        tir_reference.phase,
        TemplateTirPhase::Formatted,
        "$raw linear bodies advance through the formatter adapter even though no formatter runs"
    );

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(
        children.len(),
        1,
        "$raw body should remain a single contiguous text node"
    );
    assert_eq!(
        parser_tir_text(children[0], &store, &string_table),
        "\n    Hello\n    World\n",
        "$raw parser TIR must preserve authored whitespace exactly"
    );

    // User-facing folded output is owned by the existing whitespace test; repeat
    // it here so the TIR phase assertion and the output assertion stay coupled.
    assert_eq!(
        folded_template_output("[$raw:\n    Hello\n    World\n]"),
        "\n    Hello\n    World\n"
    );
}

// -------------------------
//  Nested directive isolation
// -------------------------

/// Finds the first `ChildTemplate` node among the root children and returns the
/// referenced template ID. Panics if no child-template anchor exists.
fn first_child_template_id(
    template: &Template,
    store: &TemplateIrStore,
) -> crate::compiler_frontend::ast::templates::tir::TemplateIrId {
    tir_root_child_ids(template, store)
        .iter()
        .find_map(
            |child_id| match &store.get_node(*child_id).expect("child should exist").kind {
                TemplateIrNodeKind::ChildTemplate { reference, .. } => Some(reference.root),
                _ => None,
            },
        )
        .expect("root should contain a ChildTemplate node")
}

#[test]
fn parent_formatter_does_not_leak_into_nested_child_without_formatter() {
    // A `$md` parent must not format the body of a nested child template
    // that has no formatter of its own.
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md: outer [: <b>inner</b> ]]", &mut string_table);
    let store = store.borrow();

    let parent_reference = &template.tir_reference;
    assert_eq!(
        parent_reference.phase,
        TemplateTirPhase::Formatted,
        "parent formatter should claim formatted phase"
    );

    let child_id = first_child_template_id(&template, &store);
    let child_template = store
        .get_template(child_id)
        .expect("nested child template should exist in the module-local store");
    assert!(
        child_template.style.formatter.is_none(),
        "nested child without its own formatter must not inherit the parent formatter"
    );

    let child_root_children = sequence_child_ids(child_template.root, &store);
    let child_body_text = child_root_children
        .iter()
        .map(|child_id| parser_tir_text(*child_id, &store, &string_table))
        .collect::<Vec<_>>()
        .join("");
    assert!(
        child_body_text.contains("<b>inner</b>"),
        "parent markdown formatter must not escape HTML inside the opaque child boundary"
    );
    assert!(
        !child_body_text.contains("&lt;b&gt;"),
        "parent markdown formatter must not escape HTML inside the opaque child boundary"
    );
}

#[test]
fn nested_child_with_own_formatter_is_formatted_independently() {
    // A nested child that redeclares `$md` must be formatted independently
    // through the TIR formatter path, not inherit the parent formatter state.
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md: outer [$md: <b>inner</b>]]", &mut string_table);
    let store = store.borrow();

    let parent_reference = &template.tir_reference;
    assert_eq!(
        parent_reference.phase,
        TemplateTirPhase::Formatted,
        "parent formatter should claim formatted phase"
    );

    let child_id = first_child_template_id(&template, &store);
    let child_template = store
        .get_template(child_id)
        .expect("nested child template should exist in the module-local store");
    assert!(
        child_template.style.formatter.is_some(),
        "nested child with `$md` must carry its own formatter"
    );

    // The child root stores the child's own markdown-formatted output. The
    // parent markdown formatter treats the child as an opaque anchor, so the
    // final folded output must show the child HTML escaped by the child
    // formatter while the parent paragraph wrapper remains unescaped.
    let child_root_children = sequence_child_ids(child_template.root, &store);
    let child_body_text = child_root_children
        .iter()
        .map(|child_id| parser_tir_text(*child_id, &store, &string_table))
        .collect::<Vec<_>>()
        .join("");
    assert!(
        child_body_text.contains("&lt;b&gt;inner&lt;/b&gt;"),
        "child markdown formatter must independently escape HTML-sensitive characters"
    );
    assert!(
        !child_body_text.contains("<b>inner</b>"),
        "child markdown formatter must not leave raw HTML unescaped"
    );
}

#[test]
fn formatted_tir_reference_installs_formatted_output_for_simple_template() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md: body]", &mut string_table);
    let store = store.borrow();
    let tir_reference = &template.tir_reference;

    assert_eq!(
        tir_reference.phase,
        TemplateTirPhase::Formatted,
        "linear render-unit preparation should install formatter output as a formatted TIR reference"
    );
    assert_eq!(
        parser_tir_texts(&template, &store, &string_table),
        vec!["<p> body</p>"],
        "formatted TIR output must reflect the formatted body, not the pre-format body text"
    );
}

#[test]
fn formatted_tir_reference_installs_with_opaque_body_child_template() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md: before [: child] after]", &mut string_table);
    let store = store.borrow();
    let tir_reference = &template.tir_reference;

    assert_eq!(
        tir_reference.phase,
        TemplateTirPhase::Formatted,
        "explicit formatter body children are eligible for formatted TIR installation"
    );

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert_eq!(parent_template.summary.child_template_count, 1);

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(
        children.len(),
        3,
        "formatter output should preserve text around the opaque child anchor"
    );
    assert_eq!(
        parser_tir_text(children[0], &store, &string_table),
        "<p> before "
    );
    assert_eq!(
        parser_tir_text(children[2], &store, &string_table),
        " after</p>"
    );

    let child_template_id = match &store
        .get_node(children[1])
        .expect("middle child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => {
            panic!("expected formatter output to preserve ChildTemplate node, found {other:?}")
        }
    };
    let child_template = store
        .get_template(child_template_id)
        .expect("referenced child parser TIR template should exist");
    assert_eq!(child_template.summary.text_node_count, 1);
}

#[test]
fn formatter_head_chain_composition_keeps_formatted_reference() {
    let mut string_table = StringTable::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let wrapper_name = string_table.intern("wrapper");
    let wrapper_path = wrapper_scope.append(wrapper_name);

    let mut wrapper_tokens =
        template_tokens_from_source("[:<article>[$slot]</article>]", &mut string_table);
    let wrapper_context = new_constant_context(wrapper_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(&shared_store));
    let wrapper = Template::new(
        &mut wrapper_tokens,
        &wrapper_context,
        vec![],
        &mut string_table,
    )
    .expect("head-chain wrapper should parse");

    let declaration = Declaration {
        id: wrapper_path,
        value: Expression::template(wrapper, ValueMode::ImmutableOwned),
    };

    let mut parent_tokens = template_tokens_from_source("[wrapper, $md: body]", &mut string_table);
    let parent_context = constant_template_context(&parent_tokens.src_path, &[declaration])
        .with_template_ir_store(Rc::clone(&shared_store));
    let template = Template::new(
        &mut parent_tokens,
        &parent_context,
        vec![],
        &mut string_table,
    )
    .expect("formatted head-chain template should parse");

    let reference = &template.tir_reference;

    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "formatted head-chain output should install a Formatted TIR reference"
    );

    let folded = fold_template_in_context(&template, &parent_context, &mut string_table);
    assert_eq!(
        string_table.resolve(folded),
        "<article><p> body</p></article>",
        "head-chain composition should consume the formatted body without changing output shape"
    );
}

#[test]
fn positional_default_slot_children_preserve_separator_whitespace() {
    let mut string_table = StringTable::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let wrapper_name = string_table.intern("wrapper");
    let wrapper_path = wrapper_scope.append(wrapper_name);

    let mut wrapper_tokens = template_tokens_from_source(
        "[:\n    [$children([:H: [$slot]]):[$slot(1)]]\n    [$children([:R: [$slot]]):[$slot]]\n]",
        &mut string_table,
    );
    let wrapper_context = new_constant_context(wrapper_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(&shared_store));
    let wrapper = Template::new(
        &mut wrapper_tokens,
        &wrapper_context,
        vec![],
        &mut string_table,
    )
    .expect("slot wrapper should parse");

    let declaration = Declaration {
        id: wrapper_path,
        value: Expression::template(wrapper, ValueMode::ImmutableOwned),
    };

    let mut parent_tokens = template_tokens_from_source(
        "[wrapper:\n    [: First]\n    [: Second]\n    [: Third]\n]",
        &mut string_table,
    );
    let parent_context = constant_template_context(&parent_tokens.src_path, &[declaration])
        .with_template_ir_store(Rc::clone(&shared_store));
    let template = Template::new(
        &mut parent_tokens,
        &parent_context,
        vec![],
        &mut string_table,
    )
    .expect("slot application should parse");

    let folded = fold_template_in_context(&template, &parent_context, &mut string_table);

    assert_eq!(
        string_table.resolve(folded),
        "H:  First\n\nR:  Second\nR:  Third",
        "positional slot output and the first default-slot contribution both preserve their separators"
    );
}

#[test]
fn formatter_children_wrapper_composition_keeps_formatted_reference() {
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template(
        "[$md, $children([:<b>[$slot]</b>]): hello [:child] ]",
        &mut string_table,
    );

    let reference = &template.tir_reference;

    assert!(
        reference.phase.is_at_least(TemplateTirPhase::Composed),
        "direct-child wrapper composition should still advance the derived TIR reference to at least Composed"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "child-wrapper composition should preserve Formatted phase when it consumes formatted output"
    );
    assert_eq!(
        folded_template_output("[$md, $children([:<b>[$slot]</b>]): hello [:child] ]"),
        "<p> hello <b>child</b> </p>",
        "direct-child wrappers should continue to apply only to the child-template anchor"
    );
}

#[test]
fn formatter_named_insert_installs_formatted_reference_and_preserves_routing() {
    let mut string_table = StringTable::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);

    let mut wrapper_tokens = template_tokens_from_source(
        "[$md:title[$slot(\"title\")]body[$slot]]",
        &mut string_table,
    );
    let wrapper_context = new_constant_context(wrapper_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(&shared_store));
    let wrapper = Template::new(
        &mut wrapper_tokens,
        &wrapper_context,
        vec![],
        &mut string_table,
    )
    .expect("formatted named-slot wrapper should parse");

    let wrapper_reference = &wrapper.tir_reference;
    assert_eq!(
        wrapper_reference.phase,
        TemplateTirPhase::Formatted,
        "explicit formatter named-slot receivers can install formatted TIR"
    );

    let mut insert_tokens =
        template_tokens_from_source("[$md, $insert(\"title\"):Heading]", &mut string_table);
    let insert_context = new_constant_context(insert_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(&shared_store));
    let insert = Template::new(
        &mut insert_tokens,
        &insert_context,
        vec![],
        &mut string_table,
    )
    .expect("formatted named insert should parse");

    let insert_reference = &insert.tir_reference;
    assert_eq!(
        insert_reference.phase,
        TemplateTirPhase::Formatted,
        "explicit formatter insert helpers can install formatted TIR"
    );

    let declarations = vec![
        Declaration {
            id: scope.append(string_table.intern("wrapper")),
            value: Expression::template(wrapper, ValueMode::ImmutableOwned),
        },
        Declaration {
            id: scope.append(string_table.intern("heading")),
            value: Expression::template(insert, ValueMode::ImmutableOwned),
        },
    ];

    let mut parent_tokens =
        template_tokens_from_source("[wrapper, heading:Body]", &mut string_table);
    let parent_context = constant_template_context(&parent_tokens.src_path, &declarations)
        .with_template_ir_store(Rc::clone(&shared_store));
    let template = Template::new(
        &mut parent_tokens,
        &parent_context,
        vec![],
        &mut string_table,
    )
    .expect("formatted named-slot application should parse");

    let folded = fold_template_in_context(&template, &parent_context, &mut string_table);
    assert_eq!(
        string_table.resolve(folded),
        "<p>title</p><p>Heading</p><p>body</p>Body",
        "named insert routing should preserve the formatter output"
    );
}

#[test]
fn no_formatter_slot_receiver_does_not_claim_formatted_phase() {
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template("[:before[$slot]after]", &mut string_table);

    let reference = &template.tir_reference;

    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "no-formatter linear templates now reach Formatted through the TIR default-whitespace formatter"
    );
    assert_eq!(
        folded_template_output("[:before[$slot]after]"),
        "beforeafter",
        "missing default slot should continue to render as empty output"
    );
}

#[test]
fn no_formatter_child_wrapper_reaches_formatted_phase_through_tir() {
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template(
        "[$children([:<b>[$slot]</b>]): hello [:child] ]",
        &mut string_table,
    );

    let reference = &template.tir_reference;

    assert!(
        reference.phase.is_at_least(TemplateTirPhase::Formatted),
        "no-formatter child-template roots with TIR-normalized wrappers should reach Formatted through the TIR formatter view"
    );
    assert_eq!(
        folded_template_output("[$children([:<b>[$slot]</b>]): hello [:child] ]"),
        " hello <b>child</b> ",
        "no-formatter direct-child wrappers should preserve existing folded output shape"
    );
}

#[test]
fn formatted_tir_reference_installs_formatted_control_flow_branch_body() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md, if true: body]", &mut string_table);
    let store = store.borrow();

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");

    assert!(
        parent_template.summary.has_control_flow,
        "owner parser TIR must record control-flow presence"
    );

    let branch_chain = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::BranchChain { branches, fallback } => (branches, fallback),
        other => panic!("expected parser TIR BranchChain node, found {other:?}"),
    };

    let (branches, fallback) = branch_chain;
    assert_eq!(branches.len(), 1);
    assert!(fallback.is_none());

    let branch_body_text = body_text(branches[0].body, &store, &string_table);
    assert_eq!(
        branch_body_text, "<p> body</p>",
        "render-unit preparation should install finalized formatter output into the branch body node"
    );
}

#[test]
fn formatted_tir_reference_installs_formatted_branch_and_fallback_bodies() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template(
        "[$md, if false:\nbody\n[else]\nfallback\n]",
        &mut string_table,
    );
    let store = store.borrow();

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert!(parent_template.summary.has_control_flow);

    let branch_chain = match parser_tir_control_flow_root_kind(&template, &store) {
        TemplateIrNodeKind::BranchChain { branches, fallback } => (branches, fallback),
        other => panic!("expected parser TIR BranchChain node, found {other:?}"),
    };

    let (branches, fallback) = branch_chain;
    assert_eq!(branches.len(), 1);
    let fallback_body = fallback
        .as_ref()
        .copied()
        .expect("fallback body should exist");

    assert_eq!(
        body_text(branches[0].body, &store, &string_table),
        "<p>body</p>"
    );
    assert_eq!(
        body_text(fallback_body, &store, &string_table),
        "<p>fallback</p>"
    );

    assert_ne!(
        branches[0].location,
        SourceLocation::default(),
        "prepared branch should retain a concrete parser source location"
    );
    assert_ne!(
        store
            .get_node(branches[0].body)
            .expect("prepared branch body root should exist")
            .location,
        SourceLocation::default(),
        "prepared branch body root should retain a concrete parser source location"
    );
    assert_ne!(
        store
            .get_node(fallback_body)
            .expect("prepared fallback body root should exist")
            .location,
        SourceLocation::default(),
        "prepared fallback body root should retain a concrete parser source location"
    );
}

#[test]
fn formatted_tir_reference_installs_formatted_loop_body() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$md, loop true: body]", &mut string_table);
    let store = store.borrow();

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert!(parent_template.summary.has_control_flow);

    let (header, body, aggregate_wrapper) =
        match parser_tir_control_flow_root_kind(&template, &store) {
            TemplateIrNodeKind::Loop {
                header,
                body,
                aggregate_wrapper,
                ..
            } => (header, body, aggregate_wrapper),
            other => panic!("expected parser TIR Loop node, found {other:?}"),
        };

    assert!(matches!(
        header,
        crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader::Conditional { .. }
    ));
    assert!(
        aggregate_wrapper.is_some(),
        "loop aggregate wrapper should be installed during render-unit preparation"
    );
    assert!(
        tir_subtree_contains_aggregate_output(aggregate_wrapper.unwrap(), &store),
        "installed aggregate wrapper subtree should contain the AggregateOutput marker"
    );

    assert_eq!(body_text(*body, &store, &string_table), "<p> body</p>");
    assert_ne!(
        store
            .get_node(*body)
            .expect("prepared loop body root should exist")
            .location,
        SourceLocation::default(),
        "prepared loop body root should retain a concrete parser source location"
    );
}

#[test]
fn no_formatter_control_flow_owner_reaches_formatted_phase() {
    // No-formatter branch/fallback/loop bodies run through the same TIR formatter
    // adapter used by explicit-formatter bodies (default-whitespace normalization
    // / `$raw` preservation). Their owning TIR references should reach the
    // owning `Formatted` phase while preserving normalized body output.
    let mut string_table = StringTable::new();

    let (branch_template, store) = parse_template("[if true: body]", &mut string_table);
    let store = store.borrow();
    let branch_chain = match parser_tir_control_flow_root_kind(&branch_template, &store) {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches,
        other => panic!("expected parser TIR BranchChain node, found {other:?}"),
    };
    assert_eq!(branch_chain.len(), 1);
    assert_eq!(
        body_text(branch_chain[0].body, &store, &string_table),
        " body",
        "no-formatter branch body should preserve normalized output"
    );
    assert_eq!(
        branch_template.tir_reference.phase,
        TemplateTirPhase::Formatted,
    );

    let (fallback_template, store) =
        parse_template("[if false:\nbranch\n[else]\nfallback\n]", &mut string_table);
    let store = store.borrow();
    let (branches, fallback) = branch_chain_from_root(&fallback_template, &store);
    assert_eq!(branches.len(), 1);
    let fallback_body = fallback.expect("branch chain should have a fallback body");
    assert_eq!(
        body_text(branches[0].body, &store, &string_table),
        "branch",
        "no-formatter branch body should preserve normalized output"
    );
    assert_eq!(
        body_text(fallback_body, &store, &string_table),
        "fallback",
        "no-formatter fallback body should preserve normalized output"
    );
    assert_eq!(
        fallback_template.tir_reference.phase,
        TemplateTirPhase::Formatted,
    );

    let (loop_template, store) = parse_template("[loop true: body]", &mut string_table);
    let store = store.borrow();
    let loop_node = match parser_tir_control_flow_root_kind(&loop_template, &store) {
        TemplateIrNodeKind::Loop { body, .. } => body,
        other => panic!("expected parser TIR Loop node, found {other:?}"),
    };
    assert_eq!(
        body_text(*loop_node, &store, &string_table),
        " body",
        "no-formatter loop body should preserve normalized output"
    );
    assert_eq!(
        loop_template.tir_reference.phase,
        TemplateTirPhase::Formatted,
    );

    let (raw_template, store) = parse_template("[$raw, if true:\n    raw\n]", &mut string_table);
    let store = store.borrow();
    let raw_branch_chain = match parser_tir_control_flow_root_kind(&raw_template, &store) {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches,
        other => panic!("expected parser TIR BranchChain node, found {other:?}"),
    };
    assert_eq!(
        body_text(raw_branch_chain[0].body, &store, &string_table),
        "\n    raw\n",
        "$raw branch body should preserve authored whitespace"
    );
    assert_eq!(
        raw_template.tir_reference.phase,
        TemplateTirPhase::Formatted,
    );
}

// -------------------------
//  No-formatter default whitespace
// -------------------------

#[test]
fn default_whitespace_linear_records_formatted_tir_phase() {
    // Default template-body whitespace normalization is applied by the TIR
    // formatter adapter, so no-formatter linear bodies produce a formatted TIR
    // root.
    let mut string_table = StringTable::new();
    let (template, _store) = parse_template("[:\n    Hello\n    World\n]", &mut string_table);

    let reference = &template.tir_reference;
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Formatted,
        "default-whitespace linear bodies should advance through the formatter adapter"
    );
    assert_eq!(
        folded_template_output("[:\n    Hello\n    World\n]"),
        "Hello\nWorld",
        "default-whitespace formatted TIR root must preserve normalized output"
    );
}

#[test]
fn parser_tir_records_finalized_child_template_as_child_template_node() {
    let mut string_table = StringTable::new();
    let (template, store) =
        parse_const_required_template("[:before[:child]after]", &mut string_table);
    let store = store.borrow();

    let children = tir_root_child_ids(&template, &store);
    assert_eq!(children.len(), 3);
    assert_eq!(
        parser_tir_text(children[0], &store, &string_table),
        "before"
    );
    assert_eq!(parser_tir_text(children[2], &store, &string_table), "after");

    let child_template_id = match &store
        .get_node(children[1])
        .expect("middle child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected ChildTemplate node for finalized child, found {other:?}"),
    };

    let child_template = store
        .get_template(child_template_id)
        .expect("referenced child parser TIR template should exist");
    assert_eq!(child_template.summary.text_node_count, 1);
    assert!(!child_template.summary.has_control_flow);

    let parent_template = store
        .get_template(template.tir_reference.root)
        .expect("parent parser TIR template should exist");
    assert_eq!(parent_template.summary.child_template_count, 1);
    assert_eq!(parent_template.summary.dynamic_expression_count, 0);
    assert_eq!(parent_template.summary.slot_count, 0);
    assert!(!parent_template.summary.has_control_flow);
    assert!(!parent_template.summary.has_reactivity);

    assert_eq!(parent_template.summary.max_depth, 1);
}

#[test]
fn parser_records_template_valued_head_as_structural_child_before_body_parse() {
    let mut string_table = StringTable::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let wrapper_name = string_table.intern("wrapper");
    let wrapper_path = wrapper_scope.append(wrapper_name);

    let mut wrapper_tokens = template_tokens_from_source("[:head]", &mut string_table);
    let wrapper_context = new_constant_context(wrapper_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(&shared_store));
    let wrapper = Template::new(
        &mut wrapper_tokens,
        &wrapper_context,
        vec![],
        &mut string_table,
    )
    .expect("wrapper template should parse");

    let declaration = Declaration {
        id: wrapper_path,
        value: Expression::template(wrapper.clone(), ValueMode::ImmutableOwned),
    };
    let mut parent_tokens = template_tokens_from_source("[wrapper: body]", &mut string_table);
    let parent_context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        parent_tokens.src_path.to_owned(),
        Rc::new(TopLevelDeclarationTable::new(vec![declaration])),
        Arc::new(ExternalPackageRegistry::default()),
        vec![],
        0,
    )
    .with_template_ir_store(Rc::clone(&shared_store));
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut build_state = TemplateBuildState::new();
    let mut construction_context = TemplateConstructionContext::new(
        Rc::clone(&shared_store),
        parent_tokens.current_location(),
    );

    let _parsed_head = parse_template_head(
        &mut parent_tokens,
        TemplateHeadParseRequest {
            context: &parent_context,
            type_interner: &mut type_interner,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
        },
    )
    .expect("template-valued head should parse");

    let root_children = construction_context.root_children();
    assert_eq!(root_children.len(), 1);
    let store = shared_store.borrow();
    let child_node = store
        .get_node(root_children[0])
        .expect("parser head child should exist");
    match &child_node.kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            assert_eq!(reference.root, wrapper.tir_reference.root);
            assert_eq!(reference.phase, wrapper.tir_reference.phase);
            assert_eq!(reference.context, wrapper.tir_reference.context);
        }
        other => panic!(
            "template-valued head must be structural before render-unit preparation, found {other:?}"
        ),
    }
}

#[test]
fn parser_tir_records_template_valued_head_reference_as_child_template() {
    let mut string_table = StringTable::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let wrapper_name = string_table.intern("wrapper");
    let wrapper_path = wrapper_scope.append(wrapper_name);

    let mut wrapper_tokens = template_tokens_from_source("[:head]", &mut string_table);
    let wrapper_context = new_constant_context(wrapper_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(&shared_store));
    let wrapper = Template::new(
        &mut wrapper_tokens,
        &wrapper_context,
        vec![],
        &mut string_table,
    )
    .expect("wrapper template should parse");

    let declaration = Declaration {
        id: wrapper_path,
        value: Expression::template(wrapper, ValueMode::ImmutableOwned),
    };

    let mut parent_tokens = template_tokens_from_source("[wrapper: body]", &mut string_table);
    let parent_context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        parent_tokens.src_path.to_owned(),
        Rc::new(TopLevelDeclarationTable::new(vec![declaration])),
        Arc::new(ExternalPackageRegistry::default()),
        vec![],
        0,
    )
    .with_template_ir_store(Rc::clone(&shared_store));
    let parent = Template::new(
        &mut parent_tokens,
        &parent_context,
        vec![],
        &mut string_table,
    )
    .expect("parent template should parse");
    let store = shared_store.borrow();
    let parent_child_ids = tir_root_child_ids(&parent, &store);
    assert_eq!(parent_child_ids.len(), 2);

    let child_template_id = match &store
        .get_node(parent_child_ids[0])
        .expect("head child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected ChildTemplate head node, found {other:?}"),
    };

    let child_template = store
        .get_template(child_template_id)
        .expect("referenced child parser TIR template should exist");
    assert_eq!(child_template.summary.text_node_count, 1);

    let child_root_children = sequence_child_ids(child_template.root, &store);
    assert_eq!(child_root_children.len(), 1);
    assert_eq!(
        parser_tir_text(child_root_children[0], &store, &string_table),
        "head"
    );

    assert_eq!(
        parser_tir_text(parent_child_ids[1], &store, &string_table),
        " body"
    );
}

#[test]
fn parser_tir_skips_conditional_child_wrappers_for_fresh_control_flow_child() {
    let mut string_table = StringTable::new();
    let (parent, store) = parse_template(
        "[$children([:wrap]): [$fresh, if true: body]]",
        &mut string_table,
    );
    let store = store.borrow();

    let parent_child_ids = tir_root_child_ids(&parent, &store);
    // The body starts with a leading space before the control-flow child template.
    assert_eq!(parent_child_ids.len(), 2);

    let child_template_id = match &store
        .get_node(parent_child_ids[1])
        .expect("parent child node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => reference.root,
        other => panic!("expected ChildTemplate node, found {other:?}"),
    };

    let child_template = store
        .get_template(child_template_id)
        .expect("control-flow child parser TIR template should exist");
    assert!(
        child_template.conditional_child_wrapper_set.is_none(),
        "$fresh control-flow child should have no conditional wrapper set"
    );
    assert_eq!(
        child_template.summary.wrapper_count, 0,
        "$fresh control-flow child should have zero wrapper count"
    );
}

#[test]
fn doc_comment_with_formatter_records_comment_kind() {
    // `$doc` applies markdown formatting automatically while retaining the
    // comment directive kind on the module-local TIR root.
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[$doc: doc body]", &mut string_table);
    let store = store.borrow();
    let template_kind = store
        .get_template(template.tir_reference.root)
        .expect("TIR entry should exist")
        .kind
        .clone();

    assert_eq!(
        template_kind,
        TemplateType::Comment(CommentDirectiveKind::Doc),
        "$doc should produce a doc-comment template kind"
    );
}

// -------------------------
//  Direct control-flow root after render-unit refresh
// -------------------------

#[test]
fn no_prefix_if_finalizes_with_direct_branch_chain_root() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[if true: body]", &mut string_table);
    let store = store.borrow();

    // After render-unit preparation refreshes every branch body, the owner
    // root is the BranchChain directly — not a Sequence wrapping it.
    assert!(
        matches!(
            parser_tir_root_kind(&template, &store),
            TemplateIrNodeKind::BranchChain { .. }
        ),
        "no-prefix if should finalize with a direct BranchChain root"
    );
}

#[test]
fn no_prefix_loop_finalizes_with_direct_loop_root() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[loop true: body]", &mut string_table);
    let store = store.borrow();

    // After render-unit preparation refreshes the loop body, the owner root is
    // the Loop directly — not a Sequence wrapping it.
    assert!(
        matches!(
            parser_tir_root_kind(&template, &store),
            TemplateIrNodeKind::Loop { .. }
        ),
        "no-prefix loop should finalize with a direct Loop root"
    );
}

#[test]
fn linear_template_preserves_sequence_root_shape() {
    let mut string_table = StringTable::new();
    let (template, store) = parse_template("[: before [: child] after]", &mut string_table);
    let store = store.borrow();

    // Linear templates (no control flow) always finalize with a Sequence root,
    // unaffected by the direct-control-flow-root optimization.
    assert!(
        matches!(
            parser_tir_root_kind(&template, &store),
            TemplateIrNodeKind::Sequence { .. }
        ),
        "linear template should finalize with a Sequence root"
    );
}
