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
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;

/// Run the feature-gated Boracle source service for one single-file entry.
pub(crate) fn run_boracle(
    path: &str,
    dump: BoracleDump,
    rule_selection: BoracleRuleSelection,
) -> Result<String, CompilerMessages> {
    let (input, string_table) = compile_boracle_input(path)?;
    run_hir_module(
        &input,
        BoracleServiceOptions {
            dump,
            rule_selection,
        },
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))
}

/// Solve the same compiler-owned source payload as [`run_boracle`] without flattening the typed
/// function reports into a developer dump.
#[cfg(test)]
fn solve_boracle(path: &str) -> Result<BoracleModuleReport, CompilerMessages> {
    let (input, string_table) = compile_boracle_input(path)?;
    solve_hir_module(&input).map_err(|error| CompilerMessages::from_error_ref(error, &string_table))
}

fn compile_boracle_input(
    path: &str,
) -> Result<(BoracleModuleInput, StringTable), CompilerMessages> {
    let normalized_path = if path.trim().is_empty() { "." } else { path };
    let mut path_string_table = StringTable::new();
    let valid_path = check_if_valid_path(normalized_path, &mut path_string_table)
        .map_err(|error| CompilerMessages::from_error(error, path_string_table.clone()))?;

    let project_builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let BuildBootstrap {
        config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        ..
    } = bootstrap_project_build(&project_builder, valid_path)?;

    if config.entry_dir.is_dir() {
        let error = CompilerError::compiler_error(
            "Boracle source mode currently accepts one .moth file, not a directory project",
        );
        return Err(CompilerMessages::from_error_ref(error, &string_table));
    }

    let input = compile_single_file_boracle(
        &config,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )?;
    Ok((input, string_table))
}

#[cfg(test)]
#[path = "tests/boracle_tests.rs"]
mod tests;
