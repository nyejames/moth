//! Trait conformance diagnostic construction.
//!
//! WHAT: Constructs compact diagnostic drafts and their secondary labels for trait conformance failures.
//! WHY: Centralizes reporting structure for missing requirements, override issues, duplicate conformances,
//!      and signature mismatches, keeping them separated from validation logic.

use crate::compiler_frontend::ast::ReceiverMethodEntry;
use crate::compiler_frontend::compiler_messages::InvalidTraitConformanceReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;

use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::traits::definitions::ResolvedTraitRequirement;

pub(super) fn invalid_conformance(
    target_name: StringId,
    trait_name: Option<StringId>,
    reason: InvalidTraitConformanceReason,
    primary_span: Option<SourceSpan>,
    secondary_labels: impl IntoIterator<Item = (Option<SourceSpan>, LabelDraftMessage)>,
) -> DiagnosticDraft {
    let mut diagnostic = generics_traits_api::invalid_trait_conformance(
        target_name,
        trait_name,
        reason,
        primary_span,
    );
    for (span, message) in secondary_labels {
        diagnostic = diagnostic.with_label(span, Some(message));
    }
    diagnostic
}

pub(super) fn previous_declaration_label(
    previous_span: Option<SourceSpan>,
) -> impl Iterator<Item = (Option<SourceSpan>, LabelDraftMessage)> {
    previous_span
        .map(|span| (Some(span), LabelDraftMessage::PreviousDeclaration))
        .into_iter()
}

pub(super) fn requirement_label(
    requirement: &ResolvedTraitRequirement,
) -> impl Iterator<Item = (Option<SourceSpan>, LabelDraftMessage)> {
    std::iter::once((requirement.span, LabelDraftMessage::TraitRequirementSite))
}

pub(super) fn requirement_and_method_labels(
    requirement: &ResolvedTraitRequirement,
    method: &ReceiverMethodEntry,
) -> [(Option<SourceSpan>, LabelDraftMessage); 2] {
    [
        (requirement.span, LabelDraftMessage::TraitRequirementSite),
        (
            method
                .signature
                .parameters
                .first()
                .and_then(|parameter| parameter.binding_span),
            LabelDraftMessage::ReceiverMethodSite,
        ),
    ]
}
