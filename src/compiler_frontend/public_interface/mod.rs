//! The one aggregate pre-HIR public-semantic handoff for a compiled module.
//!
//! WHAT: owns the [`PublicInterfaceDraftBuilder`] and the [`PublicInterfaceDraft`] it produces.
//! The draft is the sole pre-HIR public-semantic handoff that crosses the semantic compilation
//! boundary. It is declaration-centric: one [`PublicDeclarationRecord`] per stable
//! [`OriginDeclarationId`], carrying a closed [`PublicDeclarationSemantics`] enum that
//! distinguishes free functions, structs, choices, transparent aliases, constants and traits.
//! Receiver methods are attached to their owning struct or choice record, not stored as a
//! top-level parallel vector. Direct and re-exported [`ExportBinding`] values remain distinct
//! from declaration records so a public alias can change the consumer-facing name without
//! changing the donor-owned semantic origin. Exported-name diagnostic provenance is carried in
//! its own portable side table for cross-module remapping.
//!
//! The builder internalizes the projection components as private builder steps:
//! - the pre-AST direct-export seed ([`DirectExportSeed`]) carrying the module origin, export
//!   bindings and the transient nominal-type origin lookup,
//! - the callable seed table built from the seed and the resolved root table,
//! - the canonical type projection (free functions, nominals, aliases, constants and
//!   receiver-method signatures),
//! - the direct trait projection ([`DirectTraitProjection`]) producing final
//!   [`PublicTraitSemantics`] per trait binding,
//! - the per-binding declaration-record projection that consumes each resolved root directly,
//! - the direct reusable-evidence projection ([`project_reusable_evidence`]).
//!
//! These intermediates are consumed before the draft boundary: the draft stores only `Public*`
//! semantic leaf types, stable export bindings and portable exported-name diagnostic provenance.
//! The transient projection indexes and the seed are destructured and dropped before the draft.
//! No `DefinedPublic*` aggregate surface crosses orchestration.
//!
//! WHY: the compiler design overview and the recovery plan require one aggregate producer
//! boundary with a declaration-centric shape instead of parallel `DefinedPublic*` fields that
//! every later phase would have to rejoin. Keeping the projections behind one builder
//! preserves their proven, total projection logic while ensuring only one draft crosses
//! orchestration. Reusable evidence is the final step: it consumes the already-finalized
//! receiver-method surface attached to each struct or choice declaration record, so the
//! evidence projection never iterates [`crate::compiler_frontend::ast::ReceiverMethodCatalog`] and never reconstructs a
//! receiver-method origin. Stable receiver origins have one construction owner in
//! `receiver_projection`; the per-binding projection carries those exact values into the draft
//! records consumed here.
//!
//! ## Module structure
//!
//! - [`model`] owns every `Public*` semantic value type, the declaration-centric record model,
//!   the reusable-evidence record model, the pre-HIR draft aggregate and the completed phase.
//! - [`export_projection`] owns the pre-AST direct-export seed and the public source-nominal
//!   and source-trait origin indexes built from header shells.
//! - [`type_projection`] owns the transient nominal and generic-parameter origin resolvers,
//!   the root-to-binding join index, the shared trait-source-fact projection and the per-root
//!   canonical type projection helpers that produce final `Public*` semantic parts.
//! - [`receiver_projection`] owns the callable seed table and the receiver-method signature
//!   projection.
//! - [`direct_projection`] owns the builder, named input/result, the folded-value context and
//!   the per-binding declaration-record projection.
//! - [`trait_projection`] owns the direct trait-requirement projection producing final
//!   [`PublicTraitSemantics`] per binding.
//! - [`evidence_projection`] owns the reusable evidence projection.
//! - [`local_finalization`] owns the post-borrow-validation completed-phase join.
//!
//! Boundary: the draft is private to compiler/build orchestration and never reaches backends.
//! It is not the final `PublicSemanticInterface`.

mod direct_projection;
mod evidence_projection;
mod export_projection;
mod import_bindings;
mod interface_closure;
mod interface_validation;
mod interface_view;
mod local_finalization;
mod model;
mod receiver_projection;
mod trait_projection;
mod type_projection;

// Re-export the production API surface so callers import from
// `crate::compiler_frontend::public_interface::{...}`.
pub(crate) use direct_projection::{PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput};
#[cfg(test)]
pub(crate) use export_projection::DirectExportSeed;
pub(crate) use export_projection::{
    build_direct_export_seed, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
pub(crate) use import_bindings::{
    ProviderImportKind, ProviderInterfaceId, SourceProviderImport, SourceProviderImportSet,
};
#[cfg(test)]
pub(crate) use model::LocalPublicInterface;
#[cfg(test)]
pub(crate) use model::PublicExportDiagnosticProvenance;
pub(crate) use model::{
    PublicChoiceSemantics, PublicConstantSemantics, PublicDeclarationRecord,
    PublicDeclarationSemantics, PublicDiagnosticLocation, PublicEvidenceRecord,
    PublicFunctionCategory, PublicGenericParameterSurface, PublicInterfaceDraft,
    PublicParameterTypeSlot, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
    PublicReturnTypeSlot, PublicSemanticInterface, PublicStructSemantics,
    PublicTraitReceiverAccess, PublicTraitRequirementSurface, TraitSurfaceTypeIdentity,
};
pub(crate) use receiver_projection::CallableSeed;

// Re-export items needed by focused child test modules. They remain confined to this module
// family; the `#[cfg(test)]` gate avoids unused-import warnings in production builds.
#[cfg(test)]
use evidence_projection::{EvidenceProjectionContext, project_reusable_evidence};
#[cfg(test)]
pub(crate) use model::{
    PublicEvidenceOwnership, PublicFunctionSemantics, PublicGenericTemplateDescriptor,
    PublicTraitSemantics,
};
#[cfg(test)]
pub(crate) use receiver_projection::CallableSeedKind;
#[cfg(test)]
pub(crate) use receiver_projection::build_callable_seed_table;
#[cfg(test)]
use receiver_projection::receiver_method_semantics_from_seed;
#[cfg(test)]
use trait_projection::DirectTraitProjection;
#[cfg(test)]
pub(crate) use trait_projection::DirectTraitProjectionInput;
#[cfg(test)]
use type_projection::TransientNominalOriginResolver;

#[cfg(test)]
mod tests;
