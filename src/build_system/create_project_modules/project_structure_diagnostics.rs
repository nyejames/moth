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
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::projects::settings::Config;

use std::path::Path;

/// Build an invalid-config diagnostic tied to a specific config key.
pub(super) fn config_diagnostic(
    config: &Config,
    key: &str,
    reason: InvalidConfigReason,
    strings: &mut DiagnosticStringBuilder<'_>,
) -> DiagnosticDraft {
    let key_id = strings.intern(key);
    project_config::invalid_config_reason(Some(key_id), reason, config.setting_span(key))
}

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
