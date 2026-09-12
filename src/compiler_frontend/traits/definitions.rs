//! Resolved trait metadata produced during AST environment construction.
//!
//! WHAT: stores trait definitions after names, visibility, `This`, and requirement signatures
//! have been resolved into compiler-owned identities.
//! WHY: traits are compile-time metadata, not `DataType` declarations. Keeping them here gives
//! conformance and bounds a focused lookup surface without widening the normal value/type
//! declaration table.

use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::traits::ids::{TraitId, TraitRequirementId};
use crate::compiler_frontend::value_mode::ValueMode;

/// Source/visibility ownership for a resolved trait definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TraitVisibility {
    /// Authored source trait. `exported` means the trait is part of the module public surface.
    Source { exported: bool },
    /// Compiler-owned core trait metadata visible without a user declaration.
    Core,
}

/// Resolved method requirement inside a trait declaration.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Complete requirement facts are retained for diagnostics and static bounds.
pub(crate) struct ResolvedTraitRequirement {
    pub(crate) id: TraitRequirementId,
    pub(crate) name: StringId,
    pub(crate) receiver: TraitReceiverRequirement,
    pub(crate) parameters: Vec<ResolvedTraitParameter>,
    pub(crate) returns: Vec<ResolvedTraitReturn>,
    pub(crate) span: Option<SourceSpan>,
}

/// Required receiver access for a trait method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraitReceiverRequirement {
    Immutable { this_type: TypeId },
    Mutable { this_type: TypeId },
}

/// One non-receiver requirement parameter.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Parameter names and spans remain available for precise diagnostics.
pub(crate) struct ResolvedTraitParameter {
    pub(crate) name: PathId,
    pub(crate) value_mode: ValueMode,
    pub(crate) type_id: TypeId,
    pub(crate) span: Option<SourceSpan>,
}

/// One requirement return slot.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Return spans remain available for precise diagnostics.
pub(crate) struct ResolvedTraitReturn {
    pub(crate) type_id: TypeId,
    pub(crate) channel: ReturnChannel,
    pub(crate) span: Option<SourceSpan>,
}

/// Complete resolved trait definition.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Complete trait facts are retained for conformance and static bounds.
pub(crate) struct ResolvedTraitDefinition {
    pub(crate) id: TraitId,
    pub(crate) name: StringId,
    pub(crate) canonical_path: PathId,
    pub(crate) source_file: PathId,
    pub(crate) this_type: TypeId,
    pub(crate) requirements: Vec<ResolvedTraitRequirement>,
    pub(crate) declaration_span: Option<SourceSpan>,
    pub(crate) visibility: TraitVisibility,
}
