//! Output-plan orchestration for prepared artifact emission and manifest cleanup.
//!
//! WHAT: owns the public write entrypoint that sequences preflight, ownership preparation, file
//! emission, stale cleanup, and manifest persistence.
//! WHY: callers should choose a validated output plan and write mode while the output subsystem
//! keeps the mutation order and safety boundaries in one current path.
use crate::timing_scope;

use super::manifest::{OutputCleanupFinalization, finalize_output_cleanup, prepare_output_cleanup};
use super::policy::OutputPlan;
use super::writer::{emit_prepared_output_files, prepare_output_write};
use crate::build_system::build::Project;
use crate::build_system::utils::file_error_messages;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::fs;

/// Write-mode selection for prepared output destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteMode {
    AlwaysWrite,
    SkipUnchanged,
}

/// Options for writing a compiled project to disk.
pub(crate) struct WriteOptions {
    pub(crate) output_plan: OutputPlan,
    pub(crate) write_mode: WriteMode,
}

/// Write built project artifacts to the provided output plan.
///
/// Artifact paths are explicit and must already include any desired extension. The complete
/// output plan carries the output root, cleanup safety boundary and manifest owner; callers do
/// not reconstruct those facts from config after compilation.
pub(crate) fn write_project_outputs(
    project: &Project,
    options: &WriteOptions,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    timing_scope!(
        timing_guard_output_write_total,
        crate::timing::TimingMetric::OutputWriteTotal
    );

    // Keep the aggregate output timing visible even when filesystem validation or writes fail.
    write_project_outputs_inner(project, options, string_table)
}

fn write_project_outputs_inner(
    project: &Project,
    options: &WriteOptions,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    // ---------------------------------------
    //  Preflight the complete output batch
    // ---------------------------------------
    // WHAT: validate every non-NotBuilt output path, reject duplicate destinations, compute the
    // complete managed-path set, and prepare canonical destinations before any filesystem mutation.
    // WHY: a late invalid or duplicate path must not leave earlier files already written.
    let prepared_write = prepare_output_write(project, &options.output_plan, string_table)?;

    // ---------------------------------------
    //  Prepare cleanup and create output root
    // ---------------------------------------

    let output_root = options.output_plan.output_root();
    let output_owner = options.output_plan.owner();

    // WHAT: load and validate the previous manifest's ownership before creating the output root.
    // WHY: a foreign known owner must fail before emission; a matching recoverable owner may
    // proceed while preserving stale artifacts.
    let cleanup_state = prepare_output_cleanup(
        output_root,
        options.output_plan.project_root(),
        options.output_plan.entry_root(),
        output_owner,
        options.output_plan.setting_location(),
        &project.cleanup_policy,
        string_table,
    )?;

    fs::create_dir_all(output_root).map_err(|error| {
        file_error_messages(
            output_root,
            format!(
                "Failed to create output root '{}': {error}",
                output_root.display()
            ),
            string_table,
        )
    })?;

    // ---------------------------------------
    //  Emit individual output files
    // ---------------------------------------

    emit_prepared_output_files(project, &prepared_write, options.write_mode, string_table)?;

    // ---------------------------------------
    //  Finalize cleanup and write manifest
    // ---------------------------------------
    // WHAT: clean stale artifacts for valid ownership and write the updated manifest.
    // WHY: artifacts from removed pages must not persist, while recoverable manifests must not
    // drive deletion under uncertain metadata.
    let finalization = OutputCleanupFinalization {
        output_root,
        manifest_destination: &prepared_write.manifest_destination,
        current_managed_artifact_paths: &prepared_write.managed_artifact_paths,
        current_explicit_directory_paths: &prepared_write.explicit_directory_paths,
        owner: output_owner,
        cleanup_policy: &project.cleanup_policy,
        write_mode: options.write_mode,
        string_table,
    };
    finalize_output_cleanup(&cleanup_state, &finalization)?;

    Ok(())
}
