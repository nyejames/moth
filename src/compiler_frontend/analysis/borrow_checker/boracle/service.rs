//! Compiler-owned Boracle service for validated-HIR analysis and deterministic dumps.
//!
//! WHAT: converts one compiler-produced validated-HIR payload into normalized problems, solves
//!       each function independently and renders one selected research view.
//! WHY: source-mode orchestration belongs to the compiler boundary; the CLI should select typed
//!       options and print the resulting report without rebuilding frontend stages.

use super::super::problem::from_hir;
use super::{BoracleReport, BoracleSolver};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoracleDump {
    Problem,
    Origins,
    Relations,
    Precision,
    Loans,
    LastUse,
    Conflicts,
    Witnesses,
}

impl BoracleDump {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Problem => "problem",
            Self::Origins => "origins",
            Self::Relations => "relations",
            Self::Precision => "precision",
            Self::Loans => "loans",
            Self::LastUse => "last-use",
            Self::Conflicts => "conflicts",
            Self::Witnesses => "witnesses",
        }
    }
}

impl FromStr for BoracleDump {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "problem" => Ok(Self::Problem),
            "origins" => Ok(Self::Origins),
            "relations" => Ok(Self::Relations),
            "precision" => Ok(Self::Precision),
            "loans" => Ok(Self::Loans),
            "last-use" => Ok(Self::LastUse),
            "conflicts" => Ok(Self::Conflicts),
            "witnesses" => Ok(Self::Witnesses),
            _ => Err(format!(
                "Unknown Boracle dump '{value}'. Supported dumps are problem, origins, relations, precision, loans, last-use, conflicts, and witnesses."
            )),
        }
    }
}

/// The versioned reference rule-set selected by Boracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub(crate) enum BoracleReferenceRuleSet {
    #[default]
    V1,
}

impl BoracleReferenceRuleSet {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::V1 => "boracle-reference-v1",
        }
    }
}

/// Stable status for whether an experiment has been promoted into the reference rule-set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BoracleReferencePromotionStatus {
    NotPromoted,
}

/// Metadata describing one named Boracle experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoracleExperimentMetadata {
    pub(crate) name: &'static str,
    pub(crate) may_change_legality: bool,
    pub(crate) prerequisites: &'static [BoracleExperiment],
    pub(crate) reference_promotion: BoracleReferencePromotionStatus,
}

/// One composable Boracle experiment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BoracleExperiment {
    DeadExclusiveLoan,
}

impl BoracleExperiment {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::DeadExclusiveLoan => "dead-exclusive-loan",
        }
    }

    pub(crate) const fn metadata(self) -> BoracleExperimentMetadata {
        match self {
            Self::DeadExclusiveLoan => BoracleExperimentMetadata {
                name: "dead-exclusive-loan",
                may_change_legality: true,
                prerequisites: &[],
                reference_promotion: BoracleReferencePromotionStatus::NotPromoted,
            },
        }
    }
}

impl FromStr for BoracleExperiment {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dead-exclusive-loan" => Ok(Self::DeadExclusiveLoan),
            _ => Err(format!(
                "Unknown Boracle experiment '{value}'. Supported experiment is dead-exclusive-loan."
            )),
        }
    }
}

/// Typed, composable rule selection owned by the compiler service boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoracleRuleSelection {
    pub(crate) reference_rule_set: BoracleReferenceRuleSet,
    pub(crate) experiments: BTreeSet<BoracleExperiment>,
}

impl Default for BoracleRuleSelection {
    fn default() -> Self {
        Self {
            reference_rule_set: BoracleReferenceRuleSet::V1,
            experiments: BTreeSet::new(),
        }
    }
}

impl BoracleRuleSelection {
    /// Validate the selected rule-set and experiment combination.
    ///
    /// The pairwise hook is intentionally present even while the current experiment set has one
    /// member: adding a second experiment must not bypass compatibility validation.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for experiment in &self.experiments {
            let metadata = experiment.metadata();
            for prerequisite in metadata.prerequisites {
                if !self.experiments.contains(prerequisite) {
                    return Err(format!(
                        "Boracle experiment '{}' requires '{}'",
                        metadata.name,
                        prerequisite.name()
                    ));
                }
            }
        }

        let experiments = self.experiments.iter().copied().collect::<Vec<_>>();
        for (index, left) in experiments.iter().enumerate() {
            for right in experiments.iter().skip(index + 1) {
                Self::validate_experiment_combination(*left, *right)?;
            }
        }
        Ok(())
    }

    fn validate_experiment_combination(
        left: BoracleExperiment,
        right: BoracleExperiment,
    ) -> Result<(), String> {
        if left == right {
            return Err(format!(
                "Boracle experiments cannot select '{}' more than once",
                left.name()
            ));
        }
        Ok(())
    }
}

/// Typed options owned by the compiler service boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoracleServiceOptions {
    pub(crate) dump: BoracleDump,
    pub(crate) rule_selection: BoracleRuleSelection,
}

/// One typed function result from the compiler-owned Boracle source boundary.
#[derive(Debug)]
pub(crate) struct BoracleFunctionReport {
    pub(crate) function_id: u32,
    pub(crate) problem: super::super::problem::BorrowProblem,
    pub(crate) report: BoracleReport,
}

/// Typed reports for every HIR function in one source module.
#[derive(Debug)]
pub(crate) struct BoracleModuleReport {
    pub(crate) rule_selection: BoracleRuleSelection,
    pub(crate) functions: Box<[BoracleFunctionReport]>,
}

impl BoracleModuleReport {
    pub(crate) fn functions(&self) -> &[BoracleFunctionReport] {
        &self.functions
    }
}

/// Run Boracle over every function in one compiler-produced HIR module.
pub(crate) fn run_hir_module(
    input: &BoracleModuleInput,
    options: BoracleServiceOptions,
) -> Result<String, CompilerError> {
    let selection = options.rule_selection;
    let report = solve_hir_module_parts(&input.hir, &input.external_package_registry, &selection)?;
    let entry_point = input.entry_point.to_string_lossy();
    Ok(render_module_report(
        &report,
        entry_point.as_ref(),
        options.dump,
    ))
}

/// Solve one compiler-produced HIR module without flattening its typed results into a dump.
#[cfg(test)]
pub(crate) fn solve_hir_module(
    input: &BoracleModuleInput,
) -> Result<BoracleModuleReport, CompilerError> {
    solve_hir_module_parts(
        &input.hir,
        &input.external_package_registry,
        &BoracleRuleSelection::default(),
    )
}

fn solve_hir_module_parts(
    module: &HirModule,
    external_package_registry: &Arc<ExternalPackageRegistry>,
    rule_selection: &BoracleRuleSelection,
) -> Result<BoracleModuleReport, CompilerError> {
    rule_selection
        .validate()
        .map_err(CompilerError::compiler_error)?;

    if module.functions.is_empty() {
        return Err(CompilerError::compiler_error(
            "Boracle source service received a HIR module without functions",
        ));
    }

    let mut functions = module.functions.iter().collect::<Vec<_>>();
    functions.sort_by_key(|function| function.id.0);

    let mut reports = Vec::with_capacity(functions.len());
    for function in functions {
        let problem = from_hir(
            module,
            function,
            None,
            Some(external_package_registry.as_ref()),
        )?;
        let report = BoracleSolver::solve_with_rule_selection(&problem, rule_selection.clone())?;
        reports.push(BoracleFunctionReport {
            function_id: function.id.0,
            problem,
            report,
        });
    }

    Ok(BoracleModuleReport {
        rule_selection: rule_selection.clone(),
        functions: reports.into_boxed_slice(),
    })
}

fn render_module_report(
    module_report: &BoracleModuleReport,
    entry_point: &str,
    dump: BoracleDump,
) -> String {
    let mut output = String::new();
    output.push_str("Boracle internal developer report\n");
    output.push_str(&format!("entry = {entry_point}\n"));
    output.push_str(&format!("dump = {}\n", dump.name()));
    output.push_str(&format!(
        "rule-set = {}\n",
        module_report.rule_selection.reference_rule_set.name()
    ));
    let experiments = module_report
        .rule_selection
        .experiments
        .iter()
        .map(|experiment| experiment.name())
        .collect::<Vec<_>>();
    output.push_str(&format!(
        "experiments = {}\n",
        if experiments.is_empty() {
            "none".to_owned()
        } else {
            experiments.join(",")
        }
    ));

    for function in module_report.functions() {
        output.push_str(&format!("\nfunction {}\n", function.function_id));
        output.push_str(&render_dump(&function.problem, &function.report, dump));
    }

    output
}

fn render_dump(
    problem: &super::super::problem::BorrowProblem,
    report: &BoracleReport,
    dump: BoracleDump,
) -> String {
    match dump {
        BoracleDump::Problem => problem.debug_dump(),
        BoracleDump::Origins => report.origin.debug_dump(),
        BoracleDump::Relations => report.origin.relations().debug_dump(),
        BoracleDump::Precision => report.origin.relations().precision_debug_dump(),
        BoracleDump::Loans => report.loans.debug_dump(),
        BoracleDump::LastUse => report.last_use_debug_dump(),
        BoracleDump::Conflicts => report.conflicts_debug_dump(),
        BoracleDump::Witnesses => report.witnesses_debug_dump(),
    }
}

impl fmt::Display for BoracleDump {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BoracleDump, BoracleExperiment, BoracleReferencePromotionStatus, BoracleReferenceRuleSet,
        BoracleRuleSelection,
    };

    #[test]
    fn dump_and_experiment_names_are_stable() {
        assert_eq!("last-use".parse::<BoracleDump>(), Ok(BoracleDump::LastUse));
        assert_eq!(
            "relations".parse::<BoracleDump>(),
            Ok(BoracleDump::Relations)
        );
        assert_eq!(
            "precision".parse::<BoracleDump>(),
            Ok(BoracleDump::Precision)
        );
        assert_eq!(
            "dead-exclusive-loan".parse::<BoracleExperiment>(),
            Ok(BoracleExperiment::DeadExclusiveLoan)
        );
    }

    #[test]
    fn default_rule_selection_is_reference_without_experiments() {
        let selection = BoracleRuleSelection::default();
        assert_eq!(selection.reference_rule_set, BoracleReferenceRuleSet::V1);
        assert!(selection.experiments.is_empty());
        assert!(selection.validate().is_ok());
    }

    #[test]
    fn dead_exclusive_loan_metadata_is_not_reference_promoted() {
        let metadata = BoracleExperiment::DeadExclusiveLoan.metadata();
        assert_eq!(metadata.name, "dead-exclusive-loan");
        assert!(metadata.may_change_legality);
        assert!(metadata.prerequisites.is_empty());
        assert_eq!(
            metadata.reference_promotion,
            BoracleReferencePromotionStatus::NotPromoted
        );
    }
}
