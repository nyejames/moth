//! Hand-authored normalized BorrowProblem fixtures and invariant tests.

mod fixtures;

use super::{
    BlockId, BorrowProblem, Event, EventId, EventKind, EventSource, OriginKind, PlaceId,
    PlaceOverlap, PointId, ProgramPoint, ProjectionElem, RebindValue, UseId, ValueOriginId,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use fixtures::{
    branch_join, copy, empty, field_accesses, loop_with_rebind, old_alias_after_rebind,
    same_statement_access_order,
};

#[test]
fn borrow_problem_copy_fixture_validates_and_keeps_source_and_result_origins_distinct() {
    let problem = BorrowProblem::new(copy()).expect("copy fixture should validate");

    assert_eq!(problem.origins().len(), 2);
    assert!(matches!(
        &problem.events()[1].kind,
        EventKind::Copy {
            origin,
            ..
        } if *origin == ValueOriginId::new(1)
    ));
    assert_eq!(problem.events()[2].id.raw(), 2);
}

#[test]
fn borrow_problem_fresh_rebind_fixture_preserves_old_alias_after_new_definition() {
    let problem = BorrowProblem::new(old_alias_after_rebind())
        .expect("old alias after rebind fixture should validate");

    assert!(matches!(
        &problem.events()[1].kind,
        EventKind::Alias {
            origins,
            ..
        } if origins.as_ref() == [ValueOriginId::new(0)]
    ));
    assert!(matches!(
        &problem.events()[2].kind,
        EventKind::Rebind {
            value: RebindValue::Fresh(origin),
            ..
        } if *origin == ValueOriginId::new(1)
    ));
}

#[test]
fn borrow_problem_branch_join_fixture_retains_explicit_join_origin() {
    let problem = BorrowProblem::new(branch_join()).expect("branch fixture should validate");

    assert_eq!(problem.control_flow().edges.len(), 4);
    assert!(matches!(
        &problem.origins()[3].kind,
        OriginKind::Join(origins)
            if origins.as_ref() == [ValueOriginId::new(1), ValueOriginId::new(2)]
    ));
}

#[test]
fn borrow_problem_loop_fixture_accepts_a_back_edge_without_flattening_the_cfg() {
    let problem = BorrowProblem::new(loop_with_rebind()).expect("loop fixture should validate");

    assert!(
        problem
            .control_flow()
            .edges
            .iter()
            .any(|edge| edge.from.raw() == 1 && edge.to.raw() == 1)
    );
}

#[test]
fn borrow_problem_field_fixture_keeps_disjoint_fields_and_base_overlap_explicit() {
    let problem = BorrowProblem::new(field_accesses()).expect("field fixture should validate");
    let base = &problem.places()[0];
    let left = &problem.places()[1];
    let right = &problem.places()[2];

    assert_eq!(left.overlap(right), PlaceOverlap::Disjoint);
    assert_eq!(base.overlap(left), PlaceOverlap::Overlap);
    assert_eq!(
        problem.places()[1].projections.as_ref(),
        [ProjectionElem::Field(0)]
    );
}

#[test]
fn borrow_problem_same_statement_access_fixture_preserves_event_order_at_one_point() {
    let problem = BorrowProblem::new(same_statement_access_order())
        .expect("same-statement access fixture should validate");

    assert_eq!(problem.events()[0].point, problem.events()[1].point);
    assert_eq!(
        problem.control_flow().blocks[0].events.as_ref(),
        [EventId::new(0), EventId::new(1),]
    );
    assert!(matches!(
        &problem.events()[0].kind,
        EventKind::Access { use_id } if *use_id == UseId::new(0)
    ));
    assert!(matches!(
        &problem.events()[1].kind,
        EventKind::Access { use_id } if *use_id == UseId::new(1)
    ));
}

#[test]
fn borrow_problem_malformed_dense_ids_fail_through_the_internal_compiler_error_lane() {
    let mut parts = empty();
    parts.points[0].id = PointId::new(1);

    let error = BorrowProblem::new(parts).expect_err("non-dense point IDs must be rejected");

    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("program-point IDs must be dense"));
}

#[test]
fn borrow_problem_malformed_unowned_event_fails_atomic_construction() {
    let mut parts = empty();
    parts.events.push(Event::new(
        EventId::new(0),
        PointId::new(1),
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        },
        EventSource::none(),
    ));

    let error = BorrowProblem::new(parts).expect_err("every event must belong to one block");

    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("every normalized event"));
}

#[test]
fn borrow_problem_rejects_points_outside_their_block_range() {
    let mut parts = empty();
    parts
        .points
        .push(ProgramPoint::new(PointId::new(2), BlockId::new(0), 2));

    let error = BorrowProblem::new(parts).expect_err("point range must be coherent");

    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("outside the entry/exit range"));
}

#[test]
fn borrow_problem_deterministic_debug_dump_is_stable_for_equal_problems() {
    let first = BorrowProblem::new(copy()).expect("copy fixture should validate");
    let second = BorrowProblem::new(copy()).expect("copy fixture should validate");

    assert_eq!(first.debug_dump(), second.debug_dump());
}
