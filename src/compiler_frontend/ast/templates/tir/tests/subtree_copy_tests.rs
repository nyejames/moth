//! Copied TIR subtree identity tests.
//!
//! Protects the invariant that an independent subtree copy remaps every
//! expression-bearing site: dynamic expressions, branch selectors and loop
//! headers. Overlay authority must not alias the source tree.

use super::super::builder::TemplateIrBuilder;
use super::super::copy_state::TirCopyState;
use super::super::ids::TemplateIrNodeId;
use super::super::node::{TemplateIrNodeKind, TemplateLoopHeaderExpressionSites};
use super::super::store::TemplateIrStore;
use super::super::subtree_copy::copy_tir_subtree_with_active_slot_plan;
use super::super::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrBranch;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, empty_location(), ValueMode::ImmutableOwned)
}

fn text_node(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrNodeId {
    let text_id = string_table.intern(text);
    let byte_len = u32::try_from(string_table.resolve(text_id).len()).unwrap_or(u32::MAX);
    let mut builder = TemplateIrBuilder::new(store);
    builder.push_text_node(
        text_id,
        byte_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    )
}

#[ignore = "Phase 0 reproduced: copied branch/loop sites reuse source ExpressionSiteIds; un-ignore in Phase 2B"]
#[test]
fn copied_branch_and_loop_expression_sites_are_independent() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let branch_body = text_node(&mut store, &mut string_table, "branch");
    let loop_body = text_node(&mut store, &mut string_table, "loop");
    let mut builder = TemplateIrBuilder::new(&mut store);
    let branch_root = builder.push_branch_chain_node(
        vec![TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            branch_body,
            empty_location(),
        )],
        None,
        empty_location(),
    );
    let loop_root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        loop_body,
        None,
        empty_location(),
    );
    let source_root = builder.push_sequence_node(vec![branch_root, loop_root], empty_location());
    let _template = builder.finish_template(
        source_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        empty_location(),
    );

    let source_selector = match &store
        .get_node(branch_root)
        .expect("source branch should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected BranchChain, found {other:?}"),
    };
    let source_header_sites = match &store
        .get_node(loop_root)
        .expect("source loop should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { header_sites, .. } => *header_sites,
        other => panic!("expected Loop, found {other:?}"),
    };

    let mut copy_state = TirCopyState::new();
    let copied_root =
        copy_tir_subtree_with_active_slot_plan(source_root, None, &mut store, &mut copy_state)
            .expect("independent subtree copy should succeed");

    let copied_children = match &store
        .get_node(copied_root)
        .expect("copied sequence should exist")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expected copied Sequence, found {other:?}"),
    };
    assert_eq!(copied_children.len(), 2);

    let copied_selector = match &store
        .get_node(copied_children[0])
        .expect("copied branch should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected copied BranchChain, found {other:?}"),
    };
    let copied_header_sites = match &store
        .get_node(copied_children[1])
        .expect("copied loop should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { header_sites, .. } => *header_sites,
        other => panic!("expected copied Loop, found {other:?}"),
    };

    assert_ne!(
        copied_selector, source_selector,
        "copied branch selector site must not alias the source site"
    );
    assert_ne!(
        copied_header_sites, source_header_sites,
        "copied loop header sites must not alias the source sites"
    );
    match (source_header_sites, copied_header_sites) {
        (
            TemplateLoopHeaderExpressionSites::Conditional {
                condition: source_condition,
            },
            TemplateLoopHeaderExpressionSites::Conditional {
                condition: copied_condition,
            },
        ) => {
            assert_ne!(
                copied_condition, source_condition,
                "copied loop condition site must not alias the source site"
            );
        }
        other => panic!("expected conditional loop header sites, found {other:?}"),
    }
}
