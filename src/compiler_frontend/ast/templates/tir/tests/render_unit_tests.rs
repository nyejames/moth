//! TIR render-unit construction tests.
//
// WHAT: exercises same-store wrapper-reference normalization, aggregate-wrapper
// candidate construction, and required render-unit node authority.
// WHY: these focused tests protect the current module-local TIR owner and
// render-unit identity invariants.

use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::ConstValueStoreError;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlayId,
};
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference,
};
use crate::compiler_frontend::ast::templates::tir::render_unit::{
    build_aggregate_wrapper_candidate_root_from_tir_nodes,
    build_conditional_body_candidate_root_from_tir_nodes, tir_formatter_messages_to_template_error,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::wrapper_sets::wrapper_reference_for_template;
use crate::compiler_frontend::ast::templates::tir::{head_prefix_tir_nodes, sequence_children};
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

fn push_text_node(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrNodeId {
    let text_id = string_table.intern(text);
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: text_id,
            byte_len: text.len(),
            origin: TemplateSegmentOrigin::Body,
        },
        None,
    ))
}

fn push_template_entry(
    store: &mut TemplateIrStore,
    root: TemplateIrNodeId,
    kind: TemplateType,
) -> TemplateIrId {
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        kind,
        TemplateIrSummary::default(),
        None,
    ))
}

fn candidate_root_children(
    store: &TemplateIrStore,
    root_node: TemplateIrNodeId,
) -> Vec<TemplateIrNodeId> {
    let TemplateIrNodeKind::Sequence { children } = &store
        .get_node(root_node)
        .expect("candidate root should exist")
        .kind
    else {
        panic!("candidate root should be a sequence");
    };

    children.to_owned()
}

#[test]
fn same_store_wrapper_reference_is_normalized_without_materialization() {
    let mut store = TemplateIrStore::new();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        None,
    ));
    let template_id = push_template_entry(&mut store, root, TemplateType::String);
    let context = TemplateViewContext::default();

    let template = crate::compiler_frontend::ast::templates::template::Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
        span: None,
    };

    let reference = wrapper_reference_for_template(&template, &store)
        .expect("same-store wrapper should normalize");

    assert_eq!(reference.root, template_id);
    assert_eq!(reference.phase, TemplateTirPhase::Parsed);
    assert_eq!(reference.context, context);
}

#[test]
fn wrapper_reference_rejects_missing_view_context_and_missing_template() {
    // A reference whose expression overlay does not exist in the store is rejected.
    let mut store = TemplateIrStore::new();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        None,
    ));
    let template_id = push_template_entry(&mut store, root, TemplateType::String);
    let missing_context_template = crate::compiler_frontend::ast::templates::template::Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context: TemplateViewContext {
                expression_overlay: Some(TirExpressionOverlayId::new(999)),
                ..TemplateViewContext::default()
            },
        },
        span: None,
    };
    let missing_context_error = wrapper_reference_for_template(&missing_context_template, &store)
        .expect_err("missing view context should be rejected");
    assert!(
        missing_context_error.msg.contains("expression overlay")
            && missing_context_error.msg.contains("does not exist"),
        "expected a missing-view-context error, got: {}",
        missing_context_error.msg
    );

    // A reference whose template root does not exist in the store is rejected.
    let empty_store = TemplateIrStore::new();
    let missing_template = crate::compiler_frontend::ast::templates::template::Template {
        tir_reference: TemplateTirReference {
            root: TemplateIrId::new(99),
            phase: TemplateTirPhase::Parsed,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let missing_template_error = wrapper_reference_for_template(&missing_template, &empty_store)
        .expect_err("missing template should be rejected");
    assert!(
        missing_template_error.msg.contains("template")
            && missing_template_error.msg.contains("was missing"),
        "expected a missing-template error, got: {}",
        missing_template_error.msg
    );
}

#[test]
fn wrapper_candidates_reuse_parser_structural_child_template() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let context = TemplateViewContext::default();

    let child_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        None,
    ));
    let child_template_id = push_template_entry(&mut store, child_root, TemplateType::String);
    let parser_reference =
        TemplateTirChildReference::new(child_template_id, TemplateTirPhase::Finalized, context);
    let parser_occurrence_id = store.next_child_template_occurrence_id();
    let parser_child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: parser_reference,
            occurrence_id: parser_occurrence_id,
        },
        None,
    ));
    let body_node = push_text_node(&mut store, &mut string_table, "body");

    let aggregate_root =
        build_aggregate_wrapper_candidate_root_from_tir_nodes(&[parser_child_node], &mut store)
            .expect("aggregate candidate should reuse parser structural child");
    let aggregate_children = candidate_root_children(&store, aggregate_root);
    assert_eq!(aggregate_children[0], parser_child_node);
    assert!(matches!(
        store.get_node(aggregate_children[0]).expect("aggregate child should exist").kind,
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } if reference == parser_reference && occurrence_id == parser_occurrence_id
    ));
    assert!(matches!(
        store
            .get_node(
                *aggregate_children
                    .last()
                    .expect("aggregate marker should exist")
            )
            .expect("aggregate marker should exist")
            .kind,
        TemplateIrNodeKind::AggregateOutput
    ));

    let conditional_root = build_conditional_body_candidate_root_from_tir_nodes(
        &[parser_child_node],
        &[body_node],
        &mut store,
    )
    .expect("conditional body candidate should reuse parser structural child");
    let conditional_children = candidate_root_children(&store, conditional_root);
    assert_eq!(conditional_children, vec![parser_child_node, body_node]);
}

#[test]
fn sequence_children_rejects_missing_and_non_sequence_roots() {
    let store = TemplateIrStore::new();
    let missing_error = sequence_children(&store, TemplateIrNodeId::new(99))
        .expect_err("missing node should be rejected");
    assert!(
        missing_error.msg.contains("sequence-children lookup")
            && missing_error.msg.contains("was missing"),
        "expected a missing-node error, got: {}",
        missing_error.msg
    );

    let mut string_table = StringTable::new();
    let mut non_sequence_store = TemplateIrStore::new();
    let text_node = push_text_node(&mut non_sequence_store, &mut string_table, "leaf");
    let non_sequence_error = sequence_children(&non_sequence_store, text_node)
        .expect_err("non-sequence root should be rejected");
    assert!(
        non_sequence_error.msg.contains("was not a Sequence root."),
        "expected a non-sequence-root error, got: {}",
        non_sequence_error.msg
    );
}

#[test]
fn head_prefix_rejects_missing_child_and_accepts_empty_prefix() {
    let store = TemplateIrStore::new();
    let missing_error = head_prefix_tir_nodes(&store, &[TemplateIrNodeId::new(99)])
        .expect_err("missing root child should be rejected");
    assert!(
        missing_error.msg.contains("head-prefix extraction")
            && missing_error.msg.contains("was missing"),
        "expected a missing-child error, got: {}",
        missing_error.msg
    );

    assert!(
        head_prefix_tir_nodes(&store, &[])
            .expect("empty prefix should succeed")
            .is_empty(),
        "empty prefix should produce no head-prefix nodes"
    );
}

#[test]
fn formatter_infrastructure_aggregate_survives_ast_error_adapters() {
    let mut primary = CompilerError::compiler_error("formatter infrastructure cause");
    primary.new_metadata_entry(
        CompilerErrorMetadataKey::CompilationStage,
        "formatter".to_owned(),
    );
    let mut secondary = CompilerError::file_error(
        Path::new("formatter-input.moth"),
        "formatter secondary cause",
    );
    secondary.new_metadata_entry(
        CompilerErrorMetadataKey::CompilationStage,
        "read input".to_owned(),
    );
    let expected = [primary.clone(), secondary.clone()];
    let mut messages = CompilerMessages::from_error(primary, StringTable::new());
    messages.set_infrastructure_error(secondary);

    // Template folding and stored values re-enter expression construction through these adapters.
    let template_error = tir_formatter_messages_to_template_error(messages);
    let fold_error = ConstantFoldError::from(template_error);
    let typing_error = ExpressionTypingError::from(fold_error);
    let expression_error = ExpressionParseError::from(typing_error);
    let template_error = TemplateError::from(expression_error);
    let store_error = ConstValueStoreError::from(template_error);
    let construction_error = ExpressionConstructionFailure::from(store_error);
    let expression_error = ExpressionParseError::from(construction_error);
    let errors = expression_error
        .at_boundary_without_numeric_retention("the formatter adapter test")
        .expect_err("both formatter causes must remain infrastructure failures");

    let actual = errors.iter().collect::<Vec<_>>();
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_eq!(actual.msg, expected.msg);
        assert_eq!(actual.error_type, expected.error_type);
        assert_eq!(actual.host_path, expected.host_path);
        assert_eq!(actual.metadata, expected.metadata);
    }
}
