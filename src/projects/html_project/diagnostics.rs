//! Typed diagnostics for HTML-project policy checks.
//!
//! WHAT: turns deterministic routing and output-path policy failures into structured config
//! diagnostics.
//! WHY: HTML builder mistakes are user-facing project feedback, not infrastructure failures.

use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, InvalidConfigReason,
};
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::projects::html_project::resource_output_plan::ResourceDiagnosticSite;
use std::path::Path;

pub(crate) fn missing_homepage_messages(
    entry_root: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    html_config_messages(
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
    existing_span: Option<ResourceDiagnosticSite>,
    conflicting_origin: &str,
    conflicting_span: Option<ResourceDiagnosticSite>,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let reason = InvalidConfigReason::ResourceOutputPathCollision {
        output_path: path_id(output_path, string_table),
        existing_origin: string_table.intern(existing_origin),
        conflicting_origin: string_table.intern(conflicting_origin),
    };
    let mut diagnostic = CompilerDiagnostic::invalid_config_reason(
        None,
        reason,
        conflicting_span.as_ref().map(|site| site.span),
    );
    if let Some(domain) = conflicting_span
        .as_ref()
        .and_then(|site| site.source_domain.clone())
    {
        diagnostic = diagnostic
            .with_primary_frozen_identity_handle(FrozenIdentityHandle::for_domain(domain));
    }

    let existing_label = match existing_span {
        Some(site) => match site.source_domain {
            Some(domain) => DiagnosticLabel::secondary_with_frozen_identity(
                Some(site.span),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
                FrozenIdentityHandle::for_domain(domain),
            ),
            None => DiagnosticLabel::secondary(
                Some(site.span),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ),
        },
        None => DiagnosticLabel::secondary(None, Some(DiagnosticLabelMessage::PreviousDeclaration)),
    };
    let diagnostic = diagnostic.with_labels(vec![existing_label]);

    CompilerMessages::from_diagnostic_ref(diagnostic, string_table)
}

/// Build a typed diagnostic for a resource claiming a builder-owned artefact path.
pub(crate) fn resource_output_path_reserved_messages(
    output_path: &Path,
    origin: &str,
    artefact_kind: &str,
    span: Option<ResourceDiagnosticSite>,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let reason = InvalidConfigReason::ResourceOutputPathReserved {
        output_path: path_id(output_path, string_table),
        origin: string_table.intern(origin),
        artefact_kind: string_table.intern(artefact_kind),
    };
    let mut diagnostic = CompilerDiagnostic::invalid_config_reason(
        None,
        reason,
        span.as_ref().map(|site| site.span),
    );
    if let Some(domain) = span.as_ref().and_then(|site| site.source_domain.clone()) {
        diagnostic = diagnostic
            .with_primary_frozen_identity_handle(FrozenIdentityHandle::for_domain(domain));
    }

    CompilerMessages::from_diagnostic_ref(diagnostic, string_table)
}

fn html_config_messages(
    reason: impl FnOnce(&mut StringTable) -> InvalidConfigReason,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let diagnostic = CompilerDiagnostic::invalid_config_reason(None, reason(string_table), None);

    CompilerMessages::from_diagnostic_ref(diagnostic, string_table)
}

fn path_id(path: &Path, string_table: &mut StringTable) -> StringId {
    string_table.get_or_intern(portable_path_text(path))
}
