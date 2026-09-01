//! Invariants for deterministic bounded problem generation.

use super::super::generator::{GENERATED_SHAPE_COUNT, generated_problem};
use super::super::{OracleBounds, OracleOutcome, execute_bounded};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    BorrowProblem, CallResultProvenance, EventKind, OriginKind, PlaceId, TerminatorEventKind,
    UseKind,
};
use std::collections::BTreeSet;

#[test]
fn boracle_generated_shape_space_is_valid_and_executable() {
    let mut checked = 0usize;
    let mut complete_executions = 0usize;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            assert_eq!(generated.seed, seed, "generator did not retain seed {seed}");
            assert_eq!(
                generated.cyclic, cyclic,
                "generator did not retain cyclic flag"
            );
            generated
                .problem
                .validate()
                .unwrap_or_else(|error| panic!("generated seed {seed} should validate: {error:?}"));
            let outcome = execute_bounded(&generated.problem, OracleBounds::default())
                .unwrap_or_else(|error| {
                    panic!(
                        "generated seed {seed} cyclic={cyclic} should execute without CompilerError: \
                         {error:?}"
                    )
                });
            let completed_executions = match &outcome {
                OracleOutcome::CompleteSafe { executions, .. } => *executions,
                _ => 0,
            };
            let is_complete = matches!(&outcome, OracleOutcome::CompleteSafe { .. });
            let is_conflict = if let OracleOutcome::RuntimeConflict { trace } = &outcome {
                assert!(
                    trace.conflict.is_some(),
                    "generated runtime conflict had no witness: seed={seed} cyclic={cyclic}"
                );
                true
            } else {
                false
            };
            if !cyclic {
                assert!(
                    is_complete || is_conflict,
                    "acyclic generated seed {seed} must complete or report a runtime conflict, \
                     got {outcome:?}"
                );
            }
            complete_executions += completed_executions;
            checked += 1;
        }
    }

    assert!(
        complete_executions > 0,
        "generated shape space produced no complete oracle execution"
    );
    assert_eq!(
        checked,
        (GENERATED_SHAPE_COUNT as usize) * 2,
        "the validity sweep did not cover both cyclic modes for every shape"
    );
}

#[test]
fn boracle_generated_shape_space_contains_every_required_family() {
    let mut block_shape_variants = BTreeSet::new();
    let mut branches = false;
    let mut back_edges = false;
    let mut fresh_origins = false;
    let mut fresh_events = false;
    let mut aliases = false;
    let mut copies = false;
    let mut projections = false;
    let mut aggregates = false;
    let mut call_arguments = false;
    let mut call_effects = false;
    let mut loan_kills = false;
    let mut scope_exits = false;

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let generated = generated_problem(seed, cyclic);
            let problem = &generated.problem;
            for block in &problem.control_flow().blocks {
                let Some(event_id) = block.events.last() else {
                    continue;
                };
                if let EventKind::Terminator {
                    kind: TerminatorEventKind::Branch { targets },
                } = &problem.events()[event_id.index()].kind
                {
                    branches = true;
                    block_shape_variants.insert(targets.len());
                }
            }
            back_edges |= problem
                .control_flow()
                .edges
                .iter()
                .any(|edge| edge.to.raw() <= edge.from.raw());
            fresh_origins |= problem
                .origins()
                .iter()
                .any(|origin| matches!(&origin.kind, OriginKind::Fresh));
            for event in problem.events() {
                match &event.kind {
                    EventKind::Fresh { .. } => fresh_events = true,
                    EventKind::AliasFromPlace { .. }
                    | EventKind::ExclusiveAliasFromPlace { .. }
                    | EventKind::Alias { .. }
                    | EventKind::ExclusiveAlias { .. } => aliases = true,
                    EventKind::Copy { .. } => copies = true,
                    EventKind::Projection { .. } => projections = true,
                    EventKind::Aggregate { .. } => aggregates = true,
                    EventKind::CallArgument { .. } => call_arguments = true,
                    EventKind::CallEffect(_) => call_effects = true,
                    EventKind::LoanKill { .. } => loan_kills = true,
                    EventKind::ScopeExit { .. } => scope_exits = true,
                    _ => {}
                }
            }
        }
    }

    assert!(
        block_shape_variants.len() >= 2,
        "block-shape digit emitted only one branch target shape: {block_shape_variants:?}"
    );
    assert!(branches, "generated shape space did not emit a branch");
    assert!(back_edges, "generated shape space did not emit a back edge");
    assert!(
        fresh_origins && fresh_events,
        "generated shape space did not emit both a fresh origin and Fresh event"
    );
    assert!(aliases, "generated shape space did not emit an alias");
    assert!(copies, "generated shape space did not emit a copy");
    assert!(
        projections,
        "generated shape space did not emit a projection"
    );
    assert!(
        aggregates,
        "generated shape space did not emit an aggregate"
    );
    assert!(
        call_arguments,
        "generated shape space did not emit a CallArgument event"
    );
    assert!(
        call_effects,
        "generated shape space did not emit a CallEffect event"
    );
    assert!(loan_kills, "generated shape space did not emit a loan kill");
    assert!(
        scope_exits,
        "generated shape space did not emit a scope exit"
    );
}

fn block_terminator(problem: &BorrowProblem, block: usize) -> &TerminatorEventKind {
    let event_id = problem.control_flow().blocks[block]
        .events
        .last()
        .expect("generated block should end in a terminator");
    let event = &problem.events()[event_id.index()];
    let EventKind::Terminator { kind } = &event.kind else {
        panic!("generated block {block} did not end in a terminator event");
    };
    kind
}

#[test]
fn boracle_generated_seed_digits_select_independent_choices() {
    let has_observation = |problem: &BorrowProblem| {
        problem
            .events()
            .iter()
            .any(|event| matches!(&event.kind, EventKind::ReactiveObserve { .. }))
    };
    let fresh_event_count = |problem: &BorrowProblem| {
        problem
            .events()
            .iter()
            .filter(|event| matches!(&event.kind, EventKind::Fresh { .. }))
            .count()
    };
    let has_shared_alias = |problem: &BorrowProblem| {
        problem
            .events()
            .iter()
            .any(|event| matches!(&event.kind, EventKind::AliasFromPlace { .. }))
    };
    let has_exclusive_alias = |problem: &BorrowProblem| {
        problem
            .events()
            .iter()
            .any(|event| matches!(&event.kind, EventKind::ExclusiveAliasFromPlace { .. }))
    };
    let copy_destination = |problem: &BorrowProblem| {
        problem.events().iter().find_map(|event| {
            if let EventKind::Copy { destination, .. } = &event.kind {
                Some(*destination)
            } else {
                None
            }
        })
    };
    let projection_kind = |problem: &BorrowProblem| {
        problem.origins().iter().find_map(|origin| {
            if let OriginKind::Projection { projection, .. } = &origin.kind {
                Some(*projection)
            } else {
                None
            }
        })
    };
    let aggregate_field_count = |problem: &BorrowProblem| {
        problem.events().iter().find_map(|event| {
            if let EventKind::Aggregate { fields, .. } = &event.kind {
                Some(fields.len())
            } else {
                None
            }
        })
    };
    let has_fresh_call_result = |problem: &BorrowProblem| {
        problem.origins().iter().any(|origin| {
            matches!(
                &origin.kind,
                OriginKind::CallResult {
                    provenance: CallResultProvenance::Fresh,
                    ..
                }
            )
        })
    };
    let has_alias_params_call_result = |problem: &BorrowProblem| {
        problem.origins().iter().any(|origin| {
            matches!(
                &origin.kind,
                OriginKind::CallResult {
                    provenance: CallResultProvenance::AliasParams(_),
                    ..
                }
            )
        })
    };
    let loan_kind = |problem: &BorrowProblem| problem.loans().first().map(|loan| loan.kind);
    let scope_exit_binding_count = |problem: &BorrowProblem| {
        problem.events().iter().find_map(|event| {
            if let EventKind::ScopeExit { bindings } = &event.kind {
                Some(bindings.len())
            } else {
                None
            }
        })
    };
    let has_explicit_loan_kill = |problem: &BorrowProblem| {
        problem
            .events()
            .iter()
            .any(|event| matches!(&event.kind, EventKind::LoanKill { .. }))
    };
    let has_nondefining_owner_write = |problem: &BorrowProblem| {
        problem.uses().iter().any(|use_row| {
            use_row.place == PlaceId::new(0)
                && use_row.kind == UseKind::Write
                && !use_row.definition
        })
    };

    for digit in 0..=10 {
        let cyclic = digit == 2;
        let baseline = generated_problem(0, cyclic);
        let variant = generated_problem(1_u32 << digit, cyclic);
        match digit {
            0 => assert_ne!(
                baseline.problem.control_flow().blocks.len(),
                variant.problem.control_flow().blocks.len(),
                "block-shape digit did not change the block family"
            ),
            1 => {
                assert_ne!(
                    block_terminator(&baseline.problem, 1),
                    block_terminator(&variant.problem, 1),
                    "branch-shape digit did not change the branch successor family"
                );
                assert!(!has_observation(&baseline.problem));
                assert!(has_observation(&variant.problem));
            }
            2 => assert_ne!(
                block_terminator(&baseline.problem, 1),
                block_terminator(&variant.problem, 1),
                "back-edge digit did not change the cyclic successor family"
            ),
            3 => assert_ne!(
                fresh_event_count(&baseline.problem),
                fresh_event_count(&variant.problem),
                "fresh-origin digit did not change the Fresh event family"
            ),
            4 => {
                assert_ne!(
                    has_shared_alias(&baseline.problem),
                    has_shared_alias(&variant.problem),
                    "alias digit did not change the alias access family"
                );
                assert_ne!(
                    has_exclusive_alias(&baseline.problem),
                    has_exclusive_alias(&variant.problem),
                    "alias digit did not change the exclusive-alias family"
                );
            }
            5 => assert_ne!(
                copy_destination(&baseline.problem),
                copy_destination(&variant.problem),
                "copy digit did not change the copy destination family"
            ),
            6 => assert_ne!(
                projection_kind(&baseline.problem),
                projection_kind(&variant.problem),
                "projection digit did not change the projection family"
            ),
            7 => assert_ne!(
                aggregate_field_count(&baseline.problem),
                aggregate_field_count(&variant.problem),
                "aggregate digit did not change the aggregate field family"
            ),
            8 => {
                assert_ne!(
                    has_fresh_call_result(&baseline.problem),
                    has_fresh_call_result(&variant.problem),
                    "call-provenance digit did not change the call result family"
                );
                assert_ne!(
                    has_alias_params_call_result(&baseline.problem),
                    has_alias_params_call_result(&variant.problem),
                    "call-provenance digit did not change the alias-parameter family"
                );
            }
            9 => {
                assert_ne!(
                    loan_kind(&baseline.problem),
                    loan_kind(&variant.problem),
                    "cleanup digit did not change the loan cleanup family"
                );
                assert_ne!(
                    scope_exit_binding_count(&baseline.problem),
                    scope_exit_binding_count(&variant.problem),
                    "cleanup digit did not change the scope-exit family"
                );
            }
            10 => {
                assert!(has_explicit_loan_kill(&baseline.problem));
                assert!(!has_explicit_loan_kill(&variant.problem));
                assert!(!has_nondefining_owner_write(&baseline.problem));
                assert!(has_nondefining_owner_write(&variant.problem));
            }
            _ => unreachable!(),
        }
        assert_ne!(
            baseline.problem.debug_dump(),
            variant.problem.debug_dump(),
            "seed digit {digit} did not select a distinct normalized shape"
        );
    }
    for seed in 0..(GENERATED_SHAPE_COUNT / 2) {
        let generated = generated_problem(seed, false);
        assert!(
            has_explicit_loan_kill(&generated.problem),
            "conflict_shape=0 acyclic seed {seed} lost its original loan kill"
        );
        assert!(
            !has_nondefining_owner_write(&generated.problem),
            "conflict_shape=0 acyclic seed {seed} gained the conflict probe"
        );
    }

    let first = generated_problem(7, false);
    let repeated = generated_problem(GENERATED_SHAPE_COUNT + 7, false);
    assert_eq!(
        first.problem.debug_dump(),
        repeated.problem.debug_dump(),
        "shape mapping did not repeat at GENERATED_SHAPE_COUNT"
    );
    assert_ne!(first.seed, repeated.seed, "raw seed was not retained");
}

#[test]
fn boracle_generated_problems_have_byte_identical_debug_dumps() {
    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let first = generated_problem(seed, cyclic);
            let second = generated_problem(seed, cyclic);
            assert_eq!(
                first.problem.debug_dump(),
                second.problem.debug_dump(),
                "same seed produced different normalized input: seed={seed} cyclic={cyclic}"
            );
        }
    }
}
