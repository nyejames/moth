use super::{compile_boracle_input, run_boracle, solve_boracle};
use crate::compiler_frontend::analysis::borrow_checker::{
    AccessKind, BindingDestination, BoracleDump, BoracleExperiment, BoracleModuleReport,
    BoracleRuleSelection, CallResultProvenance, CallResultUnknownReason, EventKind, OriginKind,
    OriginOverlapDecision, RebindValue, TerminatorEventKind, UseKind,
};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use std::collections::BTreeSet;
use std::fs;
#[test]
fn boracle_service_source_smoke_uses_real_moth_input() {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, "value = 1\n").expect("source should be writable");

    let first = run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        BoracleDump::Origins,
        dead_exclusive_selection(),
    )
    .expect("real source should reach Boracle");
    let second = run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        BoracleDump::Origins,
        dead_exclusive_selection(),
    )
    .expect("real source should reach Boracle");

    assert_eq!(first, second);
    assert!(first.contains("rule-set = boracle-reference-v1"));
    assert!(first.contains("experiments = dead-exclusive-loan"));
    assert!(!first.contains("experiment = "));
    assert!(first.contains("OriginSolution"));
}

#[test]
fn boracle_source_service_accepts_stage0_resource_structural_strings() {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let assets = temporary.path().join("assets");
    fs::create_dir_all(&assets).expect("resource directory should be creatable");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, "logo #= @assets/logo.svg\nruntime = logo\n")
        .expect("source should be writable");
    fs::write(assets.join("logo.svg"), "<svg></svg>\n").expect("resource should be writable");

    let (input, _) = compile_boracle_input(entry.to_str().expect("temporary path should be UTF-8"))
        .expect("resource-bearing source should produce validated HIR");
    let resource_piece = input
        .hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let value = match &statement.kind {
                HirStatementKind::Write { value, .. }
                | HirStatementKind::Expr(value)
                | HirStatementKind::PushRuntimeFragment { value, .. } => value,
                _ => return None,
            };
            let HirExpressionKind::StructuralString { pieces } =
                &input.hir.expressions.expression(*value).kind
            else {
                return None;
            };
            input
                .hir
                .expressions
                .string_pieces(*pieces)
                .iter()
                .find_map(|piece| match piece {
                    ConstStringPiece::Resource(resource_id) => Some(*resource_id),
                    _ => None,
                })
        });
    assert!(
        resource_piece.is_some(),
        "validated HIR should preserve the resolved Resource structural piece"
    );

    let report = solve_boracle(entry.to_str().expect("temporary path should be UTF-8"))
        .expect("resource-bearing source should reach Boracle");
    let problem = &report
        .functions()
        .first()
        .expect("resource-bearing source should produce one function report")
        .problem;
    let runtime_place = problem
        .places()
        .iter()
        .find(|place| {
            problem
                .bindings()
                .iter()
                .any(|binding| binding.id == place.root && !binding.compiler_temporary)
        })
        .expect("resource-bearing source should retain its runtime binding place")
        .id;

    assert!(
        problem.events().iter().any(|event| {
            matches!(
                &event.kind,
                EventKind::Fresh {
                    destination: BindingDestination::Define(place),
                    ..
                } if *place == runtime_place
            )
        }),
        "runtime structural strings should receive fresh value storage"
    );
    assert!(
        !problem.events().iter().any(|event| match &event.kind {
            EventKind::Alias { destination, .. }
            | EventKind::AliasFromPlace { destination, .. }
            | EventKind::ExclusiveAlias { destination, .. }
            | EventKind::ExclusiveAliasFromPlace { destination, .. } => {
                destination.place() == runtime_place
            }
            _ => false,
        }),
        "resource structural strings should not create alias edges"
    );
}

#[test]
fn boracle_source_differential_replays_plain_value() {
    let source = "value = 1\n";
    let report = solve_source(source);
    let function = report
        .functions()
        .first()
        .expect("plain-value source should produce one function report");
    let binding_place = function
        .problem
        .places()
        .iter()
        .find(|place| {
            function
                .problem
                .bindings()
                .iter()
                .any(|binding| binding.id == place.root && !binding.compiler_temporary)
        })
        .expect("plain-value source should retain its user binding place");
    assert!(
        function.problem.events().iter().any(|event| {
            matches!(
                &event.kind,
                EventKind::Fresh {
                    destination: BindingDestination::Define(place),
                    ..
                } if *place == binding_place.id
            )
        }),
        "plain-value source should define a fresh value for its binding"
    );
    assert!(
        function.problem.uses().iter().any(|use_row| {
            use_row.place == binding_place.id
                && use_row.definition
                && use_row.kind.access_kind() == AccessKind::Exclusive
        }),
        "plain-value source should retain a defining write for its binding"
    );
    assert_source_differential_replay(source, &["Agreement", "Agreement"]);
}

#[test]
fn boracle_source_differential_replays_boolean_mutated_binding() {
    let source = "value ~= false\nvalue = not value\nresult = value\n";
    assert_mutable_reassignment_access_order(source);
    let differential = assert_source_differential_replay(source, &["Agreement", "Agreement"]);
    let oracle_outcomes = differential
        .lines()
        .filter_map(|line| line.strip_prefix("oracle-outcome = "))
        .collect::<Vec<_>>();
    assert_eq!(
        oracle_outcomes,
        vec!["CompleteSafe { executions: 1 }"; 2],
        "both differential comparisons should finish one conflict-free execution:\n{differential}"
    );
}

#[test]
fn boracle_source_differential_marks_checked_numeric_reassignment_inconclusive() {
    let source = "value ~= 1\nvalue = value + 1\nresult = value\n";
    assert_mutable_reassignment_access_order(source);
    let differential =
        assert_source_differential_replay(source, &["OracleInconclusive", "OracleInconclusive"]);
    let oracle_outcomes = differential
        .lines()
        .filter_map(|line| line.strip_prefix("oracle-outcome = "))
        .collect::<Vec<_>>();
    assert_eq!(
        oracle_outcomes.len(),
        2,
        "each differential comparison should report its bounded-oracle outcome:\n{differential}"
    );
    assert!(
        oracle_outcomes.iter().all(|outcome| {
            outcome.starts_with("Inconclusive {")
                && outcome.contains("reason: UndecidableOverlap {")
                && outcome.contains("path: [DynamicIndex]")
                && outcome.contains("path: []")
                && outcome.contains("completed_executions: 0")
        }),
        "computed numeric reassignment should preserve the oracle's undecidable-overlap refusal \
         and zero complete executions:\n{differential}"
    );
}

fn assert_mutable_reassignment_access_order(source: &str) {
    let report = solve_source(source);
    let function = report
        .functions()
        .first()
        .expect("mutable-reassignment source should produce one function report");
    let value_place = function
        .problem
        .places()
        .iter()
        .find(|place| {
            function.problem.bindings().iter().any(|binding| {
                binding.id == place.root && binding.mutable && !binding.compiler_temporary
            })
        })
        .expect("mutable-reassignment source should retain its mutable binding place");
    let mut defining_write_points = function
        .problem
        .uses()
        .iter()
        .filter(|use_row| {
            use_row.place == value_place.id
                && use_row.definition
                && use_row.kind.access_kind() == AccessKind::Exclusive
        })
        .map(|use_row| use_row.point)
        .collect::<Vec<_>>();
    defining_write_points.sort_unstable();
    assert_eq!(
        defining_write_points.len(),
        2,
        "mutable-reassignment source should retain an initial and reassignment defining write: {defining_write_points:?}"
    );
    let mut destinations = function
        .problem
        .uses()
        .iter()
        .filter_map(|use_row| {
            if use_row.place != value_place.id {
                return None;
            }
            use_row
                .kind
                .binding_destination()
                .map(|destination| (use_row.point, destination))
        })
        .collect::<Vec<_>>();
    destinations.sort_unstable_by_key(|(point, _)| point.raw());
    assert_eq!(
        destinations
            .into_iter()
            .map(|(_, destination)| destination)
            .collect::<Vec<_>>(),
        vec![
            BindingDestination::Define(value_place.id),
            BindingDestination::Update(value_place.id),
        ],
        "mutable-reassignment source should define its initial value and update that binding"
    );
    let mut read_points = function
        .problem
        .uses()
        .iter()
        .filter(|use_row| {
            use_row.place == value_place.id
                && !use_row.definition
                && use_row.kind.access_kind() == AccessKind::Shared
        })
        .map(|use_row| use_row.point)
        .collect::<Vec<_>>();
    read_points.sort_unstable();
    assert_eq!(
        read_points.len(),
        2,
        "mutable-reassignment source should retain the reassignment read and later value read: {read_points:?}"
    );
    assert!(
        defining_write_points[0] < read_points[0]
            && read_points[0] < defining_write_points[1]
            && defining_write_points[1] < read_points[1],
        "mutable-reassignment source should write, read before reassignment, write again, then read the assigned value: writes={defining_write_points:?} reads={read_points:?}"
    );
}

#[test]
fn boracle_source_differential_replays_shared_collection_alias() {
    let source = r#"
items ~= {"a"}
shared = items
result = shared
"#;
    let report = solve_source(source);
    let function = report
        .functions()
        .first()
        .expect("shared-alias source should produce one function report");
    let collection_place = function
        .problem
        .events()
        .iter()
        .find_map(|event| {
            let EventKind::Aggregate { destination, .. } = &event.kind else {
                return None;
            };
            if !matches!(destination, BindingDestination::Define(_)) {
                return None;
            }
            let place = function
                .problem
                .places()
                .iter()
                .find(|place| place.id == destination.place())?;
            let binding = function
                .problem
                .bindings()
                .iter()
                .find(|binding| binding.id == place.root)?;
            (!binding.compiler_temporary && binding.mutable).then_some(destination.place())
        })
        .expect("shared-alias source should retain its mutable collection");
    let alias_destination = function.problem.events().iter().find_map(|event| {
        let EventKind::AliasFromPlace {
            source,
            destination,
        } = &event.kind
        else {
            return None;
        };
        (*source == collection_place).then_some(*destination)
    });
    assert!(
        alias_destination.is_some(),
        "shared-alias source should retain an alias edge from its collection"
    );
    let alias_destination = alias_destination.expect("alias assertion should pass");
    assert!(
        matches!(alias_destination, BindingDestination::Define(_)),
        "a new shared binding should define its alias destination"
    );
    let alias_destination_place = alias_destination.place();
    assert!(
        !function.problem.events().iter().any(|event| {
            matches!(
                &event.kind,
                EventKind::Fresh { destination, .. }
                    if destination.place() == alias_destination_place
            )
        }),
        "shared-alias source should not make its alias destination a fresh value"
    );
    assert_source_differential_replay(source, &["Agreement", "Agreement"]);
}

#[test]
fn boracle_source_differential_replays_mutable_parameter_return() {
    let source = r#"
measure |items ~{Int}| -> Int:
    shared = items
    items = {2}
    return shared.length()
;
"#;
    assert_mutable_parameter_is_write_through(source);
    let report = solve_source(source);
    let event_kinds = report
        .functions()
        .iter()
        .map(|function| {
            function
                .problem
                .events()
                .iter()
                .map(|event| &event.kind)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(
        report.functions().iter().any(|function| {
            function.problem.events().iter().any(|event| {
                matches!(
                    &event.kind,
                    EventKind::CallEffect(effect)
                        if !effect.arguments.is_empty() && effect.result.is_some()
                )
            })
        }),
        "mutable-parameter source should retain a call effect with arguments and a result, events={event_kinds:?}"
    );
    assert!(
        report.functions().iter().any(|function| {
            function.problem.events().iter().any(|event| {
                let EventKind::CallEffect(effect) = &event.kind else {
                    return false;
                };
                let Some(result) = effect.result else {
                    return false;
                };
                function.problem.uses().iter().any(|use_row| {
                    use_row.place == result.destination.place()
                        && !use_row.definition
                        && format!("{:?}", use_row.kind) == "Read"
                })
            })
        }),
        "mutable-parameter source should read the returned call result"
    );

    assert_source_differential_replay(source, &["Agreement"; 4]);
}

#[test]
fn boracle_source_differential_replays_if_branch() {
    let source = r#"
choose |seed Bool| -> Int:
    condition ~= seed
    value ~= 0
    if condition:
        value = 1
    else
        value = 2
    ;
    return value
;
"#;
    let report = solve_source(source);
    let function = report
        .functions()
        .iter()
        .find(|function| function.problem.control_flow().edges.len() >= 3)
        .expect("if source should produce a branching function report");
    let entry = function.problem.control_flow().entry;
    let value_place = function
        .problem
        .places()
        .iter()
        .filter(|place| {
            function.problem.bindings().iter().any(|binding| {
                binding.id == place.root && binding.mutable && !binding.compiler_temporary
            })
        })
        .max_by_key(|place| {
            function
                .problem
                .events()
                .iter()
                .filter(|event| {
                    let EventKind::Fresh { destination, .. } = &event.kind else {
                        return false;
                    };
                    destination.place() == place.id
                        && function
                            .problem
                            .points()
                            .iter()
                            .any(|point| point.id == event.point && point.block != entry)
                })
                .count()
        })
        .expect("if source should retain its mutable branch value");
    let arm_blocks = function
        .problem
        .events()
        .iter()
        .filter_map(|event| {
            let EventKind::Fresh {
                destination: BindingDestination::Update(destination),
                ..
            } = &event.kind
            else {
                return None;
            };
            if *destination != value_place.id {
                return None;
            }
            let point = function
                .problem
                .points()
                .iter()
                .find(|point| point.id == event.point)?;
            (point.block != entry).then_some(point.block)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        arm_blocks.len(),
        2,
        "if source should retain both arm assignments, arm blocks={arm_blocks:?}"
    );
    assert!(
        function.problem.events().iter().any(|event| {
            matches!(
                &event.kind,
                EventKind::Fresh {
                    destination: BindingDestination::Define(place),
                    ..
                } if *place == value_place.id
            )
        }),
        "if source should retain the value's initial definition separately from arm updates"
    );
    let merge_blocks = function
        .problem
        .control_flow()
        .edges
        .iter()
        .filter_map(|edge| arm_blocks.contains(&edge.from).then_some(edge.to))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        merge_blocks.len(),
        1,
        "if source should join both arm assignments at one merge block, merge blocks={merge_blocks:?}"
    );
    let merge_block = *merge_blocks
        .iter()
        .next()
        .expect("join assertion should retain one merge block");
    assert!(
        function
            .problem
            .control_flow()
            .blocks
            .iter()
            .find(|block| block.id == merge_block)
            .is_some_and(|block| {
                block.events.iter().any(|event_id| {
                    let Some(event) = function.problem.events().get(event_id.index()) else {
                        return false;
                    };
                    let EventKind::Access { use_id } = &event.kind else {
                        return false;
                    };
                    function.problem.uses().iter().any(|use_row| {
                        use_row.id == *use_id
                            && use_row.place == value_place.id
                            && !use_row.definition
                            && format!("{:?}", use_row.kind) == "Read"
                    })
                })
            }),
        "if source should read its branch value at the merge block"
    );
    let output = assert_source_differential_replay(source, &["Agreement"; 4]);
    assert!(
        output
            .lines()
            .any(|line| line == "oracle-outcome = CompleteSafe { executions: 2 }"),
        "if source should enumerate both concrete paths:\n{output}"
    );
}

#[test]
fn boracle_source_differential_replays_short_circuit_jump_arguments() {
    let source = r#"
choose |left Bool, right Bool| -> Bool:
    return left and right
;
"#;
    let report = solve_source(source);
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(&event.kind, EventKind::Terminator {
                    kind: TerminatorEventKind::Jump { arguments, .. }
                } if !arguments.is_empty())
            })
        })
        .expect("short-circuit source should produce explicit jump transfers");
    let jump_events = function
        .problem
        .events()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Terminator {
                kind: TerminatorEventKind::Jump { arguments, .. },
            } if !arguments.is_empty() => Some((event, arguments)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let transfers = jump_events
        .iter()
        .flat_map(|(_, arguments)| arguments.iter())
        .map(|argument| {
            assert!(matches!(
                argument.destination,
                BindingDestination::Define(_)
            ));
            (argument.source, argument.destination.place())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        jump_events.len(),
        2,
        "short-circuit source should define the merge value on both predecessor jumps"
    );
    assert_eq!(
        transfers.len(),
        2,
        "short-circuit source should retain both explicit edge definitions, transfers={transfers:?}"
    );
    for (jump_event, arguments) in &jump_events {
        let jump_block = function
            .problem
            .points()
            .iter()
            .find(|point| point.id == jump_event.point)
            .expect("jump event should retain its point")
            .block;
        let argument_sources = arguments
            .iter()
            .map(|argument| argument.source)
            .collect::<BTreeSet<_>>();
        let argument_destinations = arguments
            .iter()
            .map(|argument| argument.destination)
            .collect::<Vec<_>>();
        let jump_ordinal = function
            .problem
            .points()
            .iter()
            .find(|point| point.id == jump_event.point)
            .expect("jump event should retain its point")
            .ordinal;
        let mut read_points = function
            .problem
            .uses()
            .iter()
            .filter(|use_row| {
                let Some(point) = function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == use_row.point)
                else {
                    return false;
                };
                argument_sources.contains(&use_row.place)
                    && use_row.kind == UseKind::Read
                    && point.block == jump_block
                    && point.ordinal < jump_ordinal
            })
            .map(|use_row| {
                function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == use_row.point)
                    .expect("filtered read should retain its point")
                    .ordinal
            })
            .collect::<Vec<_>>();
        let mut write_points = function
            .problem
            .uses()
            .iter()
            .filter(|use_row| {
                let Some(destination) = use_row.kind.binding_destination() else {
                    return false;
                };
                let Some(point) = function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == use_row.point)
                else {
                    return false;
                };
                argument_destinations.contains(&destination)
                    && destination.place() == use_row.place
                    && point.block == jump_block
                    && point.ordinal < jump_ordinal
            })
            .map(|use_row| {
                function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == use_row.point)
                    .expect("filtered write should retain its point")
                    .ordinal
            })
            .collect::<Vec<_>>();
        read_points.sort_unstable();
        write_points.sort_unstable();
        assert!(
            !read_points.is_empty() && !write_points.is_empty(),
            "each jump should retain source reads and destination definitions in its own block"
        );
        assert!(
            read_points.last() < write_points.first(),
            "a jump should capture its argument values before defining edge destinations: reads={read_points:?} writes={write_points:?}"
        );
    }
    let merge_destinations = transfers
        .iter()
        .map(|(_, destination)| *destination)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        merge_destinations.len(),
        1,
        "short-circuit source should use one merge local for both jump arguments, transfers={transfers:?}"
    );
    let merge_destination = *merge_destinations
        .iter()
        .next()
        .expect("merge-local assertion should retain one destination");
    assert!(
        !function.problem.events().iter().any(|event| {
            matches!(
                &event.kind,
                EventKind::Fresh { destination, .. } if destination.place() == merge_destination
            )
        }),
        "short-circuit source should define its merge local through jump transfers"
    );
    assert!(
        !function
            .problem
            .events()
            .iter()
            .any(|event| match &event.kind {
                EventKind::AliasFromPlace { destination, .. }
                | EventKind::ExclusiveAliasFromPlace { destination, .. } => {
                    destination.place() == merge_destination
                }
                _ => false,
            }),
        "short-circuit merge locals should use explicit jump definitions, not alias events"
    );
    assert_source_differential_replay(source, &["Agreement"; 4]);
}

#[test]
fn boracle_source_differential_rejects_selected_experiment() {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, "value = 1\n").expect("source should be writable");

    let messages = run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        BoracleDump::Differential,
        dead_exclusive_selection(),
    )
    .expect_err("differential dump should reject an explicitly selected experiment");
    let error = format!("{messages:?}");
    assert!(
        error.contains(
            "Boracle differential dump compares every legality-changing experiment and cannot"
        ),
        "unexpected differential selection error:\n{error}"
    );
    assert!(
        error.contains("dead-exclusive-loan"),
        "selection error should name the conflicting experiment:\n{error}"
    );
}

fn assert_source_differential_replay(source: &str, expected_classes: &[&str]) -> String {
    assert!(
        expected_classes.len().is_multiple_of(2),
        "each source function should produce reference and experiment comparisons"
    );
    let first = run_source_dump(source, BoracleDump::Differential);
    let second = run_source_dump(source, BoracleDump::Differential);
    let first_without_entry = first
        .lines()
        .filter(|line| !line.starts_with("entry = "))
        .collect::<Vec<_>>();
    let second_without_entry = second
        .lines()
        .filter(|line| !line.starts_with("entry = "))
        .collect::<Vec<_>>();
    assert_eq!(
        first_without_entry, second_without_entry,
        "differential report should be deterministic apart from its temporary entry path"
    );

    assert!(
        first.contains("rule-selection = per-comparison"),
        "differential report should identify per-comparison selection:\n{first}"
    );
    assert!(
        first.contains("reference-rule-set = boracle-reference-v1"),
        "differential report should identify its reference rule set:\n{first}"
    );
    assert!(
        first.contains("experiments = dead-exclusive-loan"),
        "differential report should identify its experiment set:\n{first}"
    );

    let normalized_bodies = first
        .split("normalized-problem:\n")
        .skip(1)
        .map(|section| {
            section
                .split_once("\ncomparison 0\n")
                .expect("differential report should delimit its comparisons")
                .0
                .trim()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        normalized_bodies.len(),
        expected_classes.len() / 2,
        "each source function should render one normalized problem"
    );
    assert!(
        normalized_bodies
            .iter()
            .all(|body| body.starts_with("BorrowProblem {")),
        "every differential comparison should render a normalized problem body"
    );

    let classifications = first
        .lines()
        .filter_map(|line| line.strip_prefix("classification = "))
        .collect::<Vec<_>>();
    assert_eq!(
        classifications, expected_classes,
        "differential classifications should match the expected class for every comparison"
    );
    assert!(
        !first.lines().any(|line| line == "required-failure = true"),
        "source should not produce a required differential failure:\n{first}"
    );
    first
}

#[test]
fn boracle_default_report_uses_reference_rule_set_without_experiments() {
    let report = solve_source("value = 1\n");
    assert_eq!(
        report.rule_selection.reference_rule_set,
        crate::compiler_frontend::analysis::borrow_checker::BoracleReferenceRuleSet::V1
    );
    let output = run_source_dump("value = 1\n", BoracleDump::Problem);
    assert!(output.contains("rule-set = boracle-reference-v1"));
    assert!(output.contains("experiments = none"));
    assert!(!output.contains("experiment = "));
    assert!(report.rule_selection.experiments.is_empty());
}

#[test]
fn boracle_source_relations_and_precision_dumps_are_focused() {
    let source = r#"
items ~= {"a"}
shared = items
result = shared
"#;
    let relations = run_source_dump(source, BoracleDump::Relations);
    assert!(relations.contains("origin-registrations:"));
    assert!(relations.contains("relations:"));

    let precision = run_source_dump(source, BoracleDump::Precision);
    assert!(precision.contains("unknown-origins:"));
    assert!(precision.contains("may-alias-relations:"));
    assert!(precision.contains("mixed-generation-sets:"));
    assert!(!precision.contains("OriginSolution"));
}

#[test]
fn boracle_source_shared_alias_conflict_uses_derived_loan() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
~items.push("b")
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
~items.push("b")
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.trim_end().ends_with("[]"),
        "unexpected source conflict:\n{output}"
    );
}

#[test]
fn boracle_source_mutable_alias_issue_checks_existing_shared_alias() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
writer ~= items
result = shared
"#,
        BoracleDump::Conflicts,
    );

    assert!(
        output.contains("ConflictWitness"),
        "expected mutable alias issuance to conflict with the live shared alias, got:\n{output}"
    );
}

#[test]
fn boracle_source_copy_is_independent_from_source_alias() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
snapshot ~= copy items
~snapshot.push("b")
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
~snapshot.push(2)
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
    assert!(
        matches!(destination, BindingDestination::Define(_)),
        "a copy declaration should define an independent value binding"
    );
    let destination = destination.place();
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
fn boracle_source_copy_through_alias_stays_write_through_without_path_join() {
    let report = solve_source(
        r#"
items ~= {1}
writer ~= items
~items.push(2)
payload ~= {3}
writer = copy payload
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
                .any(|event| matches!(event.kind, EventKind::Copy { .. }))
        })
        .expect("copy through a live alias should produce a typed copy event");
    let write_through_use = function
        .problem
        .uses()
        .iter()
        .find(|use_row| {
            use_row.definition && function.report.origin.is_write_through_use(use_row.id)
        })
        .expect("copy into an alias-only destination must stay write-through");
    assert!(
        write_through_use.definition && function.report.has_conflicts(),
        "a live shared holder must still conflict with the write-through copy"
    );

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
        .expect("copy event should be present");
    assert!(
        matches!(destination, BindingDestination::Update(_)),
        "copy assignment into an existing alias should update that binding"
    );
    let destination = destination.place();
    let source_origins =
        function
            .report
            .origin
            .origins_for_place_after_event(&function.problem, copy_event, source);
    let destination_origins = function.report.origin.origins_for_place_after_event(
        &function.problem,
        copy_event,
        destination,
    );
    assert!(
        matches!(
            function
                .report
                .origin
                .relations()
                .query_overlap(&source_origins, &destination_origins)
                .expect("copy write-through overlap should validate"),
            OriginOverlapDecision::Disjoint(_)
        ),
        "copying through an alias must not PathJoin the payload with the preserved referent"
    );
}

#[test]
fn boracle_source_rebind_separates_old_alias_origin() {
    let output = run_source_dump(
        r#"
items ~= {"a"}
shared = items
items = {"b"}
~items.push("c")
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
~items.push("c")
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
    let alias_point = function
        .problem
        .events()
        .iter()
        .find(|event| event.id == alias_event)
        .and_then(|event| {
            function
                .problem
                .points()
                .iter()
                .find(|point| point.id == event.point)
        })
        .expect("alias event should retain its program point");
    assert!(
        matches!(alias_destination, BindingDestination::Define(_)),
        "the shared declaration should define its alias binding"
    );
    let alias_destination = alias_destination.place();
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
                let destination_place = &function.problem.places()[destination.place().index()];
                let event_point = function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == event.point)?;
                (event_point.block == alias_point.block
                    && event_point.ordinal > alias_point.ordinal
                    && destination_place.root == source_root)
                    .then_some((event.id, *destination))
            }
            _ => None,
        })
        .expect("fresh source rebind should retain its aggregate event");
    assert!(
        matches!(new_destination, BindingDestination::Update(_)),
        "a fresh rvalue assigned to an existing slot should update that binding"
    );
    let new_destination = new_destination.place();
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
fn boracle_source_self_update_preserves_slot_before_a_different_place_update() {
    let source = r#"
source ~= {"source"}
slot ~= {"slot"}
slot = slot
slot = source
slot = {"replacement"}
~slot.push("changed")
result = source
"#;
    let report = solve_source(source);
    let function = report
        .functions()
        .first()
        .expect("self-update source should produce a function report");
    let (slot_alias_event, slot_source, slot_destination) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ExclusiveAliasFromPlace {
                source,
                destination: BindingDestination::Update(destination),
            } => Some((event.id, *source, *destination)),
            _ => None,
        })
        .expect("a different-place update should install the mutable alias");
    let slot_alias_point = function
        .problem
        .events()
        .iter()
        .find(|event| event.id == slot_alias_event)
        .and_then(|event| {
            function
                .problem
                .points()
                .iter()
                .find(|point| point.id == event.point)
        })
        .expect("the different-place update should retain its program point");
    let slot_place = slot_destination;
    let slot_root = function.problem.places()[slot_place.index()].root;
    assert_ne!(
        function.problem.places()[slot_source.index()].root,
        slot_root
    );

    let self_update_use = function
        .problem
        .uses()
        .iter()
        .find(|use_row| {
            use_row.place == slot_place
                && use_row.kind == UseKind::Write
                && !use_row.definition
                && function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == use_row.point)
                    .is_some_and(|point| point.ordinal < slot_alias_point.ordinal)
        })
        .expect("the exact self-update should be a non-defining write access");
    let self_update_event = function
        .problem
        .events()
        .iter()
        .find(|event| {
            matches!(
                &event.kind,
                EventKind::Access { use_id } if *use_id == self_update_use.id
            )
        })
        .expect("the self-update write should retain its access event");
    let initial_slot_event = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate {
                destination: BindingDestination::Define(destination),
                ..
            } if *destination == slot_place => Some(event.id),
            _ => None,
        })
        .expect("the slot should begin with its own aggregate generation");
    let initial_slot_origins = function
        .report
        .origin
        .origins_after_event(initial_slot_event, slot_place)
        .expect("the initial slot definition should publish its provenance");
    let self_update_origins = function
        .report
        .origin
        .origins_after_event(self_update_event.id, slot_place)
        .expect("the self-update should preserve the slot provenance");
    assert!(!initial_slot_origins.is_empty());
    assert_eq!(self_update_origins, initial_slot_origins);

    let (later_write_event, later_destination) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate {
                destination: BindingDestination::Update(destination),
                ..
            } if function.problem.places()[destination.index()].root == slot_root => {
                Some((event.id, *destination))
            }
            _ => None,
        })
        .expect("a later aggregate update should still target the alias-backed slot");
    assert!(
        function
            .report
            .origin
            .is_write_through_event(later_write_event)
    );
    assert_eq!(later_destination, slot_place);

    let differential = assert_source_differential_replay(source, &["Agreement", "Agreement"]);
    let oracle_outcomes = differential
        .lines()
        .filter_map(|line| line.strip_prefix("oracle-outcome = "))
        .collect::<Vec<_>>();
    assert_eq!(oracle_outcomes.len(), 2);
    assert!(
        oracle_outcomes
            .iter()
            .all(|outcome| outcome.starts_with("CompleteSafe {")),
        "both reference/oracle comparisons should complete safely:\n{differential}"
    );
}

#[test]
fn boracle_source_place_update_aliases_and_later_update_writes_through() {
    let report = solve_source(
        r#"
source ~= {"source"}
slot ~= {"slot"}
slot = source
survivor = slot
slot = {"new"}
~slot.push("changed")
result = survivor
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(
                    &event.kind,
                    EventKind::ExclusiveAliasFromPlace {
                        destination: BindingDestination::Update(_),
                        ..
                    }
                )
            })
        })
        .expect("place-update source should produce a typed mutable alias report");
    let alias_events = function
        .problem
        .events()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::AliasFromPlace {
                source,
                destination,
            } => Some((event.id, *source, *destination)),
            EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            } => Some((event.id, *source, *destination)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (slot_alias_event, slot_source, slot_destination) = alias_events
        .iter()
        .copied()
        .find(|(_, _, destination)| matches!(destination, BindingDestination::Update(_)))
        .expect("assignment from a place should update the existing slot");
    let (survivor_alias_event, survivor_source, survivor_destination) = alias_events
        .iter()
        .copied()
        .find(|(_, _, destination)| matches!(destination, BindingDestination::Define(_)))
        .expect("alias declaration should define a separate holder");
    let slot_destination_place = slot_destination.place();
    let survivor_destination_place = survivor_destination.place();
    let slot_root = function.problem.places()[slot_destination_place.index()].root;
    let slot_alias_point = function
        .problem
        .events()
        .iter()
        .find(|event| event.id == slot_alias_event)
        .and_then(|event| {
            function
                .problem
                .points()
                .iter()
                .find(|point| point.id == event.point)
        })
        .expect("slot alias should retain its program point");
    let survivor_alias_point = function
        .problem
        .events()
        .iter()
        .find(|event| event.id == survivor_alias_event)
        .and_then(|event| {
            function
                .problem
                .points()
                .iter()
                .find(|point| point.id == event.point)
        })
        .expect("survivor alias should retain its program point");
    assert_eq!(slot_alias_point.block, survivor_alias_point.block);
    assert!(slot_alias_point.ordinal < survivor_alias_point.ordinal);
    assert_eq!(
        function.problem.places()[survivor_source.index()].root,
        slot_root
    );
    assert_ne!(
        function.problem.places()[survivor_destination_place.index()].root,
        slot_root
    );
    assert_ne!(
        function.problem.places()[slot_source.index()].root,
        slot_root
    );

    let slot_alias_origins = function
        .report
        .origin
        .origins_after_event(slot_alias_event, slot_destination_place)
        .expect("place update should publish the aliased source origins");
    let survivor_origins = function
        .report
        .origin
        .origins_after_event(survivor_alias_event, survivor_destination_place)
        .expect("the declared survivor should publish the same referent origins");
    assert!(!slot_alias_origins.is_empty());
    assert_eq!(slot_alias_origins, survivor_origins);

    let (write_through_event, write_through_destination) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate { destination, .. }
                if function
                    .problem
                    .points()
                    .iter()
                    .find(|point| point.id == event.point)
                    .is_some_and(|point| {
                        point.block == slot_alias_point.block
                            && point.ordinal > slot_alias_point.ordinal
                    })
                    && function.problem.places()[destination.place().index()].root == slot_root =>
            {
                Some((event.id, *destination))
            }
            _ => None,
        })
        .expect("slot should receive a later aggregate update");
    assert!(
        matches!(write_through_destination, BindingDestination::Update(_)),
        "an aggregate assigned to an existing local should remain an update"
    );
    let write_through_destination = write_through_destination.place();
    assert!(
        function
            .report
            .origin
            .is_write_through_event(write_through_event),
        "a later assignment to the alias-backed slot should write through"
    );
    let slot_origins_after_write = function
        .report
        .origin
        .origins_after_event(write_through_event, write_through_destination)
        .expect("write-through should preserve the slot's referent origins");
    let survivor_origins_after_write = function
        .report
        .origin
        .origins_after_event(write_through_event, survivor_destination_place)
        .expect("write-through should preserve the survivor's referent origins");
    assert!(
        slot_alias_origins == slot_origins_after_write
            && survivor_origins == survivor_origins_after_write,
        "updating an alias-backed slot should preserve its referent identity"
    );
}

#[test]
fn boracle_source_write_through_keeps_alias_loan_live_before_overlap() {
    let report = solve_source(
        r#"
items ~= {1}
writer ~= items
~items.push(2)
writer = {3}
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("write-through liveness source should produce a conflict");
    let write_through_use = function
        .problem
        .uses()
        .iter()
        .find(|use_row| {
            use_row.definition && function.report.origin.is_write_through_use(use_row.id)
        })
        .expect("direct alias write should be classified as write-through");
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| { witness.keeping_use == Some(write_through_use.id) })
    );
}

#[test]
fn boracle_source_alias_valued_write_through_is_not_a_new_loan() {
    let report = solve_source(
        r#"
items ~= {1}
writer ~= items
writer = items
result = items
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
                .any(|event| function.report.origin.is_write_through_event(event.id))
        })
        .expect("alias-valued write-through should produce a typed report");
    let write_through_event = function
        .problem
        .events()
        .iter()
        .find(|event| function.report.origin.is_write_through_event(event.id))
        .expect("write-through event should be classified by origin analysis");

    assert!(
        function
            .report
            .loans
            .loans()
            .iter()
            .all(|loan| { loan.issue_event != Some(write_through_event.id) })
    );
}

#[test]
fn boracle_source_call_result_write_through_is_a_holder_use() {
    let report = solve_source(
        r#"
observe |value {Int}| -> {Int}:
    return value
;

items ~= {1}
writer ~= items
other ~= {2}
writer = observe(value = other)
result = writer
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(&event.kind, EventKind::CallEffect(effect) if effect.result.is_some())
            })
        })
        .expect("call-result write-through should produce a typed report");
    let write_through_use = function
        .problem
        .uses()
        .iter()
        .find(|use_row| {
            use_row.definition && function.report.origin.is_write_through_use(use_row.id)
        })
        .expect("call-result destination write should be classified as write-through");
    let (call_result_event, result_destination, argument_source) = function
        .problem
        .events()
        .iter()
        .find_map(|event| {
            let EventKind::CallEffect(effect) = &event.kind else {
                return None;
            };
            Some((
                event.id,
                effect.result?.destination,
                effect.arguments.first()?.place,
            ))
        })
        .expect("call-result effect should retain its destination and argument source");
    assert!(
        matches!(result_destination, BindingDestination::Define(_)),
        "the internal call-result local should be defined by its call"
    );
    let result_binding = function
        .problem
        .places()
        .get(result_destination.place().index())
        .and_then(|place| function.problem.bindings().get(place.root.index()))
        .expect("call-result destination should resolve to its internal binding");
    assert!(
        result_binding.compiler_temporary,
        "the internal call-result binding should remain a compiler temporary"
    );
    let (rebind_event, writer_destination, source_place) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Rebind {
                destination,
                value: RebindValue::AliasFromPlace(source_place),
            } if *source_place == result_destination.place() => {
                Some((event.id, *destination, *source_place))
            }
            _ => None,
        })
        .expect("the caller assignment should rebind its destination from the call-result temp");
    assert_eq!(
        writer_destination,
        BindingDestination::Update(write_through_use.place),
        "the existing alias holder should remain the update destination"
    );
    assert_ne!(
        function.problem.places()[result_destination.place().index()].root,
        function.problem.places()[argument_source.index()].root,
        "the call-result temporary should remain separate from the argument binding"
    );
    assert_ne!(
        function.problem.places()[writer_destination.place().index()].root,
        function.problem.places()[argument_source.index()].root,
        "the caller's alias-holder binding should remain separate from the argument binding"
    );
    assert_eq!(
        source_place,
        result_destination.place(),
        "the writer rebind should consume the internal call-result temporary"
    );
    assert!(
        call_result_event < rebind_event,
        "the call should define its result before the caller rebinds from it"
    );

    assert!(
        function
            .report
            .loans
            .loans()
            .iter()
            .any(|loan| loan.uses.contains(&write_through_use.id)),
        "call-result write-through should remain a use of the alias holder"
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
fn boracle_source_mutable_parameter_existing_place_is_write_through_in_callee() {
    assert_mutable_parameter_is_write_through(
        r#"
mutate |value ~{Int}| -> {Int}:
    shared = value
    value = {2}
    result = shared
    return result
;

items ~= {1}
result = mutate(value = ~items)
"#,
    );
}

#[test]
fn boracle_source_mutable_parameter_fresh_rvalue_is_write_through_in_callee() {
    assert_mutable_parameter_is_write_through(
        r#"
mutate |value ~{Int}| -> {Int}:
    shared = value
    value = {2}
    result = shared
    return result
;

result = mutate(value = {1})
"#,
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
~unknown.push(2)
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
~unknown.push(2)
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("unknown result should retain a typed conflict");
    let (result, argument_source) = function
        .problem
        .events()
        .iter()
        .find_map(|event| {
            let EventKind::CallEffect(effect) = &event.kind else {
                return None;
            };
            Some((effect.result?, effect.arguments.first()?.place))
        })
        .expect("unknown call should retain its result row");
    assert!(
        matches!(result.destination, BindingDestination::Define(_)),
        "a call result assigned to a new binding should define that binding"
    );
    assert_ne!(
        function.problem.places()[result.destination.place().index()].root,
        function.problem.places()[argument_source.index()].root,
        "a call result binding should remain separate from its argument binding"
    );
    assert!(matches!(
        &function.problem.origins()[result.origin.index()].kind,
        OriginKind::CallResult {
            provenance: CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
            ..
        }
    ));
    assert!(
        function
            .report
            .loans
            .conflicts()
            .iter()
            .any(|witness| { matches!(witness.origin_overlap, OriginOverlapDecision::Unknown(_)) })
    );
}

#[test]
fn boracle_source_generic_call_exposes_generated_typed_boundary() {
    let report = solve_source(
        r#"
wrap type T |value T| -> {T}:
    return {value}
;

item = 1
result = wrap(value = item)
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .calls()
                .iter()
                .any(|call| call.label.starts_with("Generated("))
        })
        .expect("generic source should expose a generated call target");
    let effect = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallEffect(effect)
                if function.problem.calls()[effect.call.index()]
                    .label
                    .starts_with("Generated(") =>
            {
                Some(effect)
            }
            _ => None,
        })
        .expect("generated call should retain its normalized effect");

    assert!(matches!(
        effect.arguments.as_ref(),
        [argument] if argument.access == AccessKind::Shared
    ));
    let result = effect
        .result
        .as_ref()
        .expect("generic call should retain its result row");
    assert!(
        matches!(
            &function.problem.origins()[result.origin.index()].kind,
            OriginKind::CallResult {
                provenance: CallResultProvenance::Fresh,
                ..
            }
        ),
        "generated summary should preserve the fresh wrapper result"
    );
}

#[test]
fn boracle_source_fallible_paths_keep_success_result_loan_out_of_error_handler() {
    let report = solve_source(
        r#"
load_values |value {Int}, fail Bool| -> {Int}, Error!:
    if fail:
        return! Error("failed")
    ;

    return value
;

items ~= {1}
should_fail ~= false
loaded = load_values(value = items, fail = should_fail) catch:
    ~items.push(2)
    then {0}
;
observed = loaded
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| {
            function.problem.events().iter().any(|event| {
                matches!(
                    &event.kind,
                    EventKind::CallEffect(effect)
                        if effect.arguments.len() == 2
                            && effect
                                .arguments
                                .iter()
                                .all(|argument| argument.access == AccessKind::Shared)
                            && effect.result.is_some()
                )
            })
        })
        .expect("fallible source should expose its typed call boundary");
    let (call_event, call_id, result_place, result_origin) = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallEffect(effect)
                if effect.arguments.len() == 2
                    && effect
                        .arguments
                        .iter()
                        .all(|argument| argument.access == AccessKind::Shared) =>
            {
                effect.result.as_ref().map(|result| {
                    (
                        event.id,
                        effect.call,
                        result.destination.place(),
                        result.origin,
                    )
                })
            }
            _ => None,
        })
        .expect("fallible call should retain its result provenance");
    assert!(matches!(
        &function.problem.origins()[result_origin.index()].kind,
        OriginKind::CallResult {
            provenance: CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
            ..
        }
    ));

    let success_result_loan = function
        .report
        .loans
        .loans()
        .iter()
        .find(|loan| {
            loan.issue_event != Some(call_event)
                && loan.origins.contains(&result_origin)
                && loan.holders.as_ref() != [result_place]
                && !loan.uses.is_empty()
        })
        .expect("the success result should retain an observed provenance loan");
    assert!(!success_result_loan.live_points.is_empty());

    let failure_mutation_event = function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallArgument { call, argument, .. }
                if *call != call_id && argument.access == AccessKind::Exclusive =>
            {
                Some(event.id)
            }
            _ => None,
        })
        .expect("the error handler should retain its exclusive mutation event");
    let failure_only_mutation = function
        .report
        .loans
        .decisions()
        .iter()
        .find(|decision| decision.event == failure_mutation_event)
        .expect("the error-handler mutation should retain its access decision");
    let failure_point = function.problem.events()[failure_only_mutation.event.index()].point;
    let success_issue_point = function.problem.events()[success_result_loan
        .issue_event
        .expect("success loan has an issue event")
        .index()]
    .point;
    assert!(failure_only_mutation.allowed);
    assert_ne!(
        function.problem.points()[success_issue_point.index()].block,
        function.problem.points()[failure_point.index()].block
    );
    assert!(!success_result_loan.live_points.contains(&failure_point));
    assert!(!function.report.has_conflicts());
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
~items.push(2)
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
~items.push(2)
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
            } if !fields.is_empty() => Some((event.id, destination.place(), fields[0].source)),
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
                .then_some((event.id, destination.place()))
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
        matches!(witness.origin_overlap, OriginOverlapDecision::Overlap(_))
            && witness.loan_origins.contains(&source_origin)
    }));
}

#[test]
fn boracle_source_projected_write_is_not_a_binding_definition() {
    let report = solve_source(
        r#"
Pair = |
    first {Int},
    second {Int},
|

pair ~= Pair({1}, {2})
shared = pair
pair.first = {3}
result = shared
"#,
    );

    let function = report
        .functions()
        .first()
        .expect("source should have a function");
    assert!(
        function.report.has_conflicts(),
        "expected projected mutation to conflict with the live alias, places={:?} uses={:?} events={:?} loans={:?} decisions={:?}",
        function.problem.places(),
        function.problem.uses(),
        function.problem.events(),
        function.report.loans.loans(),
        function.report.loans.decisions()
    );
}

#[test]
fn boracle_source_projected_rebind_separates_old_projection_origin() {
    let report = solve_source(
        r#"
Pair = |
    first {Int},
    second {Int},
|

pair ~= Pair({1}, {2})
old = pair.first
pair = Pair({3}, {4})
pair.first = {5}
result = old
"#,
    );

    let function = report
        .functions()
        .first()
        .expect("source should have a function");
    assert!(
        !function.report.has_conflicts(),
        "fresh projected generation should be independent from the old projection: {:?}",
        function.report.loans.conflicts()
    );
}

#[test]
fn boracle_source_distinct_projected_fields_remain_disjoint() {
    let report = solve_source(
        r#"
Pair = |
    first {Int},
    second {Int},
|

pair ~= Pair({1}, {2})
first = pair.first
pair.second = {3}
result = first
"#,
    );

    let function = report
        .functions()
        .first()
        .expect("source should have a function");
    assert!(
        !function.report.has_conflicts(),
        "distinct projected fields should not conflict: {:?}",
        function.report.loans.conflicts()
    );
}

#[test]
fn boracle_source_base_alias_protects_fresh_projected_storage() {
    let report = solve_source(
        r#"
Pair = |
    first {Int},
    second {Int},
|

pair ~= Pair({1}, {2})
pair.first = {3}
shared = pair
pair.first = {4}
result = shared
"#,
    );

    let function = report
        .functions()
        .first()
        .expect("source should have a function");
    assert!(
        function.report.has_conflicts(),
        "a base alias should protect a freshly replaced projected field"
    );
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
~items.push(2)
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
~items.push(2)
~writer.push(3)
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
    assert!(function.report.loans.conflicts().iter().any(|witness| {
        matches!(witness.origin_overlap, OriginOverlapDecision::Overlap(_))
            && witness.keeping_use.is_some()
    }));
}

#[test]
fn boracle_source_write_through_remains_live_after_an_earlier_write() {
    let report = solve_source(
        r#"
items ~= {1}
writer ~= items
writer = {2}
shared = items
writer = {3}
result = shared
"#,
    );
    let function = report
        .functions()
        .iter()
        .find(|function| function.report.has_conflicts())
        .expect("a later holder use should keep the mutable alias live");
    let write_through_uses = function
        .problem
        .uses()
        .iter()
        .filter(|use_row| {
            use_row.definition && function.report.origin.is_write_through_use(use_row.id)
        })
        .map(|use_row| use_row.id)
        .collect::<Vec<_>>();
    assert!(
        write_through_uses.len() >= 2,
        "expected both writes through writer to remain holder uses, got {write_through_uses:?}"
    );
    assert!(function.report.loans.conflicts().iter().any(|witness| {
        witness
            .keeping_use
            .is_some_and(|use_id| write_through_uses[1..].contains(&use_id))
    }));
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
    ~items.push(2)
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
    ~items.push(3)
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

    let items_root = first_function
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate {
                destination: BindingDestination::Define(destination),
                ..
            } => {
                let root = first_function.problem.places()[destination.index()].root;
                (!first_function.problem.bindings()[root.index()].compiler_temporary)
                    .then_some(root)
            }
            _ => None,
        })
        .expect("loop source should define its initial collection slot");
    assert!(
        first_function
            .problem
            .events()
            .iter()
            .any(|event| match &event.kind {
                EventKind::AliasFromPlace {
                    source,
                    destination: BindingDestination::Define(_),
                }
                | EventKind::ExclusiveAliasFromPlace {
                    source,
                    destination: BindingDestination::Define(_),
                } => first_function.problem.places()[source.index()].root == items_root,
                _ => false,
            }),
        "the loop-local alias should be a definition on each body execution"
    );
    assert!(
        first_function
            .problem
            .events()
            .iter()
            .any(|event| match &event.kind {
                EventKind::Aggregate {
                    destination: BindingDestination::Update(destination),
                    ..
                } => first_function.problem.places()[destination.index()].root == items_root,
                _ => false,
            }),
        "the collection assignment inside the loop should update its existing slot"
    );

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
fn boracle_source_typed_report_connects_origins_loans_and_conflicts() {
    let report = solve_source(
        r#"
items ~= {"a"}
shared = items
~items.push("b")
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
            .any(|witness| { matches!(witness.origin_overlap, OriginOverlapDecision::Overlap(_)) })
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
    assert!(function.report.loans.conflicts().iter().any(|witness| {
        matches!(witness.origin_overlap, OriginOverlapDecision::Overlap(_))
            && witness.keeping_use.is_some()
    }));
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
        output.contains("provenance: Unknown") && output.contains("OpaqueExternal"),
        "expected opaque map-remove provenance, got:\n{output}"
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
            provenance: CallResultProvenance::Unknown(_),
            ..
        }
    ));
}

#[test]
fn boracle_source_branch_separates_use_and_mutation() {
    let output = run_source_dump(
        include_str!("../../../tests/cases/branch_reborrow_after_last_use/input/src/@page.moth"),
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
        include_str!("../../../tests/cases/loop_borrow_independent_roots/input/src/@page.moth"),
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

fn assert_mutable_parameter_is_write_through(source: &str) {
    let report = solve_source(source);
    let callee = report
        .functions()
        .iter()
        .find(|function| {
            function
                .problem
                .origins()
                .iter()
                .any(|origin| matches!(origin.kind, OriginKind::Parameter { .. }))
        })
        .expect("source should retain a callee parameter origin");
    let parameter_origin = callee
        .problem
        .origins()
        .iter()
        .find_map(|origin| matches!(origin.kind, OriginKind::Parameter { .. }).then_some(origin.id))
        .expect("callee should retain the parameter origin identity");
    let parameter_destination = callee
        .problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Fresh {
                destination,
                origin,
            } if *origin == parameter_origin => Some(destination.place()),
            _ => None,
        })
        .expect("parameter origin should have a defining destination");
    let parameter_binding = callee
        .problem
        .places()
        .get(parameter_destination.index())
        .and_then(|place| callee.problem.bindings().get(place.root.index()))
        .expect("parameter destination should resolve to a binding");
    assert!(
        parameter_binding.mutable && !parameter_binding.compiler_temporary,
        "parameter origin should belong to a mutable non-temporary binding"
    );
    assert!(
        callee.report.has_conflicts(),
        "a shared callee alias must conflict with a later mutable-parameter write"
    );
    assert!(
        callee.problem.uses().iter().any(|use_row| {
            use_row.definition && callee.report.origin.is_write_through_use(use_row.id)
        }),
        "mutable parameter assignment should be a write-through holder use"
    );
    let write_through = callee
        .report
        .origin
        .traces()
        .iter()
        .find(|trace| callee.report.origin.is_write_through_event(trace.event))
        .expect("callee should retain the write-through origin trace");
    let destination = write_through
        .destination
        .expect("write-through trace should identify its destination");
    let post_write_origins = callee
        .report
        .origin
        .origins_after_event(write_through.event, destination)
        .expect("write-through should publish the post-event origin state");
    assert!(
        post_write_origins.contains(&parameter_origin),
        "mutable parameter write should preserve the Parameter origin, got {post_write_origins:?}"
    );
}

fn run_source_dump(source: &str, dump: BoracleDump) -> String {
    let temporary = tempfile::tempdir().expect("temporary source directory should exist");
    let entry = temporary.path().join("main.moth");
    fs::write(&entry, source).expect("source should be writable");
    run_boracle(
        entry.to_str().expect("temporary path should be UTF-8"),
        dump,
        BoracleRuleSelection::default(),
    )
    .unwrap_or_else(|messages| panic!("source should reach Boracle: {messages:?}"))
}

fn dead_exclusive_selection() -> BoracleRuleSelection {
    BoracleRuleSelection {
        reference_rule_set:
            crate::compiler_frontend::analysis::borrow_checker::BoracleReferenceRuleSet::V1,
        experiments: BTreeSet::from([BoracleExperiment::DeadExclusiveLoan]),
    }
}
