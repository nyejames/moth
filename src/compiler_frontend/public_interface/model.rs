//! Semantic value types and phase types for the pre-HIR public-semantic handoff.
//!
//! WHAT: owns every `Public*` semantic leaf type, the declaration-centric record model, the
//! reusable-evidence record model, the pre-HIR [`PublicInterfaceDraft`] aggregate and the
//! completed [`LocalPublicInterface`] phase. These types carry only owned stable values: no
//! donor-local `TypeId`, `NominalTypeId`, `GenericParameterId`, `TraitId`, `InternedPath` or
//! `StringId` crosses this boundary.
//!
//! WHY: the compiler design overview and the recovery plan require one declaration-centric
//! model with distinct phase types instead of one object with mutable pending states. The
//! draft is the sole pre-HIR public-semantic handoff; the completed phase is produced exactly
//! once by local finalization. Keeping the value and phase types in one model module gives
//! every projection and finalization step a single vocabulary owner.

use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
    ExportedGenericParameterIdentity, StableTraitRequirementIdentity,
};
use crate::compiler_frontend::external_packages::CanonicalBindingSymbolIdentity;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::public_call_summary::{PublicCallParameterAccess, PublicCallSummary};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginFunctionId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;

// ===========================================================================
//  Public semantic leaf vocabulary
// ===========================================================================

/// One parameter slot in a public function or receiver-method semantic record.
///
/// WHAT: a draft/public semantic leaf type that crosses the draft boundary inside
/// [`PublicFunctionSemantics`] and [`PublicReceiverMethodSemantics`]. `name` is the owned
/// authored parameter name, or `None` when the source signature omits it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicParameterTypeSlot {
    pub(crate) name: Option<String>,
    pub(crate) type_identity: CanonicalTypeIdentity,
    /// Source-level access selected by the resolved signature.
    ///
    /// WHAT: retains shared, mutable or reactive parameter access before HIR and borrow
    /// validation. This remains distinct from mutation, optional transfer and reactive effects.
    /// WHY: generic declarations have no base concrete summary, while every consumer still needs
    /// the declaration-stable access contract.
    pub(crate) access: PublicCallParameterAccess,
    /// The owned folded default value, or `None` when the parameter has no default.
    ///
    /// WHAT: retains the compile-time default expression as an owned backend-neutral
    /// [`PublicFoldedValue`]. Constant references are resolved and inlined by the
    /// established function-signature and struct-default owners before finalization;
    /// finalization normalizes template payloads and synchronizes emitted declarations
    /// into the retained root table and receiver catalog. The receiver parameter itself
    /// normally has no default and remains `None`. Choice payload fields remain
    /// default-free.
    pub(crate) folded_default: Option<PublicFoldedValue>,
}

/// One return slot in a public function or receiver-method semantic record.
///
/// WHAT: a draft/public semantic leaf type that crosses the draft boundary inside
/// [`PublicFunctionSemantics`] and [`PublicReceiverMethodSemantics`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublicReturnTypeSlot {
    pub(crate) type_identity: CanonicalTypeIdentity,
}

/// One generic parameter with its ordered canonical trait bound identities in a public
/// declaration record.
///
/// WHAT: a draft/public semantic leaf type that crosses the draft boundary inside
/// [`PublicFunctionSemantics`], [`PublicStructSemantics`] and [`PublicChoiceSemantics`]. It
/// pairs the stable [`ExportedGenericParameterIdentity`] (declaration owner + position +
/// authored name, unchanged) with an ordered `Vec<CanonicalTraitIdentity>` resolved from the
/// `TypeEnvironment`'s declaration-site `TraitId` bounds. The identity never carries bounds;
/// the bounds are a separate fact on this entry.
/// WHY: the exported generic parameter must carry both identity and bounds so a cross-module
/// consumer can see the full constraint shape without donor-local `TraitId`,
/// `GenericParameterId`, `InternedPath`, `StringId`, `FileId`, `CoreTraitKind` registry handle
/// or source location.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublicGenericParameterSurface {
    pub(crate) identity: ExportedGenericParameterIdentity,
    pub(crate) bounds: Vec<CanonicalTraitIdentity>,
}

/// One field in a public struct semantic record or a choice-variant payload.
///
/// WHAT: a draft/public semantic leaf type that crosses the draft boundary inside
/// [`PublicStructSemantics`] and [`PublicChoiceVariantSurface`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicFieldTypeSlot {
    pub(crate) name: String,
    pub(crate) type_identity: CanonicalTypeIdentity,
    /// The owned folded default value, or `None` when the field has no default.
    ///
    /// WHAT: retains the compile-time default expression as an owned backend-neutral
    /// [`PublicFoldedValue`]. Constant references are resolved and inlined by the
    /// established function-signature and struct-default owners before finalization;
    /// finalization normalizes template payloads and synchronizes emitted declarations
    /// into the retained root table and receiver catalog. Choice payload fields remain
    /// default-free.
    pub(crate) folded_default: Option<PublicFoldedValue>,
}

/// One choice variant in a public choice semantic record.
///
/// WHAT: a draft/public semantic leaf type that crosses the draft boundary inside
/// [`PublicChoiceSemantics`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicChoiceVariantSurface {
    pub(crate) name: String,
    pub(crate) payload_fields: Vec<PublicFieldTypeSlot>,
}

// ===========================================================================
//  Trait surface value types
// ===========================================================================

/// Trait-local vocabulary for one type identity in a trait requirement signature.
///
/// WHAT: a trait requirement parameter or return type is either the trait self type
/// (`SelfType`) or an ordinary projected canonical type (`Concrete`). The self marker is
/// trait-local: it never enters the unscoped [`CanonicalTypeIdentity`] vocabulary, so an
/// unrelated local `TypeId` cannot be misclassified as trait self.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TraitSurfaceTypeIdentity {
    SelfType,
    Concrete(Box<CanonicalTypeIdentity>),
}

/// Required receiver access for one trait requirement, stored separately from the self type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublicTraitReceiverAccess {
    Immutable,
    Mutable,
}

/// One non-receiver parameter in a trait requirement surface.
///
/// `name` is the owned authored parameter name, or `None` when the source signature omits it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicTraitRequirementParameter {
    pub(crate) name: Option<String>,
    pub(crate) value_mode: ValueMode,
    pub(crate) type_identity: TraitSurfaceTypeIdentity,
}

/// One return slot in a trait requirement surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicTraitRequirementReturn {
    pub(crate) channel: ReturnChannel,
    pub(crate) type_identity: TraitSurfaceTypeIdentity,
}

/// One method requirement in a trait surface, in authored order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicTraitRequirementSurface {
    pub(crate) name: String,
    pub(crate) receiver_access: PublicTraitReceiverAccess,
    pub(crate) parameters: Vec<PublicTraitRequirementParameter>,
    pub(crate) returns: Vec<PublicTraitRequirementReturn>,
}

// ===========================================================================
//  Declaration-centric record value types
// ===========================================================================

/// The closed semantic category for one public declaration record.
///
/// WHAT: a distinct variant per directly-defined public declaration category. Struct and choice
/// are separate variants so nominal meaning is never implicit in empty field/variant vectors.
/// Each variant carries only the semantic facts already produced at R1; folded constant
/// values are owned by the constant variant. The free-function variant carries an explicit
/// callable category that distinguishes directly-defined concrete-local functions from
/// generic-template declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublicDeclarationSemantics {
    Function(PublicFunctionSemantics),
    Struct(PublicStructSemantics),
    Choice(PublicChoiceSemantics),
    TransparentAlias(PublicAliasSemantics),
    Constant(PublicConstantSemantics),
    Trait(PublicTraitSemantics),
}

/// The explicit generic-template descriptor for one exported generic free function.
///
/// WHAT: owns the stable generic parameter identities and their ordered canonical trait
/// bounds — the current required-evidence shape — that a cross-module consumer needs for
/// generic inference. It is present only on generic free-function records: a non-generic
/// free function carries no descriptor. The enclosing [`PublicDeclarationRecord`] remains the
/// stable declaration-origin owner and the enclosing [`PublicFunctionSemantics`] remains the
/// canonical parameter and return contract owner, so the descriptor does not duplicate origin
/// or signature types. No raw tokens, donor-local path, source location,
/// `GenericParameterListId`, `GenericParameterId`, `TypeId`, `TraitId` or other local
/// registry handle enters this descriptor.
///
/// WHY: locked decision 10 separates consumer-visible generic semantic identity from the
/// declaring module's retained template body and compilation context. This descriptor is the
/// consumer-visible generic contract; the validated body artefact and materialisation context
/// remain a later compiler-metadata fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicGenericTemplateDescriptor {
    pub(crate) generic_parameters: Vec<PublicGenericParameterSurface>,
}

/// The explicit callable category for one exported free function.
///
/// WHAT: distinguishes a directly-defined concrete-local callable, which receives exactly one
/// concrete summary after borrow validation, from a generic-template declaration, which is a
/// consumer-visible contract whose generated concrete summaries belong to sidecars. The
/// generic-template variant carries the stable generic parameter identities and bounds a
/// cross-module consumer needs for inference, so a malformed concrete/declaration combination
/// is unrepresentable: a concrete-local function cannot carry a descriptor and a generic
/// declaration cannot be paired with a concrete summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublicFunctionCategory {
    /// A directly defined non-generic free function. Receives exactly one concrete summary after
    /// borrow validation, retained in [`LocalPublicInterface::concrete_call_summaries`].
    ConcreteLocal,
    /// A generic-template declaration. Never receives a base concrete summary; generated summaries
    /// remain sidecar-owned.
    GenericTemplate(PublicGenericTemplateDescriptor),
}

/// The semantic facts for one exported free function: callable category, parameter types,
/// success returns and error return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicFunctionSemantics {
    /// The callable category: concrete-local or generic-template. Concrete-local callables
    /// receive exactly one summary after borrow validation; generic declarations do not.
    pub(crate) category: PublicFunctionCategory,
    pub(crate) parameters: Vec<PublicParameterTypeSlot>,
    pub(crate) returns: Vec<PublicReturnTypeSlot>,
    pub(crate) error_return: Option<CanonicalTypeIdentity>,
}

/// The explicit callable category for one exported receiver method.
///
/// WHAT: distinguishes a directly defined concrete-local receiver method, which receives exactly
/// one concrete summary after borrow validation, from a generic-template receiver method, which is
/// a declaration contract whose generated concrete summaries belong to sidecars. A generic
/// receiver method carries no descriptor here because its stable generic parameter identities are
/// owned by the enclosing nominal record's generic parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublicReceiverMethodCategory {
    /// A directly defined non-generic receiver method. Receives exactly one concrete summary
    /// after borrow validation.
    ConcreteLocal,
    /// A generic-template receiver method. Never receives a base concrete summary.
    GenericTemplate,
}

/// The semantic facts for one exported receiver method, attached to its owning struct or choice
/// declaration record. The receiver origin is the parent record's origin and is not repeated here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicReceiverMethodSemantics {
    pub(crate) method_origin: OriginFunctionId,
    /// The callable category: concrete-local or generic-template.
    pub(crate) category: PublicReceiverMethodCategory,
    pub(crate) parameters: Vec<PublicParameterTypeSlot>,
    pub(crate) returns: Vec<PublicReturnTypeSlot>,
    pub(crate) error_return: Option<CanonicalTypeIdentity>,
}

/// The semantic facts for one exported nominal struct: generic parameters/bounds, fields and
/// receiver methods attached to this struct's surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicStructSemantics {
    pub(crate) generic_parameters: Vec<PublicGenericParameterSurface>,
    pub(crate) fields: Vec<PublicFieldTypeSlot>,
    pub(crate) receiver_methods: Vec<PublicReceiverMethodSemantics>,
}

/// The semantic facts for one exported nominal choice: generic parameters/bounds, variants and
/// receiver methods attached to this choice's surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicChoiceSemantics {
    pub(crate) generic_parameters: Vec<PublicGenericParameterSurface>,
    pub(crate) variants: Vec<PublicChoiceVariantSurface>,
    pub(crate) receiver_methods: Vec<PublicReceiverMethodSemantics>,
}

/// The semantic facts for one exported transparent alias: the resolved target type identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicAliasSemantics {
    pub(crate) target_type_identity: CanonicalTypeIdentity,
}

/// The semantic facts for one exported constant: the canonical type identity and the owned
/// fully folded value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicConstantSemantics {
    pub(crate) type_identity: CanonicalTypeIdentity,
    pub(crate) folded_value: PublicFoldedValue,
}

/// The semantic facts for one exported trait: its ordered requirements with receiver access,
/// parameter modes/types and return channels/types, plus the ordered, duplicate-free
/// canonical identities of the publicly-authored traits this trait must not be claimed
/// alongside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicTraitSemantics {
    pub(crate) requirements: Vec<PublicTraitRequirementSurface>,
    pub(crate) incompatibilities: Vec<CanonicalTraitIdentity>,
}

/// One declaration-centric record in the public interface draft.
///
/// WHAT: carries exactly one stable [`OriginDeclarationId`], the aggregate
/// [`SyntheticInterfaceProvenance`] of all public semantic values owned by that declaration and
/// its closed [`PublicDeclarationSemantics`]. The builder produces one record per stable origin in
/// the deterministic export-binding order, with receiver methods deterministically attached to
/// their owning struct or choice record.
///
/// Empty provenance means the declaration is portable. Provenance is a separate semantic fact from
/// declaration/source identity: it is cloned unchanged when an imported declaration record is
/// retained through interface closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicDeclarationRecord {
    pub(crate) origin: OriginDeclarationId,
    pub(crate) synthetic_interface_provenance: SyntheticInterfaceProvenance,
    pub(crate) semantics: PublicDeclarationSemantics,
}

// ===========================================================================
//  Reusable evidence value types
// ===========================================================================

/// Semantic ownership classification for one reusable evidence record.
///
/// WHAT: marks source-authored canonical conformance evidence owned by the declaring module.
/// Direct module drafts contain only [`PublicEvidenceOwnership::SourceCanonical`] records because
/// builtin evidence is compiler-global and must not be duplicated into every source-module draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublicEvidenceOwnership {
    /// Source-authored canonical conformance evidence owned by the declaring module.
    SourceCanonical,
}

/// One trait requirement mapped to the stable receiver-method origin that implements it.
///
/// WHAT: carries the stable requirement identity (canonical trait identity plus owned
/// defining requirement name) and the stable [`OriginFunctionId`] of the exact receiver method
/// selected by conformance validation. The mapping order matches the trait's authored
/// requirement order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicEvidenceRequirementMapping {
    pub(crate) requirement_identity: StableTraitRequirementIdentity,
    pub(crate) method_origin: OriginFunctionId,
}

/// One stable reusable evidence record in the public interface draft.
///
/// WHAT: carries one [`CanonicalEvidenceIdentity`] (the canonical target-plus-trait key),
/// a semantic ownership classification, and every trait requirement in authored order mapped
/// to the stable implementing receiver-method origin. It never embeds
/// `TraitEvidenceId`, `TraitId`, `TraitRequirementId`, `TypeId`, `InternedPath`, `StringId`,
/// source location or declaration order. Evidence for a private target or private source trait,
/// or whose requirement methods are absent from the completed public receiver surface, does not
/// enter the draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicEvidenceRecord {
    pub(crate) identity: CanonicalEvidenceIdentity,
    pub(crate) ownership: PublicEvidenceOwnership,
    pub(crate) requirement_mappings: Vec<PublicEvidenceRequirementMapping>,
}

/// The one aggregate pre-HIR public-semantic handoff for one compiled module.
///
/// WHAT: owns the owning [`StableModuleOriginIdentity`] (even when the module exports nothing),
/// the deterministic [`ExportBinding`] values distinct from declaration records, one
/// [`PublicDeclarationRecord`] per stable [`OriginDeclarationId`], and one separate
/// deterministic [`PublicEvidenceRecord`] collection for direct reusable evidence. It carries
/// only owned stable values: no donor-local `TypeId`, `NominalTypeId`, `GenericParameterId`,
/// `TraitId`, `InternedPath` or `StringId` crosses this boundary.
///
/// It is deliberately not the final `PublicSemanticInterface`. Generic template bodies and
/// cross-module call lowering remain for later phases. Exported-name diagnostic provenance is
/// already portable here, and re-export bindings already retain donor-owned origins before the
/// completed interface is published.
/// Folded constant values are owned by each constant declaration record. Reusable evidence is a
/// separate collection, not a declaration variant. Concrete callable borrow summaries never enter
/// the pre-HIR draft: [`finalize_after_borrow_validation`](Self::finalize_after_borrow_validation)
/// consumes the draft and returns the completed [`LocalPublicInterface`] phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicInterfaceDraft {
    pub(crate) module_origin: StableModuleOriginIdentity,
    pub(crate) export_bindings: Vec<ExportBinding>,
    pub(crate) export_diagnostic_provenance: Vec<PublicExportDiagnosticProvenance>,
    pub(crate) binding_exports: Vec<PublicBindingExport>,
    pub(crate) declarations: Vec<PublicDeclarationRecord>,
    pub(crate) reusable_evidence: Vec<PublicEvidenceRecord>,
}

/// One binding-backed symbol re-exported by a source module.
///
/// WHAT: owns the source-facing public name plus the canonical binding package identity,
/// structured symbol path and declaration category.
/// WHY: build-local `ExternalSymbolId` values cannot cross a source-module interface. A consumer
/// resolves this stable identity through the binding package registry supplied to its own build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicBindingExport {
    pub(crate) exporting_module: StableModuleOriginIdentity,
    pub(crate) public_name: String,
    pub(crate) target: CanonicalBindingSymbolIdentity,
}

/// Portable source coordinates retained for diagnostics on a completed provider interface.
///
/// WHAT: stores authored scope components and character spans as owned values so a provider can
///       carry declaration provenance without leaking its compiler-local `StringId` table.
/// WHY: semantic identity must remain independent from diagnostic provenance, while consumers
///      still need to remap provider declaration locations into their own string table when a
///      visible-name collision crosses a module boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicDiagnosticLocation {
    pub(crate) scope_components: Vec<String>,
    pub(crate) start_line: i32,
    pub(crate) start_column: i32,
    pub(crate) end_line: i32,
    pub(crate) end_column: i32,
}

/// Authored diagnostic provenance for one public export spelling.
///
/// WHAT: maps the provider-facing public name to its declaration location without changing the
///       stable `ExportBinding` identity or declaration semantics.
/// WHY: aliases and provider re-exports need a diagnostic side table so the visible-name registry
///      can label the actual declaration that introduced a collision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicExportDiagnosticProvenance {
    pub(crate) public_name: String,
    pub(crate) location: PublicDiagnosticLocation,
}

/// One completed concrete-local summary record, retained in stable-origin order.
///
/// WHAT: pairs a stable [`OriginFunctionId`] with its complete [`PublicCallSummary`] after borrow
/// validation. Only concrete-local callables receive a record; generic-template declarations never
/// do. The records are contiguous and ordered by stable origin so the completed phase stores no
/// durable hash map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConcreteCallSummaryRecord {
    pub(crate) origin: OriginFunctionId,
    pub(crate) summary: PublicCallSummary,
}

/// The completed direct public interface for one compiled module.
///
/// WHAT: owns the pre-HIR [`PublicInterfaceDraft`] plus the deterministic concrete-local summary
/// records produced after borrow validation. It is the sole completed direct-interface handoff
/// for one compiled module. The summary records are contiguous and
/// ordered by stable origin; construction may use transient lookup maps or sets, but none survive
/// past this boundary.
///
/// WHY: separating the pre-HIR draft from the completed phase removes same-type temporal mutation:
/// the draft is built before borrow validation and carries only direct declaration facts, while
/// this type is produced exactly once by [`PublicInterfaceDraft::finalize_after_borrow_validation`]
/// and carries exactly one summary record per concrete-local callable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalPublicInterface {
    pub(crate) draft: PublicInterfaceDraft,
    /// Exactly one record per concrete-local public callable, in stable-origin order. Generic
    /// declarations and private callables never appear here.
    pub(crate) concrete_call_summaries: Vec<ConcreteCallSummaryRecord>,
}

/// Immutable semantic surface published to modules in later graph waves.
///
/// WHAT: owns the stable export bindings, declaration records, reusable evidence and concrete
/// callable summaries that a consumer may observe. Construction consumes the completed local
/// interface so an incomplete pre-borrow draft cannot enter the provider store.
/// WHY: graph scheduling needs a phase-distinct success value. Keeping provider lookup on this
/// closed owned surface prevents consumers from opening provider headers, AST, HIR or local type
/// environments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicSemanticInterface {
    pub(crate) module_origin: StableModuleOriginIdentity,
    pub(crate) export_bindings: Vec<ExportBinding>,
    pub(crate) export_diagnostic_provenance: Vec<PublicExportDiagnosticProvenance>,
    pub(crate) binding_exports: Vec<PublicBindingExport>,
    pub(crate) declarations: Vec<PublicDeclarationRecord>,
    pub(crate) reusable_evidence: Vec<PublicEvidenceRecord>,
    pub(crate) concrete_call_summaries: Vec<ConcreteCallSummaryRecord>,
}

impl PublicSemanticInterface {
    pub(crate) fn declaration(
        &self,
        origin: &OriginDeclarationId,
    ) -> Option<&PublicDeclarationRecord> {
        self.declarations
            .binary_search_by(|declaration| declaration.origin.cmp(origin))
            .ok()
            .map(|index| &self.declarations[index])
    }
}
