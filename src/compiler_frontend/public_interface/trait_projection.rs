//! Direct trait-requirement projection.
//!
//! WHAT: owns [`DirectTraitProjection`], which projects one direct trait export binding at a
//! time into its final [`PublicTraitSemantics`]. Each trait binding joins exactly one resolved
//! trait root, preserves authored requirement order, and carries the publicly-authored
//! incompatibilities as stable canonical identities.
//!
//! WHY: the trait-requirement projection validates receiver `this_type`, maps direct self-type
//! occurrences to [`TraitSurfaceTypeIdentity::SelfType`] and projects every other `TypeId`
//! through the existing canonical type projection. Keeping this projection in its own module
//! separates trait-semantics construction from the per-binding declaration join, evidence
//! projection and local finalization. The declaration join calls `project_binding` once per
//! direct trait binding and reads `remaining_names` to prove every trait root joined.

use super::model::{
    PublicTraitReceiverAccess, PublicTraitRequirementParameter, PublicTraitRequirementReturn,
    PublicTraitRequirementSurface, PublicTraitSemantics, TraitSurfaceTypeIdentity,
};
use super::type_projection::project_trait_source_fact_to_canonical_identity;
use crate::compiler_frontend::ast::{
    ResolvedPublicTraitRoot, ResolvedTraitRequirementFact, ResolvedTraitSourceFact,
    TraitReceiverAccessKind,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeProjectionContext, ExportedGenericParameterIdentity,
    GenericParameterOriginResolver, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginTraitId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::FxHashMap;

/// A generic-parameter resolver that rejects every request.
///
/// WHAT: trait requirement types never legitimately reference an exported generic parameter:
/// the only generic parameter in a trait signature is the trait `this_type`, which the
/// projection special-cases as [`TraitSurfaceTypeIdentity::SelfType`] before canonical
/// projection. Any other `GenericParameterId` reaching the canonical projection is an
/// internal invariant violation, so this resolver returns a precise `CompilerError` instead
/// of inventing an identity.
struct TraitRequirementGenericParameterResolver;

impl GenericParameterOriginResolver for TraitRequirementGenericParameterResolver {
    fn resolve_generic_parameter_origin(
        &self,
        parameter_id: GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError> {
        Err(CompilerError::compiler_error(format!(
            "public-interface draft trait projection: GenericParameterId({}) reached canonical \
             projection inside a trait requirement; only the trait self type may appear and it \
             is special-cased as SelfType, so a nested or unrelated generic parameter is an \
             internal invariant violation",
            parameter_id.0
        )))
    }
}

/// Per-binding direct trait projection state.
///
/// WHAT: indexes the resolved trait roots by public name and projects one direct trait export
/// binding at a time into its final [`PublicTraitSemantics`]. Each binding joins exactly one
/// root, validates the root's canonical path resolves through the public source-trait origin
/// index to the exact binding origin, and projects the requirements and incompatibilities.
/// Roots are marked consumed so a duplicate trait binding is rejected and a leftover root is
/// reported through [`DirectTraitProjection::remaining_names`].
pub(super) struct DirectTraitProjection<'a> {
    roots_by_name: FxHashMap<&'a str, &'a ResolvedPublicTraitRoot>,
    consumed: FxHashMap<&'a str, ()>,
    trait_source_facts: &'a FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_nominal_type_origins:
        &'a FxHashMap<InternedPath, crate::compiler_frontend::semantic_identity::OriginTypeId>,
    public_source_trait_origins: &'a FxHashMap<InternedPath, OriginTraitId>,
    type_environment: &'a TypeEnvironment,
    external_registry: &'a ExternalPackageRegistry,
    string_table: &'a StringTable,
}

/// Named inputs for [`DirectTraitProjection::new`].
///
/// WHAT: bundles the resolved trait roots and the shared projection side tables (trait source
/// facts, both public source origin indexes, the type environment, the external registry and
/// the string table) into one construction value so the projection state does not take a long
/// positional parameter list.
pub(crate) struct DirectTraitProjectionInput<'a> {
    pub(super) trait_roots: &'a [ResolvedPublicTraitRoot],
    pub(super) trait_source_facts: &'a FxHashMap<TraitId, ResolvedTraitSourceFact>,
    pub(super) public_source_nominal_type_origins:
        &'a FxHashMap<InternedPath, crate::compiler_frontend::semantic_identity::OriginTypeId>,
    pub(super) public_source_trait_origins: &'a FxHashMap<InternedPath, OriginTraitId>,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) external_registry: &'a ExternalPackageRegistry,
    pub(super) string_table: &'a StringTable,
}

impl<'a> DirectTraitProjection<'a> {
    /// Build the projection state, indexing trait roots by public name.
    ///
    /// A root without a resolvable name is a `CompilerError`, and two roots sharing a public
    /// name is a duplicate that is rejected rather than silently overwriting the first.
    pub(super) fn new(input: DirectTraitProjectionInput<'a>) -> Result<Self, CompilerError> {
        let DirectTraitProjectionInput {
            trait_roots,
            trait_source_facts,
            public_source_nominal_type_origins,
            public_source_trait_origins,
            type_environment,
            external_registry,
            string_table,
        } = input;
        let mut roots_by_name: FxHashMap<&'a str, &'a ResolvedPublicTraitRoot> =
            FxHashMap::default();
        for root in trait_roots {
            let name = root.canonical_path.name_str(string_table).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "public-interface draft trait projection: a trait root has no resolvable \
                     defining name (canonical path: {:?})",
                    root.canonical_path
                ))
            })?;
            if roots_by_name.insert(name, root).is_some() {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft trait projection: two trait roots share the public \
                     name '{}'; a duplicate trait root must not silently overwrite the first",
                    name
                )));
            }
        }
        Ok(Self {
            roots_by_name,
            consumed: FxHashMap::default(),
            trait_source_facts,
            public_source_nominal_type_origins,
            public_source_trait_origins,
            type_environment,
            external_registry,
            string_table,
        })
    }

    /// Project one direct trait export binding into its final [`PublicTraitSemantics`].
    ///
    /// WHAT: keys the projection by the exact matching direct [`OriginTraitId`] export binding,
    /// preserves authored requirement order, and validates every requirement receiver
    /// `this_type` against the owning trait `this_type`. Direct parameter or return occurrences
    /// of the owning `this_type` become [`TraitSurfaceTypeIdentity::SelfType`]; every other
    /// `TypeId` projects through the existing canonical type projection as
    /// [`TraitSurfaceTypeIdentity::Concrete`]. Each projection also carries the
    /// publicly-authored `must not` incompatibilities for the trait, canonicalized through the
    /// shared `trait_source_facts` source/core mapping owner, preserving authored source order.
    /// A missing, duplicate, self, unmatched, wrong-origin or malformed-self fact is a
    /// `CompilerError`.
    pub(super) fn project_binding(
        &mut self,
        binding: &'a ExportBinding,
    ) -> Result<PublicTraitSemantics, CompilerError> {
        let OriginDeclarationId::Trait(trait_origin) = binding.origin() else {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft trait projection: a non-trait export binding '{}' was \
                 passed to the trait projection",
                binding.public_name()
            )));
        };

        let public_name = binding.public_name();
        let defining_name = trait_origin.defining_name();
        let root = self
            .roots_by_name
            .get(defining_name)
            .or_else(|| self.roots_by_name.get(public_name))
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "public-interface draft trait projection: the trait export binding '{}' for \
                     defining trait '{}' has no matching trait root; every direct trait binding \
                     must join exactly one root",
                    public_name, defining_name
                ))
            })?;

        // The trait root canonical path must resolve through the public source-trait origin
        // index to the exact binding origin.
        let resolved_origin = self
            .public_source_trait_origins
            .get(&root.canonical_path)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "public-interface draft trait projection: the trait root '{}' canonical path \
                     has no retained public source-trait origin; a private, unexported or unowned \
                     trait must not enter the public interface",
                    public_name
                ))
            })?;
        if resolved_origin != trait_origin {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft trait projection: the trait export binding '{}' origin \
                 {:?} disagrees with its root resolved origin {:?}; the binding and root must \
                 name the same trait",
                public_name, trait_origin, resolved_origin
            )));
        }

        if self.consumed.insert(defining_name, ()).is_some() {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft trait projection: trait '{}' joined more than once; \
                 aliases preserve one declaration record and must not project its root twice",
                defining_name
            )));
        }

        // Validate the trait root this_type before projecting requirements.
        validate_trait_root_this_type(root, self.type_environment, self.string_table)?;

        let nominal_resolver = super::type_projection::TransientNominalOriginResolver::new(
            self.type_environment,
            self.public_source_nominal_type_origins,
        );
        let generic_resolver = TraitRequirementGenericParameterResolver;
        let projection_context = CanonicalTypeProjectionContext::new(
            &nominal_resolver,
            &generic_resolver,
            self.external_registry,
        );

        let requirements = root
            .requirements
            .iter()
            .map(|requirement| {
                project_trait_requirement(
                    requirement,
                    root.this_type,
                    self.type_environment,
                    &projection_context,
                    self.string_table,
                )
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;

        let incompatibilities = project_trait_incompatibilities(
            &root.incompatible_trait_ids,
            trait_origin,
            self.trait_source_facts,
            self.public_source_trait_origins,
        )?;

        Ok(PublicTraitSemantics {
            requirements,
            incompatibilities,
        })
    }

    /// The trait root names that have not joined a binding, in deterministic sorted order.
    ///
    /// A leftover root is extra and must not leak; the caller reports it.
    pub(super) fn remaining_names(&self) -> Vec<&'a str> {
        let mut leftover: Vec<&str> = self
            .roots_by_name
            .keys()
            .filter(|name| !self.consumed.contains_key(**name))
            .copied()
            .collect();
        leftover.sort();
        leftover
    }
}

/// Validate that a trait root's `this_type` is the trait-local synthetic generic
/// parameter named exactly `This`.
fn validate_trait_root_this_type(
    root: &ResolvedPublicTraitRoot,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<(), CompilerError> {
    let Some(definition) = type_environment.get(root.this_type) else {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft trait projection: the trait root '{}' this_type TypeId({}) \
             is not registered in the TypeEnvironment; the trait self type must be a synthetic \
             generic parameter",
            root.canonical_path.to_string(string_table),
            root.this_type.0
        )));
    };

    let TypeDefinition::GenericParameter(parameter) = definition else {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft trait projection: the trait root '{}' this_type TypeId({}) \
             resolved to {:?}, not a GenericParameter; the trait self type must be the synthetic \
             generic parameter named exactly \"This\"",
            root.canonical_path.to_string(string_table),
            root.this_type.0,
            definition
        )));
    };

    let name = string_table.resolve(parameter.name);
    if name != "This" {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft trait projection: the trait root '{}' this_type is a \
             GenericParameter named '{}', not \"This\"; the trait self type must be named \
             exactly \"This\"",
            root.canonical_path.to_string(string_table),
            name
        )));
    }

    Ok(())
}

/// Project one resolved trait requirement into a stable surface requirement.
fn project_trait_requirement(
    requirement: &ResolvedTraitRequirementFact,
    owning_this_type: TypeId,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    string_table: &StringTable,
) -> Result<PublicTraitRequirementSurface, CompilerError> {
    if requirement.receiver.this_type != owning_this_type {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft trait projection: a requirement receiver this_type TypeId({}) \
             does not equal the owning trait this_type TypeId({}); receiver self must match the \
             owning trait before mapping immutable or mutable access",
            requirement.receiver.this_type.0, owning_this_type.0
        )));
    }

    let receiver_access = match requirement.receiver.access {
        TraitReceiverAccessKind::Immutable => PublicTraitReceiverAccess::Immutable,
        TraitReceiverAccessKind::Mutable => PublicTraitReceiverAccess::Mutable,
    };

    let name = string_table.resolve(requirement.name).to_owned();

    let parameters = requirement
        .parameters
        .iter()
        .map(|parameter| {
            let name = parameter
                .name
                .name_str(string_table)
                .map(|name| name.to_owned());
            let type_identity = project_trait_surface_type_identity(
                parameter.type_id,
                owning_this_type,
                type_environment,
                context,
            )?;
            Ok(PublicTraitRequirementParameter {
                name,
                value_mode: parameter.value_mode.clone(),
                type_identity,
            })
        })
        .collect::<Result<Vec<_>, CompilerError>>()?;

    let returns = requirement
        .returns
        .iter()
        .map(|return_slot| {
            let type_identity = project_trait_surface_type_identity(
                return_slot.type_id,
                owning_this_type,
                type_environment,
                context,
            )?;
            Ok(PublicTraitRequirementReturn {
                channel: return_slot.channel,
                type_identity,
            })
        })
        .collect::<Result<Vec<_>, CompilerError>>()?;

    Ok(PublicTraitRequirementSurface {
        name,
        receiver_access,
        parameters,
        returns,
    })
}

/// Project one trait requirement type identity.
fn project_trait_surface_type_identity(
    type_id: TypeId,
    owning_this_type: TypeId,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<TraitSurfaceTypeIdentity, CompilerError> {
    if type_id == owning_this_type {
        return Ok(TraitSurfaceTypeIdentity::SelfType);
    }

    let canonical = project_type_id_to_canonical_identity(type_id, type_environment, context)?;
    Ok(TraitSurfaceTypeIdentity::Concrete(Box::new(canonical)))
}

/// Project the publicly-authored incompatibilities for one direct public trait into ordered,
/// duplicate-free [`CanonicalTraitIdentity`] values.
fn project_trait_incompatibilities(
    incompatible_trait_ids: &[TraitId],
    owning_trait_origin: &OriginTraitId,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
) -> Result<Vec<CanonicalTraitIdentity>, CompilerError> {
    let owning_canonical = CanonicalTraitIdentity::Source(owning_trait_origin.clone());

    let mut incompatibilities = Vec::with_capacity(incompatible_trait_ids.len());
    for trait_id in incompatible_trait_ids {
        let Some(source_fact) = trait_source_facts.get(trait_id) else {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft trait projection: an incompatibility TraitId({}) for trait origin {:?} has no retained trait source fact; a missing local mapping is an internal invariant violation",
                trait_id.0, owning_trait_origin
            )));
        };

        let canonical_identity = project_trait_source_fact_to_canonical_identity(
            source_fact,
            public_source_trait_origins,
        )?;

        if canonical_identity == owning_canonical {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft trait projection: the trait origin {:?} carries an incompatibility that resolves to itself; an internal self-relation must not enter the public trait surface",
                owning_trait_origin
            )));
        }

        if incompatibilities.contains(&canonical_identity) {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft trait projection: two incompatibility trait ids for trait origin {:?} resolved to the same canonical trait identity {:?}; a duplicate must not enter the public trait surface",
                owning_trait_origin, canonical_identity
            )));
        }

        incompatibilities.push(canonical_identity);
    }

    Ok(incompatibilities)
}
