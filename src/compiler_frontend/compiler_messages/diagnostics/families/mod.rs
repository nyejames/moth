//! Family rows, grouped by the compiler stage that owns their diagnostics.
//!
//! WHAT: each child module holds the rows for one group. `registry` joins the groups into one table.
//! WHY: a group keeps its rows beside the diagnostics they describe, so a stage owner edits one file.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::{DiagnosticKind, DiagnosticPayload};
use crate::compiler_frontend::source::SourceSpan;

pub(crate) mod calls_data;
pub(crate) mod choices_control_flow;
pub(crate) mod generics_traits_api;
pub(crate) mod imports;
pub(crate) mod names_declarations;
pub(crate) mod project_config;
pub(crate) mod templates;
pub(crate) mod token_lexical;
pub(crate) mod types_borrows_targets;

type LegacyFamilyAdapter = fn(
    DiagnosticKind,
    DiagnosticPayload,
    Option<SourceSpan>,
) -> Result<DiagnosticDraft, DiagnosticPayload>;

/// The compact draft for a legacy diagnostic, from whichever group owns its kind.
pub(super) fn from_legacy(
    kind: DiagnosticKind,
    payload: DiagnosticPayload,
    span: Option<SourceSpan>,
) -> Option<DiagnosticDraft> {
    let adapters: [LegacyFamilyAdapter; 9] = [
        token_lexical::from_legacy,
        imports::from_legacy,
        templates::from_legacy,
        project_config::from_legacy,
        names_declarations::from_legacy,
        calls_data::from_legacy,
        choices_control_flow::from_legacy,
        generics_traits_api::from_legacy,
        types_borrows_targets::from_legacy,
    ];
    let mut unmatched = payload;
    for adapter in adapters {
        match adapter(kind, unmatched, span) {
            Ok(draft) => return Some(draft),
            Err(payload) => unmatched = payload,
        }
    }
    None
}
