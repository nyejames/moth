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
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

/// Deterministic report section selected by the internal developer command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoracleDump {
    Problem,
    Origins,
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
            "loans" => Ok(Self::Loans),
            "last-use" => Ok(Self::LastUse),
            "conflicts" => Ok(Self::Conflicts),
            "witnesses" => Ok(Self::Witnesses),
            _ => Err(format!(
                "Unknown Boracle dump '{value}'. Supported dumps are problem, origins, loans, last-use, conflicts, and witnesses."
            )),
        }
    }
}

/// Named Boracle rule selection.
///
/// The reference mode keeps unused exclusive capabilities conservatively live. The
/// `DeadExclusiveLoan` mode selects the named use-driven experiment and is never used implicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoracleExperiment {
    Reference,
    DeadExclusiveLoan,
}

impl BoracleExperiment {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::DeadExclusiveLoan => "dead-exclusive-loan",
        }
    }

    pub(crate) fn rule_set(self) -> &'static str {
        match self {
            Self::Reference => "boracle-reference-v1",
            Self::DeadExclusiveLoan => "boracle-dead-exclusive-loan-v1",
        }
    }
}

impl FromStr for BoracleExperiment {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "reference" => Ok(Self::Reference),
            "dead-exclusive-loan" => Ok(Self::DeadExclusiveLoan),
            _ => Err(format!(
                "Unknown Boracle experiment '{value}'. Supported experiments are reference and dead-exclusive-loan."
            )),
        }
    }
}

/// Typed options owned by the compiler service boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoracleServiceOptions {
    pub(crate) dump: BoracleDump,
    pub(crate) experiment: BoracleExperiment,
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
    let report = solve_hir_module_parts(
        &input.hir,
        &input.external_package_registry,
        options.experiment,
    )?;
    let entry_point = input.entry_point.to_string_lossy();
    Ok(render_module_report(&report, entry_point.as_ref(), options))
}

/// Solve one compiler-produced HIR module without flattening its typed results into a dump.
#[cfg(test)]
pub(crate) fn solve_hir_module(
    input: &BoracleModuleInput,
) -> Result<BoracleModuleReport, CompilerError> {
    solve_hir_module_parts(
        &input.hir,
        &input.external_package_registry,
        BoracleExperiment::Reference,
    )
}

fn solve_hir_module_parts(
    module: &HirModule,
    external_package_registry: &Arc<ExternalPackageRegistry>,
    experiment: BoracleExperiment,
) -> Result<BoracleModuleReport, CompilerError> {
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
        let report = BoracleSolver::solve_with_experiment(&problem, experiment)?;
        reports.push(BoracleFunctionReport {
            function_id: function.id.0,
            problem,
            report,
        });
    }

    Ok(BoracleModuleReport {
        functions: reports.into_boxed_slice(),
    })
}

fn render_module_report(
    module_report: &BoracleModuleReport,
    entry_point: &str,
    options: BoracleServiceOptions,
) -> String {
    let mut output = String::new();
    output.push_str("Boracle internal developer report\n");
    output.push_str(&format!("entry = {entry_point}\n"));
    output.push_str(&format!("dump = {}\n", options.dump.name()));
    output.push_str(&format!("experiment = {}\n", options.experiment.name()));
    output.push_str(&format!("rule-set = {}\n", options.experiment.rule_set()));

    for function in module_report.functions() {
        output.push_str(&format!("\nfunction {}\n", function.function_id));
        output.push_str(&render_dump(
            &function.problem,
            &function.report,
            options.dump,
        ));
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
    use super::{BoracleDump, BoracleExperiment};

    #[test]
    fn dump_and_experiment_names_are_stable() {
        assert_eq!("last-use".parse::<BoracleDump>(), Ok(BoracleDump::LastUse));
        assert_eq!(
            "dead-exclusive-loan".parse::<BoracleExperiment>(),
            Ok(BoracleExperiment::DeadExclusiveLoan)
        );
    }
}
