//! Family rows, grouped by the compiler stage that owns their diagnostics.
//!
//! WHAT: each child module holds the rows for one group. `registry` joins the groups into one table.
//! WHY: a group keeps its rows beside the diagnostics they describe, so a stage owner edits one file.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;

pub(crate) mod calls_data;
pub(crate) mod choices_control_flow;
pub(crate) mod generics_traits_api;
pub(crate) mod imports;
pub(crate) mod names_declarations;
pub(crate) mod project_config;
pub(crate) mod templates;
pub(crate) mod token_lexical;
pub(crate) mod types_borrows_targets;

/// The compact draft for a legacy diagnostic, from whichever group owns its kind. `None` when no pilot family
/// carries the diagnostic, so the caller keeps the legacy payload.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    token_lexical::from_legacy(diagnostic)
        .or_else(|| imports::from_legacy(diagnostic))
        .or_else(|| templates::from_legacy(diagnostic))
        .or_else(|| names_declarations::from_legacy(diagnostic))
        .or_else(|| calls_data::from_legacy(diagnostic))
        .or_else(|| choices_control_flow::from_legacy(diagnostic))
        .or_else(|| types_borrows_targets::from_legacy(diagnostic))
}
