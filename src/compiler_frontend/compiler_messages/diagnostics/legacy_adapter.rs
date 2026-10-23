//! Temporary consuming conversion from the legacy diagnostic model into compact family drafts.
//!
//! WHAT: moves legacy payload, label and identity-owner data into a draft during the P4.6 producer migration.
//! WHY: the draft-only bag can accept unmigrated producers without cloning their values.

use super::super::diagnostic_label::{DiagnosticLabel, DiagnosticLabelMessage};
use super::draft::DiagnosticDraft;
use super::families;
use super::foreign::{ForeignSiteDraft, ForeignSlot};
use super::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;

impl From<DiagnosticLabelMessage> for LabelDraftMessage {
    fn from(message: DiagnosticLabelMessage) -> Self {
        match message {
            DiagnosticLabelMessage::PreviousDeclaration => Self::PreviousDeclaration,
            DiagnosticLabelMessage::ConflictingAccess => Self::ConflictingAccess,
            DiagnosticLabelMessage::ValueMovedHere => Self::ValueMovedHere,
            DiagnosticLabelMessage::RenderedText(text) => Self::RenderedText(text),
            DiagnosticLabelMessage::GenericInstantiationBodySite => {
                Self::GenericInstantiationBodySite
            }
            DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions } => {
                Self::GenericInstantiationSubstitutions { substitutions }
            }
            DiagnosticLabelMessage::ImmutableBindingDeclaration => {
                Self::ImmutableBindingDeclaration
            }
            DiagnosticLabelMessage::TypedFailureProducer { error_type_id } => {
                Self::TypedFailureProducer { error_type_id }
            }
            DiagnosticLabelMessage::ImplicitFailureProducer => Self::ImplicitFailureProducer,
            DiagnosticLabelMessage::BuiltinFailureCall => Self::BuiltinFailureCall,
            DiagnosticLabelMessage::BuiltinFailureOrigin => Self::BuiltinFailureOrigin,
            DiagnosticLabelMessage::BuiltinWriteBackOrigin { target_type_id } => {
                Self::BuiltinWriteBackOrigin { target_type_id }
            }
        }
    }
}

impl From<CompilerDiagnostic> for DiagnosticDraft {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        let CompilerDiagnostic {
            kind,
            severity,
            primary_span,
            primary_frozen_identity_handle,
            labels,
            payload,
        } = diagnostic;
        let mut draft = families::from_legacy(kind, payload, primary_span).unwrap_or_else(|| {
            panic!("legacy diagnostic has no compact family; the boundary converter cannot represent it")
        });
        draft = draft.with_severity(severity);
        for (index, label) in labels.into_iter().enumerate() {
            let DiagnosticLabel {
                span,
                frozen_identity_handle,
                message,
                ..
            } = label;
            draft = draft.with_label(span, message.map(LabelDraftMessage::from));
            if let Some(context) = frozen_identity_handle {
                draft = draft.with_foreign(ForeignSiteDraft {
                    slot: ForeignSlot::Label(index),
                    context,
                });
            }
        }
        if let Some(context) = primary_frozen_identity_handle {
            draft = draft.with_primary_frozen_identity_handle(context);
        }
        draft
    }
}
