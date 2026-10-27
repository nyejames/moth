//! Internal, unstable Boracle source command adapter.
//!
//! WHAT: owns only Stage 0 bootstrap and terminal-facing diagnostic conversion for the Boracle
//!       developer command.
//! WHY: frontend lowering and analysis remain compiler-owned; this module keeps the CLI surface
//!      small without making Boracle part of normal `check`, build or project compilation.

use crate::build_system::build::{BuildBootstrap, ProjectBuilder, bootstrap_project_build};
use crate::build_system::create_project_modules::compile_single_file_boracle;
use crate::build_system::path_validation::check_if_valid_path;
use crate::compiler_frontend::analysis::borrow_checker::{
    BoracleDump, BoracleRuleSelection, BoracleServiceOptions, run_hir_module,
};
#[cfg(test)]
use crate::compiler_frontend::analysis::borrow_checker::{BoracleModuleReport, solve_hir_module};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
/// Run the feature-gated Boracle source service for one single-file entry.
pub fn run_boracle(
    path: &str,
    dump: BoracleDump,
    rule_selection: BoracleRuleSelection,
) -> CompilationResult<String> {
    let (input, string_table, warnings) = match compile_boracle_input(path)? {
        CompilationOutcome::Success {
            value: (input, string_table),
            warnings,
        } => (input, string_table, warnings),
        CompilationOutcome::Diagnosed(reports) => {
            return Ok(CompilationOutcome::Diagnosed(reports));
        }
    };
    let value = run_hir_module(
        &input,
        BoracleServiceOptions {
            dump,
            rule_selection,
        },
    )
    .map_err(InfrastructureErrors::from)?;
    drop(string_table);
    Ok(CompilationOutcome::Success { value, warnings })
}

/// Solve the same compiler-owned source payload as [`run_boracle`] without flattening the typed
/// function reports into a developer dump.
#[cfg(test)]
fn solve_boracle(path: &str) -> CompilationResult<BoracleModuleReport> {
    let (input, string_table, warnings) = match compile_boracle_input(path)? {
        CompilationOutcome::Success {
            value: (input, string_table),
            warnings,
        } => (input, string_table, warnings),
        CompilationOutcome::Diagnosed(reports) => {
            return Ok(CompilationOutcome::Diagnosed(reports));
        }
    };
    let value = solve_hir_module(&input).map_err(InfrastructureErrors::from)?;
    drop(string_table);
    Ok(CompilationOutcome::Success { value, warnings })
}

fn compile_boracle_input(path: &str) -> CompilationResult<(BoracleModuleInput, StringTable)> {
    let normalized_path = if path.trim().is_empty() { "." } else { path };
    let valid_path = check_if_valid_path(normalized_path)?;
    let project_builder = ProjectBuilder::new(HtmlProjectBuilder::new());
    let BuildBootstrap {
        config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        ..
    } = match bootstrap_project_build(&project_builder, valid_path, &BuildConfigInputSet::new()) {
        Ok(bootstrap) => bootstrap,
        Err(failure) => {
            return failure
                .into_diagnostic_reports()
                .map(CompilationOutcome::Diagnosed);
        }
    };

    if config.entry_dir.is_dir() {
        let error = InfrastructureFailure::filesystem(
            CompilerStage::ProjectBuildAssembly,
            &config.entry_dir,
            "Boracle source mode currently accepts one .moth file, not a directory project",
        );
        return Err(error.into());
    }

    let (input, warnings) = match compile_single_file_boracle(
        &config,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )? {
        CompilationOutcome::Success { value, warnings } => (value, warnings),
        CompilationOutcome::Diagnosed(reports) => {
            return Ok(CompilationOutcome::Diagnosed(reports));
        }
    };
    Ok(CompilationOutcome::Success {
        value: (input, string_table),
        warnings,
    })
}

#[cfg(test)]
#[path = "tests/boracle_tests.rs"]
mod tests;
