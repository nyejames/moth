use super::{run_boracle, solve_boracle};
use crate::compiler_frontend::analysis::borrow_checker::{
    AccessKind, BoracleDump, BoracleExperiment, BoracleModuleReport, CallResultProvenance,
    EventKind, OriginKind,
};
use std::fs;

#[test]
fn boracle_service_source_smoke_uses_real_moth_input() {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, "value = 1\n").expect("source should be writable");

    let first = run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        BoracleDump::Origins,
        BoracleExperiment::DeadExclusiveLoan,
    )
    .expect("real source should reach Boracle");
    let second = run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        BoracleDump::Origins,
        BoracleExperiment::DeadExclusiveLoan,
    )
    .expect("real source should reach Boracle");

    assert_eq!(first, second);
    assert!(first.contains("rule-set = boracle-reference-v1"));
    assert!(first.contains("experiment = dead-exclusive-loan"));
    assert!(first.contains("OriginSolution"));
}

#[test]
fn boracle_source_shared_alias_conflict_uses_derived_loan() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
~items.push("b") catch:
;
result = shared
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.contains("ConflictWitness"),
        "expected a source alias conflict, got:\n{output}"
    );
}

#[test]
fn boracle_source_shared_alias_final_use_allows_mutation() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
result = shared
~items.push("b") catch:
;
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.trim_end().ends_with("[]"),
        "unexpected source conflict:\n{output}"
    );
}

#[test]
fn boracle_source_copy_is_independent_from_source_alias() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
snapshot ~= copy items
~snapshot.push("b") catch:
;
result = shared
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.trim_end().ends_with("[]"),
        "unexpected copy conflict:\n{output}"
    );
}

#[test]
fn boracle_source_copy_report_keeps_origins_independent() {
    let report = solve_source(
        r#"
items ~= {1}
shared = items
snapshot ~= copy items
~snapshot.push(2) catch:
;
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            !function.report.has_conflicts()
                && function
                    .problem
                    .events()
                    .iter()
                    .any(|event| matches!(event.kind, EventKind::Copy { .. }))
        })
        .expect("copy source should produce an independent typed report");
    let (copy_event, source, destination) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Copy {
                source,
                destination,
                ..
            } => Some((event.id, *source, *destination)),
            _ => None,
        })
        .expect("copy event should be present in the typed source problem");
    let source_origins =
        function
            .report
            .origin
            .origins_for_place_after_event(&function.problem, copy_event, source);
    let destination_origins = function
        .report
        .origin
        .origins_after_event(copy_event, destination)
        .expect("copy destination should publish a typed origin row");
    assert!(!source_origins.is_empty());
    assert!(!destination_origins.is_empty());
    assert!(
        source_origins
            .iter()
            .all(|origin| !destination_origins.contains(origin))
    );
}

#[test]
fn boracle_source_rebind_separates_old_alias_origin() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
items = {"b"}
~items.push("c") catch:
;
result = shared
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.trim_end().ends_with("[]"),
        "unexpected rebind conflict:\n{output}"
    );
}

#[test]
fn boracle_source_rebind_report_separates_old_and_new_generations() {
    let report = solve_source(
        r#"
items ~= {"a"}
shared = items
items = {"b"}
~items.push("c") catch:
;
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            !function.report.has_conflicts()
                && function
                    .problem
                    .events()
                    .iter()
                    .any(|event| matches!(event.kind, EventKind::Aggregate { .. }))
        })
        .expect("rebind source should produce a typed generation report");
    let (alias_event, alias_destination, source_root) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::AliasFromPlace {
                source,
                destination,
            }
            | EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            } => {
                let source_place = &function.problem.places()[source.index()];
                Some((event.id, *destination, source_place.root))
            }
            _ => None,
        })
        .expect("rebind source should retain its alias event");
    let old_origins = function
        .report
        .origin
        .origins_after_event(alias_event, alias_destination)
        .expect("old alias should publish its origin");
    let (new_event, new_destination) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate { destination, .. } => {
                let destination_place = &function.problem.places()[destination.index()];
                (event.id > alias_event && destination_place.root == source_root)
                    .then_some((event.id, *destination))
            }
            _ => None,
        })
        .expect("fresh source rebind should retain its aggregate event");
    let new_origins = function
        .report
        .origin
        .origins_after_event(new_event, new_destination)
        .expect("new source generation should publish its origin");
    assert!(!old_origins.is_empty());
    assert!(!new_origins.is_empty());
    assert!(
        old_origins
            .iter()
            .all(|origin| !new_origins.contains(origin))
    );
}

#[test]
fn boracle_source_slot_alias_rebind_then_fresh_rebind_separates_origins() {
    let report = solve_source(
        r#"
source ~= {"source"}
slot ~= {"slot"}
slot = source
survivor = slot
slot = {"new"}
~slot.push("changed") catch:
;
result = survivor
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            !function.report.has_conflicts()
                && function
                    .problem
                    .events()
                    .iter()
                    .filter(|event| {
                        matches!(
                            event.kind,
                            EventKind::AliasFromPlace { .. }
                                | EventKind::ExclusiveAliasFromPlace { .. }
                        )
                    })
                    .count()
                    >= 2
        })
        .expect("slot alias rebind source should produce a conflict-free typed report");
    let alias_events = function
        .problem
        .events()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::AliasFromPlace {
                source,
                destination,
            }
            | EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            } => Some((event.id, *source, *destination)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (slot_alias_event, slot_source, slot_destination) = alias_events[0];
    let (survivor_alias_event, survivor_source, survivor_destination) = alias_events[1];
    let slot_root = function.problem.places()[slot_destination.index()].root;
    assert_eq!(
        function.problem.places()[survivor_source.index()].root,
        slot_root
    );
    assert_ne!(
        function.problem.places()[slot_source.index()].root,
        slot_root
    );

    let intermediate_origins = function
        .report
        .origin
        .origins_after_event(survivor_alias_event, survivor_destination)
        .expect("surviving alias should publish intermediate origins");
    let (fresh_rebind_event, fresh_rebind_destination) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate { destination, .. }
                if event.id > survivor_alias_event
                    && function.problem.places()[destination.index()].root == slot_root =>
            {
                Some((event.id, *destination))
            }
            _ => None,
        })
        .expect("slot should have a later fresh aggregate rebind");
    let fresh_origins = function
        .report
        .origin
        .origins_after_event(fresh_rebind_event, fresh_rebind_destination)
        .expect("fresh slot rebind should publish new origins");
    assert!(!intermediate_origins.is_empty());
    assert!(!fresh_origins.is_empty());
    assert!(
        intermediate_origins
            .iter()
            .all(|origin| !fresh_origins.contains(origin))
    );
    assert!(slot_alias_event < survivor_alias_event);
}

#[test]
fn boracle_source_optional_transfer_sees_write_through_alias_observation() {
    let report = solve_source(
        r#"
observe |value {Int}| -> {Int}:
    return value
;

items ~= {1}
result = observe(value = items)
writer ~= items
writer = {2}
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .events()
                .iter()
                .any(|event| matches!(event.kind, EventKind::CallArgument { .. }))
                && function.problem.uses().iter().any(|use_row| {
                    use_row.definition && function.report.origin.is_write_through_use(use_row.id)
                })
        })
        .expect("write-through transfer source should produce a typed call report");
    let (event_id, place, point) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallArgument { argument, .. } => {
                Some((event.id, argument.place, event.point))
            }
            _ => None,
        })
        .expect("call argument event should be present");
    let origin = function
        .report
        .origin
        .origins_for_place_after_event(&function.problem, event_id, place)
        .first()
        .copied()
        .expect("call argument should retain its source origin");
    assert!(
        !function
            .report
            .optional_transfer_allowed_for_origin_after_event(origin, event_id, point)
    );
}

#[test]
fn boracle_source_local_mutable_parameter_is_exclusive() {
    let output = run_source_dump(
        r#"
increment |value ~Int| -> Int:
    value = value + 1
    return value
;

x ~= 10
result = increment(value = ~x)
"#,
        BoracleDump::Loans,
    );

    assert!(
        output.contains("kind: Exclusive"),
        "expected an exclusive call argument loan, got:\n{output}"
    );
}

#[test]
fn boracle_source_local_mutable_parameter_report_is_exclusive() {
    let report = solve_source(
        r#"
increment |value ~Int| -> Int:
    value = value + 1
    return value
;

x ~= 10
result = increment(value = ~x)
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(
                    &event.kind,
                    EventKind::CallArgument { argument, .. }
                        if argument.access == AccessKind::Exclusive
                )
            })
        })
        .expect("mutable parameter call should retain a typed exclusive argument");
    assert!(
        function
            .problem
            .events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::CallEffect(_)))
    );
}

#[test]
fn boracle_source_unknown_result_does_not_prove_independence() {
    let output = run_source_dump(
        r#"
keep_values |value {Int}| -> {Int}:
    return value
;

items ~= {1}
shared = items
unknown ~= keep_values(value = items)
~unknown.push(2) catch:
;
result = shared
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.contains("ConflictWitness"),
        "expected conservative unknown-result conflict, got:\n{output}"
    );
}

#[test]
fn boracle_source_unknown_result_report_keeps_conservative_overlap() {
    let report = solve_source(
        r#"
keep_values |value {Int}| -> {Int}:
    return value
;

items ~= {1}
shared = items
unknown ~= keep_values(value = items)
~unknown.push(2) catch:
;
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("unknown result should retain a typed conflict");
    let result = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallEffect(effect) => effect.result,
            _ => None,
        })
        .expect("unknown call should retain its result row");
    assert!(matches!(
        &function.problem.origins()[result.origin.index()].kind,
        OriginKind::CallResult {
            provenance: CallResultProvenance::Unknown,
            ..
        }
    ));
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| witness.origin_overlap)
    );
}

#[test]
fn boracle_source_aggregate_field_keeps_stored_alias_live() {
    let output = run_source_dump(
        r#"
Pair = |
    first {Int},
    second {Int},
|

items ~= {1}
pair ~= Pair(items, items)
alias = pair.first
~items.push(2) catch:
;
result = alias
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.contains("ConflictWitness"),
        "expected aggregate field alias conflict, got:\n{output}"
    );
}

#[test]
fn boracle_source_aggregate_field_report_keeps_typed_origin_lineage() {
    let report = solve_source(
        r#"
Pair = |
    first {Int},
    second {Int},
|

items ~= {1}
pair ~= Pair(items, items)
alias = pair.first
~items.push(2) catch:
;
result = alias
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .events()
                .iter()
                .any(|event| matches!(event.kind, EventKind::Aggregate { .. }))
        })
        .expect("source report should contain aggregate storage");
    let (aggregate_id, aggregate_destination, source_place) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate {
                destination,
                fields,
                ..
            } if !fields.is_empty() => Some((event.id, *destination, fields[0].source)),
            _ => None,
        })
        .expect("aggregate event should retain its first child");
    let source_origin = function
        .report
        .origin
        .origins_for_place_after_event(&function.problem, aggregate_id, source_place)
        .first()
        .copied()
        .expect("aggregate child should have a source origin");
    let field_alias = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::AliasFromPlace {
                source,
                destination,
            }
            | EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            } => {
                let aggregate_place = &function.problem.places()[aggregate_destination.index()];
                let source_place = &function.problem.places()[source.index()];
                (source_place.root == aggregate_place.root
                    && source_place.projections.len() > aggregate_place.projections.len()
                    && source_place
                        .projections
                        .starts_with(&aggregate_place.projections))
                .then_some((event.id, *destination))
            }
            _ => None,
        })
        .expect("source field load should emit an alias from the projected place");
    let projected_origins = function
        .report
        .origin
        .origins_after_event(field_alias.0, field_alias.1)
        .expect("projection should publish a destination origin set");

    assert_eq!(projected_origins, [source_origin]);
    assert!(function.report.loans.conflicts().iter().any(|witness| {
        witness.origin_overlap && witness.loan_origins.contains(&source_origin)
    }));
}

#[test]
fn boracle_source_alias_used_only_as_call_argument_stays_live() {
    let report = solve_source(
        r#"
observe |value {Int}| -> {Int}:
    return value
;

items ~= {1}
shared = items
~items.push(2) catch:
;
result = observe(value = shared)
"#,
    );

    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("source module should contain a conflicting function report");
    assert!(
        function.report.has_conflicts(),
        "a user alias used only as a call argument must keep the source loan live: {:?}",
        function.report.loans.loans()
    );
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| witness.keeping_use.is_some())
    );
}

#[test]
fn boracle_source_mutable_alias_used_only_as_mutable_call_stays_live() {
    let report = solve_source(
        r#"
items ~= {1}
writer ~= items
~items.push(2) catch:
;
~writer.push(3) catch:
;
result = items
"#,
    );

    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("source module should contain a conflicting function report");
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| witness.keeping_use.is_some())
    );
}

#[test]
fn boracle_source_mutable_alias_write_through_conflicts_with_shared_alias() {
    let report = solve_source(
        r#"
items ~= {1}
shared = items
writer ~= items
writer = {2}
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| !function.report.loans.conflicts().is_empty())
        .expect("mutable alias write-through should conflict with the shared alias");
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| { witness.origin_overlap && witness.keeping_use.is_some() })
    );
}

#[test]
fn boracle_source_branch_separates_typed_use_and_mutation() {
    let report = solve_source(
        r#"
observe |value {Int}| -> {Int}:
    return value
;

items ~= {1}
shared = items
condition = true
if condition:
    observed = observe(value = shared)
else
    ~items.push(2) catch:
    ;
;
result = 0
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .events()
                .iter()
                .any(|event| matches!(event.kind, EventKind::CallArgument { .. }))
        })
        .expect("branch source should produce one typed entry-function report");

    assert!(!function.report.has_conflicts());
    assert!(function.problem.control_flow().edges.len() >= 3);
    assert!(function.problem.events().iter().any(|event| {
        matches!(
            event.kind,
            EventKind::AliasFromPlace { .. } | EventKind::ExclusiveAliasFromPlace { .. }
        )
    }));
    assert!(function.problem.events().iter().any(|event| {
        matches!(
            &event.kind,
            EventKind::CallArgument { argument, .. }
                if argument.access == AccessKind::Shared
        )
    }));
    assert!(
        function
            .report
            .loans
            .decisions()
            .iter()
            .any(|decision| decision.kind == AccessKind::Exclusive)
    );
}

#[test]
fn boracle_source_loop_alias_rebind_reaches_a_deterministic_typed_fixpoint() {
    let source = r#"
items ~= {1}
counter ~= 0
loop counter < 2:
    old = items
    items = {2}
    ~items.push(3) catch:
    ;
    result = old
    counter = counter + 1
;
"#;
    let first = solve_source(source);
    let second = solve_source(source);
    let first_function = first
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(
                    event.kind,
                    EventKind::AliasFromPlace { .. } | EventKind::ExclusiveAliasFromPlace { .. }
                )
            })
        })
        .expect("loop source should produce a typed alias function report");
    let second_function = second
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(
                    event.kind,
                    EventKind::AliasFromPlace { .. } | EventKind::ExclusiveAliasFromPlace { .. }
                )
            })
        })
        .expect("repeat loop source should produce a typed alias function report");

    assert!(
        first_function
            .problem
            .control_flow()
            .edges
            .iter()
            .any(|edge| edge.to.raw() <= edge.from.raw())
    );
    assert!(first_function.problem.events().iter().any(|event| {
        matches!(
            event.kind,
            EventKind::Fresh { .. } | EventKind::Aggregate { .. }
        )
    }));
    assert_eq!(
        first_function.problem.debug_dump(),
        second_function.problem.debug_dump()
    );
    assert_eq!(
        first_function.report.debug_dump(),
        second_function.report.debug_dump()
    );
}

#[test]
fn boracle_source_final_call_argument_queries_transfer_after_exact_event() {
    let report = solve_source(
        r#"
observe |value {Int}| -> {Int}:
    return value
;

items ~= {1}
result = observe(value = items)
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .events()
                .iter()
                .any(|event| matches!(event.kind, EventKind::CallArgument { .. }))
        })
        .expect("final call source should contain a typed call argument event");
    let (event_id, place, use_id, point) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallArgument { argument, .. } => {
                Some((event.id, argument.place, argument.use_id, event.point))
            }
            _ => None,
        })
        .expect("final call should retain its exact argument event");
    let origin = function
        .report
        .origin
        .origins_for_place_after_event(&function.problem, event_id, place)
        .first()
        .copied()
        .expect("final call argument should retain its source origin");
    let use_row = function
        .problem
        .uses()
        .get(use_id.index())
        .expect("call argument should own its normalized use");
    assert_eq!(use_row.point, point);
    assert_eq!(use_row.place, place);
    assert!(
        function
            .report
            .optional_transfer_allowed_for_origin_after_event(origin, event_id, point)
    );
}

#[test]
fn boracle_source_typed_report_connects_origins_loans_and_conflicts() {
    let report = solve_source(
        r#"
items ~= {"a"}
shared = items
~items.push("b") catch:
;
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("source module should contain a conflicting function report");

    assert!(function.problem.debug_dump().contains("AliasFromPlace"));
    assert!(
        function
            .report
            .loans
            .loans()
            .iter()
            .any(|loan| !loan.origins.is_empty() && !loan.uses.is_empty())
    );
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| witness.origin_overlap)
    );
}

#[test]
fn boracle_source_map_get_keeps_receiver_protected_while_live() {
    let output = run_source_dump(
        r#"
scores ~{String = Int} = {"Priya" = 10}
score = scores.get("Priya") catch:
    then 0
;
~scores.set("Linus", 7) catch:
;
result = score
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.contains("ConflictWitness"),
        "expected live map-get conflict, got:\n{output}"
    );
}

#[test]
fn boracle_source_map_get_report_keeps_receiver_loan_live() {
    let report = solve_source(
        r#"
scores ~{String = Int} = {"Priya" = 10}
score = scores.get("Priya") catch:
    then 0
;
~scores.set("Linus", 7) catch:
;
result = score
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("map get should retain a typed receiver conflict");
    assert!(
        function
            .report
            .loans
            .loans()
            .iter()
            .any(|loan| { !loan.origins.is_empty() && !loan.uses.is_empty() })
    );
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| witness.origin_overlap && witness.keeping_use.is_some())
    );
}

#[test]
fn boracle_source_map_remove_is_not_fresh_provenance() {
    let output = run_source_dump(
        r#"
scores ~{String = Int} = {"Priya" = 10}
removed = ~scores.remove("Priya") catch:
    then 0
;
result = removed
"#,
        BoracleDump::Problem,
    );

    assert!(
        output.contains("provenance: Unknown"),
        "expected conservative map-remove provenance, got:\n{output}"
    );
}

#[test]
fn boracle_source_map_remove_report_is_unknown_not_fresh() {
    let report = solve_source(
        r#"
scores ~{String = Int} = {"Priya" = 10}
removed = ~scores.remove("Priya") catch:
    then 0
;
result = removed
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .events()
                .iter()
                .any(|event| matches!(event.kind, EventKind::CallEffect(_)))
        })
        .expect("map remove should retain a typed call result");
    let result = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallEffect(effect) => effect.result,
            _ => None,
        })
        .expect("map remove should retain its result row");
    assert!(matches!(
        &function.problem.origins()[result.origin.index()].kind,
        OriginKind::CallResult {
            provenance: CallResultProvenance::Unknown,
            ..
        }
    ));
}

#[test]
fn boracle_source_branch_separates_use_and_mutation() {
    let output = run_source_dump(
        include_str!("../../../tests/cases/branch_reborrow_after_last_use/input/@page.moth"),
        BoracleDump::Conflicts,
    );

    assert!(
        output.trim_end().ends_with("[]"),
        "unexpected branch conflict:\n{output}"
    );
}

#[test]
fn boracle_source_loop_copy_keeps_independent_roots() {
    let output = run_source_dump(
        include_str!("../../../tests/cases/loop_borrow_independent_roots/input/@page.moth"),
        BoracleDump::Conflicts,
    );

    assert!(
        output.trim_end().ends_with("[]"),
        "unexpected loop conflict:\n{output}"
    );
}

fn solve_source(source: &str) -> BoracleModuleReport {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, source).expect("source should be writable");
    solve_boracle(entry.to_str().expect("temporary path should be UTF-8"))
        .unwrap_or_else(|messages| panic!("source should reach Boracle: {messages:?}"))
}

fn run_source_dump(source: &str, dump: BoracleDump) -> String {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, source).expect("source should be writable");
    run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        dump,
        BoracleExperiment::Reference,
    )
    .unwrap_or_else(|messages| panic!("source should reach Boracle: {messages:?}"))
}
