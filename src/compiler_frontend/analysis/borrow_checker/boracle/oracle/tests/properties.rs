//! Metamorphic properties of the bounded operational oracle.
//!
//! These properties intentionally exercise a different semantic owner from the static tests in
//! `boracle/tests/mod.rs`:
//!
//! - replacing an alias with a copy checks dynamic conflict outcomes, rather than the static
//!   `OriginSolver` copy-origin relation asserted by `boracle_generated_metamorphic_properties_preserve_semantics`.
//! - fresh rebinding checks a concrete old-alias/new-owner execution, rather than comparing static
//!   `ValueOriginId` rows in `boracle_generated_problems_preserve_copy_and_rebind_semantics`.
//! - an unreachable use checks bounded execution counts and traces, rather than the static report
//!   signature compared by `boracle_generated_metamorphic_properties_preserve_semantics`.
//! - deleting a final use checks the public runtime-conflict implication, rather than static
//!   `FutureUseStatus` and transfer-candidate facts in `boracle_optional_transfer_requires_a_proven_final_use`.
//! - branch renumbering checks concrete CFG path outcomes, rather than static binding renumbering
//!   and branch-splitting signatures in `boracle_generated_metamorphic_properties_preserve_semantics`.
//! - repeated execution compares rendered `OracleOutcome` bytes, including operational traces and
//!   limit reasons, rather than only normalized-input or reference-report dumps in the generator
//!   and static determinism tests.
//!
//! `Inconclusive` is never treated as evidence for the semantic implications. A pair containing
//! one is skipped, while a changed complete/conflict result remains an assertion failure.
//!
//! Cyclic shape-zero pairs contain an inconclusive outcome because block 1 rebinds on each entry
//! and the bounded enumeration truncates at `max_block_entries`; conflict shape-one pairs find
//! their generated conflict before reaching that cycle. The measured campaign therefore has 1024
//! cyclic truncations and 3072 non-truncated generated pairs in each property.

use super::super::generator::{GENERATED_SHAPE_COUNT, generated_problem};
use super::super::{OracleBounds, OracleOutcome, execute_bounded};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, BlockId, BorrowProblem, BorrowProblemParts, CfgBlock, CfgEdge, Event, EventId,
    EventKind, EventSource, Loan, OriginKind, PlaceId, PointId, ProgramPoint, RebindValue,
    TerminatorEventKind, Use, UseId, UseKind, ValueOriginId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

#[test]
fn boracle_operational_replacing_alias_with_copy_cannot_create_runtime_conflict() {
    let mut transformed_shapes = 0usize;
    let mut skipped_not_applicable = 0usize;
    let mut skipped_invalid = 0usize;
    let mut skipped_inconclusive = 0usize;
    let mut copy_safe_shapes = 0usize;
    let mut informative_copy_safe_shapes = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            let alias_variant = match add_alias_conflict_probe(&generated.problem) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            let transformed = match replace_alias_with_copy(&alias_variant) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            assert_ne!(
                alias_variant.debug_dump(),
                transformed.debug_dump(),
                "alias-to-copy transformation was a no-op for seed={seed} cyclic={cyclic}"
            );
            assert!(
                copy_event_count(&transformed) > copy_event_count(&alias_variant),
                "alias-to-copy transformation did not add a copy event for seed={seed} cyclic={cyclic}"
            );
            transformed_shapes += 1;

            let original_outcome = run(&alias_variant, seed, cyclic, "alias replacement");
            let transformed_outcome = run(&transformed, seed, cyclic, "alias replacement");
            let copy_removed_conflict = has_runtime_conflict(&original_outcome)
                && !has_runtime_conflict(&transformed_outcome);
            if copy_removed_conflict {
                copy_safe_shapes += 1;
            }
            if has_inconclusive(&original_outcome) || has_inconclusive(&transformed_outcome) {
                skipped_inconclusive += 1;
                continue;
            }
            if copy_removed_conflict {
                informative_copy_safe_shapes += 1;
            }

            assert!(
                !has_runtime_conflict(&transformed_outcome)
                    || has_runtime_conflict(&original_outcome),
                "replacing an alias with a copy created a runtime alias conflict for seed={seed} cyclic={cyclic}:\noriginal={}\ntransformed={}",
                render_outcome(&original_outcome),
                render_outcome(&transformed_outcome),
            );
        }
    }
    assert_eq!(
        transformed_shapes + skipped_not_applicable + skipped_invalid,
        (GENERATED_SHAPE_COUNT as usize) * 2,
        "alias-to-copy property did not account for every generated shape"
    );
    assert_eq!(
        skipped_not_applicable, 0,
        "alias-to-copy property could not build its conflict probe for a generated shape"
    );
    assert_eq!(
        skipped_invalid, 0,
        "alias-to-copy property produced invalid transformed problems"
    );
    assert_eq!(
        copy_safe_shapes, 2048,
        "alias-to-copy property changed its measured conflict-removal count"
    );
    assert_eq!(
        informative_copy_safe_shapes, 1024,
        "alias-to-copy property lost an informative copy-safety pair"
    );
    assert_eq!(
        skipped_inconclusive, 1024,
        "alias-to-copy property did not observe the measured cyclic truncation count"
    );
    assert_eq!(
        transformed_shapes - skipped_inconclusive,
        3072,
        "alias-to-copy property lost an informative execution"
    );
}

#[test]
fn boracle_operational_alias_to_copy_fixture_removes_runtime_conflict() {
    let (alias_problem, copy_problem) = alias_copy_conflict_fixture();
    let alias_outcome = run(&alias_problem, 0, false, "alias-copy fixture");
    let copy_outcome = run(&copy_problem, 0, false, "copy fixture");
    assert!(
        has_runtime_conflict(&alias_outcome),
        "purpose-built alias fixture did not reach a runtime conflict:\n{}",
        render_outcome(&alias_outcome),
    );
    assert!(
        !has_runtime_conflict(&copy_outcome),
        "purpose-built copy fixture reached a runtime conflict:\n{}",
        render_outcome(&copy_outcome),
    );
}

#[test]
fn boracle_operational_fresh_rebinding_separates_dynamic_generation() {
    let mut transformed_shapes = 0usize;
    let mut skipped_not_applicable = 0usize;
    let mut skipped_invalid = 0usize;
    let mut skipped_inconclusive = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            let transformed = match add_fresh_rebinding_probe(&generated.problem, cyclic) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            assert_ne!(
                generated.problem.debug_dump(),
                transformed.debug_dump(),
                "fresh-rebinding probe was a no-op for seed={seed} cyclic={cyclic}"
            );
            assert_eq!(
                transformed.events().len(),
                generated.problem.events().len() + 3,
                "fresh-rebinding probe did not add alias/read/write events for seed={seed} cyclic={cyclic}"
            );
            transformed_shapes += 1;

            let original_outcome = run(&generated.problem, seed, cyclic, "fresh rebinding");
            let transformed_outcome = run(&transformed, seed, cyclic, "fresh rebinding");
            if has_inconclusive(&original_outcome) || has_inconclusive(&transformed_outcome) {
                skipped_inconclusive += 1;
                continue;
            }

            assert!(
                !has_runtime_conflict(&transformed_outcome)
                    || has_runtime_conflict(&original_outcome),
                "fresh rebinding failed to separate the old alias generation for seed={seed} cyclic={cyclic}:\noriginal={}\ntransformed={}",
                render_outcome(&original_outcome),
                render_outcome(&transformed_outcome),
            );
        }
    }

    assert_eq!(
        transformed_shapes + skipped_not_applicable + skipped_invalid,
        4096,
        "fresh-rebinding property did not account for every generated shape"
    );
    assert_eq!(
        skipped_not_applicable, 0,
        "fresh-rebinding property could not select a probe destination for a generated shape"
    );
    assert_eq!(
        skipped_invalid, 0,
        "fresh-rebinding property produced invalid transformed problems"
    );
    assert_eq!(
        skipped_inconclusive, 1024,
        "fresh-rebinding property did not observe the expected cyclic truncation count"
    );
    assert_eq!(
        transformed_shapes - skipped_inconclusive,
        3072,
        "fresh-rebinding property lost an informative acyclic execution"
    );
}

#[test]
fn boracle_operational_unreachable_use_changes_no_complete_execution() {
    let mut transformed_shapes = 0usize;
    let mut skipped_not_applicable = 0usize;
    let mut skipped_invalid = 0usize;
    let mut skipped_inconclusive = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            let transformed = match append_unreachable_use(&generated.problem) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            assert_ne!(
                generated.problem.debug_dump(),
                transformed.debug_dump(),
                "unreachable-use transformation was a no-op for seed={seed} cyclic={cyclic}"
            );
            assert_eq!(
                transformed.control_flow().blocks.len(),
                generated.problem.control_flow().blocks.len() + 1,
                "unreachable-use transformation did not add a CFG block for seed={seed} cyclic={cyclic}"
            );
            transformed_shapes += 1;

            let original_outcome = run(&generated.problem, seed, cyclic, "unreachable use");
            let transformed_outcome = run(&transformed, seed, cyclic, "unreachable use");
            if has_inconclusive(&original_outcome) || has_inconclusive(&transformed_outcome) {
                skipped_inconclusive += 1;
                continue;
            }

            match (&original_outcome, &transformed_outcome) {
                (
                    OracleOutcome::CompleteSafe {
                        executions: original_executions,
                        ..
                    },
                    OracleOutcome::CompleteSafe {
                        executions: transformed_executions,
                        ..
                    },
                ) => assert_eq!(
                    original_executions, transformed_executions,
                    "adding an unreachable use changed complete execution count for seed={seed} cyclic={cyclic}"
                ),
                (OracleOutcome::RuntimeConflict { .. }, OracleOutcome::RuntimeConflict { .. }) => {
                    assert_eq!(
                        render_outcome(&original_outcome),
                        render_outcome(&transformed_outcome),
                        "adding an unreachable use changed the reachable conflict trace for seed={seed} cyclic={cyclic}"
                    );
                }
                _ => panic!(
                    "adding an unreachable use changed a complete runtime outcome for seed={seed} cyclic={cyclic}:\noriginal={}\ntransformed={}",
                    render_outcome(&original_outcome),
                    render_outcome(&transformed_outcome),
                ),
            }
        }
    }

    assert_eq!(
        transformed_shapes + skipped_not_applicable + skipped_invalid,
        4096,
        "unreachable-use property did not account for every generated shape"
    );
    assert_eq!(
        skipped_not_applicable, 0,
        "unreachable-use property could not build a transformed problem for a generated shape"
    );
    assert_eq!(
        skipped_invalid, 0,
        "unreachable-use property produced invalid transformed problems"
    );
    assert_eq!(
        skipped_inconclusive, 1024,
        "unreachable-use property did not observe the expected cyclic truncation count"
    );
    assert_eq!(
        transformed_shapes - skipped_inconclusive,
        3072,
        "unreachable-use property lost an informative acyclic execution"
    );
}

#[test]
fn boracle_operational_deleting_final_use_cannot_extend_capability_usability() {
    let mut transformed_shapes = 0usize;
    let mut skipped_not_applicable = 0usize;
    let mut skipped_invalid = 0usize;
    let mut skipped_inconclusive = 0usize;
    let mut complete_non_conflicting_pairs = 0usize;
    let mut complete_conflicting_pairs = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            let with_final_use = match add_final_capability_use(&generated.problem) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            assert_ne!(
                generated.problem.debug_dump(),
                with_final_use.debug_dump(),
                "final-use probe was a no-op for seed={seed} cyclic={cyclic}"
            );
            assert_eq!(
                with_final_use.events().len(),
                generated.problem.events().len() + 3,
                "final-use probe did not add alias/read/write events for seed={seed} cyclic={cyclic}"
            );
            let transformed = match delete_final_use(&with_final_use) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            assert_ne!(
                with_final_use.debug_dump(),
                transformed.debug_dump(),
                "final-use deletion was a no-op for seed={seed} cyclic={cyclic}"
            );
            assert_eq!(
                transformed.events().len() + 1,
                with_final_use.events().len(),
                "final-use deletion did not remove exactly one event for seed={seed} cyclic={cyclic}"
            );
            assert_eq!(
                transformed.uses().len() + 1,
                with_final_use.uses().len(),
                "final-use deletion did not remove exactly one use for seed={seed} cyclic={cyclic}"
            );
            transformed_shapes += 1;

            let original_outcome = run(&with_final_use, seed, cyclic, "final-use deletion");
            let transformed_outcome = run(&transformed, seed, cyclic, "final-use deletion");
            if has_inconclusive(&original_outcome) || has_inconclusive(&transformed_outcome) {
                skipped_inconclusive += 1;
                continue;
            }
            if matches!(
                (&original_outcome, &transformed_outcome),
                (
                    OracleOutcome::RuntimeConflict { .. },
                    OracleOutcome::CompleteSafe { .. }
                )
            ) {
                complete_conflicting_pairs += 1;
            }

            if matches!(
                (&original_outcome, &transformed_outcome),
                (
                    OracleOutcome::CompleteSafe { .. },
                    OracleOutcome::CompleteSafe { .. }
                )
            ) {
                complete_non_conflicting_pairs += 1;
            }

            assert!(
                !has_runtime_conflict(&transformed_outcome)
                    || has_runtime_conflict(&original_outcome),
                "deleting a final use created a runtime conflict and therefore extended capability usability for seed={seed} cyclic={cyclic}:\noriginal={}\ntransformed={}",
                render_outcome(&original_outcome),
                render_outcome(&transformed_outcome),
            );
        }
    }

    let (fixture_with_final_use, owner_write_event) = final_use_conflict_fixture();
    let fixture_without_final_use = delete_final_use(&fixture_with_final_use)
        .expect("purpose-built final-use fixture should transform")
        .expect("purpose-built final-use fixture should expose a final read");
    let fixture_original_outcome = run(
        &fixture_with_final_use,
        0,
        false,
        "final-use conflict fixture",
    );
    let fixture_transformed_outcome = run(
        &fixture_without_final_use,
        0,
        false,
        "final-use conflict fixture",
    );
    let OracleOutcome::RuntimeConflict { trace } = &fixture_original_outcome else {
        panic!(
            "purpose-built final-use fixture should reach a runtime conflict:\n{}",
            render_outcome(&fixture_original_outcome)
        );
    };
    assert_eq!(
        trace.conflict.as_ref().map(|witness| witness.access_event),
        Some(owner_write_event),
        "purpose-built final-use fixture conflict should be the owner write"
    );
    assert!(
        matches!(
            fixture_transformed_outcome,
            OracleOutcome::CompleteSafe { .. }
        ),
        "deleting the purpose-built final use should remove the runtime conflict:\n{}",
        render_outcome(&fixture_transformed_outcome)
    );
    complete_conflicting_pairs += 1;

    assert_eq!(
        transformed_shapes + skipped_not_applicable + skipped_invalid,
        4096,
        "final-use property did not account for every generated shape"
    );
    assert_eq!(
        skipped_not_applicable, 0,
        "final-use property could not select a probe destination for a generated shape"
    );
    assert_eq!(
        skipped_invalid, 0,
        "final-use property produced invalid transformed problems"
    );
    assert_eq!(
        skipped_inconclusive, 1024,
        "final-use property did not observe the measured cyclic truncation count"
    );
    assert_eq!(
        complete_non_conflicting_pairs, 1024,
        "final-use property lost measured complete non-conflicting pairs"
    );
    assert_eq!(
        complete_conflicting_pairs, 1,
        "final-use property lost its conflict-bearing complete partition"
    );
    assert_eq!(
        transformed_shapes - skipped_inconclusive,
        3072,
        "final-use property lost an informative execution"
    );
}

#[test]
fn boracle_operational_branch_renumbering_preserves_complete_outcomes() {
    let mut transformed_shapes = 0usize;
    let mut skipped_not_applicable = 0usize;
    let mut skipped_invalid = 0usize;
    let mut skipped_inconclusive = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            let transformed = match renumber_branch_blocks(&generated.problem) {
                Ok(Some(problem)) => problem,
                Ok(None) => {
                    skipped_not_applicable += 1;
                    continue;
                }
                Err(_) => {
                    skipped_invalid += 1;
                    continue;
                }
            };
            assert_ne!(
                generated.problem.debug_dump(),
                transformed.debug_dump(),
                "branch-renumbering transformation was a no-op for seed={seed} cyclic={cyclic}"
            );
            transformed_shapes += 1;

            let original_outcome = run(&generated.problem, seed, cyclic, "branch renumbering");
            let transformed_outcome = run(&transformed, seed, cyclic, "branch renumbering");
            if has_inconclusive(&original_outcome) || has_inconclusive(&transformed_outcome) {
                skipped_inconclusive += 1;
                continue;
            }

            match (&original_outcome, &transformed_outcome) {
                (
                    OracleOutcome::CompleteSafe {
                        executions: original_executions,
                        ..
                    },
                    OracleOutcome::CompleteSafe {
                        executions: transformed_executions,
                        ..
                    },
                ) => assert_eq!(
                    original_executions, transformed_executions,
                    "branch renumbering changed complete execution count for seed={seed} cyclic={cyclic}"
                ),
                (OracleOutcome::RuntimeConflict { .. }, OracleOutcome::RuntimeConflict { .. }) => {}
                _ => panic!(
                    "branch renumbering changed a complete runtime outcome for seed={seed} cyclic={cyclic}:\noriginal={}\ntransformed={}",
                    render_outcome(&original_outcome),
                    render_outcome(&transformed_outcome),
                ),
            }
        }
    }

    assert_eq!(
        transformed_shapes + skipped_not_applicable + skipped_invalid,
        4096,
        "branch-renumbering property did not account for every generated shape"
    );
    assert_eq!(
        skipped_not_applicable, 0,
        "branch-renumbering property could not build a transformed problem for a generated shape"
    );
    assert_eq!(
        skipped_invalid, 0,
        "branch-renumbering property produced invalid transformed problems"
    );
    assert_eq!(
        skipped_inconclusive, 1024,
        "branch-renumbering property did not observe the expected cyclic truncation count"
    );
    assert_eq!(
        transformed_shapes - skipped_inconclusive,
        3072,
        "branch-renumbering property lost an informative acyclic execution"
    );
}

#[test]
fn boracle_operational_repeated_execution_is_byte_deterministic() {
    let mut repeated_shapes = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let first = generated_problem(seed, cyclic);
            let repeated = generated_problem(seed, cyclic);
            let first_outcome = run(&first.problem, seed, cyclic, "repeated execution");
            let repeated_outcome = run(&repeated.problem, seed, cyclic, "repeated execution");
            let first_rendered = render_outcome(&first_outcome);
            let repeated_rendered = render_outcome(&repeated_outcome);
            assert_eq!(
                first_rendered.as_bytes(),
                repeated_rendered.as_bytes(),
                "repeated bounded execution was not byte-for-byte deterministic for seed={seed} cyclic={cyclic}"
            );
            repeated_shapes += 1;
        }
    }

    let (conflict_fixture, _) = alias_copy_conflict_fixture();
    let first_conflict = run(&conflict_fixture, 0, false, "repeated conflict fixture");
    let repeated_conflict = run(&conflict_fixture, 0, false, "repeated conflict fixture");
    for outcome in [&first_conflict, &repeated_conflict] {
        let OracleOutcome::RuntimeConflict { trace } = outcome else {
            panic!("runtime-conflict fixture did not reach a conflict: {outcome:?}");
        };
        assert!(
            !trace.entries().is_empty(),
            "runtime-conflict fixture rendered no trace entries"
        );
        assert!(
            trace.conflict.is_some(),
            "runtime-conflict fixture rendered no conflict witness"
        );
    }
    assert_eq!(
        render_outcome(&first_conflict).as_bytes(),
        render_outcome(&repeated_conflict).as_bytes(),
        "repeated runtime-conflict execution was not byte-for-byte deterministic"
    );

    assert_eq!(
        repeated_shapes, 4096,
        "repeated-execution property did not rerun every generated shape"
    );
}

fn run(problem: &BorrowProblem, seed: u32, cyclic: bool, property: &str) -> OracleOutcome {
    execute_bounded(problem, OracleBounds::default()).unwrap_or_else(|error| {
        panic!(
            "operational property {property:?} failed to execute seed={seed} cyclic={cyclic}: {error:?}\nproblem:\n{}",
            problem.debug_dump()
        )
    })
}

fn render_outcome(outcome: &OracleOutcome) -> String {
    format!("{outcome:#?}")
}

fn has_inconclusive(outcome: &OracleOutcome) -> bool {
    matches!(outcome, OracleOutcome::Inconclusive { .. })
}

fn has_runtime_conflict(outcome: &OracleOutcome) -> bool {
    matches!(outcome, OracleOutcome::RuntimeConflict { .. })
}

struct ProblemRows {
    points: Vec<ProgramPoint>,
    blocks: Vec<CfgBlock>,
    edges: Vec<CfgEdge>,
    entry: BlockId,
    exits: Vec<BlockId>,
    loans: Vec<Loan>,
    uses: Vec<Use>,
    events: Vec<Event>,
}

fn rebuild_problem(
    problem: &BorrowProblem,
    rows: ProblemRows,
) -> Result<BorrowProblem, CompilerError> {
    BorrowProblem::new(BorrowProblemParts {
        bindings: problem.bindings().to_vec(),
        points: rows.points,
        blocks: rows.blocks,
        edges: rows.edges,
        entry: rows.entry,
        exits: rows.exits,
        places: problem.places().to_vec(),
        origins: problem.origins().to_vec(),
        loans: rows.loans,
        uses: rows.uses,
        calls: problem.calls().to_vec(),
        events: rows.events,
    })
}

fn replace_alias_with_copy(
    problem: &BorrowProblem,
) -> Result<Option<BorrowProblem>, CompilerError> {
    let Some(copy_origin) = problem
        .origins()
        .iter()
        .find_map(|origin| matches!(&origin.kind, OriginKind::Copy(_)).then_some(origin.id))
    else {
        return Ok(None);
    };
    let Some(alias_index) = problem.events().iter().position(|event| {
        matches!(
            event.kind,
            EventKind::Alias { .. }
                | EventKind::AliasFromPlace { .. }
                | EventKind::ExclusiveAlias { .. }
                | EventKind::ExclusiveAliasFromPlace { .. }
        )
    }) else {
        return Ok(None);
    };

    let (source, destination) = match &problem.events()[alias_index].kind {
        EventKind::Alias {
            source,
            destination,
            ..
        }
        | EventKind::AliasFromPlace {
            source,
            destination,
        }
        | EventKind::ExclusiveAlias {
            source,
            destination,
            ..
        }
        | EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => (*source, *destination),
        _ => return Ok(None),
    };
    let mut events = problem.events().to_vec();
    events[alias_index].kind = EventKind::Copy {
        source,
        destination,
        origin: copy_origin,
    };
    rebuild_problem(
        problem,
        ProblemRows {
            points: problem.points().to_vec(),
            blocks: problem.control_flow().blocks.to_vec(),
            edges: problem.control_flow().edges.to_vec(),
            entry: problem.control_flow().entry,
            exits: problem.control_flow().exits.to_vec(),
            loans: problem.loans().to_vec(),
            uses: problem.uses().to_vec(),
            events,
        },
    )
    .map(Some)
}

fn copy_event_count(problem: &BorrowProblem) -> usize {
    problem
        .events()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::Copy { .. }))
        .count()
}

fn alias_copy_conflict_fixture() -> (BorrowProblem, BorrowProblem) {
    let mut fixture = super::Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    let _copy_origin = fixture.origin(OriginKind::Copy(
        vec![ValueOriginId::new(0)].into_boxed_slice(),
    ));
    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);
    let alias_problem = fixture.finish();
    let copy_problem = replace_alias_with_copy(&alias_problem)
        .expect("purpose-built alias fixture should validate")
        .expect("purpose-built alias fixture should expose a copy origin");
    (alias_problem, copy_problem)
}

// WHY: The first alias read exercises the capability, the owner write lies inside its interval and
// the final alias read keeps the interval live. Deleting only that final read removes the conflict.
fn final_use_conflict_fixture() -> (BorrowProblem, EventId) {
    let mut fixture = super::Fixture::new(2);
    let owner = fixture.place(0, []);
    let alias = fixture.place(1, []);
    fixture.fresh(owner);
    fixture.alias(alias, owner, AccessKind::Shared);
    fixture.access(alias, UseKind::Read, false);
    let owner_write_event = fixture.access_event(owner, UseKind::Write, false);
    fixture.access(alias, UseKind::Read, false);
    (fixture.finish(), owner_write_event)
}

fn add_alias_conflict_probe(
    problem: &BorrowProblem,
) -> Result<Option<BorrowProblem>, CompilerError> {
    let Some((alias_block, alias_event)) = problem.events().iter().find_map(|event| {
        let is_alias = match &event.kind {
            EventKind::AliasFromPlace {
                source,
                destination,
            }
            | EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            } => *source == PlaceId::new(0) && *destination == PlaceId::new(5),
            _ => false,
        };
        is_alias
            .then(|| {
                problem
                    .control_flow()
                    .blocks
                    .iter()
                    .find(|block| block.events.contains(&event.id))
                    .map(|block| (block.id, event.id))
            })
            .flatten()
    }) else {
        return Ok(None);
    };

    let mut points = problem.points().to_vec();
    let mut uses = problem.uses().to_vec();
    let mut events = problem.events().to_vec();
    let (_, owner_write_event) = append_access(
        &mut points,
        &mut uses,
        &mut events,
        alias_block,
        PlaceId::new(0),
        UseKind::Write,
        false,
    );
    let (_, alias_access_event) = append_access(
        &mut points,
        &mut uses,
        &mut events,
        alias_block,
        PlaceId::new(5),
        UseKind::Read,
        false,
    );

    let mut blocks = problem.control_flow().blocks.to_vec();
    insert_event_after(&mut blocks, alias_block, alias_event, owner_write_event)?;
    insert_event_after(
        &mut blocks,
        alias_block,
        owner_write_event,
        alias_access_event,
    )?;
    refresh_block_bounds_and_ordinals(&mut blocks, &mut points, &events)?;
    rebuild_problem(
        problem,
        ProblemRows {
            points,
            blocks,
            edges: problem.control_flow().edges.to_vec(),
            entry: problem.control_flow().entry,
            exits: problem.control_flow().exits.to_vec(),
            loans: problem.loans().to_vec(),
            uses,
            events,
        },
    )
    .map(Some)
}

fn unreferenced_probe_destination(problem: &BorrowProblem) -> Option<PlaceId> {
    problem
        .places()
        .iter()
        .map(|place| place.id)
        .find(|candidate| {
            problem
                .events()
                .iter()
                .all(|event| !event_references_place(problem, event, *candidate))
        })
}

fn event_references_place(problem: &BorrowProblem, event: &Event, candidate: PlaceId) -> bool {
    match &event.kind {
        EventKind::Fresh { destination, .. }
        | EventKind::Copy { destination, .. }
        | EventKind::Projection { destination, .. } => *destination == candidate,
        EventKind::Alias {
            source,
            destination,
            ..
        }
        | EventKind::AliasFromPlace {
            source,
            destination,
        }
        | EventKind::ExclusiveAlias {
            source,
            destination,
            ..
        }
        | EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => *source == candidate || *destination == candidate,
        EventKind::Rebind { destination, value } => {
            *destination == candidate
                || matches!(value, RebindValue::AliasFromPlace(source) if *source == candidate)
        }
        EventKind::Aggregate {
            destination,
            fields,
            ..
        } => *destination == candidate || fields.iter().any(|field| field.source == candidate),
        EventKind::ScopeExit { bindings } => problem
            .places()
            .iter()
            .find(|place| place.id == candidate)
            .is_some_and(|place| bindings.contains(&place.root)),
        EventKind::ReactiveObserve { place } => *place == candidate,
        EventKind::CallArgument { argument, .. } => argument.place == candidate,
        EventKind::CallEffect(effect) => {
            effect
                .arguments
                .iter()
                .any(|argument| argument.place == candidate)
                || effect
                    .result
                    .is_some_and(|result| result.place == candidate)
        }
        EventKind::Access { use_id } => problem
            .uses()
            .get(use_id.index())
            .is_some_and(|use_row| use_row.place == candidate),
        EventKind::LoanIssue { loan } | EventKind::LoanKill { loan, .. } => {
            problem.loans().get(loan.index()).is_some_and(|loan_row| {
                loan_row.place == candidate || loan_row.holders.contains(&candidate)
            })
        }
        EventKind::Terminator { .. } => false,
    }
}

fn assert_probe_destination(problem: &BorrowProblem, property: &str) -> PlaceId {
    let destination = unreferenced_probe_destination(problem);
    assert!(
        destination.is_some(),
        "{property} could not find a place that no event references"
    );
    destination.expect("probe destination assertion should establish a place")
}

fn add_fresh_rebinding_probe(
    problem: &BorrowProblem,
    cyclic: bool,
) -> Result<Option<BorrowProblem>, CompilerError> {
    let Some((rebind_block, rebind_event)) = problem.events().iter().find_map(|event| {
        matches!(
            event.kind,
            EventKind::Rebind {
                value: RebindValue::Fresh(_),
                ..
            }
        )
        .then(|| {
            let block = problem
                .control_flow()
                .blocks
                .iter()
                .find(|block| block.events.contains(&event.id))
                .map(|block| block.id);
            block.map(|block| (block, event.id))
        })
        .flatten()
    }) else {
        return Ok(None);
    };
    let probe_destination = assert_probe_destination(problem, "fresh-rebinding probe");

    let mut points = problem.points().to_vec();
    let mut uses = problem.uses().to_vec();
    let mut events = problem.events().to_vec();
    let alias_event = append_event(
        &mut points,
        &mut events,
        BlockId::new(0),
        EventKind::AliasFromPlace {
            source: PlaceId::new(0),
            destination: probe_destination,
        },
    );
    let (_, owner_write_event) = append_access(
        &mut points,
        &mut uses,
        &mut events,
        rebind_block,
        PlaceId::new(0),
        UseKind::Write,
        false,
    );
    let (_, alias_access_event) = append_access(
        &mut points,
        &mut uses,
        &mut events,
        rebind_block,
        probe_destination,
        UseKind::Read,
        false,
    );

    let mut blocks = problem.control_flow().blocks.to_vec();
    let alias_anchor = if cyclic {
        blocks[BlockId::new(0).index()]
            .events
            .iter()
            .find(|event_id| matches!(events[event_id.index()].kind, EventKind::ScopeExit { .. }))
            .copied()
    } else {
        Some(rebind_event)
    };
    let Some(alias_anchor) = alias_anchor else {
        return Ok(None);
    };
    insert_event_before(&mut blocks, BlockId::new(0), alias_anchor, alias_event)?;
    insert_event_after(&mut blocks, rebind_block, rebind_event, owner_write_event)?;
    insert_event_after(
        &mut blocks,
        rebind_block,
        owner_write_event,
        alias_access_event,
    )?;

    refresh_block_bounds_and_ordinals(&mut blocks, &mut points, &events)?;
    rebuild_problem(
        problem,
        ProblemRows {
            points,
            blocks,
            edges: problem.control_flow().edges.to_vec(),
            entry: problem.control_flow().entry,
            exits: problem.control_flow().exits.to_vec(),
            loans: problem.loans().to_vec(),
            uses,
            events,
        },
    )
    .map(Some)
}

fn add_final_capability_use(
    problem: &BorrowProblem,
) -> Result<Option<BorrowProblem>, CompilerError> {
    let block_zero = problem
        .control_flow()
        .blocks
        .first()
        .ok_or_else(|| CompilerError::compiler_error("generated problem has no entry block"))?;
    let Some(scope_exit) = block_zero.events.iter().find(|event_id| {
        matches!(
            problem.events()[event_id.index()].kind,
            EventKind::ScopeExit { .. }
        )
    }) else {
        return Ok(None);
    };
    let probe_destination = assert_probe_destination(problem, "final-use probe");

    let mut points = problem.points().to_vec();
    let mut uses = problem.uses().to_vec();
    let mut events = problem.events().to_vec();
    let alias_event = append_event(
        &mut points,
        &mut events,
        block_zero.id,
        EventKind::AliasFromPlace {
            source: PlaceId::new(0),
            destination: probe_destination,
        },
    );
    let (_, final_use_event) = append_access(
        &mut points,
        &mut uses,
        &mut events,
        block_zero.id,
        probe_destination,
        UseKind::Read,
        false,
    );
    let (_, owner_write_event) = append_access(
        &mut points,
        &mut uses,
        &mut events,
        block_zero.id,
        PlaceId::new(0),
        UseKind::Write,
        false,
    );
    let mut blocks = problem.control_flow().blocks.to_vec();
    insert_event_before(&mut blocks, block_zero.id, *scope_exit, alias_event)?;
    insert_event_before(&mut blocks, block_zero.id, *scope_exit, final_use_event)?;
    insert_event_after(
        &mut blocks,
        block_zero.id,
        final_use_event,
        owner_write_event,
    )?;
    refresh_block_bounds_and_ordinals(&mut blocks, &mut points, &events)?;
    rebuild_problem(
        problem,
        ProblemRows {
            points,
            blocks,
            edges: problem.control_flow().edges.to_vec(),
            entry: problem.control_flow().entry,
            exits: problem.control_flow().exits.to_vec(),
            loans: problem.loans().to_vec(),
            uses,
            events,
        },
    )
    .map(Some)
}

fn append_event(
    points: &mut Vec<ProgramPoint>,
    events: &mut Vec<Event>,
    block: BlockId,
    kind: EventKind,
) -> EventId {
    let event_id = EventId::new(events.len() as u32);
    let point_id = PointId::new(points.len() as u32);
    points.push(ProgramPoint::new(point_id, block, 0));
    events.push(Event::new(event_id, point_id, kind, EventSource::none()));
    event_id
}

fn append_access(
    points: &mut Vec<ProgramPoint>,
    uses: &mut Vec<Use>,
    events: &mut Vec<Event>,
    block: BlockId,
    place: PlaceId,
    kind: UseKind,
    definition: bool,
) -> (UseId, EventId) {
    let use_id = UseId::new(uses.len() as u32);
    let event_id = EventId::new(events.len() as u32);
    let point_id = PointId::new(points.len() as u32);
    points.push(ProgramPoint::new(point_id, block, 0));
    uses.push(Use {
        id: use_id,
        point: point_id,
        place,
        kind,
        definition,
    });
    events.push(Event::new(
        event_id,
        point_id,
        EventKind::Access { use_id },
        EventSource::none(),
    ));
    (use_id, event_id)
}

fn insert_event_before(
    blocks: &mut [CfgBlock],
    block: BlockId,
    anchor: EventId,
    inserted: EventId,
) -> Result<(), CompilerError> {
    insert_event(blocks, block, anchor, inserted, true)
}

fn insert_event_after(
    blocks: &mut [CfgBlock],
    block: BlockId,
    anchor: EventId,
    inserted: EventId,
) -> Result<(), CompilerError> {
    insert_event(blocks, block, anchor, inserted, false)
}

fn insert_event(
    blocks: &mut [CfgBlock],
    block: BlockId,
    anchor: EventId,
    inserted: EventId,
    before: bool,
) -> Result<(), CompilerError> {
    let block_row = blocks.get_mut(block.index()).ok_or_else(|| {
        CompilerError::compiler_error(format!("property probe cannot locate block {block:?}"))
    })?;
    let position = block_row
        .events
        .iter()
        .position(|event_id| *event_id == anchor)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "property probe cannot locate anchor event {anchor:?} in block {block:?}"
            ))
        })?;
    let insertion = if before { position } else { position + 1 };
    let mut event_ids = block_row.events.to_vec();
    event_ids.insert(insertion, inserted);
    block_row.events = event_ids.into_boxed_slice();
    Ok(())
}

fn refresh_block_bounds_and_ordinals(
    blocks: &mut [CfgBlock],
    points: &mut [ProgramPoint],
    events: &[Event],
) -> Result<(), CompilerError> {
    for block in blocks {
        let Some(first_event) = block.events.first().copied() else {
            return Err(CompilerError::compiler_error(format!(
                "property transformation emptied block {:?}",
                block.id
            )));
        };
        let Some(last_event) = block.events.last().copied() else {
            return Err(CompilerError::compiler_error(format!(
                "property transformation emptied block {:?}",
                block.id
            )));
        };
        block.entry = events[first_event.index()].point;
        block.exit = events[last_event.index()].point;
        for (ordinal, event_id) in block.events.iter().copied().enumerate() {
            points[events[event_id.index()].point.index()].ordinal = ordinal as u32;
        }
    }
    Ok(())
}

fn append_unreachable_use(problem: &BorrowProblem) -> Result<Option<BorrowProblem>, CompilerError> {
    let next_block = BlockId::new(problem.control_flow().blocks.len() as u32);
    let access_point = PointId::new(problem.points().len() as u32);
    let exit_point = PointId::new(access_point.raw() + 1);
    let next_use = UseId::new(problem.uses().len() as u32);
    let access_event = EventId::new(problem.events().len() as u32);
    let terminator_event = EventId::new(problem.events().len() as u32 + 1);

    let mut points = problem.points().to_vec();
    points.extend([
        ProgramPoint::new(access_point, next_block, 0),
        ProgramPoint::new(exit_point, next_block, 1),
    ]);
    let mut uses = problem.uses().to_vec();
    uses.push(Use {
        id: next_use,
        point: access_point,
        place: PlaceId::new(0),
        kind: UseKind::Read,
        definition: false,
    });
    let mut events = problem.events().to_vec();
    events.push(Event::new(
        access_event,
        access_point,
        EventKind::Access { use_id: next_use },
        EventSource::none(),
    ));
    events.push(Event::new(
        terminator_event,
        exit_point,
        EventKind::Terminator {
            kind: TerminatorEventKind::Return,
        },
        EventSource::none(),
    ));
    let mut blocks = problem.control_flow().blocks.to_vec();
    blocks.push(CfgBlock::new(
        next_block,
        access_point,
        exit_point,
        vec![access_event, terminator_event],
    ));
    let mut exits = problem.control_flow().exits.to_vec();
    exits.push(next_block);

    rebuild_problem(
        problem,
        ProblemRows {
            points,
            blocks,
            edges: problem.control_flow().edges.to_vec(),
            entry: problem.control_flow().entry,
            exits,
            loans: problem.loans().to_vec(),
            uses,
            events,
        },
    )
    .map(Some)
}

fn delete_final_use(problem: &BorrowProblem) -> Result<Option<BorrowProblem>, CompilerError> {
    let Some((event_to_remove, use_to_remove)) = final_read_access(problem) else {
        return Ok(None);
    };
    let mut uses = Vec::with_capacity(problem.uses().len().saturating_sub(1));
    for use_row in problem.uses() {
        if use_row.id == use_to_remove {
            continue;
        }
        let mut use_row = use_row.clone();
        use_row.id = remap_use_id(use_row.id, use_to_remove);
        uses.push(use_row);
    }

    let mut events = Vec::with_capacity(problem.events().len().saturating_sub(1));
    for event in problem.events() {
        if event.id == event_to_remove {
            continue;
        }
        let mut event = event.clone();
        event.id = remap_event_id(event.id, event_to_remove);
        if !remap_event_use_references(&mut event.kind, use_to_remove) {
            return Ok(None);
        }
        events.push(event);
    }

    let mut blocks = problem.control_flow().blocks.to_vec();
    for block in &mut blocks {
        let mut event_ids = Vec::with_capacity(block.events.len().saturating_sub(1));
        for event_id in block.events.iter().copied() {
            if event_id == event_to_remove {
                continue;
            }
            event_ids.push(remap_event_id(event_id, event_to_remove));
        }
        block.events = event_ids.into_boxed_slice();
    }

    let mut loans = problem.loans().to_vec();
    for loan in &mut loans {
        let mut loan_uses = Vec::with_capacity(loan.uses.len());
        for use_id in loan.uses.iter().copied() {
            if use_id == use_to_remove {
                continue;
            }
            loan_uses.push(remap_use_id(use_id, use_to_remove));
        }
        loan.uses = loan_uses.into_boxed_slice();
    }

    rebuild_problem(
        problem,
        ProblemRows {
            points: problem.points().to_vec(),
            blocks,
            edges: problem.control_flow().edges.to_vec(),
            entry: problem.control_flow().entry,
            exits: problem.control_flow().exits.to_vec(),
            loans,
            uses,
            events,
        },
    )
    .map(Some)
}

fn final_read_access(problem: &BorrowProblem) -> Option<(EventId, UseId)> {
    problem.events().iter().rev().find_map(|event| {
        let EventKind::Access { use_id } = event.kind else {
            return None;
        };
        let use_row = problem.uses().get(use_id.index())?;
        if use_row.definition || !matches!(use_row.kind, UseKind::Read | UseKind::LoanObservation) {
            return None;
        }
        let has_later_use = problem.events().iter().any(|later| {
            if later.id.raw() <= event.id.raw() {
                return false;
            }
            match &later.kind {
                EventKind::Access { use_id } => problem
                    .uses()
                    .get(use_id.index())
                    .is_some_and(|later_use| later_use.place == use_row.place),
                EventKind::CallArgument { argument, .. } => argument.place == use_row.place,
                _ => false,
            }
        });
        (!has_later_use).then_some((event.id, use_id))
    })
}

fn remap_use_id(id: UseId, removed: UseId) -> UseId {
    UseId::new(id.raw() - u32::from(id.raw() > removed.raw()))
}

fn remap_event_id(id: EventId, removed: EventId) -> EventId {
    EventId::new(id.raw() - u32::from(id.raw() > removed.raw()))
}

fn remap_event_use_references(kind: &mut EventKind, removed: UseId) -> bool {
    match kind {
        EventKind::CallArgument { argument, .. } => {
            if argument.use_id == removed {
                return false;
            }
            argument.use_id = remap_use_id(argument.use_id, removed);
        }
        EventKind::CallEffect(effect) => {
            for argument in &mut effect.arguments {
                if argument.use_id == removed {
                    return false;
                }
                argument.use_id = remap_use_id(argument.use_id, removed);
            }
        }
        EventKind::Access { use_id } => {
            if *use_id == removed {
                return false;
            }
            *use_id = remap_use_id(*use_id, removed);
        }
        _ => {}
    }
    true
}

fn renumber_branch_blocks(problem: &BorrowProblem) -> Result<Option<BorrowProblem>, CompilerError> {
    if problem.control_flow().blocks.len() < 3 {
        return Ok(None);
    }
    let remap_block = |block: BlockId| match block.raw() {
        1 => BlockId::new(2),
        2 => BlockId::new(1),
        _ => block,
    };

    let mut points = problem.points().to_vec();
    for point in &mut points {
        point.block = remap_block(point.block);
    }
    let mut blocks = problem.control_flow().blocks.to_vec();
    for block in &mut blocks {
        block.id = remap_block(block.id);
    }
    blocks.sort_by_key(|block| block.id);

    let mut edges = problem.control_flow().edges.to_vec();
    for edge in &mut edges {
        edge.from = remap_block(edge.from);
        edge.to = remap_block(edge.to);
    }
    let entry = remap_block(problem.control_flow().entry);
    let exits = problem
        .control_flow()
        .exits
        .iter()
        .copied()
        .map(remap_block)
        .collect();

    let mut events = problem.events().to_vec();
    for event in &mut events {
        let EventKind::Terminator { kind } = &mut event.kind else {
            continue;
        };
        match kind {
            TerminatorEventKind::Jump { target }
            | TerminatorEventKind::Break { target }
            | TerminatorEventKind::Continue { target } => {
                *target = remap_block(*target);
            }
            TerminatorEventKind::Branch { targets } => {
                let mut remapped = targets.iter().copied().map(remap_block).collect::<Vec<_>>();
                remapped.sort_unstable();
                *targets = remapped.into_boxed_slice();
            }
            TerminatorEventKind::Return
            | TerminatorEventKind::ReturnSuccess
            | TerminatorEventKind::ReturnError
            | TerminatorEventKind::RuntimeFailure
            | TerminatorEventKind::AssertFailure => {}
        }
    }

    rebuild_problem(
        problem,
        ProblemRows {
            points,
            blocks,
            edges,
            entry,
            exits,
            loans: problem.loans().to_vec(),
            uses: problem.uses().to_vec(),
            events,
        },
    )
    .map(Some)
}
