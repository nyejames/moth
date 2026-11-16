//! Typed diagnostics for HTML-project policy checks.
//!
//! WHAT: turns deterministic routing and output-path policy failures into structured config
//! diagnostics.
//! WHY: HTML builder mistakes are user-facing project feedback, not infrastructure failures.

use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDraft, InvalidConfigReason, InvalidOutputFolderReason, PreparedFailure,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::resource_output_plan::ResourceDiagnosticSite;
use std::path::Path;
use std::sync::Arc;

pub(crate) fn duplicate_html_output_path_diagnostic(
    duplicate_entry_point: &Path,
    existing_entry_point: &Path,
    output_path: &Path,
    string_table: &mut DiagnosticStringBuilder,
) -> DiagnosticDraft {
    html_config_diagnostic(
        |string_table| InvalidConfigReason::DuplicateHtmlOutputPath {
            output_path: string_table.intern_path(output_path),
            entry_point: string_table.intern_path(duplicate_entry_point),
            existing_entry_point: string_table.intern_path(existing_entry_point),
        },
        string_table,
    )
}

/// Build a typed diagnostic for two resource origins claiming one output path.
///
/// The conflicting origin is primary because it is the source that made the output path
/// ambiguous. The existing origin remains attached as a secondary previous-declaration label.
pub(crate) fn resource_output_path_collision_diagnostic(
    output_path: &Path,
    existing_origin: &str,
    existing_span: Option<ResourceDiagnosticSite>,
    conflicting_origin: &str,
    conflicting_span: Option<ResourceDiagnosticSite>,
    string_table: &mut DiagnosticStringBuilder,
) -> DiagnosticDraft {
    let reason = InvalidConfigReason::ResourceOutputPathCollision {
        output_path: string_table.intern_path(output_path),
        existing_origin: string_table.intern(existing_origin),
        conflicting_origin: string_table.intern(conflicting_origin),
    };
    let mut diagnostic = project_config::invalid_config_reason(
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

    let existing_site = existing_span.as_ref();
    let mut diagnostic = diagnostic.with_label(
        existing_site.map(|site| site.span),
        Some(LabelDraftMessage::PreviousDeclaration),
    );
    if let Some(domain) = existing_site.and_then(|site| site.source_domain.clone()) {
        diagnostic = diagnostic
            .with_label_frozen_identity_handle(0, FrozenIdentityHandle::for_domain(domain));
    }
    diagnostic
}

/// Build a typed diagnostic for a resource claiming a builder-owned artefact path.
pub(crate) fn resource_output_path_reserved_diagnostic(
    output_path: &Path,
    origin: &str,
    artefact_kind: &str,
    span: Option<ResourceDiagnosticSite>,
    string_table: &mut DiagnosticStringBuilder,
) -> DiagnosticDraft {
    let reason = InvalidConfigReason::ResourceOutputPathReserved {
        output_path: string_table.intern_path(output_path),
        origin: string_table.intern(origin),
        artefact_kind: string_table.intern(artefact_kind),
    };
    span_config_diagnostic(reason, span)
}

/// Build a typed diagnostic for an authored artefact whose output path is not portable.
pub(crate) fn artefact_output_path_invalid_diagnostic(
    output_path: &Path,
    artefact_kind: &str,
    reason: InvalidOutputFolderReason,
    span: Option<ResourceDiagnosticSite>,
    string_table: &mut DiagnosticStringBuilder,
) -> DiagnosticDraft {
    // WHAT: intern the Debug spelling only when the rejected name itself caused the failure.
    // WHY: portable display text rewrites a Unix literal `\` to `/` and replaces non-UTF-8 bytes,
    // so it can present a different, valid output path beside an invalid-component reason. Every
    // other reason rejects the path's shape, where portable text names the authored path
    // accurately.
    let output_path = match reason {
        InvalidOutputFolderReason::NonUtf8 | InvalidOutputFolderReason::InvalidPathComponent => {
            string_table.get_or_intern(format!("{output_path:?}"))
        }
        _ => string_table.intern_path(output_path),
    };

    let config_reason = InvalidConfigReason::InvalidArtefactOutputPath {
        output_path,
        artefact_kind: string_table.intern(artefact_kind),
        reason,
    };
    span_config_diagnostic(config_reason, span)
}

/// Attach the authored span and its frozen identity to an HTML artefact config diagnostic.
fn span_config_diagnostic(
    reason: InvalidConfigReason,
    span: Option<ResourceDiagnosticSite>,
) -> DiagnosticDraft {
    let mut diagnostic =
        project_config::invalid_config_reason(None, reason, span.as_ref().map(|site| site.span));
    if let Some(domain) = span.as_ref().and_then(|site| site.source_domain.clone()) {
        diagnostic = diagnostic
            .with_primary_frozen_identity_handle(FrozenIdentityHandle::for_domain(domain));
    }
    diagnostic
}

fn html_config_diagnostic(
    reason: impl FnOnce(&mut DiagnosticStringBuilder) -> InvalidConfigReason,
    string_table: &mut DiagnosticStringBuilder,
) -> DiagnosticDraft {
    project_config::invalid_config_reason(None, reason(string_table), None)
}

/// Build the no-entry diagnostic the HTML builder enforces for its executable output.
pub(crate) fn no_entry_root_diagnostic(
    entry_root: &Path,
    string_table: &mut DiagnosticStringBuilder,
) -> DiagnosticDraft {
    html_config_diagnostic(
        |string_table| InvalidConfigReason::NoRootModuleEntries {
            entry_root: string_table.intern_path(entry_root),
        },
        string_table,
    )
}

pub(crate) fn capture_html_diagnostic(
    diagnostic: DiagnosticDraft,
    root_string_table: &StringTable,
    environment: Option<&TypeEnvironment>,
    path_table: impl FnOnce() -> Option<Arc<PathTable>>,
) -> PreparedFailure {
    PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from(diagnostic),
        root_string_table,
        environment,
        path_table,
    )
    .map(PreparedFailure::Diagnosed)
    .unwrap_or_else(PreparedFailure::from)
}
