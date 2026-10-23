//! Source labels attached to structured diagnostics.
//!
//! WHAT: represents secondary exact source spans with optional typed label messages.
//! WHY: diagnostics need enough structure for terminal rendering, dev-server rendering, and future
//! tooling without carrying final prose in compiler stages.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

use crate::compiler_frontend::compiler_messages::diagnostics::labels::GenericSubstitutionDiagnostic;
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

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
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

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
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
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    ConflictingAccess,
    ValueMovedHere,
    /// Render-ready label text for diagnostics that need local phrasing.
    RenderedText(StringId),
    /// Marks the generic function body location where the concrete instantiation failed.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    GenericInstantiationBodySite,
    /// Shows the concrete type substitutions selected for this generic body parse.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    GenericInstantiationSubstitutions {
        substitutions: Vec<GenericSubstitutionDiagnostic>,
    },
    /// Marks the original immutable binding declaration for assignment-target diagnostics.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    ImmutableBindingDeclaration,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    TypedFailureProducer {
        error_type_id: TypeId,
    },
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    ImplicitFailureProducer,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    BuiltinFailureCall,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    BuiltinFailureOrigin,
    /// Marks a compound-assignment write-back into the target type.
    ///
    /// WHAT: identifies the later conversion into the compound target that still
    ///       escapes after the RHS work was handled, naming the canonical target.
    /// WHY: the plain numeric-origin label would blame the already-handled
    ///      arithmetic; this label keeps the write-back distinct at the renderer.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6z deletes with legacy parity tests")
    )]
    BuiltinWriteBackOrigin {
        target_type_id: TypeId,
    },
}

impl DiagnosticLabelMessage {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            DiagnosticLabelMessage::RenderedText(message) => {
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
            | DiagnosticLabelMessage::ImmutableBindingDeclaration
            | DiagnosticLabelMessage::TypedFailureProducer { .. }
            | DiagnosticLabelMessage::ImplicitFailureProducer
            | DiagnosticLabelMessage::BuiltinFailureCall
            | DiagnosticLabelMessage::BuiltinFailureOrigin
            | DiagnosticLabelMessage::BuiltinWriteBackOrigin { .. } => {}
        }
    }
}
