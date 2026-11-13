//! Focused coverage for reducer output accounting, ceilings and iteration charges.
//!
//! These tests build fold output states directly so they can observe the shared meter without
//! constructing a complete TIR template.

use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstExecutionMeter;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateOutputMeter, TemplateWorkBudget,
};
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn template_iterations_consume_the_shared_work_meter() {
    let meter = Rc::new(RefCell::new(ConstExecutionMeter::default()));
    {
        let mut meter = meter.borrow_mut();
        meter.steps = 9;
        meter.limits.max_attempt_steps = 10;
        meter.limits.max_total_steps = 10;
    }
    let context = TirFoldContext {
        string_table: &mut StringTable::new(),
        expression_builder: &mut AstExpressionBuilder::new().expect("builder"),
        const_values: &mut ConstValueStore::default(),
        template_const_loop_iteration_limit: 10,
        template_output_limits: None,
        template_work: Some(TemplateWorkBudget {
            meter: Rc::clone(&meter),
            attempt_steps: Rc::new(Cell::new(9)),
            span: None,
        }),
        bindings: vec![],
        source_scope: None,
        external_package_registry: None,
        prepared_wrapper_slot_routing: FxHashMap::default(),
        required_call_rewriter: None,
    };
    charge_template_iteration(&context).expect("the final step is accepted");
    match charge_template_iteration(&context) {
        Err(TemplateError::Budget(exhaustion)) => {
            assert_eq!(exhaustion.category, ConstBudgetCategory::Work);
            assert_eq!(exhaustion.setting, Some("const_work_limit"));
            assert_eq!(exhaustion.limit, 10);
        }
        other => panic!("expected a typed work refusal, got {other:?}"),
    }
}

/// Output ceilings for a reducer test; one ceiling cap names both byte fields and both
/// piece fields so a row states only the limits it exercises.
fn limits(max_bytes: usize, max_structural_pieces: usize) -> TemplateOutputLimits {
    TemplateOutputLimits {
        max_bytes,
        byte_limit: max_bytes,
        byte_aggregate: false,
        max_structural_pieces,
        piece_limit: max_structural_pieces,
        piece_aggregate: false,
        span: None,
        meter: Rc::new(TemplateOutputMeter::default()),
    }
}

fn output_state_from(limits: TemplateOutputLimits) -> FoldOutputState {
    FoldOutputState::from_parts(
        String::new(),
        SyntheticInterfaceProvenance::empty(),
        Some(limits),
        false,
    )
}

fn projected_output_state_from(limits: TemplateOutputLimits) -> FoldOutputState {
    FoldOutputState::from_parts(
        String::new(),
        SyntheticInterfaceProvenance::empty(),
        Some(limits),
        true,
    )
}

fn output_state(max_bytes: usize, max_structural_pieces: usize) -> FoldOutputState {
    output_state_from(limits(max_bytes, max_structural_pieces))
}

fn projected_output_state(max_bytes: usize, max_structural_pieces: usize) -> FoldOutputState {
    projected_output_state_from(limits(max_bytes, max_structural_pieces))
}

fn retained_bytes(state: &FoldOutputState) -> usize {
    state
        .limits
        .as_ref()
        .expect("bounded test output state")
        .meter
        .retained_bytes
        .get()
}

fn retained_pieces(state: &FoldOutputState) -> usize {
    state
        .limits
        .as_ref()
        .expect("bounded test output state")
        .meter
        .retained_pieces
        .get()
}

#[test]
fn bounded_output_rejects_growth_before_appending_past_the_limit() {
    let mut state = output_state(4, 1);
    state
        .append_text("abc")
        .expect("text inside the limit is accepted");
    assert_eq!(state.output_buffer, "abc");
    assert!(
        matches!(
            state.append_text("de"),
            Err(TemplateError::Budget(ConstBudgetExhaustion {
                category: ConstBudgetCategory::Memory,
                limit: 4,
                ..
            }))
        ),
        "crossing the byte ceiling must fail with a typed memory refusal"
    );
    assert_eq!(state.output_buffer, "abc");
}

#[test]
fn structural_piece_refusal_names_pieces() {
    let mut state = output_state(6, 0);

    let exhaustion = match state.charge_piece() {
        Err(TemplateError::Budget(exhaustion)) => exhaustion,
        other => panic!("expected a typed piece refusal, got {other:?}"),
    };
    assert_eq!(exhaustion.category, ConstBudgetCategory::Memory);
    assert_eq!(exhaustion.unit, ConstBudgetUnit::Pieces);
    assert_eq!(exhaustion.limit, 0);
    assert_eq!(exhaustion.setting, None);
    assert!(!exhaustion.aggregate);
}

#[test]
fn bounded_output_charges_projection_text_and_structural_pieces() {
    let mut state = projected_output_state(6, 1);
    state
        .append_text("abc")
        .expect("the output and projection copies fit the ceiling");
    assert!(
        state.append_text("d").is_err(),
        "the projection copy must consume the same byte ceiling"
    );
}

#[test]
fn originated_slots_charge_pieces_but_received_slots_do_not() {
    let occurrence = SlotOccurrenceId::new(0);

    let mut origin = projected_output_state(16, 1);
    origin
        .append_slot(occurrence)
        .expect("the first slot fits the piece ceiling");
    assert_eq!(retained_pieces(&origin), 1);
    assert!(
        origin.append_slot(occurrence).is_err(),
        "an originated slot must pay the piece ceiling"
    );

    let mut receiver = projected_output_state(16, 1);
    receiver
        .append_pieces(Some(&[FoldedConstTemplatePiece::Slot(occurrence)]))
        .expect("a received slot is already charged by its producer");
    assert_eq!(retained_pieces(&receiver), 0);
    assert_eq!(
        receiver
            .projection_pieces
            .expect("projection enabled")
            .len(),
        1
    );
}

#[test]
fn loop_iterations_share_one_output_ceiling() {
    let shared = limits(5, 8);

    let mut first = output_state_from(shared.clone());
    first
        .append_text("abc")
        .expect("the first iteration fits the ceiling");

    let mut second = output_state_from(shared);
    assert!(
        second.append_text("def").is_err(),
        "a fresh iteration state must still draw from the fold's used allowance"
    );
}

#[test]
fn nested_folds_charge_each_retained_copy_once() {
    // All three levels share one fold meter, as they do inside a bounded fold.
    let shared = limits(6, 8);

    // One leaf writes "abc" with projection enabled, so it retains two copies of three bytes:
    // the output run and the projection text. Its ceiling is exact, which also proves the leaf
    // charge is six and not three.
    let mut leaf = projected_output_state_from(shared.clone());
    leaf.append_text("abc").expect("the leaf's two copies fit");
    assert_eq!(retained_bytes(&leaf), 6);

    let leaf_projection = leaf.projection_pieces.take();
    let mut string_table = StringTable::new();
    let leaf_emission = leaf.into_const_string_value(&mut string_table);

    // A wrapper or aggregate parent only concatenates the emission and projection it receives.
    // The producing state charged them, so depth cannot multiply the effective charge: a
    // three-level chain stays at the leaf's six bytes instead of growing to eighteen.
    let mut parent = projected_output_state_from(shared.clone());
    parent
        .append_pieces(leaf_projection.as_deref())
        .expect("the child projection is already charged");
    parent.append_emission_value(&leaf_emission, &mut string_table);
    assert_eq!(retained_bytes(&parent), 6);
    assert_eq!(retained_pieces(&parent), 0);
    assert_eq!(parent.output_buffer, "abc");

    let parent_projection = parent.projection_pieces.take();
    let parent_emission = parent.into_const_string_value(&mut string_table);
    let mut grandparent = projected_output_state_from(shared);
    grandparent
        .append_pieces(parent_projection.as_deref())
        .expect("the grandchild projection is already charged");
    grandparent.append_emission_value(&parent_emission, &mut string_table);
    assert_eq!(retained_bytes(&grandparent), 6);
    assert_eq!(grandparent.output_buffer, "abc");
}
