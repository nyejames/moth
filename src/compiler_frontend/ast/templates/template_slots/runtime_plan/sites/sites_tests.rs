//! Focused unit tests for runtime slot-site node authority.
//!
//! WHAT: protects `slot_key_for_node` and `build_tir_wrapper_render_pieces`
//!       invariants that integration output cannot inspect: missing nodes are
//!       internal errors, present non-slots are optional, same-store child
//!       templates must exist before recursion, and wrapper fill must reach
//!       slots nested in branch and loop roots.
//! WHY: authority failures and the control-flow injection hole are TIR-local
//!      facts, not user-facing output.

use super::{build_tir_wrapper_render_pieces, slot_key_for_node};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateSegmentOrigin;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId;
use crate::compiler_frontend::ast::templates::template_slots::TemplateSlotError;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBranch, TemplateIrBuilder, TemplateIrId, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateIrStore, TemplateIrSummary, TemplateSlotPlanId, TemplateSlotSiteRenderPiece,
    TemplateSlotSiteRenderPlan, TemplateTirPhase, TemplateViewContext, TirCopyState,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn assert_authority_error(
    result: Result<Vec<TemplateSlotSiteRenderPiece>, TemplateError>,
    context: &str,
    owner_marker: &str,
) {
    let error =
        result.expect_err(format!("{context} must surface as a broken-authority error").as_str());
    match error {
        TemplateError::Infrastructure(error) => {
            assert!(
                error.msg.contains(owner_marker),
                "{context} should be rejected by the {owner_marker} owner, got: {}",
                error.msg,
            );
        }
        TemplateError::Diagnostic(_) => {
            panic!("{context} must be an infrastructure error, not a user diagnostic",);
        }
    }
}

#[test]
fn missing_node_is_an_authority_error() {
    let store = TemplateIrStore::new();
    let missing_node = TemplateIrNodeId::new(7);

    let result = slot_key_for_node(&store, missing_node);

    assert!(
        result.is_err(),
        "a missing node must be an internal error, not an implicit non-slot classification"
    );
}

#[test]
fn present_non_slot_node_is_optional() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let text_id = string_table.intern("body");
    let byte_len = u32::try_from(string_table.resolve(text_id).len()).unwrap_or(u32::MAX);
    let text_node = builder.push_text_node(
        text_id,
        byte_len,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );

    let result = slot_key_for_node(&store, text_node);

    assert_eq!(result.expect("present non-slot node should classify"), None);
}

#[test]
fn present_slot_node_carries_its_key() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());

    let result = slot_key_for_node(&store, slot_node);

    assert_eq!(
        result.expect("present slot node should classify"),
        Some(SlotKey::Default)
    );
}

#[test]
fn missing_same_store_child_template_is_an_authority_error() {
    let mut store = TemplateIrStore::new();
    let same_store_missing_template = TemplateTirChildReference::new(
        TemplateIrId::new(99),
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let mut builder = TemplateIrBuilder::new(&mut store);
    let child_node = builder
        .push_child_template_node_with_reference(same_store_missing_template, empty_location());

    let mut copy_state = TirCopyState::new();
    let inner_plan = TemplateSlotSiteRenderPlan::default();

    let result = build_tir_wrapper_render_pieces(
        child_node,
        &inner_plan,
        SlotKey::Default,
        &mut store,
        &mut copy_state,
    );

    assert_authority_error(
        result,
        "missing same-store child template",
        "Runtime slot site planning",
    );
}

#[test]
fn missing_structural_authority_propagates_through_runtime_slot_site_planner() {
    let mut store = TemplateIrStore::new();
    let mut copy_state = TirCopyState::new();
    let inner_plan = TemplateSlotSiteRenderPlan::default();
    let mut planner = super::RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: TemplateSlotPlanId::new(0),
        store: &mut store,
        copy_state: &mut copy_state,
    };

    let result = planner.try_build_child_wrapper_site_pieces_from_tir_id(
        TemplateIrId::new(0),
        TemplateIrNodeId::new(7),
        &inner_plan,
    );
    let error = result.expect_err(
        "runtime slot planning must propagate missing structural authority as an error",
    );

    match error {
        TemplateSlotError::Infrastructure(error) => {
            assert_eq!(error.error_type, ErrorType::Compiler);
        }
        TemplateSlotError::Diagnostic(_) => {
            panic!("missing structural authority must not become a user diagnostic");
        }
    }
}

/// Builds a before/slot/after wrapper template with a default slot and
/// caller-supplied marker text so distinct wrappers can be told apart.
fn build_slot_text_wrapper(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    before: &str,
    after: &str,
) -> TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let before_node = builder.push_text_node(
        string_table.intern(before),
        before.len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());
    let after_node = builder.push_text_node(
        string_table.intern(after),
        after.len() as u32,
        TemplateSegmentOrigin::Body,
        empty_location(),
    );
    let root =
        builder.push_sequence_node(vec![before_node, slot_node, after_node], empty_location());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        empty_location(),
    )
}

/// Asserts that a render piece is a copied text node carrying `expected`.
fn assert_site_text_piece(
    piece: &TemplateSlotSiteRenderPiece,
    store: &TemplateIrStore,
    string_table: &StringTable,
    expected: &str,
) {
    let TemplateSlotSiteRenderPiece::Render(node_id) = piece else {
        panic!("expected Render piece, got {piece:?}");
    };
    let node = store
        .get_node(*node_id)
        .expect("render piece node should exist in the store");
    match &node.kind {
        TemplateIrNodeKind::Text { text, .. } => {
            assert_eq!(string_table.resolve(*text), expected);
        }
        other => panic!("expected Text node, got {other:?}"),
    }
}

#[test]
fn wrap_site_plan_applies_wrapper_set_innermost_to_outermost() {
    // `wrap_site_plan_with_tir_child_wrappers` must consume the
    // innermost-to-outermost wrapper-set order forward, so a single two-wrapper
    // set wraps the contribution as outer(inner(contribution)). The render
    // pieces must read outer-before, inner-before, contribution, inner-after,
    // outer-after; reverse consumption would swap the inner/outer markers.
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();

    let inner_wrapper =
        build_slot_text_wrapper(&mut store, &mut string_table, "inner-before", "inner-after");
    let outer_wrapper =
        build_slot_text_wrapper(&mut store, &mut string_table, "outer-before", "outer-after");

    let inner_ref = TemplateWrapperReference::new(
        inner_wrapper,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    );
    let outer_ref = TemplateWrapperReference::new(
        outer_wrapper,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    );
    // Wrapper sets are stored innermost-to-outermost; forward site-plan
    // consumption must yield outer(inner(contribution)).
    let wrapper_set_id = store.push_or_reuse_wrapper_set(vec![inner_ref, outer_ref]);

    let contribution_source_id = RuntimeSlotContributionSourceId(0);
    let source_plan = TemplateSlotSiteRenderPlan {
        pieces: vec![TemplateSlotSiteRenderPiece::ContributionSource(
            contribution_source_id,
        )],
    };

    let mut copy_state = TirCopyState::new();
    let mut planner = super::RuntimeWrapperSitePlanBuilder {
        sources: &[],
        slot_plan_id: TemplateSlotPlanId::new(0),
        store: &mut store,
        copy_state: &mut copy_state,
    };

    let wrapped = planner
        .wrap_site_plan_with_tir_child_wrappers(source_plan, wrapper_set_id)
        .expect("site wrapper application should succeed");

    assert_eq!(wrapped.pieces.len(), 5);
    assert_site_text_piece(&wrapped.pieces[0], &store, &string_table, "outer-before");
    assert_site_text_piece(&wrapped.pieces[1], &store, &string_table, "inner-before");
    match &wrapped.pieces[2] {
        TemplateSlotSiteRenderPiece::ContributionSource(source_id) => {
            assert_eq!(
                *source_id, contribution_source_id,
                "middle piece should be the original contribution source"
            );
        }
        other => panic!("expected ContributionSource piece, got {other:?}"),
    }
    assert_site_text_piece(&wrapped.pieces[3], &store, &string_table, "inner-after");
    assert_site_text_piece(&wrapped.pieces[4], &store, &string_table, "outer-after");
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, empty_location(), ValueMode::ImmutableOwned)
}

fn contribution_plan() -> TemplateSlotSiteRenderPlan {
    TemplateSlotSiteRenderPlan {
        pieces: vec![TemplateSlotSiteRenderPiece::ContributionSource(
            RuntimeSlotContributionSourceId(0),
        )],
    }
}

fn pieces_include_contribution(pieces: &[TemplateSlotSiteRenderPiece]) -> bool {
    pieces.iter().any(|piece| {
        matches!(
            piece,
            TemplateSlotSiteRenderPiece::ContributionSource(RuntimeSlotContributionSourceId(0))
        )
    })
}

/// A wrapper whose root is a branch containing the target slot must inject fill.
#[ignore = "Phase 0 reproduced: branch-root wrappers copy the slot unresolved; un-ignore in Phase 2E"]
#[test]
fn wrapper_render_pieces_inject_slot_inside_branch() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());
    let branch_root = builder.push_branch_chain_node(
        vec![TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            slot_node,
            empty_location(),
        )],
        None,
        empty_location(),
    );

    let mut copy_state = TirCopyState::new();
    let inner_plan = contribution_plan();
    let pieces = build_tir_wrapper_render_pieces(
        branch_root,
        &inner_plan,
        SlotKey::Default,
        &mut store,
        &mut copy_state,
    )
    .expect("branch-contained wrapper slot should be injectable");

    assert!(
        pieces_include_contribution(&pieces),
        "runtime wrapper fill must reach a slot inside a branch, got {pieces:?}"
    );
}

/// A wrapper whose root is a loop containing the target slot must inject fill.
#[ignore = "Phase 0 reproduced: loop-root wrappers copy the slot unresolved; un-ignore in Phase 2E"]
#[test]
fn wrapper_render_pieces_inject_slot_inside_loop() {
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let slot_node = builder.push_slot_node(SlotKey::Default, empty_location());
    let loop_root = builder.push_loop_node(
        TemplateLoopHeader::Conditional {
            condition: Box::new(bool_expression(true)),
        },
        slot_node,
        None,
        empty_location(),
    );

    let mut copy_state = TirCopyState::new();
    let inner_plan = contribution_plan();
    let pieces = build_tir_wrapper_render_pieces(
        loop_root,
        &inner_plan,
        SlotKey::Default,
        &mut store,
        &mut copy_state,
    )
    .expect("loop-contained wrapper slot should be injectable");

    assert!(
        pieces_include_contribution(&pieces),
        "runtime wrapper fill must reach a slot inside a loop, got {pieces:?}"
    );
}
