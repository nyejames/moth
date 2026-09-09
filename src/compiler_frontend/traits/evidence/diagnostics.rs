//! Trait conformance diagnostic construction.
//!
//! WHAT: Constructs `CompilerDiagnostic` payloads and diagnostic labels for trait conformance failures.
//! WHY: Centralizes reporting structure for missing requirements, override issues, duplicate conformances,
//!      and signature mismatches, keeping them separated from validation logic.

use crate::compiler_frontend::ast::ReceiverMethodEntry;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, InvalidTraitConformanceReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::definitions::ResolvedTraitRequirement;

pub(super) fn invalid_conformance(
    target_name: StringId,
    trait_name: Option<StringId>,
    reason: InvalidTraitConformanceReason,
    primary_span: Option<SourceSpan>,
    secondary_labels: Vec<DiagnosticLabel>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_trait_conformance(target_name, trait_name, reason, primary_span)
        .with_labels(secondary_labels)
}

pub(super) fn previous_declaration_label(
    previous_span: Option<SourceSpan>,
) -> Vec<DiagnosticLabel> {
    previous_span
        .map(|span| {
            vec![DiagnosticLabel::secondary(
                Some(span),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            )]
        })
        .unwrap_or_default()
}

pub(super) fn requirement_label(
    requirement: &ResolvedTraitRequirement,
    string_table: &mut StringTable,
) -> Vec<DiagnosticLabel> {
    vec![DiagnosticLabel::secondary(
        requirement.span,
        Some(DiagnosticLabelMessage::RenderedText(
            string_table.intern("trait requirement"),
        )),
    )]
}

pub(super) fn requirement_and_method_labels(
    requirement: &ResolvedTraitRequirement,
    method: &ReceiverMethodEntry,
    string_table: &mut StringTable,
) -> Vec<DiagnosticLabel> {
    vec![
        DiagnosticLabel::secondary(
            requirement.span,
            Some(DiagnosticLabelMessage::RenderedText(
                string_table.intern("trait requirement"),
            )),
        ),
        DiagnosticLabel::secondary(
            method
                .signature
                .parameters
                .first()
                .and_then(|parameter| parameter.value.span),
            Some(DiagnosticLabelMessage::RenderedText(
                string_table.intern("receiver method"),
            )),
        ),
    ]
}
