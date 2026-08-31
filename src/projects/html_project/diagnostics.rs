//! Typed diagnostics for HTML-project policy checks.
//!
//! WHAT: turns deterministic routing and output-path policy failures into structured config
//! diagnostics.
//! WHY: HTML builder mistakes are user-facing project feedback, not infrastructure failures.

use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, InvalidConfigReason,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::path::Path;

pub(crate) fn missing_homepage_messages(
    config_path: &Path,
    entry_root: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    html_config_messages(
        config_path,
        |string_table| InvalidConfigReason::MissingHtmlHomepage {
            entry_root: path_id(entry_root, string_table),
        },
        string_table,
    )
}

pub(crate) fn duplicate_html_output_path_messages(
    duplicate_entry_point: &Path,
    existing_entry_point: &Path,
    output_path: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    html_config_messages(
        duplicate_entry_point,
        |string_table| InvalidConfigReason::DuplicateHtmlOutputPath {
            output_path: path_id(output_path, string_table),
            entry_point: path_id(duplicate_entry_point, string_table),
            existing_entry_point: path_id(existing_entry_point, string_table),
        },
        string_table,
    )
}

/// Build a typed diagnostic for two resource origins claiming one output path.
///
/// The conflicting origin is primary because it is the source that made the output path
/// ambiguous. The existing origin remains attached as a secondary previous-declaration label.
pub(crate) fn resource_output_path_collision_messages(
    output_path: &Path,
    existing_origin: &str,
    existing_location: &SourceLocation,
    conflicting_origin: &str,
    conflicting_location: &SourceLocation,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let reason = InvalidConfigReason::ResourceOutputPathCollision {
        output_path: path_id(output_path, string_table),
        existing_origin: string_table.intern(existing_origin),
        conflicting_origin: string_table.intern(conflicting_origin),
    };
    let diagnostic =
        CompilerDiagnostic::invalid_config_reason(None, reason, conflicting_location.clone())
            .with_labels(vec![
                DiagnosticLabel::primary(conflicting_location.clone()),
                DiagnosticLabel::secondary(
                    existing_location.clone(),
                    Some(DiagnosticLabelMessage::PreviousDeclaration),
                ),
            ]);

    CompilerMessages::from_diagnostic_ref(diagnostic, string_table)
}

/// Build a typed diagnostic for a resource claiming a builder-owned artefact path.
pub(crate) fn resource_output_path_reserved_messages(
    output_path: &Path,
    origin: &str,
    artefact_kind: &str,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let reason = InvalidConfigReason::ResourceOutputPathReserved {
        output_path: path_id(output_path, string_table),
        origin: string_table.intern(origin),
        artefact_kind: string_table.intern(artefact_kind),
    };
    let diagnostic = CompilerDiagnostic::invalid_config_reason(None, reason, location.clone());

    CompilerMessages::from_diagnostic_ref(diagnostic, string_table)
}

fn html_config_messages(
    location_path: &Path,
    reason: impl FnOnce(&mut StringTable) -> InvalidConfigReason,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let location = SourceLocation::from_path(location_path, string_table);
    let diagnostic =
        CompilerDiagnostic::invalid_config_reason(None, reason(string_table), location);

    CompilerMessages::from_diagnostic_ref(diagnostic, string_table)
}

fn path_id(path: &Path, string_table: &mut StringTable) -> StringId {
    string_table.get_or_intern(path.display().to_string())
}
