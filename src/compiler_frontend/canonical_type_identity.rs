//! Canonical cross-module type identity vocabulary and projection from module-local `TypeId`.
//!
//! WHAT: owns the owned, hashable, cross-build identity values for the closed types and the
//! exported generic parameters that a `TypeEnvironment` can resolve, plus the narrow projection
//! owner that converts a module-local `TypeId` into one canonical identity.
//! WHY: cross-module interfaces must compare canonical type identities rather than donor-local
//! `TypeId` values. This module is the single owner of the canonical type identity vocabulary
//! and its projection, so later phases embed stable identities without leaking process-local IDs,
//! source locations, absolute paths or rendered display names.
//!
//! Boundary: this module does not own `PublicSemanticInterface` or the public semantic surface
//! projection. It owns the exported generic-parameter identity and the generic-parameter origin
//! resolver trait, but the production resolver implementation belongs to the public semantic
//! surface projection owner. The existing `datatypes::generic_identity_bridge::TypeIdentityKey` remains the
//! module-local HIR/diagnostic bridge and is not repurposed here. The two are intentionally
//! separate: `TypeIdentityKey` carries `InternedPath`, `StringId` and `ExternalTypeId` because
//! HIR lowering and diagnostics operate inside one module's `TypeEnvironment` and `StringTable`.
//! `CanonicalTypeIdentity` carries only owned, stable, cross-build values because it crosses
//! module boundaries. Consolidating their recursive shape-matching would blur the
//! HIR/diagnostic bridge boundary, so the duplication is superficial and the owners remain

use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, GenericParameterId, NominalTypeId, TypeConstructor,
    TypeId,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::ExternalSymbolPath;
use crate::compiler_frontend::semantic_identity::{
    FunctionOriginKind, OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId,
    StableModuleOriginIdentity, StablePackageIdentity,
};

// ---------------------------------------------------------------------------
//  Canonical type identity vocabulary
// ---------------------------------------------------------------------------

/// Owned, hashable, cross-build canonical identity for one closed type or exported generic
/// parameter that a `TypeEnvironment` can resolve.
///
/// WHAT: carries only stable, owned values. It never embeds `TypeId`, `NominalTypeId`,
/// `GenericParameterId`, `GenericParameterListId`, `InternedPath`, `StringId`,
/// `ExternalPackageId`, `ExternalTypeId`, source locations, absolute paths or rendered display
/// names.
/// WHY: this is the identity a cross-module consumer compares. Two types with the same canonical
/// identity are the same semantic type across module boundaries, checkout roots and build
/// invocations.
///
/// Transparent aliases are transparent by construction: the projection resolves an alias to its
/// target `TypeId` before producing a canonical identity, so there is no alias variant here.
/// Exported generic parameters project through the generic-parameter origin resolver so open
/// exported type shapes recurse through the same projection owner as closed types.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum CanonicalTypeIdentity {
    Builtin(CanonicalBuiltinType),
    SourceNominal(OriginTypeId),
    /// Artefact-scoped nominal identity used only by generated requests and sidecars.
    /// Public-interface projection never constructs this variant.
    ModulePrivateNominal(ModulePrivateNominalIdentity),
    ExternalOpaque(ExternalOpaqueTypeIdentity),
    Collection(CollectionTypeIdentity),
    OrderedMap(OrderedMapTypeIdentity),
    Option(Box<CanonicalTypeIdentity>),
    FallibleCarrier(FallibleCarrierTypeIdentity),
    GenericInstance(GenericInstanceTypeIdentity),
    /// Artefact-scoped concrete instance of a private nominal, used only by generated requests
    /// and sidecars. Public-interface validation rejects this variant.
    ModulePrivateGenericInstance(ModulePrivateGenericInstanceTypeIdentity),
    GenericParameter(ExportedGenericParameterIdentity),
    /// Compile-time identity of one complete anonymous const record. Carries no payload:
    /// every anonymous const record shares the compile-time-only marker type interned once
    /// per `TypeEnvironment`, and field facts live on the folded record values.
    AnonymousConstRecord,
}

impl CanonicalTypeIdentity {
    /// Visit this identity and every recursively contained canonical type exactly once per
    /// structural occurrence.
    ///
    /// Canonical type consumers use this owner instead of each reimplementing the option,
    /// collection, map, fallible and generic-instance recursion rules.
    pub(crate) fn visit(&self, visitor: &mut impl FnMut(&CanonicalTypeIdentity)) {
        visitor(self);

        match self {
            Self::Collection(collection) => collection.element().visit(visitor),
            Self::OrderedMap(map) => {
                map.key().visit(visitor);
                map.value().visit(visitor);
            }
            Self::Option(inner) => inner.visit(visitor),
            Self::FallibleCarrier(carrier) => {
                carrier.success().visit(visitor);
                carrier.error().visit(visitor);
            }
            Self::GenericInstance(instance) => {
                for argument in instance.arguments() {
                    argument.visit(visitor);
                }
            }
            Self::ModulePrivateGenericInstance(instance) => {
                for argument in instance.arguments() {
                    argument.visit(visitor);
                }
            }
            Self::Builtin(_)
            | Self::SourceNominal(_)
            | Self::ModulePrivateNominal(_)
            | Self::ExternalOpaque(_)
            | Self::GenericParameter(_)
            | Self::AnonymousConstRecord => {}
        }
    }
}

/// Stable identity for a private nominal within one declaring module artefact.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModulePrivateNominalIdentity {
    module_origin: StableModuleOriginIdentity,
    defining_path: String,
    category: OriginTypeCategory,
}

impl ModulePrivateNominalIdentity {
    pub(crate) fn new(
        module_origin: StableModuleOriginIdentity,
        defining_path: String,
        category: OriginTypeCategory,
    ) -> Self {
        Self {
            module_origin,
            defining_path,
            category,
        }
    }

    pub(crate) fn category(&self) -> OriginTypeCategory {
        self.category
    }

    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    pub(crate) fn defining_path(&self) -> &str {
        &self.defining_path
    }
}

/// Builtin canonical type identity, including seeded scalar, `None`, and `Error` identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum CanonicalBuiltinType {
    Bool,
    Int,
    Float,
    // Decimal is intentionally inactive in the Alpha surface. The variant is kept to mirror the
    // stable builtin TypeId layout seeded by `TypeEnvironment::new`.
    Decimal,
    String,
    Char,
    Range,
    None,
    Error,
}

/// Binding-backed opaque external type identity.
///
/// WHAT: owned stable package origin/path and structured external symbol path. Never
/// `ExternalPackageId` or `ExternalTypeId` alone.
/// WHY: a binding-backed type is identified by both the package provenance and where it lives in
/// that package namespace, not by a build-local ID or path spelling alone. Two independently
/// built registries may use the same package and symbol paths for packages from different origins.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ExternalOpaqueTypeIdentity {
    package: StablePackageIdentity,
    symbol_path: ExternalSymbolPath,
}

impl ExternalOpaqueTypeIdentity {
    /// Construct the owned stable identity from a package identity and structured symbol path.
    ///
    /// Compiler-internal: only the projection owner builds these from a registry reverse lookup.
    pub(crate) fn new(package: StablePackageIdentity, symbol_path: ExternalSymbolPath) -> Self {
        Self {
            package,
            symbol_path,
        }
    }

    pub(crate) fn package(&self) -> &StablePackageIdentity {
        &self.package
    }

    pub(crate) fn symbol_path(&self) -> &ExternalSymbolPath {
        &self.symbol_path
    }
}

/// Growable or fixed collection canonical identity.
///
/// `fixed_capacity` is `None` for growable `{T}` and `Some(cap)` for fixed `{N T}`. Fixed
/// capacity is semantic identity, not an allocation hint, so the two shapes are distinct.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CollectionTypeIdentity {
    element: Box<CanonicalTypeIdentity>,
    fixed_capacity: Option<usize>,
}

impl CollectionTypeIdentity {
    /// Construct a growable or fixed collection identity.
    ///
    /// Compiler-internal: only the projection owner builds these.
    pub(crate) fn new(element: CanonicalTypeIdentity, fixed_capacity: Option<usize>) -> Self {
        Self {
            element: Box::new(element),
            fixed_capacity,
        }
    }

    pub(crate) fn element(&self) -> &CanonicalTypeIdentity {
        &self.element
    }

    pub(crate) fn fixed_capacity(&self) -> Option<usize> {
        self.fixed_capacity
    }
}

/// Ordered map canonical identity. Key and value are stored directly so `{K = V}` order is
/// preserved.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OrderedMapTypeIdentity {
    key: Box<CanonicalTypeIdentity>,
    value: Box<CanonicalTypeIdentity>,
}

impl OrderedMapTypeIdentity {
    /// Construct an ordered map identity from canonical key and value identities.
    ///
    /// Compiler-internal: only the projection owner builds these.
    pub(crate) fn new(key: CanonicalTypeIdentity, value: CanonicalTypeIdentity) -> Self {
        Self {
            key: Box::new(key),
            value: Box::new(value),
        }
    }

    pub(crate) fn key(&self) -> &CanonicalTypeIdentity {
        &self.key
    }

    pub(crate) fn value(&self) -> &CanonicalTypeIdentity {
        &self.value
    }
}

/// Fallible carrier canonical identity. Success and error are stored in order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct FallibleCarrierTypeIdentity {
    success: Box<CanonicalTypeIdentity>,
    error: Box<CanonicalTypeIdentity>,
}

impl FallibleCarrierTypeIdentity {
    /// Construct a fallible carrier identity from canonical success and error identities.
    ///
    /// Compiler-internal: only the projection owner builds these.
    pub(crate) fn new(success: CanonicalTypeIdentity, error: CanonicalTypeIdentity) -> Self {
        Self {
            success: Box::new(success),
            error: Box::new(error),
        }
    }

    pub(crate) fn success(&self) -> &CanonicalTypeIdentity {
        &self.success
    }

    pub(crate) fn error(&self) -> &CanonicalTypeIdentity {
        &self.error
    }
}

/// Concrete source nominal generic instance canonical identity.
///
/// WHAT: keyed by the stable base `OriginTypeId` plus recursively canonical concrete arguments.
/// WHY: two instances of the same generic nominal with the same canonical arguments share one
/// canonical identity across module boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GenericInstanceTypeIdentity {
    base: OriginTypeId,
    arguments: Box<[CanonicalTypeIdentity]>,
}

/// Stable artefact-scoped identity for a concrete instance of one private nominal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModulePrivateGenericInstanceTypeIdentity {
    base: ModulePrivateNominalIdentity,
    arguments: Box<[CanonicalTypeIdentity]>,
}

impl ModulePrivateGenericInstanceTypeIdentity {
    pub(crate) fn new(
        base: ModulePrivateNominalIdentity,
        arguments: Box<[CanonicalTypeIdentity]>,
    ) -> Self {
        Self { base, arguments }
    }

    pub(crate) fn base(&self) -> &ModulePrivateNominalIdentity {
        &self.base
    }

    pub(crate) fn arguments(&self) -> &[CanonicalTypeIdentity] {
        &self.arguments
    }
}

impl GenericInstanceTypeIdentity {
    /// Construct a concrete generic instance identity from a stable base origin and canonical
    /// concrete arguments.
    ///
    /// Compiler-internal: only the projection owner builds these after validating exact arity.
    pub(crate) fn new(base: OriginTypeId, arguments: Box<[CanonicalTypeIdentity]>) -> Self {
        Self { base, arguments }
    }

    pub(crate) fn base(&self) -> &OriginTypeId {
        &self.base
    }

    pub(crate) fn arguments(&self) -> &[CanonicalTypeIdentity] {
        &self.arguments
    }
}

/// The stable origin of the generic declaration that owns one exported generic parameter.
///
/// WHAT: narrows the owning generic declaration to the legal exported generic-declaration
/// owners: free functions and nominal types (structs and choices). Constants, traits,
/// transparent aliases, receiver methods and synthetic trait `This` are not valid exported
/// generic declaration owners and are unrepresentable.
/// WHY: a generic parameter's cross-module identity must derive from its owning declaration's
/// stable origin, not from a donor-local `GenericParameterId`. Keeping the owner domain narrow
/// prevents a transparent alias, receiver method or constant from being mistaken for a generic
/// declaration. The two-case internal shape is the narrowest representation consistent with the
/// existing origin vocabulary: `OriginFunctionId` for free functions and `OriginTypeId` for
/// nominal types. The inner enum is private so no crate caller can bypass the validating
/// constructors.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GenericDeclarationOrigin {
    inner: GenericDeclarationOriginInner,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum GenericDeclarationOriginInner {
    FreeFunction(OriginFunctionId),
    NominalType(OriginTypeId),
}

impl GenericDeclarationOrigin {
    /// Construct a nominal-type generic declaration origin, rejecting transparent aliases.
    ///
    /// Compiler-internal: only structs and choices declare generic parameters. A transparent
    /// alias is not a generic declaration owner and must not appear as one.
    pub(crate) fn nominal_type(origin: OriginTypeId) -> Result<Self, CompilerError> {
        match origin.category() {
            OriginTypeCategory::Struct | OriginTypeCategory::Choice => Ok(Self {
                inner: GenericDeclarationOriginInner::NominalType(origin),
            }),
            OriginTypeCategory::TransparentAlias => Err(CompilerError::compiler_error(
                "a transparent alias is not a valid exported generic declaration owner; only structs and choices declare generic parameters",
            )),
        }
    }

    /// Construct a free-function generic declaration origin, rejecting receiver methods.
    ///
    /// Compiler-internal: only free functions declare standalone exported generic parameters.
    /// A receiver method lives on its receiver type's surface and is not an independent generic
    /// declaration owner, so a `FunctionOriginKind::Receiver` origin must not appear here.
    pub(crate) fn free_function(origin: OriginFunctionId) -> Result<Self, CompilerError> {
        match origin.kind() {
            FunctionOriginKind::Free => Ok(Self {
                inner: GenericDeclarationOriginInner::FreeFunction(origin),
            }),
            FunctionOriginKind::Receiver(_) => Err(CompilerError::compiler_error(
                "a receiver method is not a valid exported generic declaration owner; only free \
                 functions declare standalone generic parameters",
            )),
        }
    }

    /// Return the nominal declaration that owns this generic parameter list, when present.
    ///
    /// Generic receiver methods reuse the enclosing nominal's local parameter handles during
    /// generated materialisation. Free-function templates keep their own declaration-local
    /// parameter list instead.
    pub(crate) fn nominal_type_origin(&self) -> Option<&OriginTypeId> {
        match &self.inner {
            GenericDeclarationOriginInner::FreeFunction(_) => None,
            GenericDeclarationOriginInner::NominalType(origin) => Some(origin),
        }
    }
}

/// Owned, hashable, cross-build canonical identity for one exported generic parameter.
///
/// WHAT: derives from the stable origin of the owning generic declaration, the
/// declaration-local parameter position and the owned authored parameter name. It stores no
/// `GenericParameterId`, `GenericParameterListId`, `TypeId`, `StringId`, `InternedPath`,
/// source location, source file, declaration order outside the parameter list or rendered
/// display name lookup.
/// WHY: cross-module interfaces must compare exported generic parameters by their stable
/// declaration origin and position, not by donor-local `GenericParameterId` values that differ
/// across module compilations. Two parameters with the same owner, position and authored name
/// share one identity even when their module-local `GenericParameterId` allocations differ.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ExportedGenericParameterIdentity {
    declaration_origin: GenericDeclarationOrigin,
    position: u32,
    authored_name: String,
}

impl ExportedGenericParameterIdentity {
    /// Construct the stable identity for one exported generic parameter.
    ///
    /// Compiler-internal: the resolver owner builds this from the owning declaration's stable
    /// origin, the declaration-local parameter position and the owned authored name. The
    /// `GenericDeclarationOrigin` constructors already reject transparent aliases and receiver
    /// methods.
    pub(crate) fn new(
        declaration_origin: GenericDeclarationOrigin,
        position: u32,
        authored_name: String,
    ) -> Self {
        Self {
            declaration_origin,
            position,
            authored_name,
        }
    }

    /// The stable origin of the owning generic declaration.
    pub(crate) fn declaration_origin(&self) -> &GenericDeclarationOrigin {
        &self.declaration_origin
    }

    pub(crate) fn authored_name(&self) -> &str {
        &self.authored_name
    }
}

// ---------------------------------------------------------------------------
//  Canonical trait identity vocabulary
// ---------------------------------------------------------------------------

/// Owned, hashable, cross-build canonical identity for one trait.
///
/// WHAT: carries only stable, owned values. It distinguishes source-declared traits
/// (`Source(OriginTraitId)`) from compiler-owned core traits
/// (`Core(CanonicalCoreTraitIdentity)`). It never embeds `TraitId`, `StringId`,
/// `InternedPath`, `FileId`, source location, rendered display name or a
/// `CoreTraitKind` registry handle.
/// WHY: cross-module generic bound surfaces must compare trait identities rather than
/// donor-local `TraitId` values. A source trait and a core trait with the same source
/// spelling are semantically distinct: the source trait's identity derives from its
/// owning module origin, while the core trait's identity is a stable compiler-owned
/// semantic classifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum CanonicalTraitIdentity {
    Source(OriginTraitId),
    Core(CanonicalCoreTraitIdentity),
    /// Artefact-scoped trait identity used only by generated requests and sidecars.
    /// Public-interface projection never constructs this variant.
    ModulePrivate(ModulePrivateTraitIdentity),
}

/// Stable identity for a private trait within one declaring module artefact.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModulePrivateTraitIdentity {
    module_origin: StableModuleOriginIdentity,
    defining_path: String,
}

impl ModulePrivateTraitIdentity {
    pub(crate) fn new(module_origin: StableModuleOriginIdentity, defining_path: String) -> Self {
        Self {
            module_origin,
            defining_path,
        }
    }
}

/// Compiler-owned core trait canonical identity.
///
/// WHAT: models `Displayable` and cast traits exactly from the existing `CoreTraitKind`
/// facts, reusing `BuiltinCastTarget` and `BuiltinCastFallibility` from the builtins
/// cast-target owner rather than introducing a second core catalogue. It stores no
/// `TraitId`, `StringId`, path, source location or rendered name.
/// WHY: core traits are identified by their stable semantic classification, not by a
/// module-local `TraitId` or a source spelling. Two builds that register the same core
/// cast trait produce the same canonical identity regardless of `TraitId` allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum CanonicalCoreTraitIdentity {
    Displayable,
    Castable {
        target: BuiltinCastTarget,
        fallibility: BuiltinCastFallibility,
    },
}

/// Named target-plus-trait identity for one reusable conformance evidence record.
///
/// WHAT: combines exactly one [`CanonicalTypeIdentity`] (the conforming target) with one
/// [`CanonicalTraitIdentity`] (the conformed trait). It is the single stable identity a
/// reusable evidence record carries across the draft boundary. It never embeds local IDs,
/// paths, locations or order.
/// WHY: target-plus-trait is the stable key a cross-module consumer can compare without
/// donor-local `TypeId` or `TraitId` handles. Keeping the pair in one owned hashable value
/// makes duplicate detection and cross-module comparison operate on one identity rather than
/// two adjacent pseudo-key fields.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CanonicalEvidenceIdentity {
    target_type_identity: CanonicalTypeIdentity,
    trait_identity: CanonicalTraitIdentity,
}

impl CanonicalEvidenceIdentity {
    /// Construct the canonical evidence identity from the target type and trait identities.
    ///
    /// Compiler-internal: only the public-interface draft evidence projection builds these,
    /// from the already-projected canonical target type identity and canonical trait
    /// identity.
    pub(crate) fn new(
        target_type_identity: CanonicalTypeIdentity,
        trait_identity: CanonicalTraitIdentity,
    ) -> Self {
        Self {
            target_type_identity,
            trait_identity,
        }
    }

    /// The canonical identity of the conformed trait.
    pub(crate) fn trait_identity(&self) -> &CanonicalTraitIdentity {
        &self.trait_identity
    }

    /// The canonical identity of the conforming target type.
    pub(crate) fn target_type_identity(&self) -> &CanonicalTypeIdentity {
        &self.target_type_identity
    }
}

/// Stable cross-module identity for one trait requirement.
///
/// WHAT: combines the canonical trait identity with the owned defining requirement name as
/// authored in the trait declaration. It never embeds `TraitRequirementId`, `TraitId`,
/// `StringId`, `InternedPath`, source location or declaration order. Two builds that assign
/// different dense `TraitRequirementId` values to the same authored requirement on the same
/// canonical trait produce the same stable identity.
/// WHY: reusable conformance evidence maps each requirement to the stable receiver-method
/// origin that implements it. The mapping key must be stable across local allocation changes
/// so a cross-module consumer can compare requirement identities without donor-local IDs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StableTraitRequirementIdentity {
    trait_identity: CanonicalTraitIdentity,
    requirement_name: String,
}

impl StableTraitRequirementIdentity {
    /// Construct the stable requirement identity from the canonical trait identity and the
    /// owned defining requirement name.
    ///
    /// Compiler-internal: only the public-interface draft evidence projection builds these,
    /// from the canonical trait identity and the requirement name resolved through the
    /// `TraitEnvironment`.
    pub(crate) fn new(trait_identity: CanonicalTraitIdentity, requirement_name: String) -> Self {
        Self {
            trait_identity,
            requirement_name,
        }
    }

    /// The owned defining requirement name as authored in the trait declaration.
    pub(crate) fn requirement_name(&self) -> &str {
        &self.requirement_name
    }

    pub(crate) fn trait_identity(&self) -> &CanonicalTraitIdentity {
        &self.trait_identity
    }
}

// ---------------------------------------------------------------------------
//  Projection context
// ---------------------------------------------------------------------------

/// Resolves a module-local `NominalTypeId` to its stable source-nominal `OriginTypeId`.
///
/// WHAT: the projection receives this resolver so it can map source nominal struct/choice types
/// to their stable cross-module origin without embedding donor-local `NominalTypeId` values in
/// the canonical identity.
/// WHY: a missing source nominal origin is a `CompilerError`, never a silently omitted fact. The
/// resolver is supplied by the public semantic surface projection owner, not by the projection
/// itself. For focused tests a simple map-backed implementation is sufficient.
pub(crate) trait NominalOriginResolver {
    /// Returns the stable origin identity for a module-local nominal, or a `CompilerError` when
    /// the nominal has no exported origin.
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError>;
}

/// Resolves a module-local `GenericParameterId` to its stable exported generic-parameter
/// identity.
///
/// WHAT: the projection receives this resolver so it can map an exported generic parameter to
/// its stable cross-module identity without embedding donor-local `GenericParameterId` values
/// in the canonical identity.
/// WHY: a missing exported generic-parameter identity is a `CompilerError`, never a silently
/// omitted fact or a guess from the parameter name alone. The resolver is supplied by the
/// public semantic surface projection owner, not by the projection itself. For focused tests a
/// simple map-backed implementation is sufficient.
pub(crate) trait GenericParameterOriginResolver {
    /// Returns the stable exported identity for a module-local generic parameter, or a
    /// `CompilerError` when the parameter has no exported origin.
    fn resolve_generic_parameter_origin(
        &self,
        parameter_id: GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError>;
}

/// Explicit context for projecting a `TypeId` into a canonical identity.
///
/// WHAT: carries the source-nominal origin resolver, the generic-parameter origin resolver and
/// the external package registry. All three are borrowed for the duration of the projection.
/// WHY: keeps the projection function's signature narrow and explicit about its three external
/// dependencies. The projection itself owns no state.
pub(crate) struct CanonicalTypeProjectionContext<'a> {
    nominal_origins: &'a dyn NominalOriginResolver,
    generic_parameter_origins: &'a dyn GenericParameterOriginResolver,
    external_registry: &'a ExternalPackageRegistry,
}

impl<'a> CanonicalTypeProjectionContext<'a> {
    /// Construct the projection context from its three borrowed dependencies.
    ///
    /// Compiler-internal: the public semantic surface projection owner builds this once per
    /// module compilation. Focused tests build it directly.
    pub(crate) fn new(
        nominal_origins: &'a dyn NominalOriginResolver,
        generic_parameter_origins: &'a dyn GenericParameterOriginResolver,
        external_registry: &'a ExternalPackageRegistry,
    ) -> Self {
        Self {
            nominal_origins,
            generic_parameter_origins,
            external_registry,
        }
    }

    /// The nominal origin resolver used to map `NominalTypeId` to stable `OriginTypeId`.
    pub(crate) fn nominal_origins(&self) -> &dyn NominalOriginResolver {
        self.nominal_origins
    }

    /// The generic-parameter origin resolver used to map `GenericParameterId` to stable
    /// `ExportedGenericParameterIdentity`.
    ///
    /// WHAT: lets the public type-surface projection owner project each exported generic
    /// parameter list entry in declaration-local order through the same total resolver that
    /// already backs open-parameter canonical type projection, without reconstructing identity
    /// from name or position a second time.
    pub(crate) fn generic_parameter_origins(&self) -> &dyn GenericParameterOriginResolver {
        self.generic_parameter_origins
    }
}

// ---------------------------------------------------------------------------
//  Projection
// ---------------------------------------------------------------------------

/// Projects a module-local `TypeId` into a canonical cross-module type identity.
///
/// WHAT: reads the `TypeDefinition` for `type_id` from `type_environment`, resolves source
/// nominal origins through the context, resolves exported generic parameters through the
/// generic-parameter origin resolver, resolves binding-backed opaque types through the external
/// registry, and recursively projects constructed and generic-instance arguments.
/// WHY: this is the single owner of the `TypeId -> CanonicalTypeIdentity` conversion. It
/// returns `CompilerError` for every incomplete or unsupported state instead of returning
/// `None`, using a sentinel, guessing from rendered names or panicking.
///
/// The following states return `CompilerError` with precise invariant context:
/// - missing source nominal origin (the nominal has no exported `OriginTypeId`)
/// - missing exported generic-parameter identity (the `GenericParameterId` has no resolver entry)
/// - missing external stable identity (the `ExternalTypeId` was never registered)
/// - function types (not a canonical type)
/// - tuple and other internal-only constructed shapes
/// - malformed arity (wrong argument count for a builtin constructor or a generic instance)
///
/// Transparent aliases are transparent: if the `TypeEnvironment` ever stores an alias, the
/// projection follows its resolved target `TypeId` and does not manufacture an alias variant.
/// Exported generic parameters project through the generic-parameter origin resolver so open
/// exported type shapes recurse through the same single projection path as closed types.
pub(crate) fn project_type_id_to_canonical_identity(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<CanonicalTypeIdentity, CompilerError> {
    if let Some(identity) = type_environment.canonical_identity_for_type_id(type_id) {
        return Ok(identity.clone());
    }

    let definition = type_environment.get(type_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "canonical type projection received an unregistered TypeId({}); the TypeEnvironment \
             has no definition for it, so this is an internal invariant violation",
            type_id.0
        ))
    })?;

    match definition {
        TypeDefinition::Builtin(builtin) => {
            Ok(CanonicalTypeIdentity::Builtin(project_builtin(builtin.key)))
        }
        TypeDefinition::Struct(def) => {
            let origin = context
                .nominal_origins
                .resolve_nominal_origin(def.id)
                .map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical type projection could not resolve a source-nominal origin for \
                         struct NominalTypeId({}): {error_msg}",
                        def.id.0,
                        error_msg = error.msg
                    ))
                })?;
            Ok(CanonicalTypeIdentity::SourceNominal(origin))
        }
        TypeDefinition::Choice(def) => {
            let origin = context
                .nominal_origins
                .resolve_nominal_origin(def.id)
                .map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical type projection could not resolve a source-nominal origin for \
                         choice NominalTypeId({}): {error_msg}",
                        def.id.0,
                        error_msg = error.msg
                    ))
                })?;
            Ok(CanonicalTypeIdentity::SourceNominal(origin))
        }
        TypeDefinition::External(def) => {
            let (package, symbol_path) = context
                .external_registry
                .resolve_type_package_and_symbol_path(def.type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "canonical type projection could not resolve a stable package/symbol \
                         identity for ExternalTypeId({}); the type was not registered through \
                         the single registration path, so this is an inconsistent-registry \
                         invariant",
                        def.type_id.0
                    ))
                })?;
            Ok(CanonicalTypeIdentity::ExternalOpaque(
                ExternalOpaqueTypeIdentity::new(package, symbol_path.clone()),
            ))
        }
        TypeDefinition::Constructed(constructed) => {
            project_constructed(constructed, type_environment, context)
        }
        TypeDefinition::GenericInstance(instance) => {
            project_generic_instance(instance, type_environment, context)
        }
        TypeDefinition::Function(_) => Err(CompilerError::compiler_error(format!(
            "canonical type projection does not support function types; TypeId({}) is a function \
             type, which is not a closed canonical type identity",
            type_id.0
        ))),
        TypeDefinition::GenericParameter(parameter) => {
            let identity = context
                .generic_parameter_origins
                .resolve_generic_parameter_origin(parameter.id)
                .map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical type projection could not resolve an exported generic-parameter identity for GenericParameterId({}): {error_msg}",
                        parameter.id.0,
                        error_msg = error.msg
                    ))
                })?;
            Ok(CanonicalTypeIdentity::GenericParameter(identity))
        }

        // The compile-time-only anonymous const-record marker projects to the payload-free
        // public identity; it has no origin type and never resolves through a nominal.
        TypeDefinition::AnonymousConstRecordMarker => {
            Ok(CanonicalTypeIdentity::AnonymousConstRecord)
        }
    }
}

/// Maps a builtin scalar key to its canonical builtin identity.
fn project_builtin(key: BuiltinTypeKey) -> CanonicalBuiltinType {
    match key {
        BuiltinTypeKey::Bool => CanonicalBuiltinType::Bool,
        BuiltinTypeKey::Int => CanonicalBuiltinType::Int,
        BuiltinTypeKey::Float => CanonicalBuiltinType::Float,
        BuiltinTypeKey::Decimal => CanonicalBuiltinType::Decimal,
        BuiltinTypeKey::String => CanonicalBuiltinType::String,
        BuiltinTypeKey::Char => CanonicalBuiltinType::Char,
        BuiltinTypeKey::Range => CanonicalBuiltinType::Range,
        BuiltinTypeKey::None => CanonicalBuiltinType::None,
    }
}

/// Projects a constructed type, validating exact arity for each builtin constructor.
fn project_constructed(
    constructed: &crate::compiler_frontend::datatypes::definitions::ConstructedTypeDefinition,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<CanonicalTypeIdentity, CompilerError> {
    let arguments = constructed.arguments.as_ref();
    match &constructed.constructor {
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
            let [element_id] = arguments else {
                return Err(malformed_arity_error("collection", 1, arguments.len()));
            };
            let element =
                project_type_id_to_canonical_identity(*element_id, type_environment, context)?;
            Ok(CanonicalTypeIdentity::Collection(
                CollectionTypeIdentity::new(element, *fixed_capacity),
            ))
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
            let [inner_id] = arguments else {
                return Err(malformed_arity_error("option", 1, arguments.len()));
            };
            let inner =
                project_type_id_to_canonical_identity(*inner_id, type_environment, context)?;
            Ok(CanonicalTypeIdentity::Option(Box::new(inner)))
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier) => {
            let [success_id, error_id] = arguments else {
                return Err(malformed_arity_error(
                    "fallible carrier",
                    2,
                    arguments.len(),
                ));
            };
            let success =
                project_type_id_to_canonical_identity(*success_id, type_environment, context)?;
            let error =
                project_type_id_to_canonical_identity(*error_id, type_environment, context)?;
            Ok(CanonicalTypeIdentity::FallibleCarrier(
                FallibleCarrierTypeIdentity::new(success, error),
            ))
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
            let [key_id, value_id] = arguments else {
                return Err(malformed_arity_error("ordered map", 2, arguments.len()));
            };
            let key = project_type_id_to_canonical_identity(*key_id, type_environment, context)?;
            let value =
                project_type_id_to_canonical_identity(*value_id, type_environment, context)?;
            Ok(CanonicalTypeIdentity::OrderedMap(
                OrderedMapTypeIdentity::new(key, value),
            ))
        }
        TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple) => {
            Err(CompilerError::compiler_error(
                "canonical type projection does not support tuple or internal-only constructed \
                 shapes; tuples are not part of the canonical closed-type identity vocabulary",
            ))
        }
    }
}

/// Projects a generic instance, validating that the argument count matches the base nominal's
/// declared generic parameter count.
fn project_generic_instance(
    instance: &crate::compiler_frontend::datatypes::definitions::GenericInstanceDefinition,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<CanonicalTypeIdentity, CompilerError> {
    let expected_arity = validate_generic_instance_base_arity(instance.base, type_environment)?;
    let base_type_id = type_environment
        .type_id_for_nominal_id(instance.base)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "canonical type projection could not resolve a TypeId for generic-instance base NominalTypeId({})",
                instance.base.0,
            ))
        })?;
    let private_base = match type_environment.canonical_identity_for_type_id(base_type_id) {
        Some(CanonicalTypeIdentity::SourceNominal(origin)) => Some(Ok(origin.clone())),
        Some(CanonicalTypeIdentity::ModulePrivateNominal(_)) => None,
        Some(identity) => {
            return Err(CompilerError::compiler_error(format!(
                "canonical generic-instance base has non-nominal identity {identity:?}",
            )));
        }
        None => Some(
            context
                .nominal_origins
                .resolve_nominal_origin(instance.base),
        ),
    };

    if instance.arguments.len() != expected_arity {
        return Err(CompilerError::compiler_error(format!(
            "canonical type projection found a malformed generic-instance arity: \
             NominalTypeId({}) declares {expected_arity} generic parameters but the instance \
             carries {} concrete arguments",
            instance.base.0,
            instance.arguments.len()
        )));
    }

    let mut projected_arguments = Vec::with_capacity(instance.arguments.len());
    for argument_id in instance.arguments.iter() {
        let projected =
            project_type_id_to_canonical_identity(*argument_id, type_environment, context)?;
        projected_arguments.push(projected);
    }

    let arguments = projected_arguments.into_boxed_slice();
    if let Some(base_origin) = private_base {
        return Ok(CanonicalTypeIdentity::GenericInstance(
            GenericInstanceTypeIdentity::new(
                base_origin.map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical type projection could not resolve a source-nominal origin for generic-instance base NominalTypeId({}): {error_msg}",
                        instance.base.0,
                        error_msg = error.msg,
                    ))
                })?,
                arguments,
            ),
        ));
    }
    let Some(CanonicalTypeIdentity::ModulePrivateNominal(base)) =
        type_environment.canonical_identity_for_type_id(base_type_id)
    else {
        return Err(CompilerError::compiler_error(
            "canonical private generic-instance base lost its private nominal identity",
        ));
    };
    Ok(CanonicalTypeIdentity::ModulePrivateGenericInstance(
        ModulePrivateGenericInstanceTypeIdentity::new(base.clone(), arguments),
    ))
}

/// Validates the generic-instance base and returns its declared generic parameter count.
///
/// WHAT: rejects an unknown/missing nominal base, a struct or choice base whose
/// `generic_parameters` is `None` (even when the instance carries zero arguments), and a
/// referenced generic parameter list missing from `TypeEnvironment`.
/// WHY: a generic instance must be built from a nominal that actually declares a complete
/// generic parameter list. The previous silent `0` fallback let a zero-argument instance of a
/// non-generic nominal project as if it were a legal concrete instance.
fn validate_generic_instance_base_arity(
    nominal_id: NominalTypeId,
    type_environment: &TypeEnvironment,
) -> Result<usize, CompilerError> {
    let (generic_parameters, kind) = match (
        type_environment.struct_definition(nominal_id),
        type_environment.choice_definition(nominal_id),
    ) {
        (Some(def), _) => (def.generic_parameters, "struct"),
        (None, Some(def)) => (def.generic_parameters, "choice"),
        (None, None) => {
            return Err(CompilerError::compiler_error(format!(
                "canonical type projection found a generic-instance base NominalTypeId({}) that \
                 is neither a registered struct nor a choice; a generic instance must be built \
                 from a known nominal base",
                nominal_id.0
            )));
        }
    };

    let list_id = generic_parameters.ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "canonical type projection found a generic instance of {kind} \
             NominalTypeId({}) whose generic parameter list is absent; a generic instance \
             requires a base that actually declares generic parameters",
            nominal_id.0
        ))
    })?;

    let list = type_environment
        .generic_parameters(list_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "canonical type projection found a generic instance of {kind} \
             NominalTypeId({}) whose declared generic parameter list \
             GenericParameterListId({}) is missing from the TypeEnvironment",
                nominal_id.0, list_id.0
            ))
        })?;

    Ok(list.parameters.len())
}

/// Constructs a `CompilerError` for a malformed constructed-type arity.
fn malformed_arity_error(constructor_name: &str, expected: usize, actual: usize) -> CompilerError {
    CompilerError::compiler_error(format!(
        "canonical type projection found a malformed {constructor_name} arity: expected \
         {expected} argument(s) but the constructed type carries {actual}"
    ))
}
