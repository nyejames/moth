//! Source labels attached to structured diagnostics.
//!
//! WHAT: represents secondary exact source spans with optional typed label messages.
//! WHY: diagnostics need enough structure for terminal rendering, dev-server rendering, and future
//! tooling without carrying final prose in compiler stages.

use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) frozen_identity_handle: Option<FrozenIdentityHandle>,
    pub(crate) style: DiagnosticLabelStyle,
    pub(crate) message: Option<DiagnosticLabelMessage>,
}

impl DiagnosticLabel {
    pub(crate) fn secondary(
        span: Option<SourceSpan>,
        message: Option<DiagnosticLabelMessage>,
    ) -> Self {
        Self {
            span,
            frozen_identity_handle: None,
            style: DiagnosticLabelStyle::Secondary,
            message,
        }
    }

    pub(crate) fn secondary_with_frozen_identity(
        span: Option<SourceSpan>,
        message: Option<DiagnosticLabelMessage>,
        frozen_identity_handle: FrozenIdentityHandle,
    ) -> Self {
        Self {
            span,
            frozen_identity_handle: Some(frozen_identity_handle),
            style: DiagnosticLabelStyle::Secondary,
            message,
        }
    }

    pub(crate) fn set_frozen_identity_handle_if_missing(
        &mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) {
        if self.frozen_identity_handle.is_none() {
            self.frozen_identity_handle = Some(frozen_identity_handle);
        }
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if let Some(message) = &mut self.message {
            message.remap_string_ids(remap);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticLabelStyle {
    Secondary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiagnosticLabelMessage {
    PreviousDeclaration,
    ConflictingAccess,
    ValueMovedHere,
    /// Identifies the retained resolver for a source `#Config` input used by a failed operation.
    ConfigInputOrigin {
        input_name: StringId,
        origin: BuildConfigValueOrigin,
    },
    /// Render-ready label text for diagnostics that need local phrasing.
    RenderedText(StringId),
    /// Marks the generic function body location where the concrete instantiation failed.
    GenericInstantiationBodySite,
    /// Marks the generic declaration that produced the instantiated body.
    GenericInstantiationDeclarationSite,
    /// Shows the concrete type substitutions selected for this generic body parse.
    GenericInstantiationSubstitutions {
        substitutions: Vec<GenericSubstitutionDiagnostic>,
    },
    /// Marks the earlier evidence that fixed a generic parameter before a later conflict.
    GenericInferencePreviousEvidence,
    /// Marks the original immutable binding declaration for assignment-target diagnostics.
    ImmutableBindingDeclaration,
    TypedFailureProducer {
        error_type_id: TypeId,
    },
    ImplicitFailureProducer,
    BuiltinFailureCall,
    BuiltinFailureOrigin,
}

impl DiagnosticLabelMessage {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            DiagnosticLabelMessage::RenderedText(message)
            | DiagnosticLabelMessage::ConfigInputOrigin {
                input_name: message,
                ..
            } => {
                *message = remap.get(*message);
            }
            DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions } => {
                for substitution in substitutions {
                    substitution.parameter_name = remap.get(substitution.parameter_name);
                }
            }
            DiagnosticLabelMessage::PreviousDeclaration
            | DiagnosticLabelMessage::ConflictingAccess
            | DiagnosticLabelMessage::ValueMovedHere
            | DiagnosticLabelMessage::GenericInstantiationBodySite
            | DiagnosticLabelMessage::GenericInstantiationDeclarationSite
            | DiagnosticLabelMessage::GenericInferencePreviousEvidence
            | DiagnosticLabelMessage::ImmutableBindingDeclaration
            | DiagnosticLabelMessage::TypedFailureProducer { .. }
            | DiagnosticLabelMessage::ImplicitFailureProducer
            | DiagnosticLabelMessage::BuiltinFailureCall
            | DiagnosticLabelMessage::BuiltinFailureOrigin => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericSubstitutionDiagnostic {
    pub(crate) parameter_name: StringId,
    pub(crate) concrete_type_id: TypeId,
}
