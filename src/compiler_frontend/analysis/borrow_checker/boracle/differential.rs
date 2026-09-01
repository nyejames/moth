//! Differential comparison between the Boracle reference solver and bounded oracle.
//!
//! WHAT: compares one normalized borrow problem against concrete bounded executions and classifies
//!       each selected static rule result.
//! WHY: a disagreement is useful only when its static rule-set identity, runtime completeness and
//!      witnesses remain visible together.

// `compare_problem_parts` is the Phase 4 generator/reducer seam; `experiments()` and
// malformed-problem classification support future corpus reporting. Keep the typed differential
// API warning-free until those callers land.
#![allow(dead_code)]

use super::super::problem::{BorrowProblem, BorrowProblemParts};
use super::oracle::{OracleBounds, OracleOutcome, execute_bounded};
use super::report::{BoracleReport, BoracleSolver};
use super::service::{BoracleExperiment, BoracleRuleSelection, format_experiment_names};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// The classified result of comparing one static rule selection with the bounded oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OracleComparisonClass {
    Agreement,
    StaticAcceptedRuntimeConflict,
    StaticRejectedBoundedSafe,
    OracleInconclusive,
    MalformedProblem,
    ExperimentOnlyAcceptedDifference,
}

impl OracleComparisonClass {
    pub(crate) const fn severity(self) -> OracleComparisonSeverity {
        match self {
            Self::StaticAcceptedRuntimeConflict => OracleComparisonSeverity::SoundnessFailure,
            Self::StaticRejectedBoundedSafe => OracleComparisonSeverity::PrecisionCandidate,
            Self::MalformedProblem => OracleComparisonSeverity::MalformedInput,
            Self::Agreement | Self::OracleInconclusive | Self::ExperimentOnlyAcceptedDifference => {
                OracleComparisonSeverity::Informational
            }
        }
    }
}

/// Severity assigned to one differential comparison class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OracleComparisonSeverity {
    SoundnessFailure,
    PrecisionCandidate,
    MalformedInput,
    Informational,
}

impl OracleComparisonSeverity {
    pub(crate) const fn is_required_failure(self) -> bool {
        matches!(self, Self::SoundnessFailure)
    }
}

/// One static rule-selection result paired with the bounded oracle outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OracleComparison {
    pub(crate) class: OracleComparisonClass,
    pub(crate) rule_selection: BoracleRuleSelection,
    pub(crate) static_report: Option<BoracleReport>,
}

/// Every comparison for one problem, with the reference selection held apart from the experiments.
///
/// Reference mode is compared first. Storing it in its own field rather than as the head of one
/// slice makes that unrepresentable otherwise, so no caller can render an experiment as the
/// reference and no accessor has to index a slice that might be empty.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OracleComparisonSet {
    pub(crate) problem: Option<BorrowProblem>,
    pub(crate) bounds: OracleBounds,
    pub(crate) oracle_outcome: Option<OracleOutcome>,
    pub(crate) reference: OracleComparison,
    pub(crate) experiments: Box<[OracleComparison]>,
}

impl OracleComparisonSet {
    /// Every comparison, reference first.
    pub(crate) fn comparisons(&self) -> impl Iterator<Item = &OracleComparison> {
        std::iter::once(&self.reference).chain(self.experiments.iter())
    }

    /// Render the complete deterministic differential report.
    pub(crate) fn report_dump(&self) -> String {
        let mut output = String::new();
        let reference = &self.reference;
        output.push_str("Boracle differential report\n");
        output.push_str(&format!(
            "reference-rule-set = {}\n",
            reference.rule_selection.reference_rule_set.name()
        ));
        output.push_str(&format!(
            "experiments = {}\n",
            format_experiment_names(
                self.comparisons().flat_map(|comparison| {
                    comparison.rule_selection.experiments.iter().copied()
                })
            )
        ));
        output.push_str(&format!("bounds = {:#?}\n", self.bounds));
        output.push_str("normalized-problem:\n");
        if let Some(problem) = self.problem.as_ref() {
            append_dump(&mut output, &problem.debug_dump());
        } else {
            output.push_str("none\n");
        }

        for (index, comparison) in self.comparisons().enumerate() {
            output.push_str(&format!("\ncomparison {index}\n"));
            append_rule_selection(&mut output, &comparison.rule_selection);
            output.push_str(&format!("classification = {:?}\n", comparison.class));
            output.push_str(&format!("severity = {:?}\n", comparison.class.severity()));
            output.push_str(&format!(
                "required-failure = {}\n",
                comparison.class.severity().is_required_failure()
            ));

            output.push_str("static-conflicts:\n");
            if let Some(report) = comparison.static_report.as_ref() {
                append_dump(&mut output, &report.conflicts_debug_dump());
            } else {
                output.push_str("none\n");
            }
            output.push_str("static-witnesses:\n");
            if let Some(report) = comparison.static_report.as_ref() {
                append_dump(&mut output, &report.witnesses_debug_dump());
            } else {
                output.push_str("none\n");
            }
            append_oracle_dump(&mut output, self.oracle_outcome.as_ref());
        }

        output
    }
}

/// Compare normalized problem parts against the reference rule-set and every legality-changing
/// experiment. A validation failure is returned as a classified malformed comparison set.
pub(crate) fn compare_problem_parts(
    parts: BorrowProblemParts,
    bounds: OracleBounds,
) -> Result<OracleComparisonSet, CompilerError> {
    let (reference, experiments) = reference_and_experiment_selections();
    match BorrowProblem::new(parts) {
        Ok(problem) => compare_valid_problem(problem, reference, experiments, bounds),
        Err(_) => malformed_comparison_set(reference, experiments, bounds),
    }
}

/// Compare the reference rule-set with every legality-changing experiment.
pub(crate) fn compare_reference_and_experiments(
    problem: BorrowProblem,
    bounds: OracleBounds,
) -> Result<OracleComparisonSet, CompilerError> {
    let (reference, experiments) = reference_and_experiment_selections();
    compare_valid_problem(problem, reference, experiments, bounds)
}

fn compare_valid_problem(
    problem: BorrowProblem,
    reference_selection: BoracleRuleSelection,
    experiment_selections: Vec<BoracleRuleSelection>,
    bounds: OracleBounds,
) -> Result<OracleComparisonSet, CompilerError> {
    validate_selection(&reference_selection)?;
    validate_selections(&experiment_selections)?;

    // The operational result is independent of the static rule selection. Execute it once and
    // retain the single owned outcome while static reports vary below.
    let oracle_outcome = execute_bounded(&problem, bounds)?;

    // Reference mode has nothing earlier to differ from, so it always classifies from its own pair.
    let reference_report =
        BoracleSolver::solve_with_rule_selection(&problem, reference_selection.clone())?;
    let reference_accepts = !reference_report.has_conflicts();
    let reference = OracleComparison {
        class: classify_pair(reference_accepts, &oracle_outcome),
        rule_selection: reference_selection,
        static_report: Some(reference_report),
    };

    let mut experiments = Vec::with_capacity(experiment_selections.len());
    for rule_selection in experiment_selections {
        let static_report =
            BoracleSolver::solve_with_rule_selection(&problem, rule_selection.clone())?;
        let static_accepts = !static_report.has_conflicts();
        // An experiment whose static legality matches reference mode produced no
        // experiment-attributable difference, so it classifies from its own pair as well.
        let class = if static_accepts == reference_accepts {
            classify_pair(static_accepts, &oracle_outcome)
        } else {
            classify_different_experiment(static_accepts, reference_accepts, &oracle_outcome)
        };
        experiments.push(OracleComparison {
            class,
            rule_selection,
            static_report: Some(static_report),
        });
    }

    Ok(OracleComparisonSet {
        problem: Some(problem),
        bounds,
        oracle_outcome: Some(oracle_outcome),
        reference,
        experiments: experiments.into_boxed_slice(),
    })
}

fn validate_selection(selection: &BoracleRuleSelection) -> Result<(), CompilerError> {
    selection.validate().map_err(CompilerError::compiler_error)
}

fn validate_selections(selections: &[BoracleRuleSelection]) -> Result<(), CompilerError> {
    for selection in selections {
        validate_selection(selection)?;
    }
    Ok(())
}

fn malformed_comparison_set(
    reference_selection: BoracleRuleSelection,
    experiment_selections: Vec<BoracleRuleSelection>,
    bounds: OracleBounds,
) -> Result<OracleComparisonSet, CompilerError> {
    validate_selection(&reference_selection)?;
    validate_selections(&experiment_selections)?;
    let malformed = |rule_selection| OracleComparison {
        class: OracleComparisonClass::MalformedProblem,
        rule_selection,
        static_report: None,
    };
    Ok(OracleComparisonSet {
        problem: None,
        bounds,
        oracle_outcome: None,
        reference: malformed(reference_selection),
        experiments: experiment_selections
            .into_iter()
            .map(malformed)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

fn classify_pair(static_accepts: bool, oracle_outcome: &OracleOutcome) -> OracleComparisonClass {
    match oracle_outcome {
        OracleOutcome::CompleteSafe { .. } if static_accepts => OracleComparisonClass::Agreement,
        OracleOutcome::CompleteSafe { .. } => OracleComparisonClass::StaticRejectedBoundedSafe,
        OracleOutcome::RuntimeConflict { .. } if static_accepts => {
            OracleComparisonClass::StaticAcceptedRuntimeConflict
        }
        OracleOutcome::RuntimeConflict { .. } => OracleComparisonClass::Agreement,
        OracleOutcome::Inconclusive { .. } => OracleComparisonClass::OracleInconclusive,
    }
}

fn classify_different_experiment(
    static_accepts: bool,
    reference_accepts: bool,
    oracle_outcome: &OracleOutcome,
) -> OracleComparisonClass {
    if static_accepts
        && !reference_accepts
        && matches!(oracle_outcome, OracleOutcome::CompleteSafe { .. })
    {
        return OracleComparisonClass::ExperimentOnlyAcceptedDifference;
    }
    classify_pair(static_accepts, oracle_outcome)
}

/// Reference mode paired with one selection per legality-changing experiment.
fn reference_and_experiment_selections() -> (BoracleRuleSelection, Vec<BoracleRuleSelection>) {
    let reference = BoracleRuleSelection::default();
    let mut experiments = Vec::new();
    for experiment in BoracleExperiment::ALL {
        if !experiment.metadata().may_change_legality {
            continue;
        }
        let mut selection = reference.clone();
        selection.experiments.insert(experiment);
        experiments.push(selection);
    }
    (reference, experiments)
}

fn append_rule_selection(output: &mut String, selection: &BoracleRuleSelection) {
    output.push_str(&format!(
        "rule-set = {}\n",
        selection.reference_rule_set.name()
    ));
    output.push_str(&format!("experiments = {}\n", selection.experiment_names()));
}

fn append_oracle_dump(output: &mut String, outcome: Option<&OracleOutcome>) {
    match outcome {
        None => {
            output.push_str("oracle-outcome = none\n");
            output.push_str("runtime-trace:\nnone\n");
        }
        Some(OracleOutcome::CompleteSafe { executions, trace }) => {
            output.push_str(&format!(
                "oracle-outcome = CompleteSafe {{ executions: {executions} }}\n"
            ));
            output.push_str("runtime-trace:\n");
            append_dump(output, &trace.debug_dump());
        }
        Some(OracleOutcome::RuntimeConflict { trace }) => {
            output.push_str("oracle-outcome = RuntimeConflict\n");
            output.push_str("runtime-trace:\n");
            append_dump(output, &trace.debug_dump());
        }
        Some(OracleOutcome::Inconclusive {
            reason,
            explored,
            completed_executions,
        }) => {
            output.push_str(&format!(
                "oracle-outcome = Inconclusive {{ reason: {reason:?}, explored: {explored}, \
                 completed_executions: {completed_executions} }}\n"
            ));
            output.push_str("runtime-trace:\nnone\n");
        }
    }
}

fn append_dump(output: &mut String, dump: &str) {
    output.push_str(dump);
    if !dump.ends_with('\n') {
        output.push('\n');
    }
}
