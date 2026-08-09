//! Frontend compilation coordinator for Moth projects.
//!
//! Dispatches to single-file or directory-project flows, then delegates to focused submodules:
//! - `frontend_orchestration`   — per-module pipeline (tokenization through borrow checking)
//! - `generated_worklist`       — boundary-local generated request state and sidecar publication
//! - `generated_summary_convergence` — transient HIR call topology and dirty-queue propagation
//! - `project_roots`            — config root interpretation and path-resolver setup
//! - `source_package_discovery` — source-package registration, boundary indexes and prefix checks
//! - `source_tree_index`        — project and source-package boundary source-tree indexing with
//!   root discovery and collision checks
//! - `module_identity`          — Stage 0 durable module identity and structural topology
//! - `module_namespace`         — boundary-aware indexed module namespaces for source-import resolution
//! - `project_module_graph`      — canonical structural project module graph and compile order
//! - `module_inventory`         — project-level module assembly
//! - `prepared_source`          — state-safe source-kind input handoff
//! - `prepared_module`          — retained module-preparation payload handed to semantic compilation
//! - `module_artifact_store`    — completed immutable artefacts, dense slot mapping and outcomes
//! - `compiled_boundary`        — retained project/source-package graph boundaries and frontend outcome
//! - `source_discovery`         — Stage 0 source traversal, owned-input preparation and structural provider resolution
//! - `source_scanning`          — retained single-pass source tokenisation and import extraction
//! - `project_structure_diagnostics` — typed Stage 0 project diagnostics
//! - `source_discovery_error`   — Stage 0 boundary between diagnostics and file/tooling errors
//! - `source_loading`           — raw file I/O
//!
//! Stage 0 config loading lives in `project_config`. This module begins after config has been
//! applied to `Config`.

mod compilation;
pub(crate) mod compiled_boundary;
mod frontend_orchestration;
mod generated_summary_convergence;
pub(crate) mod generated_worklist;
pub(crate) mod module_artifact_store;
pub(crate) mod module_identity;
mod module_inventory;
mod module_namespace;
mod prepared_module;
mod prepared_source;
pub(crate) mod project_module_graph;
mod project_roots;
mod project_structure_diagnostics;
mod source_discovery;
pub(crate) mod source_discovery_error;
pub(crate) mod source_loading;
pub(crate) mod source_package_discovery;
pub(crate) mod source_scanning;
mod source_tree_index;

#[cfg(test)]
pub(super) use module_inventory::{ModuleCompilationSchedule, discover_all_modules_in_project};

pub(crate) use project_roots::resolve_project_entry_root;
pub(crate) use source_loading::extract_source_code;

#[cfg(test)]
pub(crate) use crate::projects::settings;
#[cfg(test)]
pub(crate) use std::fs;

use crate::build_system::output::ValidatedDirectoryOutputSettings;

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::instrumentation::{log_frontend_counters, reset_frontend_counters};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::build_system::BuildProfile;
use crate::builder_surface::BuilderSurface;
use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};
use crate::timed_stage;

pub(crate) use compiled_boundary::ProjectFrontendCompilation;

// -------------------------
//  Compilation Entry Point
// -------------------------

/// Compile all project modules through the frontend pipeline.
///
/// WHAT: dispatches to single-file or directory-project flow depending on the entry path.
/// WHY: separating the two flows keeps each path readable as orchestration over named steps.
pub fn compile_project_frontend(
    config: &mut Config,
    build_profile: BuildProfile,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    // Frontend counters are command-scoped and gated by `benchmark_counters`.
    // The counter storage is atomic so directory module compilation can update
    // it safely from Rayon workers.
    reset_frontend_counters();

    let frontend_build_profile = match build_profile {
        BuildProfile::Dev => FrontendBuildProfile::Dev,
        BuildProfile::Release => FrontendBuildProfile::Release,
    };

    // ---------------------------------------
    //  Dispatch: Single File vs. Directory
    // ---------------------------------------

    let result = timed_stage!(crate::timing::TimingMetric::BuildFrontendTotal, {
        if config.entry_dir.is_dir() {
            compilation::compile_directory_frontend(
                config,
                frontend_build_profile,
                validated_output_settings,
                style_directives,
                builder_surface,
                string_table,
            )
        } else if let Some(extension) = config.entry_dir.extension() {
            compilation::compile_single_file_frontend(
                config,
                frontend_build_profile,
                style_directives,
                builder_surface,
                extension,
                string_table,
            )
        } else {
            use crate::compiler_frontend::compiler_errors::CompilerError;

            let err = CompilerError::file_error(
                &config.entry_dir,
                format!(
                    "Found a file without an extension set. Moth files use .{LANGUAGE_SOURCE_EXTENSION}"
                ),
                string_table,
            );

            Err(CompilerMessages::from_error_ref(err, string_table))
        }
    });

    log_frontend_counters();

    result
}

#[cfg(test)]
#[path = "../tests/create_project_modules_tests.rs"]
mod create_project_modules_tests;

#[cfg(test)]
#[path = "../tests/stage0_filesystem_identity_tests.rs"]
mod stage0_filesystem_identity_tests;

#[cfg(test)]
#[path = "../tests/compile_project_frontend_tests.rs"]
mod compile_project_frontend_tests;
