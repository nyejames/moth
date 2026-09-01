#![cfg(feature = "boracle_campaign")]

//! Bounded generated differential campaign for the opt-in Boracle stress lane.
//!
//! The 2048 generated shapes produce 4096 problems across two cyclic modes and 8192
//! comparisons. Acyclic inputs all complete or report their generated conflict; cyclic
//! shape-zero inputs retain a truncating cycle alongside a terminal route, while conflict
//! shape-one inputs report their conflict before control flow. Completion coverage reads
//! `completed_executions` from the inconclusive outcomes rather than static CFG reachability,
//! exact class and witness counts below keep both paths covered, and a required generated
//! failure reduces before the lane fails.

use super::super::super::reducer::reduce_problem;
use super::super::super::{
    BoracleExperiment, OracleComparisonClass, compare_reference_and_experiments,
};
use super::super::generator::{GENERATED_SHAPE_COUNT, generated_problem};
use super::super::{OracleBounds, OracleLimitReason, OracleOutcome};
use crate::compiler_frontend::analysis::borrow_checker::problem::BlockId;

#[test]
fn boracle_generated_differential_campaign_has_no_required_failures() {
    let mut agreement = 0usize;
    let mut static_accepted_runtime_conflict = 0usize;
    let mut static_rejected_bounded_safe = 0usize;
    let mut oracle_inconclusive = 0usize;
    let mut malformed_problem = 0usize;
    let mut experiment_only_accepted_difference = 0usize;
    let mut problem_count = 0usize;
    let mut conflict_witness_problems = 0usize;
    let mut cyclic_complete_execution_problems = 0usize;
    let mut agreement_by_mode = [0usize; 2];
    let mut static_accepted_runtime_conflict_by_mode = [0usize; 2];
    let mut static_rejected_bounded_safe_by_mode = [0usize; 2];
    let mut oracle_inconclusive_by_mode = [0usize; 2];
    let mut malformed_problem_by_mode = [0usize; 2];
    let mut experiment_only_accepted_difference_by_mode = [0usize; 2];

    for seed in 0..GENERATED_SHAPE_COUNT {
        for cyclic in [false, true] {
            let mode = usize::from(cyclic);
            let generated = generated_problem(seed, cyclic);
            let comparison_set =
                compare_reference_and_experiments(generated.problem, OracleBounds::default())
                    .unwrap_or_else(|error| {
                        panic!(
                            "generated differential comparison failed to build or validate for \
                             seed={seed} cyclic={cyclic}: {error:?}"
                        )
                    });
            let normalized_problem = comparison_set.problem.as_ref().unwrap_or_else(|| {
                panic!(
                    "generated differential comparison returned no normalized problem for \
                     seed={seed} cyclic={cyclic}"
                )
            });
            if let Some(OracleOutcome::RuntimeConflict { trace }) =
                comparison_set.oracle_outcome.as_ref()
            {
                assert!(
                    trace.conflict.is_some(),
                    "generated runtime conflict had no witness: seed={seed} cyclic={cyclic}"
                );
                conflict_witness_problems += 1;
            }
            if cyclic {
                match comparison_set.oracle_outcome.as_ref() {
                    Some(OracleOutcome::Inconclusive {
                        reason,
                        completed_executions,
                        ..
                    }) => {
                        assert_eq!(
                            reason,
                            &OracleLimitReason::BlockEntryBound {
                                block: BlockId::new(1),
                                limit: 8,
                            },
                            "cyclic seed={seed} reached an unexpected oracle limit"
                        );
                        // Complete-route coverage is executor-observed: the enumeration must have
                        // recorded at least one completed conflict-free execution.
                        if *completed_executions > 0 {
                            cyclic_complete_execution_problems += 1;
                        }
                    }
                    Some(OracleOutcome::RuntimeConflict { .. }) => {}
                    Some(outcome) => panic!(
                        "cyclic seed={seed} should be inconclusive or conflicting, got {outcome:?}"
                    ),
                    None => panic!("cyclic seed={seed} returned no oracle outcome"),
                }
            }
            problem_count += 1;

            for comparison in comparison_set.comparisons() {
                match comparison.class {
                    OracleComparisonClass::Agreement => {
                        agreement += 1;
                        agreement_by_mode[mode] += 1;
                    }
                    OracleComparisonClass::StaticAcceptedRuntimeConflict => {
                        static_accepted_runtime_conflict += 1;
                        static_accepted_runtime_conflict_by_mode[mode] += 1;
                    }
                    OracleComparisonClass::StaticRejectedBoundedSafe => {
                        static_rejected_bounded_safe += 1;
                        static_rejected_bounded_safe_by_mode[mode] += 1;
                    }
                    OracleComparisonClass::OracleInconclusive => {
                        oracle_inconclusive += 1;
                        oracle_inconclusive_by_mode[mode] += 1;
                    }
                    OracleComparisonClass::MalformedProblem => {
                        malformed_problem += 1;
                        malformed_problem_by_mode[mode] += 1;
                    }
                    OracleComparisonClass::ExperimentOnlyAcceptedDifference => {
                        experiment_only_accepted_difference += 1;
                        experiment_only_accepted_difference_by_mode[mode] += 1;
                    }
                }

                if comparison.class.severity().is_required_failure() {
                    let original_classes = comparison_set
                        .comparisons()
                        .map(|entry| entry.class)
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                    match reduce_problem(normalized_problem.clone(), OracleBounds::default()) {
                        Ok(reduced) => {
                            assert_eq!(
                                reduced.comparison_classes(),
                                original_classes.as_ref(),
                                "reducer changed the comparison-class vector while reducing a \
                                 required failure: seed={seed} cyclic={cyclic} class={:?}",
                                comparison.class,
                            );
                            panic!(
                                "generated differential required failure: seed={seed} \
                                 cyclic={cyclic} class={:?} severity={:?} experiment={} \
                                 applied-passes={:?} preserved-comparison-classes={:?} \
                                 original-normalized-problem:\n{} \
                                 reduced-normalized-problem:\n{} \
                                 reduced-fixture-skeleton:\n{}",
                                comparison.class,
                                comparison.class.severity(),
                                comparison.rule_selection.experiment_names(),
                                reduced.applied_passes(),
                                reduced.comparison_classes(),
                                normalized_problem.debug_dump(),
                                reduced.problem.debug_dump(),
                                reduced.fixture_skeleton(),
                            );
                        }
                        Err(error) => panic!(
                            "generated differential required failure failed to reduce: \
                             seed={seed} cyclic={cyclic} class={:?} reducer-error={error:?} \
                             original-normalized-problem:\n{}",
                            comparison.class,
                            normalized_problem.debug_dump(),
                        ),
                    }
                }
            }
        }
    }

    let expected_problems = 4096;
    assert_eq!(
        problem_count, expected_problems,
        "generated differential campaign did not account for every problem"
    );
    let comparisons_per_problem = 1 + BoracleExperiment::ALL
        .iter()
        .filter(|experiment| experiment.metadata().may_change_legality)
        .count();
    let expected_comparisons = expected_problems * comparisons_per_problem;
    let accounted_comparisons = agreement
        + static_accepted_runtime_conflict
        + static_rejected_bounded_safe
        + oracle_inconclusive
        + malformed_problem
        + experiment_only_accepted_difference;
    assert_eq!(
        accounted_comparisons, expected_comparisons,
        "generated differential campaign did not account for every comparison"
    );

    assert_eq!(
        agreement, 6144,
        "generated differential Agreement class distribution changed"
    );
    assert_eq!(
        static_accepted_runtime_conflict, 0,
        "generated differential StaticAcceptedRuntimeConflict class distribution changed"
    );
    assert_eq!(
        static_rejected_bounded_safe, 0,
        "generated differential StaticRejectedBoundedSafe class distribution changed"
    );
    assert_eq!(
        oracle_inconclusive, 2048,
        "generated differential OracleInconclusive class distribution changed"
    );
    assert_eq!(
        malformed_problem, 0,
        "generated differential MalformedProblem class distribution changed"
    );
    assert_eq!(
        experiment_only_accepted_difference, 0,
        "generated differential ExperimentOnlyAcceptedDifference class distribution changed"
    );
    assert_eq!(
        agreement_by_mode,
        [4096, 2048],
        "generated differential Agreement class distribution by mode changed \
         ([acyclic, cyclic])"
    );
    assert_eq!(
        static_accepted_runtime_conflict_by_mode,
        [0, 0],
        "generated differential StaticAcceptedRuntimeConflict class distribution by mode changed \
         ([acyclic, cyclic])"
    );
    assert_eq!(
        static_rejected_bounded_safe_by_mode,
        [0, 0],
        "generated differential StaticRejectedBoundedSafe class distribution by mode changed \
         ([acyclic, cyclic])"
    );
    assert_eq!(
        oracle_inconclusive_by_mode,
        [0, 2048],
        "generated differential OracleInconclusive class distribution by mode changed \
         ([acyclic, cyclic])"
    );
    assert_eq!(
        malformed_problem_by_mode,
        [0, 0],
        "generated differential MalformedProblem class distribution by mode changed \
         ([acyclic, cyclic])"
    );
    assert_eq!(
        experiment_only_accepted_difference_by_mode,
        [0, 0],
        "generated differential ExperimentOnlyAcceptedDifference class distribution by mode changed \
         ([acyclic, cyclic])"
    );
    assert_eq!(
        conflict_witness_problems, 2048,
        "generated problems with conflict witnesses changed"
    );
    assert_eq!(
        cyclic_complete_execution_problems, 1024,
        "cyclic generated problems with complete executions changed"
    );
}
