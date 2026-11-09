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
    DiagnosticBag, InvalidConfigReason, PreparedFailure,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use std::path::Path;

/// Build an invalid-project-structure diagnostic for path or generated infrastructure.
///
/// The canonical path remains an IO/inventory fact, not diagnostic provenance. Filesystem-derived
/// failures therefore stay spanless rather than manufacturing a source location.
pub(super) fn project_structure_diagnostic(reason: InvalidConfigReason) -> DiagnosticDraft {
    project_config::invalid_config_reason(None, reason, None)
}

/// Intern a path spelling for diagnostic payloads.
pub(super) fn path_id(path: &Path, strings: &mut DiagnosticStringBuilder<'_>) -> StringId {
    strings.get_or_intern(path.display().to_string())
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
        directory: path_id(directory, &mut diagnostic_strings),
    };
    let diagnostic = project_structure_diagnostic(reason);
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
        crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
        path,
        format!("Non-UTF-8 filesystem name cannot enter compiler identity ({context}): {path:?}"),
    )
}
