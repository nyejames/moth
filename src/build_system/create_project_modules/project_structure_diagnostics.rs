//! Stage 0 project-structure diagnostic helpers.
//!
//! WHAT: builds typed config/project-structure diagnostics and filesystem infrastructure
//!      errors at the input-preparation boundary.
//! WHY: resolver setup, package discovery, and module inventory all report the same
//! `InvalidConfigReason` payloads, so the span and string-table rules belong in one small
//! Stage 0 owner instead of being duplicated across those modules. Filesystem-origin
//! unrepresentable-name errors share the same owner because the same Stage 0 callers
//! discover both kinds of input.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticBag, InvalidConfigReason, PreparedFailure,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::path::Path;

/// Build an invalid-project-structure diagnostic for path or generated infrastructure.
///
/// The canonical path remains an IO/inventory fact, not diagnostic provenance. Filesystem-derived
/// failures therefore stay spanless rather than manufacturing a source location.
pub(super) fn project_structure_diagnostic(reason: InvalidConfigReason) -> DiagnosticDraft {
    project_config::invalid_config_reason(None, reason, None)
}

/// Reject one authored root filename outside the closed vocabulary.
///
/// WHY: directory traversal, non-regular entries and synthetic single-file selection all enforce
/// the same fixed root-name policy, so the diagnosed batch is constructed once.
pub(super) fn invalid_module_root_file_name_error(
    directory: &Path,
    file_name: &str,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let reason = InvalidConfigReason::InvalidModuleRootFileName {
        file_name: diagnostic_strings.intern(file_name),
        directory: diagnostic_strings.intern_path(directory),
    };
    capture_structure_diagnostic(project_structure_diagnostic(reason), string_table)
}

/// Reject the build-only config source when it is selected as a single-file entry.
///
/// WHY: single-file compilation has no config consumer, so the reserved source would otherwise
///      take the synthetic entry role and compile as page content.
pub(super) fn config_file_selected_as_entry_error(
    directory: &Path,
    file_name: &str,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let reason = InvalidConfigReason::ConfigFileSelectedAsEntry {
        file_name: diagnostic_strings.intern(file_name),
        directory: diagnostic_strings.intern_path(directory),
    };
    capture_structure_diagnostic(project_structure_diagnostic(reason), string_table)
}

/// Reject one local directory whose entry-root-relative path equals a registered package prefix.
///
/// WHY: directory traversal and the synthetic inventory for an ordinary selected file find the
/// same filesystem collision through different bounded scans, so the authored failure is built
/// once here. The diagnostic names the registered spelling and the authored folder path.
pub(super) fn entry_root_package_prefix_collision_error(
    registered_prefix: &str,
    colliding_folder: &Path,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let reason = InvalidConfigReason::EntryRootPackagePrefixCollision {
        prefix: diagnostic_strings.intern(registered_prefix),
        entry_folder: diagnostic_strings.intern_path(colliding_folder),
    };
    capture_structure_diagnostic(project_structure_diagnostic(reason), string_table)
}

/// Build the diagnostic for one importable project-local claim on a reserved package root.
///
/// WHAT: names the reserved root as authored and the local source that publishes the claim.
/// WHY: structure validation and synthetic dependency resolution discover the same claim from
/// different Stage 0 facts, so the authored payload has one owner while each caller keeps its
/// own failure lane.
pub(super) fn reserved_package_namespace_root_claim_diagnostic(
    root: &str,
    source: &Path,
    string_table: &mut StringTable,
) -> DiagnosticDraft {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let reason = InvalidConfigReason::ReservedPackageNamespaceRootClaim {
        root: diagnostic_strings.intern(root),
        source: diagnostic_strings.intern_path(source),
    };
    project_structure_diagnostic(reason)
}

/// Reject one importable project-local claim on a reserved package namespace root.
///
/// WHY: directory namespace validation and the directory walk for a selection without module
/// roots find the claim through different Stage 0 facts, but the authored failure is identical,
/// so the diagnosed batch is built once here instead of at both call sites.
pub(super) fn reserved_package_namespace_root_claim_error(
    root: &str,
    source: &Path,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let diagnostic = reserved_package_namespace_root_claim_diagnostic(root, source, string_table);
    capture_structure_diagnostic(diagnostic, string_table)
}

/// Freeze the diagnostic into a diagnosed failure, handing the populated table to the batch.
///
/// Stage 0 aborts discovery on these errors, so the caller's table is empty after the handoff.
fn capture_structure_diagnostic(
    diagnostic: DiagnosticDraft,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        None,
    ))
}

/// Build an infrastructure error for a filesystem name that cannot be represented as UTF-8.
///
/// WHAT: retains the offending path in an `InfrastructureFailure` so the build boundary can render it.
/// WHY: a non-UTF-8 filesystem name cannot enter the string table or dependency namespace. It is an
///      unrepresentable filesystem input, not an authored config mistake, so it uses the
///      infrastructure-error lane rather than authored diagnostic reporting.
pub(super) fn non_utf8_filesystem_name_error(path: &Path, context: &str) -> InfrastructureFailure {
    InfrastructureFailure::filesystem(
        CompilerStage::ProjectBuildAssembly,
        path,
        format!("Non-UTF-8 filesystem name cannot enter compiler identity ({context}): {path:?}"),
    )
}
