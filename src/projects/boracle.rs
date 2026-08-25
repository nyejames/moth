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
    BoracleDump, BoracleExperiment, BoracleServiceOptions, run_hir_module,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;

/// Run the feature-gated Boracle source service for one single-file entry.
pub(crate) fn run_boracle(
    path: &str,
    dump: BoracleDump,
    experiment: BoracleExperiment,
) -> Result<String, CompilerMessages> {
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
    run_hir_module(&input, BoracleServiceOptions { dump, experiment })
        .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))
}

#[cfg(test)]
mod tests {
    use super::run_boracle;
    use crate::compiler_frontend::analysis::borrow_checker::{BoracleDump, BoracleExperiment};
    use std::fs;

    #[test]
    fn boracle_service_source_smoke_uses_real_moth_input() {
        let temporary = tempfile::tempdir().expect("temporary source directory should exist");
        let entry = temporary.path().join("main.moth");
        fs::write(&entry, "value = 1\n").expect("source should be writable");

        let first = run_boracle(
            entry.to_str().expect("temporary path should be UTF-8"),
            BoracleDump::Origins,
            BoracleExperiment::DeadExclusiveLoan,
        )
        .expect("real source should reach Boracle");
        let second = run_boracle(
            entry.to_str().expect("temporary path should be UTF-8"),
            BoracleDump::Origins,
            BoracleExperiment::DeadExclusiveLoan,
        )
        .expect("real source should reach Boracle");

        assert_eq!(first, second);
        assert!(first.contains("rule-set = boracle-reference-v1"));
        assert!(first.contains("experiment = dead-exclusive-loan"));
        assert!(first.contains("OriginSolution"));
    }
}
