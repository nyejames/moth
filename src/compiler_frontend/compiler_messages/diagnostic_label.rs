//! Source labels attached to structured diagnostics.
//!
//! WHAT: represents primary and secondary spans with optional typed label messages.
//! WHY: diagnostics need enough structure for terminal rendering, dev-server rendering, and future
//! tooling without carrying final prose in compiler stages.

use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub location: SourceLocation,
    pub style: DiagnosticLabelStyle,
    pub message: Option<DiagnosticLabelMessage>,
}

impl DiagnosticLabel {
    pub(crate) fn primary(location: SourceLocation) -> Self {
        Self {
            location,
            style: DiagnosticLabelStyle::Primary,
            message: None,
        }
    }

    pub(crate) fn secondary(
        location: SourceLocation,
        message: Option<DiagnosticLabelMessage>,
    ) -> Self {
        Self {
            location,
            style: DiagnosticLabelStyle::Secondary,
            message,
        }
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.location.remap_string_ids(remap);

        if let Some(message) = &mut self.message {
            message.remap_string_ids(remap);
        }
    }

    pub(crate) fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticLabelStyle {
    Primary,
    Secondary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticLabelMessage {
    PreviousDeclaration,
    ConflictingAccess,
    ExpectedTypeDeclaredHere,
    ValueMovedHere,
    /// Render-ready label text for diagnostics that need local phrasing.
    RenderedText(StringId),
    /// Marks the call site that triggered a generic function concrete instance emission.
    GenericInstantiationCallSite,
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
            | DiagnosticLabelMessage::ExpectedTypeDeclaredHere
            | DiagnosticLabelMessage::ValueMovedHere
            | DiagnosticLabelMessage::GenericInstantiationCallSite
            | DiagnosticLabelMessage::GenericInstantiationBodySite
            | DiagnosticLabelMessage::GenericInstantiationDeclarationSite
            | DiagnosticLabelMessage::GenericInferencePreviousEvidence
            | DiagnosticLabelMessage::ImmutableBindingDeclaration => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericSubstitutionDiagnostic {
    pub(crate) parameter_name: StringId,
    pub(crate) concrete_type_id: TypeId,
}
