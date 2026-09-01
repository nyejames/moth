//! Focused invariants for the Boracle operational oracle.
//!
//! WHAT: builds compact validated normalized problems and checks concrete generations, holder
//!       coverage, capability intervals, projections, calls and deterministic traces.
//! WHY: these tests protect the independent runtime semantics before later path enumeration lands.

use super::super::super::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallId, CallResult, CallResultProvenance,
    CallResultUnknownReason, CfgBlock, CfgEdge, Event, EventId, EventKind, EventSource, KillReason,
    Loan, LoanId, OriginKind, Place, PlaceId, PointId, ProgramPoint, ProjectionElem, RebindValue,
    TerminatorEventKind, Use, UseId, UseKind, ValueOrigin, ValueOriginId,
};
#[cfg(feature = "boracle_campaign")]
mod campaign;
mod generator;
mod properties;
mod reducer;
use super::conflicts::{exercise_capabilities, find_interval_conflict};
use super::state::{
    CapabilityEndReason, CapabilitySource, DefinitionEventKind, DefinitionRole,
    DefinitionTransition, OracleState, PlaceIndex, RuntimeAccessTarget, RuntimePlaceState,
};
use super::traces::{TraceAccess, TraceEntry};
use super::{ExecutionTrace, OracleBounds, OracleLimitReason, OracleOutcome, execute_bounded};
use std::collections::{BTreeMap, BTreeSet};

use crate::compiler_frontend::compiler_errors::CompilerError;

#[test]
fn boracle_oracle_many_shared_aliases_on_one_node_do_not_conflict() {
    let mut fixture = Fixture::new(4);
    let owner = fixture.place(0, []);
    let aliases = [
        fixture.place(1, []),
        fixture.place(2, []),
        fixture.place(3, []),
    ];
    fixture.fresh(owner);
    for alias in aliases {
        fixture.alias(alias, owner, AccessKind::Shared);
    }
    for alias in aliases {
        fixture.access(alias, UseKind::Read, false);
    }

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}
#[test]
fn boracle_oracle_interval_conflict_survives_prior_undecidable_overlap() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let holder = fixture.place(1, []);
    let undecidable_target = fixture.place(0, [ProjectionElem::DynamicIndex]);
    let definite_target = fixture.place(0, [ProjectionElem::MapEntry]);
    fixture.fresh(owner);
    fixture.fresh(holder);
    fixture.loan_issue(undecidable_target, holder, AccessKind::Exclusive);
    fixture.loan_issue(definite_target, holder, AccessKind::Exclusive);
    fixture.access(definite_target, UseKind::Read, false);
    fixture.access(holder, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("definite overlap should produce a runtime conflict");
    assert_eq!(
        conflict.capability_id.raw(),
        1,
        "the definite higher-numbered capability must be the witness"
    );
    assert_eq!(conflict.capability_kind, AccessKind::Exclusive);
    assert_eq!(conflict.access_kind, AccessKind::Shared);
    assert_eq!(
        conflict.access_target.path.as_ref(),
        [ProjectionElem::MapEntry]
    );
    assert_eq!(
        conflict.capability_target.path.as_ref(),
        [ProjectionElem::MapEntry]
    );
}

#[test]
fn boracle_oracle_only_undecidable_interval_overlap_is_inconclusive() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let holder = fixture.place(1, []);
    let undecidable_target = fixture.place(0, [ProjectionElem::DynamicIndex]);
    let access_target = fixture.place(0, [ProjectionElem::MapEntry]);
    fixture.fresh(owner);
    fixture.fresh(holder);
    fixture.loan_issue(undecidable_target, holder, AccessKind::Exclusive);
    fixture.access(access_target, UseKind::Read, false);
    fixture.access(holder, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::UndecidableOverlap { left, right },
            ..
        } => {
            assert_eq!(left.path.as_ref(), [ProjectionElem::MapEntry]);
            assert_eq!(right.path.as_ref(), [ProjectionElem::DynamicIndex]);
        }
        outcome => panic!("only undecidable overlap should be inconclusive: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_shared_capability_then_overlapping_exclusive_access_conflicts() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    fixture.access(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let access = trace
        .entries
        .iter()
        .filter_map(|entry| entry.access.as_ref())
        .find(|access| access.kind == AccessKind::Exclusive)
        .expect("write access should be traced");
    assert_eq!(access.kind, AccessKind::Exclusive);
    assert_eq!(
        trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_index),
        Some(3)
    );
}

#[test]
fn boracle_oracle_exclusive_capability_then_overlapping_shared_access_conflicts() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.access(alias, UseKind::Read, false);
    fixture.access(owner, UseKind::Read, false);
    fixture.access(alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("exclusive capability should conflict");
    assert_eq!(conflict.access_index, 3);
    assert_eq!(conflict.capability_kind, AccessKind::Exclusive);
}

#[test]
fn boracle_oracle_rebinding_shared_alias_writes_through_without_killing_holder() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    fixture.access(owner, UseKind::Write, false);
    fixture.alias(alias, owner, AccessKind::Shared);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_write_through_exclusive_alias_is_safe() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.access(alias, UseKind::Write, true);
    fixture.access(owner, UseKind::Read, false);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_write_through_shared_alias_conflicts_directly() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let conflicting_alias = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.access(alias, UseKind::Write, true);
    fixture.alias(conflicting_alias, owner, AccessKind::Shared);
    let conflicting_access_event = fixture.access_event(conflicting_alias, UseKind::Write, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("write through shared alias should trigger the direct rule");
    assert_eq!(conflict.access_event, conflicting_access_event);
    assert_eq!(conflict.access_index, 4);
    assert_eq!(conflict.capability_id.raw(), 1);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
}

/// A write through a covered projection of a live shared alias exercises the alias capability
/// by holder coverage, but the exercise step never retargets anything: the capability still
/// names the alias's base target while the access resolves through the alias's residual path
/// plus its own projection, so the witness must be picked by the target overlap relation rather
/// than by equality or the access falls through as legal and the completed interval scan skips
/// the capability precisely because it was exercised.
#[test]
fn boracle_oracle_write_through_covered_projection_of_shared_alias_conflicts_directly() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let alias_entry = fixture.place(1, [ProjectionElem::MapEntry]);
    let owner_entry = fixture.place(0, [ProjectionElem::MapEntry]);
    fixture.fresh(owner);

    // The source carries a MapEntry residual on purpose: a projection whose element materialises
    // no child node keeps the access target on the capability's node, so the defect under test is
    // the exact-equality check on a longer path and not the node disjointness a materialised
    // Field child would introduce.
    fixture.alias(alias, owner_entry, AccessKind::Shared);
    let covered_write = fixture.access_event(alias_entry, UseKind::Write, false);
    match run(fixture.finish()) {
        OracleOutcome::RuntimeConflict { trace } => {
            let conflict = trace
                .conflict
                .as_ref()
                .expect("a conflict carries its witness");
            assert_eq!(conflict.access_event, covered_write);
            assert_eq!(conflict.access_kind, AccessKind::Exclusive);
            assert_eq!(conflict.capability_kind, AccessKind::Shared);
            assert_eq!(conflict.capability_id.raw(), 0);
            assert_eq!(
                conflict.access_target.path.as_ref(),
                [ProjectionElem::MapEntry, ProjectionElem::MapEntry],
                "the access resolves the alias residual plus its own projection"
            );
            assert_eq!(
                conflict.capability_target.path.as_ref(),
                [ProjectionElem::MapEntry],
                "the witness carries the capability's own target so it replays against the \
                 capability row the trace recorded"
            );
        }
        outcome => panic!(
            "a write through a covered projection of a live shared alias must conflict directly \
             at that access, got {outcome:?}"
        ),
    }
}

#[test]
fn boracle_oracle_call_result_realias_conflicts_at_owner_write_not_definition() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let result_place = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(result_place, owner, AccessKind::Shared);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "realiased-result".to_string(),
    });
    let (_, argument) = fixture.call_argument_at(CallId::new(0), 0, owner, AccessKind::Shared);
    fixture.call_effect_result(
        CallId::new(0),
        vec![argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
    );
    let owner_write = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(result_place, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("the still-live alias capability must conflict at the owner write");
    assert_eq!(
        conflict.access_event, owner_write,
        "the conflict must be the owner write, not the call result definition"
    );
}

/// The builder emits the confirming definition write immediately after the `CallEffect`, so a
/// pending result is bound to the exact generation that effect installed. The audit's
/// counterexample defeats an unbound entry: a result slot that another event retires before its
/// confirming write would leave a pending record behind, and that stale record suppresses the
/// direct rule and holder retirement for a later defining access that exercises a newly issued
/// live shared-alias capability instead of the retired generation.
#[test]
fn boracle_oracle_scope_exit_while_call_result_pending_is_malformed() {
    let mut fixture = Fixture::new(2);
    let result_place = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "unconfirmed-result".to_string(),
    });

    // 1. A fresh `CallEffect` result binds the place to a new generation and registers the
    //    pending confirmation.
    fixture.call_effect_result(
        CallId::new(0),
        Vec::new(),
        result_place,
        CallResultProvenance::Fresh,
    );
    // 2. A shared alias from the result.
    fixture.alias(alias, result_place, AccessKind::Shared);
    // 3. The result binding goes out of scope while its confirmation is still pending.
    fixture.scope_exit([BindingId::new(0)]);
    // 4. The retired result place is re-installed as a shared alias from that alias.
    fixture.alias(result_place, alias, AccessKind::Shared);
    // 5. A defining exclusive write to the re-installed result place, which conflicts only if
    //    the stale pending entry no longer exempts it.
    fixture.access(result_place, UseKind::Write, true);

    let problem = fixture.finish_result().expect("fixture should validate");
    match execute_bounded(&problem, OracleBounds::default()) {
        Err(error) => {
            let message = format!("{error:?}");
            assert!(
                message.contains("pending call result"),
                "the rejection must name the pending call result: {message}"
            );
        }
        outcome => panic!(
            "retiring a pending call result's binding must be rejected as malformed, \
             got {outcome:?}"
        ),
    }
}

#[test]
fn boracle_oracle_definition_before_call_result_confirmation_is_malformed() {
    let mut fixture = Fixture::new(2);
    let result_place = fixture.place(0, []);
    let other = fixture.place(1, []);
    fixture.fresh(other);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "replaced-result".to_string(),
    });
    fixture.call_effect_result(
        CallId::new(0),
        Vec::new(),
        result_place,
        CallResultProvenance::Fresh,
    );

    // A value-producing event replaces the result place before its confirming write arrived.
    // The pending entry must be rejected here, not silently dropped, because only the
    // confirming definition may consume it and any other definition is malformed builder
    // output.
    fixture.fresh(result_place);
    fixture.access(result_place, UseKind::Write, true);

    let problem = fixture.finish_result().expect("fixture should validate");
    match execute_bounded(&problem, OracleBounds::default()) {
        Err(error) => {
            let message = format!("{error:?}");
            assert!(
                message.contains("pending call result"),
                "the rejection must name the pending call result: {message}"
            );
        }
        outcome => panic!(
            "a definition replacing a pending call result must be rejected as malformed, \
             got {outcome:?}"
        ),
    }
}

#[test]
fn boracle_oracle_alias_from_place_rebind_kills_call_argument_holder() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.rebind_alias_from_place(alias, owner);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "realiased-call-argument".to_string(),
    });
    let (_, argument) = fixture.call_argument_at(CallId::new(0), 0, alias, AccessKind::Shared);
    fixture.rebind_alias_from_place(alias, owner);
    fixture.access(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);
    fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_projection_holder_liveness_ends_before_owner_write() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let owner_field = fixture.place(0, [ProjectionElem::Field(0)]);
    let projected = fixture.place(1, []);
    fixture.fresh(owner);

    // The projection destination is slot-backed, so the projection capability is the only live
    // hold on the projected child and the trailing reads keep its interval open. A rebinding of
    // the destination is a slot replacement that must end that capability before the owner
    // write. Had the projection kept installing alias state, the rebinding would have written
    // through, the capability would have crossed the owner write, and the exclusive field write
    // below would have conflicted with it, so the trailing projected read is what keeps the two
    // rows distinguishable.
    fixture.projection(owner, projected, ProjectionElem::Field(0));
    fixture.access(projected, UseKind::Read, false);
    fixture.rebind_alias_from_place(projected, owner_field);
    fixture.access(owner_field, UseKind::Write, false);
    fixture.access(projected, UseKind::Read, false);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_bare_defining_write_retires_overlapping_holders() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let covered = fixture.place(0, [ProjectionElem::Field(1)]);
    let watcher = fixture.place(1, []);
    fixture.fresh(owner);

    // A bare defining write has no paired provenance writer, so the writer-side slot retirement
    // never runs. The reference's Access kill still ends every capability held by a holder that
    // structurally overlaps the written place (`loans.rs:794-800`), which here is the
    // field-addressed holder and not the written root. The kill is only observable through a
    // NON-holder: the capability's own holder folds any later access back into its exercised
    // list, so the final observer must read the field's node through an independent alias while
    // a later holder read after it drags the interval over an observation that itself never
    // exercised anything. Removing the retirement therefore leaves the shared alias read inside
    // the exclusive loan's interval and produces a real conflict instead of a self-covered one.
    fixture.loan_issue(covered, covered, AccessKind::Exclusive);
    fixture.alias(watcher, covered, AccessKind::Shared);
    fixture.access_event(owner, UseKind::Write, true);
    fixture.access(watcher, UseKind::Read, false);
    fixture.access(covered, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::CompleteSafe { executions: 1, .. } => {}
        outcome => {
            panic!("the bare defining write must retire the overlapping holder: {outcome:?}")
        }
    }
}

/// The child's `Alias` state survives the retirement that ends its capability. Defining the
/// aggregate root structurally overlaps the projected holder, so `retire_overlapping_holders`
/// ends the alias capability at the root definition while the child keeps its exact `Alias`
/// state. The later exclusive write on the child therefore exercises nothing, because the only
/// capability covering that place already ended with `HolderRetired` and every covering holder
/// retired with it. Absence of a live shared capability is a legal, safe access, so the direct
/// rule must fall through instead of manufacturing a conflict from the ended capability the
/// stale state still names. Validation forbids a defining use on a projected place, so this
/// encoding keeps that write non-defining on the same place.
#[test]
fn boracle_oracle_defining_root_retires_child_alias_and_later_write_is_safe() {
    let mut fixture = Fixture::new(2);
    let source = fixture.place(0, []);
    let holder = fixture.place(1, []);
    let holder_field = fixture.place(1, [ProjectionElem::Field(0)]);
    fixture.fresh(source);
    fixture.alias(holder_field, source, AccessKind::Shared);
    fixture.fresh(holder);
    fixture.access(holder_field, UseKind::Write, false);

    match run(fixture.finish()) {
        OracleOutcome::CompleteSafe { executions: 1, .. } => {}
        outcome => {
            panic!("the retired child alias must not conflict the write on it: {outcome:?}")
        }
    }
}

#[test]
fn boracle_oracle_defining_write_on_unavailable_place_installs_nothing() {
    let mut fixture = Fixture::new(1);
    let value = fixture.place(0, []);

    // The deferral design keeps role installation in the provenance writer alone. Were the
    // access itself to install a slot, the read below would resolve instead of failing, and the
    // writer for an initial alias or projection would always reach a slot-backed destination.
    fixture.access(value, UseKind::Write, true);
    fixture.access(value, UseKind::Read, false);

    let error = execute_bounded(&fixture.finish(), OracleBounds::default())
        .expect_err("a deferred defining write must not install a role");
    assert!(error.msg.contains("non-defining access"), "{error:?}");
}
#[test]
fn boracle_oracle_slot_definition_write_defers_replacement_to_writer() {
    let mut fixture = Fixture::new(1);
    let owner = fixture.place(0, []);
    let problem = fixture.finish();
    let mut state = OracleState::new(&problem);
    let place_index = PlaceIndex::new(&problem);
    let seed = state
        .issue_generation(64)
        .expect("seed generation should fit the bound");
    state
        .apply_definition_transition(
            &problem,
            owner,
            DefinitionEventKind::Value,
            DefinitionRole::Slot { current: seed },
            0,
        )
        .expect("seed install should execute");
    let pre_writer = state
        .resolve_place(&problem, &place_index, owner, 64)
        .expect("place should resolve")
        .expect("seeded slot should resolve")
        .unwrap()
        .target
        .node;
    assert_eq!(
        pre_writer, seed,
        "the defining write must see the seed slot"
    );
    let replacement = state
        .issue_generation(64)
        .expect("replacement generation should fit the bound");
    state
        .apply_definition_transition(
            &problem,
            owner,
            DefinitionEventKind::Value,
            DefinitionRole::Slot {
                current: replacement,
            },
            1,
        )
        .expect("writer transition should execute");
    let post_writer = state
        .resolve_place(&problem, &place_index, owner, 64)
        .expect("place should resolve")
        .expect("replaced slot should resolve")
        .unwrap()
        .target
        .node;
    assert_ne!(
        pre_writer, post_writer,
        "only the writer installs the replacement generation"
    );
}
#[test]
fn boracle_oracle_slot_replacement_retires_the_destination_held_capability() {
    let mut fixture = Fixture::new(1);
    let owner = fixture.place(0, []);
    let problem = fixture.finish();
    let mut state = OracleState::new(&problem);
    let generation = state
        .issue_generation(16)
        .expect("seed generation should fit the bound");
    state.set_state(
        owner,
        RuntimePlaceState::Slot {
            current: generation,
        },
    );
    let capability = state
        .issue_capability(
            AccessKind::Shared,
            RuntimeAccessTarget {
                node: generation,
                path: Box::new([]),
            },
            BTreeSet::from([owner]),
            0,
            EventId::new(0),
            CapabilitySource::Loan(LoanId::new(0)),
        )
        .expect("the destination-held capability should issue");

    // A multi-holder loan can no longer carry this probe end to end, so the transition's
    // retirement contract is checked on the state it owns: the replaced slot row ends the
    // capability the destination held, with the holder retired at the writer's position.
    let replacement = state
        .issue_generation(16)
        .expect("replacement generation should fit the bound");
    let transition = state
        .apply_definition_transition(
            &problem,
            owner,
            DefinitionEventKind::Value,
            DefinitionRole::Slot {
                current: replacement,
            },
            1,
        )
        .expect("a slot-backed destination must replace its slot");
    let DefinitionTransition::ReplacedSlot {
        retired_capabilities,
        ..
    } = transition
    else {
        panic!("a slot-backed destination must take the slot-replacement row")
    };
    assert_eq!(retired_capabilities.as_ref(), [capability].as_slice());
    let row = state
        .capabilities
        .get(&capability)
        .expect("the retired capability row should remain addressable");
    assert_eq!(row.explicit_end, Some(1));
}

#[test]
fn boracle_oracle_copy_result_preserves_alias_topology_and_independence() {
    let mut fixture = Fixture::new(5);
    let source = fixture.place(0, []);
    let source_field0 = fixture.place(0, [ProjectionElem::Field(0)]);
    let source_field1 = fixture.place(0, [ProjectionElem::Field(1)]);
    let source_field2 = fixture.place(0, [ProjectionElem::Field(2)]);
    let destination = fixture.place(3, []);
    let destination_field0 = fixture.place(3, [ProjectionElem::Field(0)]);
    let destination_field1 = fixture.place(3, [ProjectionElem::Field(1)]);
    let destination_field2 = fixture.place(3, [ProjectionElem::Field(2)]);
    let conflicting_alias = fixture.place(4, []);
    let repeated_child = fixture.place(1, []);
    let distinct_child = fixture.place(2, []);
    fixture.fresh(repeated_child);
    fixture.fresh(distinct_child);
    fixture.aggregate(
        source,
        [
            (ProjectionElem::Field(0), repeated_child),
            (ProjectionElem::Field(1), repeated_child),
            (ProjectionElem::Field(2), distinct_child),
        ],
    );
    fixture.copy(destination, source);
    fixture.access(source_field0, UseKind::Read, false);
    fixture.access(source_field1, UseKind::Read, false);
    fixture.access(source_field2, UseKind::Read, false);
    fixture.access(destination_field0, UseKind::Read, false);
    fixture.access(destination_field1, UseKind::Read, false);
    fixture.access(destination_field2, UseKind::Read, false);
    fixture.alias(conflicting_alias, source_field0, AccessKind::Shared);
    fixture.access(conflicting_alias, UseKind::Write, false);

    let trace = conflict_trace(run(fixture.finish()));
    let accesses = trace
        .entries
        .iter()
        .filter_map(|entry| entry.access.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(accesses.len(), 7);
    assert_eq!(accesses[0].target.node, accesses[1].target.node);
    assert_ne!(accesses[0].target.node, accesses[2].target.node);
    assert_ne!(accesses[0].target.node, accesses[3].target.node);
    assert_eq!(accesses[3].target.node, accesses[4].target.node);
    assert_ne!(accesses[3].target.node, accesses[5].target.node);
    assert_ne!(accesses[2].target.node, accesses[5].target.node);
}

#[test]
fn boracle_oracle_repeated_child_alias_inside_aggregate_is_observable() {
    let mut fixture = Fixture::new(4);
    let child = fixture.place(0, []);
    let aggregate = fixture.place(1, []);
    let first_alias = fixture.place(2, []);
    let first_field = fixture.place(1, [ProjectionElem::Field(0)]);
    let second_field = fixture.place(1, [ProjectionElem::Field(1)]);
    fixture.fresh(child);
    fixture.aggregate(
        aggregate,
        [
            (ProjectionElem::Field(0), child),
            (ProjectionElem::Field(1), child),
        ],
    );
    fixture.alias(first_alias, first_field, AccessKind::Shared);
    fixture.access(first_alias, UseKind::Read, false);
    fixture.access(second_field, UseKind::Write, false);
    fixture.access(first_alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let accesses = trace
        .entries
        .iter()
        .filter_map(|entry| entry.access.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(accesses.len(), 3);
    assert_eq!(accesses[0].target.node, accesses[1].target.node);
}
#[test]
fn boracle_oracle_repeated_keyless_domain_child_with_distinct_nodes_is_typed_inconclusive() {
    let mut fixture = Fixture::new(4);
    let first_child = fixture.place(0, []);
    let second_child = fixture.place(1, []);
    let aggregate = fixture.place(2, []);
    let map_entry = fixture.place(2, [ProjectionElem::MapEntry]);
    let alias = fixture.place(3, []);
    fixture.fresh(first_child);
    fixture.fresh(second_child);
    // The children map keys on the projection alone, so two distinct nodes under one MapEntry
    // would silently keep only the last one: the map observation below would then resolve
    // against the surviving child instead of the aliased first child, and the real conflict with
    // the forgotten child would be reported as safe.
    fixture.aggregate(
        aggregate,
        [
            (ProjectionElem::MapEntry, first_child),
            (ProjectionElem::MapEntry, second_child),
        ],
    );
    fixture.alias(alias, first_child, AccessKind::Exclusive);
    fixture.access(map_entry, UseKind::Read, false);
    fixture.access(alias, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason:
                OracleLimitReason::RepeatedProjectionChild {
                    destination,
                    projection,
                    surviving,
                    forgotten,
                },
            explored,
            ..
        } => {
            assert_eq!(destination, aggregate);
            assert_eq!(projection, ProjectionElem::MapEntry);
            assert_ne!(surviving, forgotten);
            assert_eq!(explored, 3);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_repeated_identified_field_with_distinct_nodes_is_typed_inconclusive() {
    let mut fixture = Fixture::new(4);
    let first_child = fixture.place(0, []);
    let second_child = fixture.place(1, []);
    let aggregate = fixture.place(2, []);
    let field = fixture.place(2, [ProjectionElem::Field(0)]);
    let alias = fixture.place(3, []);
    fixture.fresh(first_child);
    fixture.fresh(second_child);
    // The identified repeat carries reference semantics exactly like the keyless one: the
    // solver extends the projected place's alternatives with every repeated field's origins
    // (`origins.rs:1308-1317`). The runtime children map would forget the first child just as
    // silently, so the identified case shares the keyless refusal instead of an error lane.
    fixture.aggregate(
        aggregate,
        [
            (ProjectionElem::Field(0), first_child),
            (ProjectionElem::Field(0), second_child),
        ],
    );
    fixture.alias(alias, first_child, AccessKind::Exclusive);
    let field_read = fixture.access_event(field, UseKind::Read, false);
    fixture.access(alias, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason:
                OracleLimitReason::RepeatedProjectionChild {
                    destination,
                    projection,
                    surviving,
                    forgotten,
                },
            explored,
            ..
        } => {
            assert_eq!(destination, aggregate, "{field_read:?}");
            assert_eq!(projection, ProjectionElem::Field(0));
            assert_ne!(surviving, forgotten);
            assert_eq!(explored, 3);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_same_node_repeated_keyless_domain_child_still_aliases() {
    let mut fixture = Fixture::new(3);
    let child = fixture.place(0, []);
    let aggregate = fixture.place(1, []);
    let map_entry = fixture.place(1, [ProjectionElem::MapEntry]);
    let alias = fixture.place(2, []);
    fixture.fresh(child);
    // Both fields resolve to one node, so the keyless domain legitimately holds a single shared
    // edge and the refusal must not swallow it: the map observation still reaches the child the
    // exclusive alias points at.
    fixture.aggregate(
        aggregate,
        [
            (ProjectionElem::MapEntry, child),
            (ProjectionElem::MapEntry, child),
        ],
    );
    fixture.alias(alias, child, AccessKind::Exclusive);
    let map_read = fixture.access_event(map_entry, UseKind::Read, false);
    fixture.access(alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("the repeated same-node child must stay observable as aliasing");
    assert_eq!(
        conflict.access_event, map_read,
        "the conflict must be the map-domain observation"
    );
    assert_eq!(conflict.access_kind, AccessKind::Shared);
    assert_eq!(conflict.capability_kind, AccessKind::Exclusive);
    assert_eq!(
        conflict.access_target.node, conflict.capability_target.node,
        "the map observation must resolve to the aliased child node"
    );
}

#[test]
fn boracle_oracle_aggregate_field_provenance_conflicts_after_source_mutation() {
    let mut fixture = Fixture::new(3);
    let source = fixture.place(0, []);
    let aggregate = fixture.place(1, []);
    let field = fixture.place(1, [ProjectionElem::Field(0)]);
    fixture.fresh(source);
    fixture.aggregate(aggregate, [(ProjectionElem::Field(0), source)]);
    let source_write = fixture.access_event(source, UseKind::Write, false);
    fixture.access(field, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("aggregate provenance should witness the source mutation");
    assert_eq!(conflict.access_event, source_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.source == CapabilitySource::Provenance && capability.holders.contains(&field)
        })
        .expect("aggregate field should issue a provenance capability");
    assert_eq!(capability.kind, AccessKind::Shared);
    assert_eq!(capability.holders, BTreeSet::from([field]));
    assert_eq!(capability.target, conflict.access_target.node);
}

#[test]
fn boracle_oracle_write_through_aggregate_issues_field_provenance() {
    let mut fixture = Fixture::new(3);

    let source = fixture.place(0, []);
    let destination = fixture.place(1, []);
    let initial_referent = fixture.place(2, []);
    let field = fixture.place(1, [ProjectionElem::Field(0)]);
    fixture.fresh(source);
    fixture.fresh(initial_referent);
    fixture.alias(destination, initial_referent, AccessKind::Shared);
    let aggregate_event = EventId::new(fixture.parts.events.len() as u32);
    fixture.aggregate(destination, [(ProjectionElem::Field(0), source)]);
    let source_write = fixture.access_event(source, UseKind::Write, false);
    fixture.access(field, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("write-through aggregate provenance should witness the source mutation");
    assert_eq!(conflict.access_event, source_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.issue_event == aggregate_event
                && capability.source == CapabilitySource::Provenance
                && capability.holders.contains(&field)
        })
        .expect("write-through aggregate should issue field provenance");
    assert_eq!(
        conflict.capability_issue, capability.issue_index,
        "the aggregate provenance capability must witness the source mutation"
    );
    assert_eq!(capability.kind, AccessKind::Shared);
    assert_eq!(capability.holders, BTreeSet::from([field]));
    assert_eq!(capability.target, conflict.access_target.node);
}

#[test]
fn boracle_oracle_aggregate_field_provenance_falls_back_to_destination_holder() {
    let mut fixture = Fixture::new(2);
    let source = fixture.place(0, []);
    let destination = fixture.place(1, []);
    fixture.fresh(source);
    let aggregate_event = EventId::new(fixture.parts.events.len() as u32);
    fixture.aggregate(destination, [(ProjectionElem::Field(0), source)]);
    let source_write = fixture.access_event(source, UseKind::Write, false);
    fixture.access(destination, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("aggregate provenance should witness the source mutation");
    assert_eq!(conflict.access_event, source_write);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.issue_event == aggregate_event
                && capability.source == CapabilitySource::Provenance
        })
        .expect("aggregate should issue provenance with a fallback holder");
    assert_eq!(capability.kind, AccessKind::Shared);
    assert_eq!(capability.holders, BTreeSet::from([destination]));
}

#[test]
fn boracle_oracle_projection_access_resolves_to_child_and_checks_child_legality() {
    let mut fixture = Fixture::new(4);
    let child = fixture.place(0, []);
    let aggregate = fixture.place(1, []);
    let field = fixture.place(1, [ProjectionElem::Field(0)]);
    let alias = fixture.place(2, []);
    let conflicting_alias = fixture.place(3, []);
    fixture.fresh(child);
    fixture.aggregate(aggregate, [(ProjectionElem::Field(0), child)]);
    let projection_event = fixture.projection(aggregate, alias, ProjectionElem::Field(0));
    fixture.access(field, UseKind::Read, false);
    fixture.access(alias, UseKind::Read, false);
    fixture.alias(conflicting_alias, field, AccessKind::Shared);
    fixture.access(conflicting_alias, UseKind::Write, false);

    let trace = conflict_trace(run(fixture.finish()));
    let accesses = trace
        .entries
        .iter()
        .filter_map(|entry| entry.access.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(accesses[0].target.node, accesses[1].target.node);
    let projection_capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.issue_event == projection_event)
        .expect("projection should issue a capability");
    assert_eq!(projection_capability.source, CapabilitySource::Provenance);
    let conflict = trace
        .conflict
        .as_ref()
        .expect("child capability should conflict");
    assert_eq!(conflict.access_target.node, conflict.capability_target.node);
    assert!(conflict.access_target.path.is_empty());
}

#[test]
fn boracle_oracle_scope_exit_retires_places_and_ends_holder_intervals() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let conflicting_alias = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    let scope_exit_event = fixture.scope_exit([BindingId::new(1)]);
    fixture.access(owner, UseKind::Write, false);
    fixture.alias(conflicting_alias, owner, AccessKind::Shared);
    fixture.access(conflicting_alias, UseKind::Write, false);

    let trace = conflict_trace(run(fixture.finish()));
    let capability = trace
        .capabilities
        .first()
        .expect("alias should issue one capability");
    assert_eq!(capability.explicit_end, Some(scope_exit_event));
    assert_eq!(
        capability.end_reason,
        Some(CapabilityEndReason::HolderRetired)
    );
    assert_eq!(capability.retired_holders, BTreeSet::from([alias]));

    let mut exited = Fixture::new(2);
    let exited_owner = exited.place(0, []);
    let exited_alias = exited.place(1, []);
    exited.fresh(exited_owner);
    exited.alias(exited_alias, exited_owner, AccessKind::Exclusive);
    exited.scope_exit([BindingId::new(1)]);
    exited.access(exited_alias, UseKind::Read, false);
    let error = execute_bounded(&exited.finish(), OracleBounds::default())
        .expect_err("access through an exited alias must be rejected");
    assert!(error.msg.contains("non-defining access"));
}

#[test]
fn boracle_oracle_scope_exit_with_surviving_holder_is_typed_inconclusive() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let retired_holder = fixture.place(1, []);
    let surviving_holder = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.alias(retired_holder, owner, AccessKind::Shared);
    fixture.alias(surviving_holder, owner, AccessKind::Shared);
    // The surviving-holder observation of an ended capability requires holder cardinality: with
    // one holder the ended row is skipped as retired, so nothing could observe the scope exit at
    // all. No producer emits this shape, so the oracle refuses the loan row before any interval
    // semantics gets to guess which holder the capability still covers.
    fixture.loan_issue_with_holders(
        owner,
        [retired_holder, surviving_holder],
        AccessKind::Shared,
    );
    fixture.scope_exit([BindingId::new(1)]);
    fixture.access(surviving_holder, UseKind::Read, false);

    match execute_bounded(&fixture.finish(), OracleBounds::default()) {
        Ok(OracleOutcome::Inconclusive {
            reason: OracleLimitReason::MultiHolderLoan { loan, holders },
            ..
        }) => {
            assert_eq!(loan, LoanId::new(0));
            assert_eq!(holders, 2);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_scope_exit_accumulates_retired_holders_across_stages_is_typed_inconclusive() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let first_holder = fixture.place(1, []);
    let second_holder = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.fresh(first_holder);
    fixture.fresh(second_holder);
    // Accumulating retired holders across stages needs two distinct holders on one capability:
    // a single-holder row retires once and a second stage can only re-extend the same holder
    // set. The multi-holder shape this observation needs is refused at issue time, and the
    // staged retirements never execute.
    fixture.loan_issue_with_holders(owner, [first_holder, second_holder], AccessKind::Shared);
    fixture.scope_exit([BindingId::new(1)]);
    fixture.scope_exit([BindingId::new(2)]);
    fixture.fresh(second_holder);
    fixture.access(second_holder, UseKind::Read, false);

    match execute_bounded(&fixture.finish(), OracleBounds::default()) {
        Ok(OracleOutcome::Inconclusive {
            reason: OracleLimitReason::MultiHolderLoan { loan, holders },
            ..
        }) => {
            assert_eq!(loan, LoanId::new(0));
            assert_eq!(holders, 2);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}
#[test]
fn boracle_oracle_multi_holder_loan_issue_is_typed_inconclusive() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let first_holder = fixture.place(1, []);
    let second_holder = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.fresh(first_holder);
    fixture.fresh(second_holder);
    fixture.loan_issue_with_holders(owner, [first_holder, second_holder], AccessKind::Shared);

    // No producer emits a multi-holder loan row and the static solver has no per-holder
    // semantics for it, so the oracle refuses the shape at issue time instead of ending the
    // whole capability at the first holder retirement.
    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::MultiHolderLoan { loan, holders },
            explored,
            ..
        } => {
            assert_eq!(loan, LoanId::new(0));
            assert_eq!(holders, 2);
            assert_eq!(explored, 4);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_multi_holder_loan_surviving_hold_is_typed_inconclusive() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let retiring_holder = fixture.place(1, []);
    let surviving_holder = fixture.place(0, [ProjectionElem::Field(1)]);
    fixture.fresh(owner);
    fixture.fresh(retiring_holder);
    // The first holder's scope exit ends the whole capability, so the surviving holder's
    // continuing hold stops protecting the referent and the exclusive write below ran to
    // CompleteSafe before the refusal existed. The typed refusal carries exactly this gap.
    fixture.loan_issue_with_holders(
        owner,
        [retiring_holder, surviving_holder],
        AccessKind::Exclusive,
    );
    fixture.scope_exit([BindingId::new(1)]);
    fixture.access(owner, UseKind::Write, false);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::MultiHolderLoan { loan, holders },
            ..
        } => {
            assert_eq!(loan, LoanId::new(0));
            assert_eq!(holders, 2);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}
/// A row that repeats one place names one holder: validation does not require uniqueness and the
/// capability set collapses the duplicate, so the cardinality refusal must count distinct places
/// and the row must reach a real outcome instead of the typed inconclusive.
#[test]
fn boracle_oracle_duplicated_holder_loan_reaches_a_real_outcome() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.loan_issue_with_holders(owner, [owner, owner], AccessKind::Exclusive);
    let alias_write = fixture.access_event(alias, UseKind::Write, false);
    fixture.access(owner, UseKind::Read, false);

    let trace = match run(fixture.finish()) {
        OracleOutcome::RuntimeConflict { trace } => trace,
        outcome => panic!(
            "a repeated holder must collapse to one and issue a real capability, \
             not refuse the row: {outcome:?}"
        ),
    };
    let conflict = trace
        .conflict
        .as_ref()
        .expect("the loan conflict must carry its witness");
    assert_eq!(
        conflict.access_event, alias_write,
        "the witness must be the alias write against the live loan"
    );
}
#[test]
fn boracle_oracle_unavailable_loan_place_is_compiler_error_before_refusal() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let first_holder = fixture.place(1, []);
    let second_holder = fixture.place(2, []);
    fixture.fresh(first_holder);
    fixture.fresh(second_holder);
    // The loan place was never written, so the issue cannot resolve it: the authority
    // classifies that as malformed input, and the availability checks must keep their error
    // lane even though the multi-holder refusal runs right after them.
    fixture.loan_issue_with_holders(owner, [first_holder, second_holder], AccessKind::Shared);

    let error = execute_bounded(&fixture.finish(), OracleBounds::default())
        .expect_err("an unresolvable loan place must stay malformed");
    assert!(
        error.msg.contains("is unavailable"),
        "unexpected error: {error:?}"
    );
}

#[test]
fn boracle_oracle_unavailable_multi_holder_loan_holder_is_compiler_error() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let first_holder = fixture.place(1, []);
    let second_holder = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.fresh(first_holder);
    // The second holder only becomes available after the issue, so resolution cannot name a
    // holder for the row at all. That ordering is structurally accepted by validation and
    // malformed by the authority, which the cardinality refusal must not swallow.
    fixture.loan_issue_with_holders(owner, [first_holder, second_holder], AccessKind::Shared);
    fixture.fresh(second_holder);

    let error = execute_bounded(&fixture.finish(), OracleBounds::default())
        .expect_err("an unavailable holder must keep its error lane");
    assert!(
        error.msg.contains("holder") && error.msg.contains("is unavailable"),
        "unexpected error: {error:?}"
    );
}
#[test]
fn boracle_oracle_killed_capability_with_different_target_is_not_superseded() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let holder = fixture.place(1, []);
    let body = BlockId::new(1);
    let kill = BlockId::new(2);
    let access = BlockId::new(3);

    fixture.fresh(owner);
    fixture.alias(holder, owner, AccessKind::Shared);
    let mut entry_events = fixture.take_events();
    fixture.jump(body);
    entry_events.extend(fixture.take_events());

    let loan = fixture.loan_issue(owner, holder, AccessKind::Shared);
    fixture.branch([kill, access]);
    let body_events = fixture.take_events();

    fixture.loan_kill(loan);
    fixture.rebind_fresh(owner);
    fixture.jump(body);
    let kill_events = fixture.take_events();

    fixture.access(holder, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let access_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (body, body_events),
            (kill, kill_events),
            (access, access_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, kill),
            CfgEdge::new(body, access),
            CfgEdge::new(kill, body),
        ],
        BlockId::new(0),
        vec![access],
    );

    let error = execute_bounded(&problem, OracleBounds::new(256, 4096, 8, 4096))
        .expect_err("a later loan row for another generation must not supersede the ended row");
    assert!(error.msg.contains("after its end"));
}

#[test]
fn boracle_oracle_killed_capability_same_generation_overlapping_projection_is_superseded() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let owner_dynamic = fixture.place(0, [ProjectionElem::DynamicIndex]);
    let holder = fixture.place(1, []);
    let holder_field = fixture.place(1, [ProjectionElem::Field(0)]);
    let body = BlockId::new(1);
    let kill = BlockId::new(2);
    let access = BlockId::new(3);

    fixture.fresh(owner);
    fixture.alias(holder, owner_dynamic, AccessKind::Shared);
    let mut entry_events = fixture.take_events();
    fixture.jump(body);
    entry_events.extend(fixture.take_events());

    let loan = fixture.loan_issue(owner_dynamic, holder, AccessKind::Shared);
    fixture.branch([kill, access]);
    let body_events = fixture.take_events();

    fixture.loan_kill(loan);
    fixture.jump(body);
    let kill_events = fixture.take_events();

    fixture.access(holder_field, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let access_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (body, body_events),
            (kill, kill_events),
            (access, access_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, kill),
            CfgEdge::new(body, access),
            CfgEdge::new(kill, body),
        ],
        BlockId::new(0),
        vec![access],
    );
    let outcome =
        execute_bounded(&problem, OracleBounds::default()).expect("projection should be safe");
    assert!(matches!(
        outcome,
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::BlockEntryBound { block, .. },
            ..
        } if block == body
    ));
}

#[test]
fn boracle_oracle_call_argument_interval_extends_to_call_effect() {
    let mut fixture = Fixture::new(1);
    let argument_place = fixture.place(0, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "interval".to_string(),
    });
    let (first_event, first_argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let (second_event, second_argument) =
        fixture.call_argument_at(CallId::new(0), 1, argument_place, AccessKind::Shared);
    fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![first_argument, second_argument].into_boxed_slice(),
        result: None,
    }));

    let trace = conflict_trace(run(fixture.finish()));
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.issue_event == first_event)
        .expect("first call argument should issue a capability");
    assert_eq!(capability.last_exercised, 3);
    let conflict = trace
        .conflict
        .as_ref()
        .expect("incompatible call arguments should conflict");
    assert_eq!(conflict.access_event, second_event);
    assert_eq!(conflict.access_index, 2);
    assert_eq!(conflict.capability_issue, capability.issue_index);
    assert_eq!(conflict.capability_kind, AccessKind::Exclusive);
    assert_eq!(conflict.access_kind, AccessKind::Shared);
}

#[test]
fn boracle_oracle_call_argument_ends_at_call_effect_before_later_storage_access() {
    let mut fixture = Fixture::new(2);
    let argument_place = fixture.place(0, []);
    let writer = fixture.place(1, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "call-boundary".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Shared);
    let effect_event = fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));
    fixture.alias(writer, argument_place, AccessKind::Exclusive);
    let storage_write_event = fixture.access_event(writer, UseKind::Write, false);
    let post_call_access_event = fixture.access_event(argument_place, UseKind::Read, false);

    let outcome = run(fixture.finish());
    assert!(
        matches!(outcome, OracleOutcome::CompleteSafe { executions: 1, .. }),
        "a call argument capability must end at its CallEffect before a later storage write \
         and argument access: argument={argument_event:?}, effect={effect_event:?}, \
         storage_write={storage_write_event:?}, post_call_access={post_call_access_event:?}, \
         outcome={outcome:?}"
    );
}

#[test]
fn boracle_oracle_loan_holder_call_argument_exercises_loan_capability() {
    let mut fixture = Fixture::new(3);
    let loan_place = fixture.place(0, []);
    let unrelated_owner = fixture.place(1, []);
    let unrelated_alias = fixture.place(2, []);
    fixture.fresh(loan_place);
    let loan = fixture.loan_issue(loan_place, loan_place, AccessKind::Shared);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "loan-holder-argument".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, loan_place, AccessKind::Shared);
    fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));

    fixture.fresh(unrelated_owner);
    fixture.alias(unrelated_alias, unrelated_owner, AccessKind::Shared);
    fixture.access(unrelated_alias, UseKind::Read, false);
    fixture.access(unrelated_owner, UseKind::Write, false);
    fixture.access(unrelated_alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let argument_entry = trace
        .entries
        .iter()
        .find(|entry| entry.event == argument_event)
        .expect("call argument should be traced");
    assert!(
        argument_entry.access.is_some(),
        "call argument should record its access"
    );
    let loan_capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.source == CapabilitySource::Loan(loan))
        .expect("loan should issue a capability");
    assert_eq!(loan_capability.last_exercised, argument_entry.index);
}

#[test]
fn boracle_oracle_trace_and_debug_rendering_are_reproducible() {
    let mut fixture = Fixture::new(7);
    let owner = fixture.place(0, []);
    let owner_field0 = fixture.place(0, [ProjectionElem::Field(0)]);
    let owner_field1 = fixture.place(0, [ProjectionElem::Field(1)]);
    let shared_alias0 = fixture.place(1, []);
    let shared_alias1 = fixture.place(2, []);
    let exclusive_alias0 = fixture.place(3, []);
    let exclusive_alias1 = fixture.place(4, []);
    let child0 = fixture.place(5, []);
    let child1 = fixture.place(6, []);
    fixture.fresh(child0);
    fixture.fresh(child1);
    let aggregate_event = EventId::new(fixture.parts.events.len() as u32);
    fixture.aggregate(
        owner,
        [
            (ProjectionElem::Field(0), child0),
            (ProjectionElem::Field(1), child1),
        ],
    );
    let shared_alias0_event = fixture.alias(shared_alias0, owner_field0, AccessKind::Shared);
    let shared_alias1_event = fixture.alias(shared_alias1, owner_field0, AccessKind::Shared);
    let exclusive_alias0_event =
        fixture.alias(exclusive_alias0, owner_field1, AccessKind::Exclusive);
    let exclusive_alias1_event =
        fixture.alias(exclusive_alias1, owner_field1, AccessKind::Exclusive);
    fixture.access(shared_alias0, UseKind::Read, false);
    fixture.access(shared_alias1, UseKind::Read, false);
    let conflicting_access_event = EventId::new(fixture.parts.events.len() as u32);
    fixture.access(owner_field1, UseKind::Read, false);
    fixture.access(exclusive_alias0, UseKind::Read, false);
    fixture.access(exclusive_alias1, UseKind::Read, false);

    let problem = fixture.finish();
    let first = conflict_trace(run(problem.clone()));
    let second = conflict_trace(run(problem));
    assert_eq!(first, second);
    assert_eq!(first.debug_dump(), second.debug_dump());

    let issue_events = first
        .capabilities
        .iter()
        .map(|capability| capability.issue_event)
        .collect::<Vec<_>>();
    assert_eq!(
        issue_events,
        vec![
            aggregate_event,
            aggregate_event,
            shared_alias0_event,
            shared_alias1_event,
            exclusive_alias0_event,
            exclusive_alias1_event,
        ]
    );
    let conflict = first
        .conflict
        .as_ref()
        .expect("completed trace should report the interval conflict");
    assert_eq!(conflict.access_event, conflicting_access_event);
    let first_exclusive = first
        .capabilities
        .iter()
        .find(|capability| capability.issue_event == exclusive_alias0_event)
        .expect("first exclusive alias should issue a capability");
    assert_eq!(conflict.capability_issue, first_exclusive.issue_index);

    let dump = first.debug_dump();
    let rendered_issue_positions = dump
        .match_indices("issue_event:")
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    assert_eq!(rendered_issue_positions.len(), issue_events.len());
    assert!(
        rendered_issue_positions
            .windows(2)
            .all(|window| window[0] < window[1])
    );
}

#[test]
fn boracle_oracle_single_alias_params_result_confirmation_is_safe() {
    let mut fixture = Fixture::new(2);
    let argument_place = fixture.place(0, []);
    let result_place = fixture.place(1, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "alias-result".to_string(),
    });
    let (_, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Shared);
    fixture.call_effect_result(
        CallId::new(0),
        vec![argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
    );
    fixture.access(result_place, UseKind::Write, true);
    fixture.access(result_place, UseKind::Read, false);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_alias_params_result_slot_write_after_effect_is_safe() {
    let mut fixture = Fixture::new(2);
    let argument_place = fixture.place(0, []);
    let result_place = fixture.place(1, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "alias-result-write".to_string(),
    });
    let (_, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Shared);
    fixture.call_effect_result(
        CallId::new(0),
        vec![argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
    );

    // An alias-parameters result is slot-backed: the call's hold on its argument ends at the
    // effect, so a later exclusive write through the result slot reaches the same generation
    // with no live incompatible hold across it and must stay safe. Under the closed
    // alias-installing exception this write fabricated a conflict on the shared argument
    // capability.
    fixture.access(result_place, UseKind::Write, false);
    fixture.access(result_place, UseKind::Read, false);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_call_result_provenance_conflicts_after_argument_mutation() {
    let mut fixture = Fixture::new(2);
    let argument_place = fixture.place(0, []);
    let result_place = fixture.place(1, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "alias-result-provenance".to_string(),
    });
    let (_, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let effect_event = fixture.call_effect_result(
        CallId::new(0),
        vec![argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
    );
    let argument_write = fixture.access_event(argument_place, UseKind::Write, false);
    fixture.access(result_place, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("call-result provenance should witness the argument mutation");
    assert_eq!(conflict.access_event, argument_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.issue_event == effect_event
                && capability.source == CapabilitySource::Provenance
        })
        .expect("AliasParams should issue a provenance capability");
    assert_eq!(capability.kind, AccessKind::Shared);
    assert_eq!(capability.holders, BTreeSet::from([result_place]));
    assert_eq!(capability.target, conflict.access_target.node);
}

#[test]
fn boracle_oracle_write_through_call_result_issues_provenance() {
    let mut fixture = Fixture::new(3);
    let argument_place = fixture.place(0, []);
    let result_referent = fixture.place(1, []);
    let result_place = fixture.place(2, []);
    fixture.fresh(argument_place);
    fixture.fresh(result_referent);
    fixture.alias(result_place, result_referent, AccessKind::Shared);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "write-through-alias-result-provenance".to_string(),
    });
    let (_, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let effect_event = fixture.call_effect_result(
        CallId::new(0),
        vec![argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
    );
    let argument_write = fixture.access_event(argument_place, UseKind::Write, false);
    fixture.access(result_place, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("write-through call-result provenance should witness the argument mutation");
    assert_eq!(conflict.access_event, argument_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.issue_event == effect_event
                && capability.source == CapabilitySource::Provenance
        })
        .expect("write-through AliasParams should issue a provenance capability");
    assert_eq!(
        conflict.capability_issue, capability.issue_index,
        "the call-result provenance capability must witness the argument mutation"
    );
    assert_eq!(capability.kind, AccessKind::Shared);
    assert_eq!(capability.holders, BTreeSet::from([result_place]));
    assert_eq!(capability.target, conflict.access_target.node);
}

#[test]
fn boracle_oracle_copy_of_decidable_remaining_path_executes() {
    let mut fixture = Fixture::new(3);
    let source = fixture.place(0, []);
    let source_field = fixture.place(0, [ProjectionElem::Field(0)]);
    let destination = fixture.place(1, []);
    let conflicting_alias = fixture.place(2, []);
    fixture.fresh(source);
    fixture.copy(destination, source_field);
    fixture.access(source_field, UseKind::Read, false);
    fixture.access(destination, UseKind::Read, false);
    fixture.alias(conflicting_alias, source_field, AccessKind::Shared);
    fixture.access(conflicting_alias, UseKind::Write, false);

    let trace = conflict_trace(run(fixture.finish()));
    let accesses = trace
        .entries
        .iter()
        .filter_map(|entry| entry.access.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(accesses.len(), 3);
    assert_ne!(accesses[0].target.node, accesses[1].target.node);
}

#[test]
fn boracle_oracle_lazy_projection_materialisation_is_idempotent() {
    let mut fixture = Fixture::new(4);
    let source = fixture.place(0, []);
    let source_field = fixture.place(0, [ProjectionElem::Field(0)]);
    let first_alias = fixture.place(1, []);
    let second_alias = fixture.place(2, []);
    let conflicting_alias = fixture.place(3, []);
    fixture.fresh(source);
    fixture.alias(first_alias, source_field, AccessKind::Shared);
    fixture.alias(second_alias, source_field, AccessKind::Shared);
    fixture.access(first_alias, UseKind::Read, false);
    fixture.access(second_alias, UseKind::Read, false);
    fixture.alias(conflicting_alias, source_field, AccessKind::Shared);
    fixture.access(conflicting_alias, UseKind::Write, false);

    let trace = conflict_trace(run(fixture.finish()));
    let accesses = trace
        .entries
        .iter()
        .filter_map(|entry| entry.access.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(accesses.len(), 3);
    assert_eq!(accesses[0].target.node, accesses[1].target.node);
    assert_eq!(accesses[1].target.node, accesses[2].target.node);
}

#[test]
fn boracle_oracle_rebind_alias_origins_is_typed_inconclusive() {
    let mut fixture = Fixture::new(1);
    let destination = fixture.place(0, []);
    let origin = fixture.origin(OriginKind::Fresh);
    fixture.rebind_alias(destination, vec![origin]);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::RebindAliasOrigins { origins },
            explored,
            ..
        } => {
            assert_eq!(origins.as_ref(), [origin]);
            assert_eq!(explored, 1);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_rebind_alias_from_place_issues_no_capability() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let destination = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.rebind_alias_from_place(destination, owner);
    fixture.access(destination, UseKind::Read, false);
    // If the rebind issued a capability for `destination`, these uses would keep the
    // intervening exclusive owner access inside that capability's interval.
    fixture.access(owner, UseKind::Write, false);
    fixture.access(destination, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::CompleteSafe { executions, .. } => assert_eq!(executions, 1),
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_slot_alias_rebind_issues_shared_provenance() {
    let mut fixture = Fixture::new(2);
    let source = fixture.place(0, []);
    let destination = fixture.place(1, []);
    fixture.fresh(source);
    fixture.fresh(destination);
    let alias_event = fixture.alias(destination, source, AccessKind::Exclusive);
    fixture.access(destination, UseKind::Read, false);
    let source_write = fixture.access_event(source, UseKind::Write, false);
    fixture.access(destination, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("slot alias rebind provenance should witness the source mutation");
    assert_eq!(conflict.access_event, source_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.issue_event == alias_event
                && capability.source == CapabilitySource::Provenance
        })
        .expect("slot alias rebind should issue provenance");
    assert_eq!(capability.kind, AccessKind::Shared);
    assert_eq!(capability.holders, BTreeSet::from([destination]));
    assert_eq!(capability.target, conflict.access_target.node);
}

#[test]
fn boracle_oracle_call_result_alias_origins_is_typed_inconclusive() {
    let mut fixture = Fixture::new(1);
    let result_place = fixture.place(0, []);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "alias-origins".to_string(),
    });
    let origins = vec![ValueOriginId::new(0)].into_boxed_slice();
    fixture.call_effect_result(
        CallId::new(0),
        Vec::new(),
        result_place,
        CallResultProvenance::Alias(origins.clone()),
    );

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason:
                OracleLimitReason::CallResultAliasOrigins {
                    call,
                    origins: actual,
                },
            explored,
            ..
        } => {
            assert_eq!(call, CallId::new(0));
            assert_eq!(actual, origins);
            assert_eq!(explored, 1);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_call_result_unknown_is_typed_inconclusive() {
    let mut fixture = Fixture::new(1);
    let result_place = fixture.place(0, []);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "unknown-result".to_string(),
    });
    fixture.call_effect_result(
        CallId::new(0),
        Vec::new(),
        result_place,
        CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown),
    );

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::CallResultUnknown { call, reason },
            explored,
            ..
        } => {
            assert_eq!(call, CallId::new(0));
            assert_eq!(reason, CallResultUnknownReason::SummaryUnknown);
            assert_eq!(explored, 1);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_multiple_call_result_alias_params_issue_each_provenance_capability() {
    let mut fixture = Fixture::new(3);
    let first_place = fixture.place(0, []);
    let second_place = fixture.place(1, []);
    let result_place = fixture.place(2, []);
    fixture.fresh(first_place);
    fixture.fresh(second_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "multiple-results".to_string(),
    });
    let (_, first_argument) =
        fixture.call_argument_at(CallId::new(0), 0, first_place, AccessKind::Exclusive);
    let (_, second_argument) =
        fixture.call_argument_at(CallId::new(0), 1, second_place, AccessKind::Exclusive);
    let effect_event = fixture.call_effect_result(
        CallId::new(0),
        vec![first_argument, second_argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0, 1].into_boxed_slice()),
    );
    let first_write = fixture.access_event(first_place, UseKind::Write, false);
    fixture.access(second_place, UseKind::Write, false);
    fixture.access(result_place, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("multiple AliasParams provenance should witness a source mutation");
    assert_eq!(conflict.access_event, first_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let provenance_capabilities = trace
        .capabilities
        .iter()
        .filter(|capability| capability.source == CapabilitySource::Provenance)
        .collect::<Vec<_>>();
    assert_eq!(provenance_capabilities.len(), 2);
    assert!(
        provenance_capabilities
            .iter()
            .all(|capability| capability.issue_event == effect_event
                && capability.kind == AccessKind::Shared
                && capability.holders == BTreeSet::from([result_place]))
    );
    assert_ne!(
        provenance_capabilities[0].target,
        provenance_capabilities[1].target
    );
}

#[test]
fn boracle_oracle_executed_event_bound_is_typed_inconclusive() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.fresh(place);

    match run_with_bounds(fixture.finish(), OracleBounds::new(256, 0, 8, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::EventBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 0);
            assert_eq!(explored, 0);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_dynamic_generation_bound_is_typed_inconclusive() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.fresh(place);

    match run_with_bounds(fixture.finish(), OracleBounds::new(256, 4096, 8, 0)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::GenerationBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 0);
            assert_eq!(explored, 1);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_path_separates_shared_use_and_mutation() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    let mut entry_events = fixture.take_events();
    fixture.branch([BlockId::new(1), BlockId::new(2)]);
    entry_events.extend(fixture.take_events());

    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let shared_events = fixture.take_events();

    fixture.access(owner, UseKind::Write, false);
    fixture.terminator(TerminatorEventKind::Return);
    let mutation_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (BlockId::new(1), shared_events),
            (BlockId::new(2), mutation_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(0), BlockId::new(2)),
        ],
        BlockId::new(0),
        vec![BlockId::new(1), BlockId::new(2)],
    );

    match run(problem) {
        OracleOutcome::CompleteSafe { executions, .. } => assert_eq!(executions, 2),
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_branch_join_preserves_swapped_independent_values() {
    let mut fixture = Fixture::new(4);
    let a = fixture.place(0, []);
    let b = fixture.place(1, []);
    let alias_excl = fixture.place(2, []);
    let alias_shared = fixture.place(3, []);
    let lower = BlockId::new(1);
    let higher = BlockId::new(2);
    let join = BlockId::new(3);

    fixture.fresh(a);
    fixture.fresh(b);
    let mut entry_events = fixture.take_events();
    fixture.branch([lower, higher]);
    entry_events.extend(fixture.take_events());

    fixture.alias(alias_excl, a, AccessKind::Exclusive);
    fixture.alias(alias_shared, b, AccessKind::Shared);
    fixture.jump(join);
    let lower_events = fixture.take_events();

    fixture.alias(alias_excl, b, AccessKind::Exclusive);
    fixture.alias(alias_shared, a, AccessKind::Shared);
    fixture.jump(join);
    let higher_events = fixture.take_events();

    fixture.access(alias_excl, UseKind::Write, false);
    fixture.access(alias_shared, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let join_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (lower, lower_events),
            (higher, higher_events),
            (join, join_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), lower),
            CfgEdge::new(BlockId::new(0), higher),
            CfgEdge::new(lower, join),
            CfgEdge::new(higher, join),
        ],
        BlockId::new(0),
        vec![join],
    );

    match run(problem) {
        OracleOutcome::CompleteSafe { executions, .. } => assert_eq!(executions, 2),
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_early_return_does_not_mask_higher_arm_conflict() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let lower = BlockId::new(1);
    let higher = BlockId::new(2);
    fixture.fresh(owner);
    let mut entry_events = fixture.take_events();
    fixture.branch([lower, higher]);
    entry_events.extend(fixture.take_events());

    fixture.terminator(TerminatorEventKind::Return);
    let lower_events = fixture.take_events();

    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    let conflict_event = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let higher_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (lower, lower_events),
            (higher, higher_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), lower),
            CfgEdge::new(BlockId::new(0), higher),
        ],
        BlockId::new(0),
        vec![lower, higher],
    );

    let trace = conflict_trace(run(problem));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("higher arm access should conflict");
    assert_eq!(conflict.access_event, conflict_event);
    assert_eq!(
        trace
            .entries
            .iter()
            .find(|entry| entry.event == conflict_event)
            .map(|entry| entry.block),
        Some(higher)
    );
}

#[test]
fn boracle_oracle_success_only_alias_does_not_leak_to_error_arm() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let success = BlockId::new(1);
    let error = BlockId::new(2);
    fixture.fresh(owner);
    let mut entry_events = fixture.take_events();
    fixture.branch([success, error]);
    entry_events.extend(fixture.take_events());

    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::ReturnSuccess);
    let success_events = fixture.take_events();

    fixture.access(owner, UseKind::Write, false);
    fixture.terminator(TerminatorEventKind::ReturnError);
    let error_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (success, success_events),
            (error, error_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), success),
            CfgEdge::new(BlockId::new(0), error),
        ],
        BlockId::new(0),
        vec![success, error],
    );

    match run(problem) {
        OracleOutcome::CompleteSafe { executions, .. } => assert_eq!(executions, 2),
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_error_only_mutation_does_not_leak_to_success_arm() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let success = BlockId::new(1);
    let error = BlockId::new(2);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    let mut entry_events = fixture.take_events();
    fixture.branch([success, error]);
    entry_events.extend(fixture.take_events());

    fixture.access(alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::ReturnSuccess);
    let success_events = fixture.take_events();

    fixture.access(owner, UseKind::Write, false);
    fixture.terminator(TerminatorEventKind::ReturnError);
    let error_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (success, success_events),
            (error, error_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), success),
            CfgEdge::new(BlockId::new(0), error),
        ],
        BlockId::new(0),
        vec![success, error],
    );

    match run(problem) {
        OracleOutcome::CompleteSafe { executions, .. } => assert_eq!(executions, 2),
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_loop_preserves_slot_role_across_alias_rebinds() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let exit = BlockId::new(1);
    let body = BlockId::new(2);
    fixture.fresh(owner);
    fixture.fresh(alias);
    let mut entry_events = fixture.take_events();
    fixture.branch([exit, body]);
    entry_events.extend(fixture.take_events());

    fixture.terminator(TerminatorEventKind::Return);
    let exit_events = fixture.take_events();

    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.branch([exit, body]);
    let body_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (exit, exit_events),
            (body, body_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), exit),
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, exit),
            CfgEdge::new(body, body),
        ],
        BlockId::new(0),
        vec![exit],
    );

    // With one and two execution slots, the completed prefixes are respectively the zero-body
    // and one-body paths. Their explored-event totals pin those two paths before truncation.
    match run_with_bounds(problem.clone(), OracleBounds::new(1, 4096, 8, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::ExecutionBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 1);
            assert_eq!(explored, 4);
        }
        outcome => panic!("unexpected zero-iteration outcome: {outcome:?}"),
    }
    match run_with_bounds(problem.clone(), OracleBounds::new(2, 4096, 8, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::ExecutionBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 2);
            assert_eq!(explored, 7);
        }
        outcome => panic!("unexpected one-iteration outcome: {outcome:?}"),
    }

    match run(problem) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::BlockEntryBound { block, limit },
            explored,
            ..
        } => {
            assert_eq!(block, body);
            assert_eq!(limit, 8);
            assert_eq!(explored, 28);
        }
        outcome => panic!("expected the bounded loop to remain inconclusive: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_reissued_capability_after_scope_exit_exercises_newest_instance() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let holder = fixture.place(1, []);
    let conflict_alias = fixture.place(2, []);
    fixture.fresh(owner);
    fixture.fresh(holder);

    let first_alias = fixture.alias(holder, owner, AccessKind::Exclusive);
    let first_holder_access = fixture.access_event(holder, UseKind::Read, false);
    let scope_exit = fixture.scope_exit([BindingId::new(1)]);

    // Scope exit clears the holder place and closes its first capability. Reinitialising the
    // holder before the second alias creates a distinct capability, so the second holder access
    // must exercise the newest instance only. A separate later conflict supplies a completed
    // trace without conflating that liveness proof with the holder's own access.
    fixture.fresh(holder);
    let second_alias = fixture.alias(holder, owner, AccessKind::Exclusive);
    let second_holder_access = fixture.access_event(holder, UseKind::Read, false);
    let conflict_alias_event = fixture.alias(conflict_alias, owner, AccessKind::Exclusive);
    let conflicting_access = fixture.access_event(owner, UseKind::Read, false);
    fixture.access(conflict_alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("the later conflict must carry its witness");
    assert_eq!(
        conflict.access_event, conflicting_access,
        "the conflict must come from the independent later read"
    );
    let holder_capabilities = trace
        .capabilities
        .iter()
        .filter(|capability| {
            capability.source == CapabilitySource::Provenance
                && capability.holders.contains(&holder)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        holder_capabilities.len(),
        2,
        "the holder must have exactly one capability before and after scope exit"
    );
    assert!(
        holder_capabilities[0].issue_index < holder_capabilities[1].issue_index,
        "reinitialisation must issue a newer capability"
    );
    assert_eq!(holder_capabilities[0].issue_event, first_alias);
    assert_eq!(holder_capabilities[1].issue_event, second_alias);
    assert_eq!(
        holder_capabilities[0].explicit_end,
        Some(scope_exit),
        "scope exit must end the first holder capability"
    );
    assert_eq!(
        holder_capabilities[0].end_reason,
        Some(CapabilityEndReason::HolderRetired)
    );
    assert!(
        holder_capabilities[1].explicit_end.is_none(),
        "the newest holder capability must remain live"
    );

    let first_access = trace
        .entries
        .iter()
        .find(|entry| entry.event == first_holder_access)
        .and_then(|entry| entry.access.as_ref())
        .expect("the first holder access must be traced");
    let second_access = trace
        .entries
        .iter()
        .find(|entry| entry.event == second_holder_access)
        .and_then(|entry| entry.access.as_ref())
        .expect("the second holder access must be traced");
    let holder_capability_ids = holder_capabilities
        .iter()
        .map(|capability| {
            trace
                .capabilities
                .iter()
                .position(|candidate| std::ptr::eq(candidate, *capability))
                .expect("holder capability must belong to the trace")
        })
        .collect::<Vec<_>>();
    assert_eq!(first_access.exercised.len(), 1);
    assert_eq!(
        first_access.exercised[0].raw() as usize,
        holder_capability_ids[0],
        "the first holder access must exercise the first instance"
    );
    assert_eq!(second_access.exercised.len(), 1);
    assert_eq!(
        second_access.exercised[0].raw() as usize,
        holder_capability_ids[1],
        "the second holder access must exercise the newest instance"
    );
    assert_ne!(
        second_access.exercised[0].raw() as usize,
        holder_capability_ids[0],
        "the retired instance must not be exercised after scope exit"
    );
    let witness_id = conflict.capability_id;
    let witness = &trace.capabilities[witness_id.raw() as usize];
    assert_eq!(witness.issue_event, conflict_alias_event);
    assert_eq!(witness.source, CapabilitySource::Alias);
    assert!(witness.holders.contains(&conflict_alias));
    assert!(
        !holder_capability_ids.contains(&(witness_id.raw() as usize)),
        "the independent conflict must not hide the holder exercise proof"
    );
    let _ = (first_alias, second_alias);
}
#[test]
fn boracle_oracle_loop_reaches_write_through_conflict_on_second_iteration() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let exit = BlockId::new(1);
    let body = BlockId::new(2);
    fixture.fresh(owner);
    let mut entry_events = fixture.take_events();
    fixture.branch([exit, body]);
    entry_events.extend(fixture.take_events());

    fixture.terminator(TerminatorEventKind::Return);
    let exit_events = fixture.take_events();

    // Declaring the alias inside the body makes the first iteration install the alias role for
    // the still-unavailable destination, and every later iteration write through that same role.
    let owner_read = fixture.access_event(owner, UseKind::Read, false);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.access(alias, UseKind::Read, false);
    fixture.branch([exit, body]);
    let body_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (exit, exit_events),
            (body, body_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), exit),
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, exit),
            CfgEdge::new(body, body),
        ],
        BlockId::new(0),
        vec![exit],
    );

    // The completed zero-body and one-body prefixes own the one- and two-execution budgets, and
    // neither reaches a conflict: the alias role does not exist before the first declaration, so
    // the owner read precedes every issue_index within the first iteration.
    match run_with_bounds(problem.clone(), OracleBounds::new(1, 4096, 8, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::ExecutionBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 1);
            assert_eq!(explored, 3);
        }
        outcome => panic!("unexpected zero-iteration outcome: {outcome:?}"),
    }
    match run_with_bounds(problem.clone(), OracleBounds::new(2, 4096, 8, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::ExecutionBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 2);
            assert_eq!(explored, 8);
        }
        outcome => panic!("unexpected one-iteration outcome: {outcome:?}"),
    }

    // From the second iteration on, the declaration writes through the surviving exclusive alias
    // capability, whose interval the trailing alias read extends back over the owner read.
    match run(problem) {
        OracleOutcome::RuntimeConflict { trace } => {
            let conflict = trace
                .conflict
                .as_ref()
                .expect("a runtime conflict must carry its witness");
            assert_eq!(conflict.access_event, owner_read);
            assert_eq!(
                trace.block_entries().get(&body).copied(),
                Some(2),
                "the witness must come from the second body entry"
            );
        }
        outcome => panic!("expected a conflict after several iterations: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_break_terminator_executes_exact_successor() {
    assert_single_successor_terminator(
        TerminatorEventKind::Break {
            target: BlockId::new(1),
        },
        BlockId::new(1),
    );
}

#[test]
fn boracle_oracle_continue_terminator_executes_exact_successor() {
    assert_single_successor_terminator(
        TerminatorEventKind::Continue {
            target: BlockId::new(1),
        },
        BlockId::new(1),
    );
}

fn assert_single_successor_terminator(kind: TerminatorEventKind, target: BlockId) {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    let mut entry_events = fixture.take_events();
    fixture.terminator(kind);
    entry_events.extend(fixture.take_events());

    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    let conflict_event = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let target_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([(BlockId::new(0), entry_events), (target, target_events)]),
        vec![CfgEdge::new(BlockId::new(0), target)],
        BlockId::new(0),
        vec![target],
    );
    let trace = conflict_trace(run(problem));
    assert_eq!(
        trace
            .entries
            .iter()
            .find(|entry| entry.event == conflict_event)
            .map(|entry| entry.block),
        Some(target)
    );
}

#[test]
fn boracle_oracle_runtime_failure_is_explicit_trace_scanned_exit() {
    assert_failure_terminator(TerminatorEventKind::RuntimeFailure);
}

#[test]
fn boracle_oracle_assert_failure_is_explicit_trace_scanned_exit() {
    assert_failure_terminator(TerminatorEventKind::AssertFailure);
}

fn assert_failure_terminator(kind: TerminatorEventKind) {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    let conflict_event = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);
    let failure_event = fixture.terminator(kind);
    let events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([(BlockId::new(0), events)]),
        Vec::new(),
        BlockId::new(0),
        vec![BlockId::new(0)],
    );

    let trace = conflict_trace(run(problem));
    assert_eq!(
        trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_event),
        Some(conflict_event)
    );
    assert_eq!(
        trace.entries.last().map(|entry| entry.event),
        Some(failure_event)
    );
}

#[test]
fn boracle_oracle_unreachable_block_never_executes_or_keeps_capability_live() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    let mut entry_events = fixture.take_events();
    fixture.terminator(TerminatorEventKind::Return);
    entry_events.extend(fixture.take_events());

    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    fixture.access(owner, UseKind::Write, false);
    fixture.terminator(TerminatorEventKind::Return);
    let unreachable_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (BlockId::new(1), unreachable_events),
        ]),
        Vec::new(),
        BlockId::new(0),
        vec![BlockId::new(0), BlockId::new(1)],
    );
    // The reachable entry creates no generation. Executing the unreachable block would hit this
    // zero generation budget at its Fresh event instead of completing safely.
    assert!(matches!(
        run_with_bounds(problem, OracleBounds::new(256, 4096, 8, 0)),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_repeated_loan_issue_is_independent_and_kill_scoped() {
    let mut killed_fixture = Fixture::new(2);
    let owner = killed_fixture.place(0, []);
    let holder = killed_fixture.place(1, []);
    let body = BlockId::new(1);
    let exit = BlockId::new(2);
    killed_fixture.fresh(owner);
    killed_fixture.fresh(holder);
    let mut entry_events = killed_fixture.take_events();
    killed_fixture.jump(body);
    entry_events.extend(killed_fixture.take_events());

    let loan = killed_fixture.loan_issue(owner, holder, AccessKind::Shared);
    killed_fixture.loan_kill(loan);
    killed_fixture.jump(body);
    let body_events = killed_fixture.take_events();
    killed_fixture.terminator(TerminatorEventKind::Return);
    let exit_events = killed_fixture.take_events();
    let killed_problem = killed_fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (body, body_events),
            (exit, exit_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, body),
        ],
        BlockId::new(0),
        vec![exit],
    );

    // Two executions of the static issue and kill events prove that each iteration gets a fresh
    // capability: reusing the ended capability would make the second kill a CompilerError.
    match run_with_bounds(killed_problem, OracleBounds::new(256, 4096, 2, 4096)) {
        OracleOutcome::Inconclusive {
            reason:
                OracleLimitReason::BlockEntryBound {
                    block: bounded_block,
                    limit,
                },
            explored,
            ..
        } => {
            assert_eq!(bounded_block, body);
            assert_eq!(limit, 2);
            assert_eq!(explored, 9);
        }
        outcome => panic!("unexpected killed-loop outcome: {outcome:?}"),
    }
    let mut ended_fixture = Fixture::new(4);
    let ended_owner = ended_fixture.place(0, []);
    let ended_holder = ended_fixture.place(1, []);
    let ended_conflict_owner = ended_fixture.place(2, []);
    let ended_conflict_alias = ended_fixture.place(3, []);
    let ended_body = BlockId::new(1);
    let ended_exit = BlockId::new(2);
    ended_fixture.fresh(ended_owner);
    ended_fixture.fresh(ended_holder);
    ended_fixture.fresh(ended_conflict_owner);
    let mut ended_entry_events = ended_fixture.take_events();
    ended_fixture.jump(ended_body);
    ended_entry_events.extend(ended_fixture.take_events());

    let ended_loan = ended_fixture.loan_issue(ended_owner, ended_holder, AccessKind::Shared);
    let ended_kill = ended_fixture.loan_kill(ended_loan);
    ended_fixture.jump(ended_exit);
    let ended_body_events = ended_fixture.take_events();
    ended_fixture.alias(
        ended_conflict_alias,
        ended_conflict_owner,
        AccessKind::Shared,
    );
    ended_fixture.access(ended_conflict_alias, UseKind::Read, false);
    let ended_conflict = ended_fixture.access_event(ended_conflict_owner, UseKind::Write, false);
    ended_fixture.access(ended_conflict_alias, UseKind::Read, false);
    ended_fixture.terminator(TerminatorEventKind::Return);
    let ended_exit_events = ended_fixture.take_events();
    let ended_problem = ended_fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), ended_entry_events),
            (ended_body, ended_body_events),
            (ended_exit, ended_exit_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), ended_body),
            CfgEdge::new(ended_body, ended_exit),
        ],
        BlockId::new(0),
        vec![ended_exit],
    );
    let ended_trace = conflict_trace(run(ended_problem));
    assert_eq!(
        ended_trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_event),
        Some(ended_conflict)
    );
    let ended_capability = ended_trace
        .capabilities
        .iter()
        .find(|capability| capability.source == CapabilitySource::Loan(ended_loan))
        .expect("the killed loan should issue a capability");
    let kill_index = ended_trace
        .entries
        .iter()
        .find(|entry| entry.event == ended_kill)
        .map(|entry| entry.index);
    assert_eq!(ended_capability.explicit_end, kill_index);
    assert_eq!(
        ended_capability.end_reason,
        Some(CapabilityEndReason::LoanKill)
    );

    let mut live_fixture = Fixture::new(4);
    let owner = live_fixture.place(0, []);
    let holder = live_fixture.place(1, []);
    let conflict_owner = live_fixture.place(2, []);
    let conflict_alias = live_fixture.place(3, []);
    let exit = BlockId::new(1);
    let body = BlockId::new(2);
    live_fixture.fresh(owner);
    live_fixture.fresh(holder);
    live_fixture.fresh(conflict_owner);
    let mut entry_events = live_fixture.take_events();
    live_fixture.jump(body);
    entry_events.extend(live_fixture.take_events());

    let loan = live_fixture.loan_issue(owner, holder, AccessKind::Shared);
    live_fixture.branch([exit, body]);
    let body_events = live_fixture.take_events();
    live_fixture.alias(conflict_alias, conflict_owner, AccessKind::Shared);
    live_fixture.access(conflict_alias, UseKind::Read, false);
    let conflict_event = live_fixture.access_event(conflict_owner, UseKind::Write, false);
    live_fixture.access(conflict_alias, UseKind::Read, false);
    live_fixture.terminator(TerminatorEventKind::Return);
    let exit_events = live_fixture.take_events();
    let live_problem = live_fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (exit, exit_events),
            (body, body_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, exit),
            CfgEdge::new(body, body),
        ],
        BlockId::new(0),
        vec![exit],
    );

    let trace = conflict_trace(run(live_problem));
    assert_eq!(
        trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_event),
        Some(conflict_event)
    );
    let live_capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.source == CapabilitySource::Loan(loan))
        .expect("the un-killed loan iteration should issue a capability");
    assert_eq!(live_capability.explicit_end, None);
}

#[test]
fn boracle_oracle_live_loan_conflict_survives_expired_call_argument() {
    let mut fixture = Fixture::new(3);
    let argument_place = fixture.place(0, []);
    let observation = fixture.place(1, []);
    let support_holder = fixture.place(2, []);
    let exit = BlockId::new(1);
    let body = BlockId::new(2);
    let gap = BlockId::new(3);
    fixture.fresh(argument_place);
    fixture.fresh(support_holder);
    fixture.rebind_alias_from_place(observation, argument_place);
    let mut entry_events = fixture.take_events();
    fixture.jump(body);
    entry_events.extend(fixture.take_events());

    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "repeated-effect".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let effect_event = fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));
    fixture.jump(gap);
    let body_events = fixture.take_events();
    let support_loan = fixture.loan_issue(argument_place, support_holder, AccessKind::Exclusive);

    let observation_access = fixture.access_event(observation, UseKind::Read, false);
    fixture.access(support_holder, UseKind::Read, false);
    fixture.branch([exit, body]);
    let gap_events = fixture.take_events();

    fixture.terminator(TerminatorEventKind::Return);
    let exit_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (exit, exit_events),
            (body, body_events),
            (gap, gap_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, gap),
            CfgEdge::new(gap, exit),
            CfgEdge::new(gap, body),
        ],
        BlockId::new(0),
        vec![exit],
    );

    // The observation is after the call effect. Its conflict must come from the independent loan,
    // whose holder use keeps that capability live, rather than from the expired call argument.
    let trace = match run_with_bounds(problem, OracleBounds::new(256, 4096, 8, 4096)) {
        OracleOutcome::RuntimeConflict { trace } => trace,
        outcome => panic!("unexpected live-loan outcome: {outcome:?}"),
    };
    let conflict = trace
        .conflict
        .as_ref()
        .expect("a conflict outcome carries its witness");
    assert_eq!(conflict.access_event, observation_access);
    assert_eq!(conflict.access_index, 8);
    assert_eq!(conflict.access_kind, AccessKind::Shared);
    assert_eq!(conflict.capability_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_issue, 7);
    let loan_capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.source == CapabilitySource::Loan(support_loan))
        .expect("the supporting loan should issue a capability");
    assert_eq!(loan_capability.issue_index, 7);
    assert_eq!(loan_capability.last_exercised, 9);
    let call_capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.issue_event == argument_event)
        .expect("the call argument should issue a capability");
    assert_eq!(
        call_capability.call_effect_index,
        Some(effect_event.index())
    );
    assert_eq!(call_capability.last_exercised, effect_event.index());
}

/// A call argument capability's interval reaches the CallEffect of its own call and ends there.
/// The effect is both floor and ceiling, so a second invocation of the same call must be a fresh
/// dynamic instance while the first invocation's capability stays folded onto the first effect.
///
/// The fixture owes its conflict-free first pass to ordering. Each pass issues a shared loan on
/// the argument place after the effect and exercises it through the observation read, so the
/// first pass completes clean and the second invocation's exclusive argument meets the still
/// open loan from the first pass. A conflict on the first pass would end the exploration before
/// the loop edge, which is what hid repeated invocation coverage before this test existed.
#[test]
fn boracle_oracle_second_call_invocation_issues_fresh_argument_capabilities() {
    let mut fixture = Fixture::new(2);
    let argument_place = fixture.place(0, []);
    let observation = fixture.place(1, []);
    let exit = BlockId::new(1);
    let body = BlockId::new(2);
    let gap = BlockId::new(3);
    fixture.fresh(argument_place);
    fixture.rebind_alias_from_place(observation, argument_place);
    let mut entry_events = fixture.take_events();
    fixture.jump(body);
    entry_events.extend(fixture.take_events());

    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "second-invocation".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let effect_event = fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));
    fixture.jump(gap);
    let body_events = fixture.take_events();
    let loan = fixture.loan_issue(argument_place, observation, AccessKind::Shared);
    fixture.access(observation, UseKind::Read, false);
    fixture.branch([exit, body]);
    let gap_events = fixture.take_events();

    fixture.terminator(TerminatorEventKind::Return);
    let exit_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (exit, exit_events),
            (body, body_events),
            (gap, gap_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, gap),
            CfgEdge::new(gap, exit),
            CfgEdge::new(gap, body),
        ],
        BlockId::new(0),
        vec![exit],
    );

    // The conflict surfaces at the second invocation's argument access, whose trace carries both
    // passes. The first invocation's capability stays capped at the first effect and the loan from
    // the first gap is what survives to meet it.
    let trace = conflict_trace(run(problem.clone()));
    assert_eq!(
        trace.block_entries.get(&body),
        Some(&2),
        "the conflicting execution must run both invocations"
    );
    let argument_entries = trace
        .entries
        .iter()
        .filter(|entry| entry.event == argument_event)
        .map(|entry| entry.index)
        .collect::<Vec<_>>();
    assert_eq!(argument_entries, vec![3, 9]);

    // The conflict is the outstanding shared loan from the first gap, never the expired argument
    // capability: a shared witness is only possible because the exclusive argument interval ended
    // at the first effect.
    let conflict = trace
        .conflict
        .as_ref()
        .expect("a conflict outcome carries its witness");
    assert_eq!(conflict.access_event, argument_event);
    assert_eq!(conflict.access_index, argument_entries[1]);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let outstanding_loan = trace
        .capabilities
        .iter()
        .find(|capability| {
            capability.source == CapabilitySource::Loan(loan) && capability.issue_index == 6
        })
        .expect("the first pass should issue the outstanding loan");
    assert_eq!(conflict.capability_issue, outstanding_loan.issue_index);

    // The trace snapshots capabilities in id order, so the position doubles as the capability id.
    let call_argument_rows = trace
        .capabilities
        .iter()
        .enumerate()
        .filter(|(_, capability)| {
            capability.source == CapabilitySource::CallArgument(CallId::new(0))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        call_argument_rows.len(),
        2,
        "both invocations should issue their own capability"
    );
    let (first_id, first_capability) = call_argument_rows[0];
    let (second_id, second_capability) = call_argument_rows[1];

    assert_eq!(first_capability.issue_index, 3);
    assert_eq!(
        first_capability.call_effect_index,
        Some(effect_event.index())
    );
    assert_eq!(
        first_capability.last_exercised,
        effect_event.index(),
        "the first invocation's capability must stay capped at the first effect across the loop"
    );

    let effect_entries = trace
        .entries
        .iter()
        .filter(|entry| entry.event == effect_event)
        .map(|entry| entry.index)
        .collect::<Vec<_>>();
    assert_eq!(effect_entries, vec![4, 10]);
    let second_effect_index = effect_entries[1];
    assert_eq!(second_capability.issue_index, argument_entries[1]);
    assert_eq!(second_capability.issue_event, argument_event);
    assert_eq!(
        second_capability.call_effect_index,
        Some(second_effect_index),
        "the second invocation's fresh capability must reach the second effect"
    );
    assert_eq!(second_capability.last_exercised, second_effect_index);

    let second_access = trace.entries[argument_entries[1]]
        .access
        .as_ref()
        .expect("the second argument access should be traced");
    assert!(
        !second_access
            .exercised
            .iter()
            .any(|id| id.raw() == first_id as u32),
        "the first invocation's capability must be absent from the second argument access"
    );
    assert_eq!(
        second_access
            .exercised
            .iter()
            .map(|id| id.raw())
            .collect::<Vec<_>>(),
        vec![second_id as u32],
        "the second argument access must exercise its own fresh capability only"
    );

    // The first pass is clean, so the enumeration reaches the loop continuation instead of
    // resolving on the exit path, and a body bound of one truncates exactly there. `explored`
    // counts dispatched events: the ten events of the clean first pass, with the refused loop
    // continuation contributing none.
    match run_with_bounds(problem, OracleBounds::new(256, 4096, 1, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::BlockEntryBound { block, limit },
            explored,
            ..
        } => {
            assert_eq!(block, body);
            assert_eq!(limit, 1);
            assert_eq!(explored, 10);
        }
        outcome => panic!("unexpected bounded second-invocation outcome: {outcome:?}"),
    }
}

/// The exercise guard is outcome-masked by the interval cap, so its contract lives in the
/// recorded exercise bookkeeping. This probes `exercise_capabilities` directly: a call argument
/// capability whose effect has run must be skipped, leaving `last_exercised` at the effect, while
/// a live capability over the same place stays exercisable at the same late index.
#[test]
fn boracle_oracle_exercise_capabilities_skip_expired_call_argument() {
    let mut fixture = Fixture::new(1);
    let argument_place = fixture.place(0, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "expired-argument".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let effect_event = fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));
    let problem = fixture.finish();
    let effect_index = effect_event.index();

    let mut state = OracleState::new(&problem);
    let generation = state
        .issue_generation(1)
        .expect("the fixture needs one runtime generation");
    state.set_state(
        argument_place,
        RuntimePlaceState::Slot {
            current: generation,
        },
    );
    let target = RuntimeAccessTarget {
        node: generation,
        path: Box::new([]),
    };
    let argument_capability = state
        .issue_capability(
            AccessKind::Exclusive,
            target.clone(),
            BTreeSet::from([argument_place]),
            argument_event.index(),
            argument_event,
            CapabilitySource::CallArgument(CallId::new(0)),
        )
        .expect("the call argument capability should issue");
    state.extend_call_capabilities(CallId::new(0), effect_index);
    let loan_capability = state
        .issue_capability(
            AccessKind::Exclusive,
            target.clone(),
            BTreeSet::from([argument_place]),
            effect_index,
            effect_event,
            CapabilitySource::Loan(LoanId::new(0)),
        )
        .expect("the loan capability should issue");

    let post_effect = effect_index + 3;
    let exercised = exercise_capabilities(
        &problem,
        &mut state,
        argument_place,
        &target,
        post_effect,
        None,
    )
    .expect("the live loan should stay exercisable past the call effect");
    assert_eq!(
        exercised.as_ref(),
        &[loan_capability],
        "an access after the effect must not exercise the expired call argument capability"
    );
    assert_eq!(
        state
            .capabilities
            .get(&argument_capability)
            .expect("the expired capability should remain recorded")
            .last_exercised,
        effect_index,
        "a post-effect access must leave the expired call argument capability's last exercise at the effect"
    );
}

/// The interval scan owns the conflict outcome. Even if a post-effect exercise ever drags
/// `last_exercised` past the recorded effect (which the exercise guard exists to prevent), the
/// clamp in `interval_end` must fold the ceiling back onto the effect so the legal later access is
/// never reported as a conflict.
#[test]
fn boracle_oracle_call_effect_ceiling_clamps_dragged_call_argument_interval() {
    let mut fixture = Fixture::new(1);
    let argument_place = fixture.place(0, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "ceiling".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let effect_event = fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));
    let problem = fixture.finish();
    let effect_index = effect_event.index();

    let mut state = OracleState::new(&problem);
    let generation = state
        .issue_generation(1)
        .expect("the fixture needs one runtime generation");
    state.set_state(
        argument_place,
        RuntimePlaceState::Slot {
            current: generation,
        },
    );
    let target = RuntimeAccessTarget {
        node: generation,
        path: Box::new([]),
    };
    let capability = state
        .issue_capability(
            AccessKind::Exclusive,
            target.clone(),
            BTreeSet::from([argument_place]),
            argument_event.index(),
            argument_event,
            CapabilitySource::CallArgument(CallId::new(0)),
        )
        .expect("the call argument capability should issue");
    state.extend_call_capabilities(CallId::new(0), effect_index);

    // Simulate the exercise the guard prevents: a later access drags `last_exercised` past the
    // recorded effect before any scan sees the capability.
    let post_effect = effect_index + 3;
    state
        .capabilities
        .get_mut(&capability)
        .expect("the simulated capability should be recorded")
        .last_exercised = post_effect;
    assert_eq!(
        state
            .capabilities
            .get(&capability)
            .expect("the simulated capability should be recorded")
            .interval_end(),
        effect_index,
        "the recorded call effect must clamp the interval ceiling"
    );

    let dragged_entry = TraceEntry {
        index: post_effect - 1,
        event: argument_event,
        point: PointId::new(0),
        block: BlockId::new(0),
        access: Some(TraceAccess {
            place: argument_place,
            kind: AccessKind::Shared,
            target: target.clone(),
            definition: false,
            exercised: Box::new([]),
        }),
        issued_capabilities: Box::new([]),
        ended_capabilities: Box::new([]),
    };
    let scan = find_interval_conflict(&[dragged_entry], &state.capabilities)
        .expect("the fixture should not hold an undecidable pair");
    assert!(
        scan.is_none(),
        "the call effect ceiling must clamp the dragged interval: {scan:?}"
    );
}

#[test]
fn boracle_oracle_call_effect_respects_explicit_end() {
    let mut fixture = Fixture::new(3);
    let argument_place = fixture.place(0, []);
    let conflict_owner = fixture.place(1, []);
    let conflict_alias = fixture.place(2, []);
    fixture.fresh(argument_place);
    fixture.fresh(conflict_owner);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "ended-argument".to_string(),
    });
    let (argument_event, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Exclusive);
    let scope_exit_event = fixture.scope_exit([BindingId::new(0)]);
    let effect_event = fixture.event(EventKind::CallEffect(CallEffect {
        call: CallId::new(0),
        arguments: vec![argument].into_boxed_slice(),
        result: None,
    }));
    fixture.alias(conflict_alias, conflict_owner, AccessKind::Shared);
    fixture.access(conflict_alias, UseKind::Read, false);
    let conflict_event = fixture.access_event(conflict_owner, UseKind::Write, false);
    fixture.access(conflict_alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([(BlockId::new(0), events)]),
        Vec::new(),
        BlockId::new(0),
        vec![BlockId::new(0)],
    );
    let trace = conflict_trace(run(problem));
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.source == CapabilitySource::CallArgument(CallId::new(0)))
        .expect("call argument should issue a capability");
    assert_eq!(capability.explicit_end, Some(scope_exit_event));
    assert_eq!(
        capability.end_reason,
        Some(CapabilityEndReason::HolderRetired)
    );
    assert_eq!(capability.call_effect_index, Some(effect_event.index()));
    assert_eq!(capability.last_exercised, capability.issue_index);
    assert_eq!(
        trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_event),
        Some(conflict_event)
    );
    assert!(
        trace
            .entries
            .iter()
            .any(|entry| entry.event == argument_event)
    );
}

#[test]
fn boracle_oracle_defining_write_through_shared_alias_conflicts_at_the_write() {
    // A definition write to a shared-alias-backed place is a write-through whose paired access
    // is reclassified as an ordinary mutation (`loans.rs:227-234`), so it stays under the
    // direct rule. Only a confirming definition for a pending call result is exempt, and this
    // access is not one.
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    let defining_write = fixture.access_event(alias, UseKind::Write, true);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace
        .conflict
        .as_ref()
        .expect("a defining write through a shared alias must conflict directly at that write");
    assert_eq!(conflict.access_event, defining_write);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
}

#[test]
fn boracle_oracle_alias_params_provenance_survives_its_confirming_write() {
    // The real builder shape: `CallEffect` with `AliasParams` binds a fresh result generation
    // and issues provenance capabilities, then the builder emits the confirming defining
    // access for the result place. That confirmation must retire neither the bound generation
    // nor the provenance capabilities, or every later use degrades into a retired-holder
    // exercise and the argument mutation below reports safe.
    let mut fixture = Fixture::new(2);
    let argument_place = fixture.place(0, []);
    let result_place = fixture.place(1, []);
    fixture.fresh(argument_place);
    fixture.parts.calls.push(Call {
        id: CallId::new(0),
        label: "confirmed-alias-result".to_string(),
    });
    let (_, argument) =
        fixture.call_argument_at(CallId::new(0), 0, argument_place, AccessKind::Shared);
    fixture.call_effect_result(
        CallId::new(0),
        vec![argument],
        result_place,
        CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
    );
    fixture.access(result_place, UseKind::Write, true);
    let argument_mutation = fixture.access_event(argument_place, UseKind::Write, false);
    fixture.access(result_place, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));
    let conflict = trace.conflict.as_ref().expect(
        "provenance capabilities issued by AliasParams must survive their confirming write \
         and witness the argument mutation",
    );
    assert_eq!(conflict.access_event, argument_mutation);
    assert_eq!(conflict.access_kind, AccessKind::Exclusive);
    assert_eq!(conflict.capability_kind, AccessKind::Shared);
    let capability = trace
        .capabilities
        .iter()
        .find(|capability| capability.source == CapabilitySource::Provenance)
        .expect("AliasParams should issue a provenance capability");
    assert_eq!(capability.holders, BTreeSet::from([result_place]));
    assert_eq!(capability.target, conflict.access_target.node);
    assert!(
        capability.explicit_end.is_none(),
        "the confirming write must not retire the provenance capability it confirms: \
         {capability:?}"
    );
}

#[test]
fn boracle_oracle_projection_onto_residual_path_is_typed_inconclusive() {
    // A slot stores a bare generation, so installing `target.node` for a projection whose
    // descent leaves a residual undecidable path would make the destination compare equal to
    // the whole base node and manufacture the definite overlap the contract classifies as
    // UNDECIDABLE. The Copy and Aggregate arms refuse the same shape, so the projection must
    // take the identical typed refusal before installing anything.
    let mut fixture = Fixture::new(2);
    let source = fixture.place(0, []);
    let source_dynamic = fixture.place(0, [ProjectionElem::DynamicIndex]);
    let destination = fixture.place(1, []);
    fixture.fresh(source);
    fixture.projection(source_dynamic, destination, ProjectionElem::Field(0));

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::UndecidableOverlap { left, right },
            ..
        } => {
            assert_eq!(left.node, right.node);
            assert!(
                left.path.as_ref()
                    == [ProjectionElem::DynamicIndex, ProjectionElem::Field(0)].as_slice(),
                "the refusal must carry the residual projection path: {left:?}"
            );
            assert!(right.path.is_empty());
        }
        outcome => panic!(
            "a residual-path projection must refuse typed before installing a slot: {outcome:?}"
        ),
    }
}

#[test]
fn boracle_oracle_rebind_alias_from_place_with_residual_path_is_typed_inconclusive() {
    // The rebind's slot generation is `resolve(place).node` by contract, so a source that
    // keeps a residual undecidable path has no generation to hand over. Collapsing it onto
    // the base node would fabricate the definite overlap the contract calls UNDECIDABLE.
    let mut fixture = Fixture::new(2);
    let source = fixture.place(0, []);
    let source_dynamic = fixture.place(0, [ProjectionElem::DynamicIndex]);
    let destination = fixture.place(1, []);
    fixture.fresh(source);
    fixture.rebind_alias_from_place(destination, source_dynamic);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::UndecidableOverlap { left, right },
            ..
        } => {
            assert_eq!(left.node, right.node);
            assert_eq!(
                left.path.as_ref(),
                [ProjectionElem::DynamicIndex].as_slice()
            );
            assert!(right.path.is_empty());
        }
        outcome => panic!(
            "a residual-path rebind source must refuse typed before installing a slot: \
             {outcome:?}"
        ),
    }
}

#[test]
fn boracle_oracle_alias_replacing_slot_with_residual_source_is_typed_inconclusive() {
    // An unavailable alias destination stores the alias with its full residual path, so that
    // case stays faithful. A slot-backed destination replaces through
    // `DefinitionRole::slot_target`, which drops the path, so the same source must refuse
    // instead of collapsing.
    let mut fixture = Fixture::new(2);
    let source = fixture.place(0, []);
    let source_dynamic = fixture.place(0, [ProjectionElem::DynamicIndex]);
    let destination = fixture.place(1, []);
    fixture.fresh(source);
    fixture.fresh(destination);
    fixture.alias(destination, source_dynamic, AccessKind::Shared);

    match run(fixture.finish()) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::UndecidableOverlap { left, right },
            ..
        } => {
            assert_eq!(left.node, right.node);
            assert_eq!(
                left.path.as_ref(),
                [ProjectionElem::DynamicIndex].as_slice()
            );
            assert!(right.path.is_empty());
        }
        outcome => panic!(
            "an alias onto a slot-backed destination must refuse when its source keeps a \
             residual undecidable path: {outcome:?}"
        ),
    }

    // The contrast: the same residual alias onto an unavailable destination installs with the
    // path intact, so the alias capability still names the projected position and a mutation
    // of the base stays a direct conflict whose witness carries that path.
    let mut faithful = Fixture::new(2);
    let faithful_source = faithful.place(0, []);
    let faithful_dynamic = faithful.place(0, [ProjectionElem::DynamicIndex]);
    let faithful_alias = faithful.place(1, []);
    faithful.fresh(faithful_source);
    faithful.alias(faithful_alias, faithful_dynamic, AccessKind::Shared);
    let direct = faithful.access_event(faithful_alias, UseKind::Write, false);
    let trace = conflict_trace(run(faithful.finish()));
    let conflict = trace.conflict.as_ref().unwrap_or_else(|| {
        panic!("the faithful residual alias must carry its path into the witness")
    });
    assert_eq!(conflict.access_event, direct);
    assert_eq!(
        conflict.capability_target.path.as_ref(),
        [ProjectionElem::DynamicIndex].as_slice(),
        "an installed alias must keep its residual path instead of collapsing onto the base \
         node: {conflict:?}"
    );
}

#[test]
fn boracle_oracle_bare_slot_rebind_retires_overlapping_projection_holder() {
    // The generated corpus emits a provenance capability held by a projected child place and
    // then rebinds the root with no paired defining access. The transition's slot row must
    // retire every holder that structurally overlaps the destination (`holder_kills`,
    // `loans.rs:790-805`), so the stale capability cannot be dragged across the source
    // mutation by the later holder read and fabricate a conflict.
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let owner_field = fixture.place(0, [ProjectionElem::Field(0)]);
    let source = fixture.place(1, []);
    fixture.fresh(source);
    fixture.aggregate(owner, [(ProjectionElem::Field(0), source)]);
    fixture.access(owner_field, UseKind::Read, false);
    fixture.rebind_fresh(owner);
    let source_mutation = fixture.access_event(source, UseKind::Write, false);
    fixture.access(owner_field, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::CompleteSafe { executions: 1, .. } => {}
        outcome => panic!(
            "a bare rebind of the root must end the capability held by the covered \
             projection instead of surviving the source mutation: {outcome:?}, write: \
             {source_mutation:?}"
        ),
    }
}

#[test]
fn boracle_oracle_install_definition_retires_overlapping_holder_on_unavailable_destination() {
    // The installed row retires overlapping holders too: a capability held by a covered
    // projection dies even though the destination itself was unavailable and held nothing.
    let mut fixture = Fixture::new(2);
    let value = fixture.place(0, []);
    let value_field = fixture.place(0, [ProjectionElem::Field(0)]);
    let source = fixture.place(1, []);
    fixture.fresh(source);
    // The alias installs on the covered projection of the never-initialized parent, so its
    // capability is held by `value_field` while `value` itself is unavailable. The trailing
    // exclusive source mutation is the non-holder probe: if the installed definition of
    // `value` skipped its overlapping-holder retirement, the second holder read below drags
    // that shared capability across the mutation and fabricates a conflict.
    fixture.alias(value_field, source, AccessKind::Shared);
    fixture.access(value_field, UseKind::Read, false);
    fixture.fresh(value);
    let source_mutation = fixture.access_event(source, UseKind::Write, false);
    fixture.access(value_field, UseKind::Read, false);

    match run(fixture.finish()) {
        OracleOutcome::CompleteSafe { executions: 1, .. } => {}
        outcome => panic!(
            "the installed row must retire the holder on the covered projection of its \
             unavailable destination: {outcome:?}, write: {source_mutation:?}"
        ),
    }
}

#[test]
fn boracle_oracle_reactive_observe_is_metadata_only_and_never_materialises() {
    // The contract makes `ReactiveObserve` metadata-only: no capability, no access check and
    // no conflict, and the place must be initialized. A materialising availability check
    // would create a generation for a missing Field under a tight bound and turn a complete
    // safe execution into GenerationBound, so the bound here is exactly the count the
    // execution's real writer consumes.
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    let observed = fixture.place(0, [ProjectionElem::Field(7)]);
    fixture.fresh(place);
    fixture.reactive_observe(observed);

    match run_with_bounds(fixture.finish(), OracleBounds::new(256, 4096, 8, 1)) {
        OracleOutcome::CompleteSafe { executions: 1, .. } => {}
        outcome => panic!(
            "a metadata-only observation must not materialise graph state or consume the \
             generation bound: {outcome:?}"
        ),
    }
}

#[test]
fn boracle_oracle_reactive_observe_of_uninitialized_place_is_compiler_error() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    let observed = fixture.place(0, [ProjectionElem::Field(7)]);
    fixture.reactive_observe(observed);
    fixture.fresh(place);

    let error = execute_bounded(&fixture.finish(), OracleBounds::default())
        .expect_err("the observed place itself must stay initialized");
    assert!(error.msg.contains("reactive observation"), "{error:?}");
}

fn safe_and_closed_cycle_problem() -> BorrowProblem {
    let mut fixture = Fixture::new(0);
    let safe = BlockId::new(1);
    let loop_block = BlockId::new(2);
    fixture.jump(loop_block);
    let entry_events = fixture.take_events();

    fixture.terminator(TerminatorEventKind::Return);
    let safe_events = fixture.take_events();

    fixture.branch([safe, loop_block]);
    let loop_events = fixture.take_events();

    fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (safe, safe_events),
            (loop_block, loop_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), loop_block),
            CfgEdge::new(loop_block, loop_block),
            CfgEdge::new(loop_block, safe),
        ],
        BlockId::new(0),
        vec![safe],
    )
}

fn scope_exit_closed_cycle_problem() -> BorrowProblem {
    let mut fixture = Fixture::new(1);
    fixture.place(0, []);
    let safe = BlockId::new(1);
    let loop_block = BlockId::new(2);
    fixture.jump(loop_block);
    let entry_events = fixture.take_events();

    fixture.scope_exit([BindingId::new(0)]);
    fixture.branch([safe, loop_block]);
    let loop_events = fixture.take_events();

    fixture.terminator(TerminatorEventKind::Return);
    let safe_events = fixture.take_events();

    fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (safe, safe_events),
            (loop_block, loop_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), loop_block),
            CfgEdge::new(loop_block, loop_block),
            CfgEdge::new(loop_block, safe),
        ],
        BlockId::new(0),
        vec![safe],
    )
}

#[test]
fn boracle_oracle_scope_exit_closed_cycle_is_non_terminating_cycle() {
    match run_with_bounds(
        scope_exit_closed_cycle_problem(),
        OracleBounds::new(256, 4096, 8, 4096),
    ) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::NonTerminatingCycle { block },
            ..
        } => assert_eq!(block, BlockId::new(2)),
        outcome => panic!("scope-exit cycle should be reported as a closed cycle: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_closed_cycle_precedes_exhausted_execution_bound() {
    match run_with_bounds(
        safe_and_closed_cycle_problem(),
        OracleBounds::new(1, 4096, 8, 4096),
    ) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::NonTerminatingCycle { block },
            explored,
            ..
        } => {
            assert_eq!(block, BlockId::new(2));
            assert_eq!(explored, 3);
        }
        outcome => panic!("unexpected cycle-precedence outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_safe_branch_and_closed_cycle_is_inconclusive() {
    match run_with_bounds(
        safe_and_closed_cycle_problem(),
        OracleBounds::new(256, 4096, 8, 4096),
    ) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::NonTerminatingCycle { block },
            explored,
            ..
        } => {
            assert_eq!(block, BlockId::new(2));
            assert_eq!(explored, 3);
        }
        outcome => panic!("safe and cyclic arms must not be reported safe: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_non_terminating_cycle_is_inconclusive() {
    let problem = Fixture::new(0).finish_reentered();
    match run_with_bounds(problem, OracleBounds::new(256, 4096, 64, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::NonTerminatingCycle { block },
            explored,
            ..
        } => {
            assert_eq!(block, BlockId::new(1));
            assert_eq!(explored, 2);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_bound_changes_preserve_complete_outcomes() {
    let mut safe_fixture = Fixture::new(0);
    let mut safe_entry_events = safe_fixture.take_events();
    safe_fixture.branch([BlockId::new(1), BlockId::new(2)]);
    safe_entry_events.extend(safe_fixture.take_events());
    safe_fixture.terminator(TerminatorEventKind::Return);
    let safe_first_events = safe_fixture.take_events();
    safe_fixture.terminator(TerminatorEventKind::Return);
    let safe_second_events = safe_fixture.take_events();
    let safe_problem = safe_fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), safe_entry_events),
            (BlockId::new(1), safe_first_events),
            (BlockId::new(2), safe_second_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(0), BlockId::new(2)),
        ],
        BlockId::new(0),
        vec![BlockId::new(1), BlockId::new(2)],
    );
    let safe_default = run(safe_problem.clone());
    let safe_expanded = run_with_bounds(safe_problem, OracleBounds::new(2048, 32768, 64, 32768));
    assert_eq!(safe_default, safe_expanded);

    let mut conflict_fixture = Fixture::new(2);
    let owner = conflict_fixture.place(0, []);
    let alias = conflict_fixture.place(1, []);
    conflict_fixture.fresh(owner);
    conflict_fixture.alias(alias, owner, AccessKind::Shared);
    conflict_fixture.access(alias, UseKind::Read, false);
    conflict_fixture.access(owner, UseKind::Write, false);
    conflict_fixture.access(alias, UseKind::Read, false);
    let conflict_problem = conflict_fixture.finish();
    let conflict_default = run(conflict_problem.clone());
    let conflict_expanded =
        run_with_bounds(conflict_problem, OracleBounds::new(2048, 32768, 64, 32768));
    assert_eq!(conflict_default, conflict_expanded);

    let mut truncating_fixture = Fixture::new(1);
    let value = truncating_fixture.place(0, []);
    truncating_fixture.fresh(value);
    truncating_fixture.access(value, UseKind::Read, false);
    let truncating_problem = truncating_fixture.finish();
    let truncating = run_with_bounds(
        truncating_problem.clone(),
        OracleBounds::new(256, 2, 8, 4096),
    );
    assert!(matches!(
        truncating,
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::EventBound { limit: 2 },
            explored: 2,
            ..
        }
    ));
    let truncating_default = run(truncating_problem.clone());
    assert!(matches!(
        &truncating_default,
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
    let truncating_expanded = run_with_bounds(
        truncating_problem,
        OracleBounds::new(2048, 32768, 64, 32768),
    );
    assert_eq!(truncating_default, truncating_expanded);
}

#[test]
fn boracle_oracle_deterministic_path_order_selects_lowest_conflict() {
    let mut fixture = Fixture::new(3);
    let owner = fixture.place(0, []);
    let lower_alias = fixture.place(1, []);
    let higher_alias = fixture.place(2, []);
    let lower = BlockId::new(1);
    let higher = BlockId::new(2);
    fixture.fresh(owner);
    let mut entry_events = fixture.take_events();
    // BorrowProblem validation requires branch targets to be ascending. Keep that semantic list,
    // but make the independent CFG edge and event declaration order adversarial instead.
    fixture.branch([lower, higher]);
    entry_events.extend(fixture.take_events());

    fixture.alias(higher_alias, owner, AccessKind::Shared);
    fixture.access(higher_alias, UseKind::Read, false);
    let higher_conflict = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(higher_alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let higher_events = fixture.take_events();

    fixture.alias(lower_alias, owner, AccessKind::Shared);
    fixture.access(lower_alias, UseKind::Read, false);
    let lower_conflict = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(lower_alias, UseKind::Read, false);
    fixture.terminator(TerminatorEventKind::Return);
    let lower_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (higher, higher_events),
            (BlockId::new(0), entry_events),
            (lower, lower_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), higher),
            CfgEdge::new(BlockId::new(0), lower),
        ],
        BlockId::new(0),
        vec![lower, higher],
    );

    let first = conflict_trace(run(problem.clone()));
    let second = conflict_trace(run(problem));
    let conflict = first
        .conflict
        .as_ref()
        .expect("lower arm should provide the first conflict");
    assert_eq!(conflict.access_event, lower_conflict);
    assert_ne!(conflict.access_event, higher_conflict);
    assert_eq!(
        first
            .entries
            .iter()
            .find(|entry| entry.event == lower_conflict)
            .map(|entry| entry.block),
        Some(lower)
    );
    assert_eq!(first.debug_dump(), second.debug_dump());
}

#[test]
fn boracle_oracle_execution_bound_is_typed_inconclusive() {
    let fixture = Fixture::new(0);
    match run_with_bounds(
        fixture.finish_branch([BlockId::new(1), BlockId::new(2)]),
        OracleBounds::new(1, 4096, 8, 4096),
    ) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::ExecutionBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 1);
            assert_eq!(explored, 2);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_block_entry_bound_is_typed_inconclusive() {
    let mut fixture = Fixture::new(1);
    let owner = fixture.place(0, []);
    let body = BlockId::new(1);
    let exit = BlockId::new(2);
    fixture.jump(body);
    let entry_events = fixture.take_events();

    fixture.fresh(owner);
    fixture.jump(body);
    let body_events = fixture.take_events();

    fixture.terminator(TerminatorEventKind::Return);
    let exit_events = fixture.take_events();

    let problem = fixture.finish_cfg(
        BTreeMap::from([
            (BlockId::new(0), entry_events),
            (body, body_events),
            (exit, exit_events),
        ]),
        vec![
            CfgEdge::new(BlockId::new(0), body),
            CfgEdge::new(body, body),
        ],
        BlockId::new(0),
        vec![exit],
    );
    match run_with_bounds(problem, OracleBounds::new(256, 4096, 2, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::BlockEntryBound { block, limit },
            explored,
            ..
        } => {
            assert_eq!(block, body);
            assert_eq!(limit, 2);
            assert_eq!(explored, 5);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

/// One writer of the transition matrix, dispatched through the real event executor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MatrixWriter {
    FreshValue,
    CopyValue,
    AggregateValue,
    RebindFresh,
    RebindAliasFromPlace,
    CallResultFresh,
    CallResultAliasParams,
    Projection,
    AliasShared,
    AliasExclusive,
    AliasOriginsShared,
    AliasOriginsExclusive,
    MutableParameter,
}

impl MatrixWriter {
    /// Only direct alias and mutable-parameter writers install alias state onto an
    /// unavailable destination. Every other writer is value-producing and installs a slot.
    fn installs_alias(self) -> bool {
        matches!(
            self,
            MatrixWriter::AliasShared
                | MatrixWriter::AliasExclusive
                | MatrixWriter::AliasOriginsShared
                | MatrixWriter::AliasOriginsExclusive
                | MatrixWriter::MutableParameter
        )
    }
}

/// The destination's state before the matrix writer runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MatrixPreState {
    Unavailable,
    Slot,
    AliasShared,
    AliasExclusive,
}

/// Dispatches one matrix writer through the fixture's event stream. Call writers create their
/// own call row and argument event, then confirm the result with a defining access. Ordinary
/// writers receive their defining access first. Alias-origin writers carry a single seed origin.
fn apply_matrix_writer(
    fixture: &mut Fixture,
    writer: MatrixWriter,
    destination: PlaceId,
    definition_place: PlaceId,
    seed_source: PlaceId,
    argument_source: PlaceId,
) {
    if !matches!(
        writer,
        MatrixWriter::CallResultFresh
            | MatrixWriter::CallResultAliasParams
            | MatrixWriter::MutableParameter
    ) {
        fixture.access(definition_place, UseKind::Write, true);
    }
    match writer {
        MatrixWriter::FreshValue => fixture.fresh(destination),
        MatrixWriter::CopyValue => fixture.copy(destination, seed_source),
        MatrixWriter::AggregateValue => {
            fixture.aggregate(destination, [(ProjectionElem::Field(0), seed_source)]);
        }
        MatrixWriter::RebindFresh => fixture.rebind_fresh(destination),
        MatrixWriter::RebindAliasFromPlace => {
            fixture.rebind_alias_from_place(destination, seed_source);
        }
        MatrixWriter::CallResultFresh => {
            fixture.parts.calls.push(Call {
                id: CallId::new(0),
                label: "matrix-call-fresh".to_string(),
            });
            fixture.call_effect_result(
                CallId::new(0),
                Vec::new(),
                destination,
                CallResultProvenance::Fresh,
            );
            fixture.access(definition_place, UseKind::Write, true);
        }
        MatrixWriter::CallResultAliasParams => {
            fixture.parts.calls.push(Call {
                id: CallId::new(0),
                label: "matrix-call-alias-params".to_string(),
            });
            let (_, argument) =
                fixture.call_argument_at(CallId::new(0), 0, argument_source, AccessKind::Shared);
            fixture.call_effect_result(
                CallId::new(0),
                vec![argument],
                destination,
                CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
            );
            fixture.access(definition_place, UseKind::Write, true);
        }
        MatrixWriter::Projection => {
            fixture.projection(seed_source, destination, ProjectionElem::Field(0));
        }
        MatrixWriter::AliasShared => {
            fixture.alias(destination, seed_source, AccessKind::Shared);
        }
        MatrixWriter::AliasExclusive => {
            fixture.alias(destination, seed_source, AccessKind::Exclusive);
        }
        MatrixWriter::AliasOriginsShared => {
            fixture.alias_with_origins(destination, seed_source, AccessKind::Shared);
        }
        MatrixWriter::AliasOriginsExclusive => {
            fixture.alias_with_origins(destination, seed_source, AccessKind::Exclusive);
        }
        MatrixWriter::MutableParameter => {
            let index = fixture.parts.origins.len() as u32;
            let origin = fixture.origin(OriginKind::Parameter { index });
            fixture.event(EventKind::Fresh {
                destination,
                origin,
            });
        }
    }
}

#[test]
fn boracle_oracle_definition_transition_matrix_covers_root_and_projection_places() {
    // The matrix dispatches every writer through the real event executor and decides each cell
    // with suffix probes that separate the four transition rows, so a routing defect cannot hide
    // behind a direct transition call. Every assertion names its destination, pre-state and event
    // cell.
    const MATRIX_WRITERS: &[(&str, MatrixWriter)] = &[
        ("Fresh", MatrixWriter::FreshValue),
        ("Copy", MatrixWriter::CopyValue),
        ("Aggregate", MatrixWriter::AggregateValue),
        ("Rebind Fresh", MatrixWriter::RebindFresh),
        ("Rebind AliasFromPlace", MatrixWriter::RebindAliasFromPlace),
        ("Call Fresh", MatrixWriter::CallResultFresh),
        ("Call AliasParams", MatrixWriter::CallResultAliasParams),
        ("Projection", MatrixWriter::Projection),
        ("Shared Alias", MatrixWriter::AliasShared),
        ("Exclusive Alias", MatrixWriter::AliasExclusive),
        ("Shared Alias Origins", MatrixWriter::AliasOriginsShared),
        (
            "Exclusive Alias Origins",
            MatrixWriter::AliasOriginsExclusive,
        ),
        ("MutableParameter", MatrixWriter::MutableParameter),
    ];
    let matrix_states: &[(&str, MatrixPreState)] = &[
        ("Unavailable", MatrixPreState::Unavailable),
        ("Slot", MatrixPreState::Slot),
        ("Shared Alias", MatrixPreState::AliasShared),
        ("Exclusive Alias", MatrixPreState::AliasExclusive),
    ];

    for (destination_name, projections) in [
        ("root", Vec::new()),
        ("projection", vec![ProjectionElem::Field(0)]),
    ] {
        for (state_name, pre_state) in matrix_states {
            for (event_name, writer) in MATRIX_WRITERS {
                let mut fixture = Fixture::new(5);
                fixture.parts.bindings[1].mutable = true;
                let seed_source = fixture.place(0, []);
                let destination_root = fixture.place(1, []);
                let destination = if projections.is_empty() {
                    destination_root
                } else {
                    fixture.place(1, projections.iter().copied())
                };
                // A projected use cannot carry the normalised definition flag. An isolated
                // unavailable binding keeps the event order under test without retiring a
                // projected holder through a root definition.
                let definition_place = if projections.is_empty() {
                    destination
                } else {
                    fixture.place(4, [])
                };
                let observer = fixture.place(2, []);
                let keeper = fixture.place(3, []);
                fixture.fresh(seed_source);
                fixture.fresh(observer);
                fixture.fresh(keeper);
                // The projection writer resolves an existing child here, so the saturated alias
                // probe measures only generations that a write-through writer creates.
                fixture.aggregate(seed_source, [(ProjectionElem::Field(0), observer)]);

                let cell = format!("{destination_name} × {state_name} × {event_name}");
                match pre_state {
                    MatrixPreState::Unavailable => {
                        apply_matrix_writer(
                            &mut fixture,
                            *writer,
                            destination,
                            definition_place,
                            seed_source,
                            observer,
                        );

                        // The independent loan is held by the destination, so a slot replacement
                        // must retire it while a write-through preserves it. The second alias
                        // gives the following exclusive referent write a target regardless of
                        // whether the matrix writer issued a capability itself.
                        fixture.alias(keeper, destination, AccessKind::Exclusive);
                        fixture.loan_issue(destination, destination, AccessKind::Shared);
                        fixture.rebind_fresh(destination);
                        let referent_write = fixture.access_event(keeper, UseKind::Write, false);
                        fixture.access(destination, UseKind::Read, false);

                        if writer.installs_alias() {
                            let trace = match run_matrix_cell(fixture.finish(), &cell) {
                                OracleOutcome::RuntimeConflict { trace } => trace,
                                outcome => panic!(
                                    "{cell}: an unavailable destination must install the event's \
                                     alias so a later exclusive referent write conflicts: {outcome:?}"
                                ),
                            };
                            let conflict = trace.conflict.as_ref().unwrap_or_else(|| {
                                panic!("{cell}: alias cell must carry its conflict witness")
                            });
                            assert_eq!(
                                conflict.access_event, referent_write,
                                "{cell}: the witness must be the intervening referent write"
                            );
                        } else {
                            assert!(
                                matches!(
                                    run_matrix_cell(fixture.finish(), &cell),
                                    OracleOutcome::CompleteSafe { executions: 1, .. }
                                ),
                                "{cell}: an unavailable value must install only a slot"
                            );
                        }
                    }
                    MatrixPreState::Slot => {
                        // Every cell here must execute its writer, so the old generation stays
                        // reachable behind an independent alias probe: the loan is a
                        // single-holder row on the destination itself, the alias keeps the old
                        // generation readable after the writer replaced the slot, and a holder
                        // read after the stale read drags the loan interval over it when and
                        // only when the replacement failed to retire the destination's held
                        // capability. The post-replacement destination read resolves to the new
                        // generation, so a healthy cell ends the loan at the writer and shows
                        // the stale read as safe; the state-level retirement contract also keeps
                        // its direct probe in
                        // `boracle_oracle_slot_replacement_retires_the_destination_held_capability`.
                        fixture.fresh(destination);
                        fixture.loan_issue(destination, destination, AccessKind::Exclusive);
                        fixture.alias(keeper, destination, AccessKind::Shared);
                        apply_matrix_writer(
                            &mut fixture,
                            *writer,
                            destination,
                            definition_place,
                            seed_source,
                            observer,
                        );
                        fixture.access(keeper, UseKind::Read, false);
                        fixture.access(destination, UseKind::Read, false);

                        match run_matrix_cell(fixture.finish(), &cell) {
                            OracleOutcome::CompleteSafe { executions: 1, .. } => {}
                            OracleOutcome::RuntimeConflict { trace } => panic!(
                                "{cell}: slot replacement must retire the destination-held \
                                 loan before the stale alias read can meet it: {trace:?}"
                            ),
                            outcome => {
                                panic!("{cell}: the slot row must execute its writer: {outcome:?}")
                            }
                        }
                    }
                    MatrixPreState::AliasShared | MatrixPreState::AliasExclusive => {
                        let seeded_access = match pre_state {
                            MatrixPreState::AliasShared => AccessKind::Shared,
                            MatrixPreState::AliasExclusive => AccessKind::Exclusive,
                            MatrixPreState::Unavailable | MatrixPreState::Slot => unreachable!(),
                        };
                        let seeded_alias = fixture.alias(destination, seed_source, seeded_access);
                        apply_matrix_writer(
                            &mut fixture,
                            *writer,
                            destination,
                            definition_place,
                            seed_source,
                            observer,
                        );
                        let referent_write =
                            fixture.access_event(seed_source, UseKind::Write, false);
                        fixture.access(destination, UseKind::Read, false);

                        // Three seed values and the aggregate root consume four generations.
                        let alias_generation_bound = 4;
                        let trace = match run_matrix_cell_with_bounds(
                            fixture.finish(),
                            OracleBounds::new(256, 4096, 8, alias_generation_bound),
                            &cell,
                        ) {
                            OracleOutcome::RuntimeConflict { trace } => trace,
                            outcome => panic!(
                                "{cell}: a write-through definition must leave its capability \
                                 covering a later exclusive referent access: {outcome:?}"
                            ),
                        };
                        let conflict = trace.conflict.as_ref().unwrap_or_else(|| {
                            panic!("{cell}: write-through cell must carry its conflict witness")
                        });
                        // The direct rule is kind-based, so its witness depends on the
                        // destination access kind alone. A shared seeded alias conflicts the
                        // first recorded exclusive access, which for a root destination is the
                        // writer's own defining write (the paired access of a write-through is
                        // an ordinary, conflict-checked mutation) and for a projection
                        // destination stays the referent write. An exclusive seeded alias
                        // cannot conflict itself, so its witness remains the referent write.
                        let expected_witness = if seeded_access == AccessKind::Shared {
                            let entry = trace
                                .entries
                                .iter()
                                .find(|entry| {
                                    entry
                                        .access
                                        .as_ref()
                                        .is_some_and(|access| access.kind == AccessKind::Exclusive)
                                })
                                .unwrap_or_else(|| {
                                    panic!(
                                        "{cell}: shared write-through cells must record an \
                                     exclusive access before the conflict"
                                    )
                                });
                            entry.event
                        } else {
                            referent_write
                        };
                        assert_eq!(
                            conflict.access_event, expected_witness,
                            "{cell}: the witness must be the first exclusive access on a \
                             shared seeded alias and the referent write on an exclusive one"
                        );
                        assert_eq!(
                            conflict.capability_kind, seeded_access,
                            "{cell}: the witness must be the seeded {seeded_access:?} capability"
                        );
                        let issue_index = trace
                            .conflict
                            .as_ref()
                            .map(|conflict| conflict.capability_issue)
                            .unwrap_or_else(|| panic!("{cell}: witness must name its issue index"));
                        let seeded_index = trace
                            .entries
                            .iter()
                            .find(|entry| entry.event == seeded_alias)
                            .map(|entry| entry.index)
                            .unwrap_or_else(|| {
                                panic!("{cell}: seeded alias must be present on the trace")
                            });
                        assert_eq!(
                            issue_index, seeded_index,
                            "{cell}: the witness must be the seeded alias capability"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn boracle_oracle_rebinding_slot_does_not_conflict_with_old_observer() {
    let mut fixture = Fixture::new(3);
    let other = fixture.place(0, []);
    let observer = fixture.place(1, []);
    let value = fixture.place(2, []);
    fixture.fresh(other);
    fixture.alias(observer, other, AccessKind::Shared);
    fixture.fresh(value);
    fixture.access(value, UseKind::Write, true);
    fixture.alias(value, other, AccessKind::Exclusive);
    fixture.access(value, UseKind::Write, true);
    fixture.fresh(value);
    fixture.access(observer, UseKind::Read, false);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_write_through_exclusive_alias_does_not_fabricate_conflict() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.access(alias, UseKind::Write, true);
    fixture.fresh(alias);
    fixture.access(owner, UseKind::Read, false);

    assert!(matches!(
        run(fixture.finish()),
        OracleOutcome::CompleteSafe { executions: 1, .. }
    ));
}

#[test]
fn boracle_oracle_write_through_alias_does_not_reissue_source_capability() {
    let mut fixture = Fixture::new(3);
    let first_value = fixture.place(0, []);
    let destination = fixture.place(1, []);
    let second_value = fixture.place(2, []);
    fixture.fresh(first_value);
    fixture.fresh(second_value);
    // The seed is exclusive because a shared seeded alias would end the cell at the builder's
    // own defining access below: the direct rule conflicts every exclusive access whose
    // candidate state is a shared alias, and the transition table itself draws no distinction
    // between the two kinds, so the write-through row under test is identical either way.
    fixture.alias(destination, first_value, AccessKind::Exclusive);

    // This is the builder's defining access before the second alias writer. The writer is a
    // write-through because the destination already aliases the first value.
    fixture.access(destination, UseKind::Write, true);
    fixture.alias(destination, second_value, AccessKind::Shared);
    let second_value_write = fixture.access_event(second_value, UseKind::Write, false);
    fixture.access(destination, UseKind::Read, false);

    let outcome = run(fixture.finish());
    assert!(
        matches!(outcome, OracleOutcome::CompleteSafe { executions: 1, .. }),
        "a write-through alias must not issue a capability for its ignored source. The later \
         disjoint source write must stay safe (witness: {outcome:?}, write: {second_value_write:?})"
    );
}

#[test]
fn boracle_oracle_write_through_exclusive_alias_resolves_to_referent_node() {
    let mut fixture = Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Exclusive);
    fixture.access(alias, UseKind::Write, true);
    fixture.fresh(alias);
    let owner_read = fixture.access_event(owner, UseKind::Read, false);
    let alias_read = fixture.access_event(alias, UseKind::Read, false);
    fixture.fresh(alias);
    let owner_reread = fixture.access_event(owner, UseKind::Read, false);
    let alias_reread = fixture.access_event(alias, UseKind::Read, false);

    let trace = conflict_trace(run(fixture.finish()));

    // The trailing alias reads keep the exclusive alias capability live after each definition,
    // so the earlier owner reads still land inside its interval and conflict with it.
    assert_eq!(
        trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_event),
        Some(owner_read)
    );

    // Each definition on the exclusive alias writes through: the alias must keep resolving to
    // the referent's dynamic node. A definition that replaced the slot would allocate a fresh
    // node for the alias while the owner read stays on the referent.
    let read_node = |event: EventId| {
        trace
            .entries
            .iter()
            .find(|entry| entry.event == event)
            .and_then(|entry| entry.access.as_ref())
            .map(|access| access.target.node)
            .expect("every read in this fixture should be traced")
    };
    assert_eq!(
        read_node(alias_read),
        read_node(owner_read),
        "the alias read after the first definition must use the referent's node"
    );
    assert_eq!(
        read_node(alias_reread),
        read_node(owner_reread),
        "a repeated definition must write through again, not replace the alias with a fresh slot"
    );
}

#[test]
fn boracle_oracle_mutable_parameter_write_reaches_external_generation() {
    let mut fixture = Fixture::new(2);
    fixture.parts.bindings[0].mutable = true;
    let parameter = fixture.place(0, []);
    let observer = fixture.place(1, []);
    let parameter_origin = fixture.origin(OriginKind::Parameter { index: 0 });
    fixture.event(EventKind::Fresh {
        destination: parameter,
        origin: parameter_origin,
    });
    fixture.alias(observer, parameter, AccessKind::Shared);
    let parameter_write = fixture.access_event(parameter, UseKind::Write, true);
    let observer_read = fixture.access_event(observer, UseKind::Read, false);
    let trace = conflict_trace(run(fixture.finish()));

    assert_eq!(
        trace
            .conflict
            .as_ref()
            .map(|conflict| conflict.access_event),
        Some(parameter_write)
    );
    let write_target = trace
        .entries
        .iter()
        .find(|entry| entry.event == parameter_write)
        .and_then(|entry| entry.access.as_ref())
        .map(|access| access.target.node)
        .expect("mutable parameter write should be traced");
    let observer_target = trace
        .entries
        .iter()
        .find(|entry| entry.event == observer_read)
        .and_then(|entry| entry.access.as_ref())
        .map(|access| access.target.node)
        .expect("observer read should be traced");
    assert_eq!(write_target, observer_target);
}

fn run(problem: BorrowProblem) -> OracleOutcome {
    execute_bounded(&problem, OracleBounds::default()).expect("fixture should execute")
}
#[test]
fn boracle_oracle_non_defining_unavailable_access_is_compiler_error() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.access(place, UseKind::Read, false);

    let error = execute_bounded(&fixture.finish(), OracleBounds::default())
        .expect_err("an unavailable non-defining access must be rejected");
    assert!(error.msg.contains("non-defining access"));
}

#[test]
fn boracle_oracle_access_after_explicit_end_is_compiler_error() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.fresh(place);
    let loan = fixture.loan_issue(place, place, AccessKind::Shared);
    fixture.loan_kill(loan);
    fixture.access(place, UseKind::Read, false);

    let error = execute_bounded(&fixture.finish(), OracleBounds::default())
        .expect_err("an access after an explicit capability end must be rejected");
    assert!(error.msg.contains("after its end"));
}

#[test]
fn boracle_oracle_concurrent_capabilities_from_same_source_are_all_exercised() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    let problem = fixture.finish();
    let mut state = OracleState::new(&problem);
    let generation = state
        .issue_generation(1)
        .expect("the fixture needs one runtime generation");
    state.set_state(
        place,
        RuntimePlaceState::Slot {
            current: generation,
        },
    );

    let target = RuntimeAccessTarget {
        node: generation,
        path: Box::new([]),
    };
    let holders = BTreeSet::from([place]);
    let first = state
        .issue_capability(
            AccessKind::Shared,
            target.clone(),
            holders.clone(),
            0,
            EventId::new(0),
            CapabilitySource::Alias,
        )
        .expect("the first capability should issue");
    let second = state
        .issue_capability(
            AccessKind::Shared,
            target.clone(),
            holders,
            1,
            EventId::new(1),
            CapabilitySource::Alias,
        )
        .expect("the second capability should issue");

    let exercised = exercise_capabilities(&problem, &mut state, place, &target, 2, None)
        .expect("live capabilities should be exercisable");
    assert_eq!(exercised.as_ref(), &[first, second]);
    assert_eq!(
        state
            .capabilities
            .get(&first)
            .expect("the first capability should remain live")
            .last_exercised,
        2
    );
    assert_eq!(
        state
            .capabilities
            .get(&second)
            .expect("the second capability should remain live")
            .last_exercised,
        2
    );
}

#[test]
fn boracle_oracle_unresolvable_event_cross_reference_is_rejected_at_construction() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.fresh(place);
    let point = fixture.new_point();
    let event_id = EventId::new(fixture.parts.events.len() as u32);
    fixture.parts.events.push(Event::new(
        event_id,
        point,
        EventKind::Access {
            use_id: UseId::new(1),
        },
        EventSource::none(),
    ));
    fixture.event_ids.push(event_id);

    // BorrowProblem::new validates event cross-references before the oracle can re-check them.
    let error = fixture
        .finish_result()
        .expect_err("an unresolvable event use must be rejected");
    assert!(error.msg.contains("access use"));
}

#[test]
fn boracle_oracle_safe_truncation_is_never_reported_as_complete_safe() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.fresh(place);
    fixture.access(place, UseKind::Read, false);

    match run_with_bounds(fixture.finish(), OracleBounds::new(256, 2, 8, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::EventBound { limit },
            explored,
            ..
        } => {
            assert_eq!(limit, 2);
            assert_eq!(explored, 2);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

#[test]
fn boracle_oracle_zero_block_entry_bound_refuses_the_entry_block() {
    let mut fixture = Fixture::new(1);
    let place = fixture.place(0, []);
    fixture.fresh(place);
    fixture.access(place, UseKind::Read, false);

    // The entry block has no recorded entry when the bound is checked, so a zero bound has to
    // refuse it from a missing count rather than from a stored one. This problem is otherwise
    // safe, so a skipped check reports CompleteSafe and truncation becomes a false safe.
    match run_with_bounds(fixture.finish(), OracleBounds::new(256, 4096, 0, 4096)) {
        OracleOutcome::Inconclusive {
            reason: OracleLimitReason::BlockEntryBound { block, limit },
            explored,
            ..
        } => {
            assert_eq!(block, BlockId::new(0));
            assert_eq!(limit, 0);
            assert_eq!(explored, 0);
        }
        outcome => panic!("unexpected outcome: {outcome:?}"),
    }
}

fn run_with_bounds(problem: BorrowProblem, bounds: OracleBounds) -> OracleOutcome {
    execute_bounded(&problem, bounds).expect("fixture should execute")
}

fn run_matrix_cell(problem: BorrowProblem, cell: &str) -> OracleOutcome {
    execute_bounded(&problem, OracleBounds::default())
        .unwrap_or_else(|error| panic!("{cell}: fixture should execute: {error:?}"))
}

fn run_matrix_cell_with_bounds(
    problem: BorrowProblem,
    bounds: OracleBounds,
    cell: &str,
) -> OracleOutcome {
    execute_bounded(&problem, bounds)
        .unwrap_or_else(|error| panic!("{cell}: fixture should execute: {error:?}"))
}

fn conflict_trace(outcome: OracleOutcome) -> ExecutionTrace {
    match outcome {
        OracleOutcome::RuntimeConflict { trace } => trace,
        OracleOutcome::CompleteSafe { .. } => panic!("expected a runtime conflict"),
        OracleOutcome::Inconclusive {
            reason, explored, ..
        } => {
            panic!("unexpected inconclusive outcome after {explored} events: {reason:?}")
        }
    }
}

struct Fixture {
    parts: BorrowProblemParts,
    next_point: u32,
    event_ids: Vec<EventId>,
}

impl Fixture {
    fn new(binding_count: u32) -> Self {
        Self {
            parts: BorrowProblemParts {
                bindings: (0..binding_count)
                    .map(|id| Binding::synthetic(BindingId::new(id)))
                    .collect(),
                points: vec![ProgramPoint::new(PointId::new(0), BlockId::new(0), 0)],
                ..BorrowProblemParts::default()
            },
            next_point: 1,
            event_ids: Vec::new(),
        }
    }

    fn place<I>(&mut self, root: u32, projections: I) -> PlaceId
    where
        I: IntoIterator<Item = ProjectionElem>,
    {
        let id = PlaceId::new(self.parts.places.len() as u32);
        self.parts.places.push(Place::new(
            id,
            BindingId::new(root),
            projections.into_iter().collect(),
        ));
        id
    }

    fn origin(&mut self, kind: OriginKind) -> ValueOriginId {
        let id = ValueOriginId::new(self.parts.origins.len() as u32);
        self.parts.origins.push(ValueOrigin::new(id, kind));
        id
    }

    fn fresh(&mut self, destination: PlaceId) {
        let origin = self.origin(OriginKind::Fresh);
        self.event(EventKind::Fresh {
            destination,
            origin,
        });
    }
    fn branch<I>(&mut self, targets: I) -> EventId
    where
        I: IntoIterator<Item = BlockId>,
    {
        self.terminator(TerminatorEventKind::Branch {
            targets: targets.into_iter().collect::<Vec<_>>().into_boxed_slice(),
        })
    }

    fn jump(&mut self, target: BlockId) -> EventId {
        self.terminator(TerminatorEventKind::Jump { target })
    }

    fn terminator(&mut self, kind: TerminatorEventKind) -> EventId {
        self.event(EventKind::Terminator { kind })
    }

    fn alias(&mut self, destination: PlaceId, source: PlaceId, access: AccessKind) -> EventId {
        let kind = match access {
            AccessKind::Shared => EventKind::AliasFromPlace {
                source,
                destination,
            },
            AccessKind::Exclusive => EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            },
        };
        self.event(kind)
    }

    /// Emits an origins-carrying alias event for the alias-origins matrix writers.
    fn alias_with_origins(
        &mut self,
        destination: PlaceId,
        source: PlaceId,
        access: AccessKind,
    ) -> EventId {
        let kind = match access {
            AccessKind::Shared => EventKind::Alias {
                source,
                destination,
                origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            },
            AccessKind::Exclusive => EventKind::ExclusiveAlias {
                source,
                destination,
                origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            },
        };
        self.event(kind)
    }

    fn projection(
        &mut self,
        source: PlaceId,
        destination: PlaceId,
        projection: ProjectionElem,
    ) -> EventId {
        let origin = self.origin(OriginKind::Projection {
            source: ValueOriginId::new(0),
            projection,
        });
        self.event(EventKind::Projection {
            source,
            destination,
            origin,
        })
    }

    fn rebind_alias(&mut self, destination: PlaceId, origins: Vec<ValueOriginId>) {
        self.event(EventKind::Rebind {
            destination,
            value: RebindValue::Alias(origins.into_boxed_slice()),
        });
    }

    fn rebind_fresh(&mut self, destination: PlaceId) {
        let origin = self.origin(OriginKind::Fresh);
        self.event(EventKind::Rebind {
            destination,
            value: RebindValue::Fresh(origin),
        });
    }

    fn rebind_alias_from_place(&mut self, destination: PlaceId, source: PlaceId) -> EventId {
        self.event(EventKind::Rebind {
            destination,
            value: RebindValue::AliasFromPlace(source),
        })
    }

    fn reactive_observe(&mut self, place: PlaceId) {
        self.event(EventKind::ReactiveObserve { place });
    }

    fn call_effect_result(
        &mut self,
        call: CallId,
        arguments: Vec<CallArgument>,
        result_place: PlaceId,
        provenance: CallResultProvenance,
    ) -> EventId {
        let origin = self.origin(OriginKind::CallResult { call, provenance });
        self.event(EventKind::CallEffect(CallEffect {
            call,
            arguments: arguments.into_boxed_slice(),
            result: Some(CallResult {
                place: result_place,
                origin,
            }),
        }))
    }

    fn copy(&mut self, destination: PlaceId, source: PlaceId) {
        let origin = self.origin(OriginKind::Copy(
            vec![ValueOriginId::new(0)].into_boxed_slice(),
        ));
        self.event(EventKind::Copy {
            source,
            destination,
            origin,
        });
    }

    fn aggregate<I>(&mut self, destination: PlaceId, fields: I)
    where
        I: IntoIterator<Item = (ProjectionElem, PlaceId)>,
    {
        let origin = self.origin(OriginKind::Fresh);
        let fields = fields
            .into_iter()
            .map(|(projection, source)| AggregateField { projection, source })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.event(EventKind::Aggregate {
            destination,
            origin,
            fields,
        });
    }

    fn scope_exit<I>(&mut self, bindings: I) -> usize
    where
        I: IntoIterator<Item = BindingId>,
    {
        let event_id = self.event(EventKind::ScopeExit {
            bindings: bindings.into_iter().collect::<Vec<_>>().into_boxed_slice(),
        });
        event_id.index()
    }

    fn loan_issue(&mut self, place: PlaceId, holder: PlaceId, kind: AccessKind) -> LoanId {
        self.loan_issue_with_holders(place, [holder], kind)
    }

    fn loan_issue_with_holders<I>(&mut self, place: PlaceId, holders: I, kind: AccessKind) -> LoanId
    where
        I: IntoIterator<Item = PlaceId>,
    {
        let id = LoanId::new(self.parts.loans.len() as u32);
        let origin = self.origin(OriginKind::Fresh);
        let point = self.new_point();
        self.parts.loans.push(Loan {
            id,
            kind,
            issued_at: point,
            place,
            origins: vec![origin].into_boxed_slice(),
            holders: holders.into_iter().collect::<Vec<_>>().into_boxed_slice(),
            uses: Box::new([]),
            kills: Box::new([]),
        });
        self.event_at(point, EventKind::LoanIssue { loan: id });
        id
    }

    fn loan_kill(&mut self, loan: LoanId) -> EventId {
        let point = self.new_point();
        self.parts.loans[loan.index()].kills = vec![point].into_boxed_slice();
        self.event_at(
            point,
            EventKind::LoanKill {
                loan,
                reason: KillReason::Explicit,
            },
        )
    }

    fn access(&mut self, place: PlaceId, kind: UseKind, definition: bool) -> UseId {
        let id = UseId::new(self.parts.uses.len() as u32);
        let point = self.new_point();
        self.parts.uses.push(Use {
            id,
            point,
            place,
            kind,
            definition,
        });
        self.event_at(point, EventKind::Access { use_id: id });
        id
    }

    fn access_event(&mut self, place: PlaceId, kind: UseKind, definition: bool) -> EventId {
        self.access(place, kind, definition);
        self.event_ids
            .last()
            .copied()
            .expect("access should append an event")
    }

    fn call_argument(&mut self, place: PlaceId, access: AccessKind) -> CallArgument {
        let (_, argument) = self.call_argument_at(CallId::new(0), 0, place, access);
        argument
    }

    fn call_argument_at(
        &mut self,
        call: CallId,
        index: u32,
        place: PlaceId,
        access: AccessKind,
    ) -> (EventId, CallArgument) {
        let use_id = UseId::new(self.parts.uses.len() as u32);
        let point = self.new_point();
        let kind = match access {
            AccessKind::Shared => UseKind::Read,
            AccessKind::Exclusive => UseKind::Write,
        };
        self.parts.uses.push(Use {
            id: use_id,
            point,
            place,
            kind,
            definition: false,
        });
        let argument = CallArgument {
            place,
            access,
            use_id,
        };
        let event_id = self.event_at(
            point,
            EventKind::CallArgument {
                call,
                index,
                argument: argument.clone(),
            },
        );
        (event_id, argument)
    }

    fn new_point(&mut self) -> PointId {
        let point = PointId::new(self.next_point);
        self.parts
            .points
            .push(ProgramPoint::new(point, BlockId::new(0), self.next_point));
        self.next_point += 1;
        point
    }

    fn event(&mut self, kind: EventKind) -> EventId {
        let point = self.new_point();
        self.event_at(point, kind)
    }

    fn event_at(&mut self, point: PointId, kind: EventKind) -> EventId {
        let id = EventId::new(self.parts.events.len() as u32);
        self.parts
            .events
            .push(Event::new(id, point, kind, EventSource::none()));
        self.event_ids.push(id);
        id
    }
    fn take_events(&mut self) -> Vec<EventId> {
        std::mem::take(&mut self.event_ids)
    }

    fn finish_cfg(
        mut self,
        mut block_events: BTreeMap<BlockId, Vec<EventId>>,
        edges: Vec<CfgEdge>,
        entry: BlockId,
        exits: Vec<BlockId>,
    ) -> BorrowProblem {
        let mut blocks = Vec::with_capacity(block_events.len());
        for (block, events) in &mut block_events {
            assert!(!events.is_empty(), "CFG blocks need a terminator event");
            for (ordinal, event_id) in events.iter().copied().enumerate() {
                let point = self.parts.events[event_id.index()].point;
                let point_row = &mut self.parts.points[point.index()];
                point_row.block = *block;
                point_row.ordinal = ordinal as u32;
            }
            let entry_point = self.parts.events[events[0].index()].point;
            let exit_point = self.parts.events[events[events.len() - 1].index()].point;
            blocks.push(CfgBlock::new(
                *block,
                entry_point,
                exit_point,
                std::mem::take(events),
            ));
        }
        assert!(
            self.event_ids.is_empty(),
            "every fixture event must be assigned to a CFG block"
        );
        self.parts.blocks = blocks;
        self.parts.edges = edges;
        self.parts.entry = entry;
        self.parts.exits = exits;
        BorrowProblem::new(self.parts).expect("CFG fixture should validate")
    }

    fn finish(self) -> BorrowProblem {
        self.finish_result()
            .expect("operational fixture should validate")
    }

    fn finish_result(mut self) -> Result<BorrowProblem, CompilerError> {
        let exit_point = self.new_point();
        let terminator = Event::new(
            EventId::new(self.parts.events.len() as u32),
            exit_point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        );
        self.parts.events.push(terminator);
        self.event_ids
            .push(EventId::new(self.parts.events.len() as u32 - 1));
        self.parts.blocks.push(CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            exit_point,
            self.event_ids,
        ));
        self.parts.entry = BlockId::new(0);
        self.parts.exits = vec![BlockId::new(0)];
        BorrowProblem::new(self.parts)
    }

    fn finish_branch(mut self, target_ids: [BlockId; 2]) -> BorrowProblem {
        let targets = target_ids.to_vec();
        let branch_point = self.new_point();
        self.event_at(
            branch_point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Branch {
                    targets: targets.clone().into_boxed_slice(),
                },
            },
        );
        let block0_events = std::mem::take(&mut self.event_ids);
        let mut blocks = vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            branch_point,
            block0_events,
        )];
        let mut edges = Vec::new();
        for target in targets.iter().copied() {
            let target_point = PointId::new(self.parts.points.len() as u32);
            self.parts
                .points
                .push(ProgramPoint::new(target_point, target, 0));
            let target_event = EventId::new(self.parts.events.len() as u32);
            self.parts.events.push(Event::new(
                target_event,
                target_point,
                EventKind::Terminator {
                    kind: TerminatorEventKind::Return,
                },
                EventSource::none(),
            ));
            blocks.push(CfgBlock::new(
                target,
                target_point,
                target_point,
                vec![target_event],
            ));
            edges.push(CfgEdge::new(BlockId::new(0), target));
        }
        self.parts.blocks = blocks;
        self.parts.edges = edges;
        self.parts.exits = targets;
        BorrowProblem::new(self.parts).expect("branch fixture should validate")
    }

    fn finish_reentered(mut self) -> BorrowProblem {
        let loop_block = BlockId::new(1);
        let jump_point = self.new_point();
        self.event_at(
            jump_point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Jump { target: loop_block },
            },
        );
        let block0_events = std::mem::take(&mut self.event_ids);

        let loop_point = PointId::new(self.parts.points.len() as u32);
        self.parts
            .points
            .push(ProgramPoint::new(loop_point, loop_block, 0));
        let loop_event = EventId::new(self.parts.events.len() as u32);
        self.parts.events.push(Event::new(
            loop_event,
            loop_point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Jump { target: loop_block },
            },
            EventSource::none(),
        ));

        let exit_block = BlockId::new(2);
        let exit_point = PointId::new(self.parts.points.len() as u32);
        self.parts
            .points
            .push(ProgramPoint::new(exit_point, exit_block, 0));
        let exit_event = EventId::new(self.parts.events.len() as u32);
        self.parts.events.push(Event::new(
            exit_event,
            exit_point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ));

        self.parts.blocks = vec![
            CfgBlock::new(BlockId::new(0), PointId::new(0), jump_point, block0_events),
            CfgBlock::new(loop_block, loop_point, loop_point, vec![loop_event]),
            CfgBlock::new(exit_block, exit_point, exit_point, vec![exit_event]),
        ];
        self.parts.edges = vec![
            CfgEdge::new(BlockId::new(0), loop_block),
            CfgEdge::new(loop_block, loop_block),
        ];
        self.parts.entry = BlockId::new(0);
        self.parts.exits = vec![exit_block];
        BorrowProblem::new(self.parts).expect("reentered fixture should validate")
    }
}
