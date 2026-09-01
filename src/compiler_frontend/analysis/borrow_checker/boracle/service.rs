//! Compiler-owned Boracle service for validated-HIR analysis and deterministic dumps.
//!
//! WHAT: converts one compiler-produced validated-HIR payload into normalized problems, solves
//!       each function independently and renders one selected research view.
//! WHY: source-mode orchestration belongs to the compiler boundary; the CLI should select typed
//!       options and print the resulting report without rebuilding frontend stages.

use super::super::problem::from_hir;
use super::{BoracleReport, BoracleSolver, OracleBounds, compare_reference_and_experiments};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::functions::HirFunction;

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
    Differential,
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
            Self::Differential => "differential",
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
            "differential" => Ok(Self::Differential),
            _ => Err(format!(
                "Unknown Boracle dump '{value}'. Supported dumps are problem, origins, relations, precision, loans, last-use, conflicts, witnesses, and differential."
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
    /// Every experiment, in the canonical `Ord` order reports and selections use.
    ///
    /// A new variant must be added here, next to its name and metadata, so that consumers which
    /// enumerate experiments cannot silently miss it.
    pub(crate) const ALL: [Self; 1] = [Self::DeadExclusiveLoan];

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

/// Format experiment names in the canonical `Ord` order, or `none` for an empty set.
///
/// Every report renders its experiment set through this, so one ordering and one spelling of the
/// empty case are used everywhere. Collecting into a `BTreeSet` also dedupes a union taken across
/// several selections, which sorting the rendered names would not.
pub(crate) fn format_experiment_names<I>(experiments: I) -> String
where
    I: IntoIterator<Item = BoracleExperiment>,
{
    let experiments = experiments.into_iter().collect::<BTreeSet<_>>();
    if experiments.is_empty() {
        return "none".to_owned();
    }
    experiments
        .iter()
        .map(|experiment| experiment.name())
        .collect::<Vec<_>>()
        .join(",")
}

impl BoracleRuleSelection {
    /// The selected experiment names, formatted by [`format_experiment_names`].
    pub(crate) fn experiment_names(&self) -> String {
        format_experiment_names(self.experiments.iter().copied())
    }

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

/// One normalised function problem awaiting a selected reference solve.
///
/// WHAT: owns the deterministic HIR function identity alongside its normalised borrow problem.
/// WHY: differential rendering transfers each problem directly into comparison without cloning or
///       running a discarded reference solve.
#[derive(Debug)]
struct BoracleFunctionProblem {
    function_id: u32,
    problem: super::super::problem::BorrowProblem,
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
    // The differential dump enumerates reference mode and every legality-changing experiment
    // itself, so a requested experiment set could not be honoured and must not be reported as
    // though it had been.
    if options.dump == BoracleDump::Differential && !selection.experiments.is_empty() {
        return Err(CompilerError::compiler_error(format!(
            "Boracle differential dump compares every legality-changing experiment and cannot \
             also select experiments '{}'",
            selection.experiment_names()
        )));
    }
    let entry_point = input.entry_point.to_string_lossy();
    if options.dump == BoracleDump::Differential {
        let problems =
            build_hir_module_problems(&input.hir, &input.external_package_registry, &selection)?;
        return render_differential_module_report(problems, entry_point.as_ref(), &selection);
    }
    let report = solve_hir_module_parts(&input.hir, &input.external_package_registry, &selection)?;
    render_module_report(&report, entry_point.as_ref(), options.dump)
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

fn ordered_hir_functions<'module>(
    module: &'module HirModule,
    rule_selection: &BoracleRuleSelection,
) -> Result<Vec<&'module HirFunction>, CompilerError> {
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
    Ok(functions)
}

fn build_hir_module_problems(
    module: &HirModule,
    external_package_registry: &Arc<ExternalPackageRegistry>,
    rule_selection: &BoracleRuleSelection,
) -> Result<Vec<BoracleFunctionProblem>, CompilerError> {
    let functions = ordered_hir_functions(module, rule_selection)?;

    let mut problems = Vec::with_capacity(functions.len());
    for function in functions {
        let problem = from_hir(
            module,
            function,
            None,
            Some(external_package_registry.as_ref()),
        )?;
        problems.push(BoracleFunctionProblem {
            function_id: function.id.0,
            problem,
        });
    }

    Ok(problems)
}

fn solve_hir_module_parts(
    module: &HirModule,
    external_package_registry: &Arc<ExternalPackageRegistry>,
    rule_selection: &BoracleRuleSelection,
) -> Result<BoracleModuleReport, CompilerError> {
    let functions = ordered_hir_functions(module, rule_selection)?;
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

fn append_module_header(
    output: &mut String,
    entry_point: &str,
    dump: BoracleDump,
    rule_selection: &BoracleRuleSelection,
) {
    output.push_str("Boracle internal developer report\n");
    output.push_str(&format!("entry = {entry_point}\n"));
    output.push_str(&format!("dump = {}\n", dump.name()));
    if dump == BoracleDump::Differential {
        // Each comparison names the selection it used, so one header selection here would name a
        // static result this dump never displays.
        output.push_str("rule-selection = per-comparison\n");
    } else {
        output.push_str(&format!(
            "rule-set = {}\n",
            rule_selection.reference_rule_set.name()
        ));
        output.push_str(&format!(
            "experiments = {}\n",
            rule_selection.experiment_names()
        ));
    }
}

fn render_differential_module_report(
    problems: Vec<BoracleFunctionProblem>,
    entry_point: &str,
    rule_selection: &BoracleRuleSelection,
) -> Result<String, CompilerError> {
    let mut output = String::new();
    append_module_header(
        &mut output,
        entry_point,
        BoracleDump::Differential,
        rule_selection,
    );

    for BoracleFunctionProblem {
        function_id,
        problem,
    } in problems
    {
        output.push_str(&format!("\nfunction {function_id}\n"));
        let comparison = compare_reference_and_experiments(problem, OracleBounds::default())?;
        output.push_str(&comparison.report_dump());
    }

    Ok(output)
}

fn render_module_report(
    module_report: &BoracleModuleReport,
    entry_point: &str,
    dump: BoracleDump,
) -> Result<String, CompilerError> {
    let mut output = String::new();
    append_module_header(
        &mut output,
        entry_point,
        dump,
        &module_report.rule_selection,
    );

    for function in module_report.functions() {
        output.push_str(&format!("\nfunction {}\n", function.function_id));
        output.push_str(&render_dump(&function.problem, &function.report, dump));
    }

    Ok(output)
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
        BoracleDump::Differential => unreachable!("differential dumps use the comparison set"),
    }
}

impl fmt::Display for BoracleDump {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
