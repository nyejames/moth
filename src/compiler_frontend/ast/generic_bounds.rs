//! Static trait-bound validation for concrete nominal generic instances.
//!
//! WHAT: validates declaration-site trait bounds on concrete `Struct of T` and `Choice of T`
//! instantiations once concrete type arguments are known.
//! WHY: nominal generic instances are keyed only by constructor plus type arguments. Until
//! only reusable canonical/compiler-owned evidence may satisfy those bounds.

use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAnnotation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::import_environment::{
    FileVisibility, NamespaceRecord, NamespaceTypeMember, SourceDeclarationTarget,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use rustc_hash::{FxHashMap, FxHashSet};

type GenericBoundValidationResult<T> = Result<T, Box<CompilerDiagnostic>>;

pub(crate) struct GenericBoundEvidenceContext<'a> {
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) trait_environment: Option<&'a TraitEnvironment>,
    pub(crate) trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
    pub(crate) visible_trait_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub(crate) visible_source_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub(crate) visible_type_alias_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
    pub(crate) visible_namespace_records: Option<&'a FxHashMap<StringId, NamespaceRecord>>,
    pub(crate) resolved_type_aliases: Option<&'a FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
}

impl<'a> GenericBoundEvidenceContext<'a> {
    pub(crate) fn from_file_visibility(
        type_environment: &'a TypeEnvironment,
        trait_environment: &'a TraitEnvironment,
        trait_evidence_environment: &'a TraitEvidenceEnvironment,
        visibility: &'a FileVisibility,
        resolved_type_aliases: &'a FxHashMap<InternedPath, ResolvedTypeAnnotation>,
    ) -> Self {
        Self {
            type_environment,
            trait_environment: Some(trait_environment),
            trait_evidence_environment: Some(trait_evidence_environment),
            visible_trait_names: Some(&visibility.visible_trait_names),
            visible_source_names: Some(&visibility.visible_source_names),
            visible_type_alias_names: Some(&visibility.visible_type_alias_names),
            visible_namespace_records: Some(&visibility.visible_namespace_records),
            resolved_type_aliases: Some(resolved_type_aliases),
        }
    }

    pub(crate) fn evidence_target_is_visible(&self, type_id: TypeId) -> bool {
        evidence_target_is_visible(
            type_id,
            self.type_environment,
            self.visible_source_names,
            self.visible_type_alias_names,
            self.visible_namespace_records,
            self.resolved_type_aliases,
        )
    }
}

pub(crate) fn validate_nominal_generic_bound_evidence(
    type_id: TypeId,
    location: SourceLocation,
    context: &GenericBoundEvidenceContext<'_>,
) -> GenericBoundValidationResult<()> {
    let mut visited = FxHashSet::default();
    validate_type_recursive(type_id, &location, context, &mut visited)
}

/// Resolve reusable evidence for one concrete type, including evidence declared on a generic
/// nominal constructor.
///
/// WHAT: checks exact builtin and canonical evidence first, then falls back from a concrete
/// generic instance to the evidence registered for its nominal constructor.
/// WHY: `Box must TRAIT` is one reusable conformance for every valid `Box of T` instance; the
/// public interface therefore carries the constructor identity while consumers validate a
/// concrete instance identity. Declaration-site bounds on the instance arguments are validated
/// separately by [`validate_type_recursive`].
pub(crate) fn evidence_for_type(
    type_id: TypeId,
    trait_id: TraitId,
    type_environment: &TypeEnvironment,
    evidence_environment: &TraitEvidenceEnvironment,
) -> Option<TraitEvidenceId> {
    let exact = evidence_environment
        .builtin_for(type_id, trait_id)
        .or_else(|| evidence_environment.canonical_for(type_id, trait_id));
    if exact.is_some() {
        return exact;
    }

    let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(type_id) else {
        return None;
    };
    let base_type_id = type_environment.type_id_for_nominal_id(instance.base)?;
    evidence_environment
        .builtin_for(base_type_id, trait_id)
        .or_else(|| evidence_environment.canonical_for(base_type_id, trait_id))
}

fn validate_type_recursive(
    type_id: TypeId,
    location: &SourceLocation,
    context: &GenericBoundEvidenceContext<'_>,
    visited: &mut FxHashSet<TypeId>,
) -> GenericBoundValidationResult<()> {
    if !visited.insert(type_id) {
        return Ok(());
    }

    match context.type_environment.get(type_id) {
        Some(TypeDefinition::GenericInstance(instance)) => {
            validate_instance_bounds(type_id, location, context)?;

            for argument in &instance.arguments {
                validate_type_recursive(*argument, location, context, visited)?;
            }
        }

        Some(TypeDefinition::Constructed(definition)) => {
            for argument in &definition.arguments {
                validate_type_recursive(*argument, location, context, visited)?;
            }
        }

        Some(TypeDefinition::Function(definition)) => {
            for parameter in &definition.parameters {
                validate_type_recursive(parameter.type_id, location, context, visited)?;
            }

            for return_type in &definition.returns {
                validate_type_recursive(*return_type, location, context, visited)?;
            }

            if let Some(error_type) = definition.error_return {
                validate_type_recursive(error_type, location, context, visited)?;
            }
        }

        Some(
            TypeDefinition::Builtin(..)
            | TypeDefinition::Struct(..)
            | TypeDefinition::Choice(..)
            | TypeDefinition::External(..)
            | TypeDefinition::GenericParameter(..),
        )
        | None => {}
    }

    Ok(())
}

fn validate_instance_bounds(
    instance_type_id: TypeId,
    location: &SourceLocation,
    context: &GenericBoundEvidenceContext<'_>,
) -> GenericBoundValidationResult<()> {
    let Some(TypeDefinition::GenericInstance(instance)) =
        context.type_environment.get(instance_type_id)
    else {
        return Ok(());
    };

    let Some(parameter_list_id) = context
        .type_environment
        .generic_parameter_list_id_for_type(instance_type_id)
    else {
        return Ok(());
    };
    let Some(parameter_list) = context
        .type_environment
        .generic_parameters(parameter_list_id)
    else {
        return Ok(());
    };

    for (parameter, concrete_type_id) in parameter_list.parameters.iter().zip(&instance.arguments) {
        for trait_id in &parameter.trait_bounds {
            validate_single_bound(
                instance_type_id,
                parameter.name,
                *concrete_type_id,
                *trait_id,
                location,
                context,
            )?;
        }
    }

    Ok(())
}

fn validate_single_bound(
    instance_type_id: TypeId,
    parameter_name: StringId,
    concrete_type_id: TypeId,
    trait_id: TraitId,
    location: &SourceLocation,
    context: &GenericBoundEvidenceContext<'_>,
) -> GenericBoundValidationResult<()> {
    let Some(trait_environment) = context.trait_environment else {
        return Ok(());
    };
    let Some(evidence_environment) = context.trait_evidence_environment else {
        return Ok(());
    };

    let trait_is_visible =
        trait_is_visible(trait_id, trait_environment, context.visible_trait_names);
    if trait_is_visible
        && generic_parameter_declares_bound(concrete_type_id, trait_id, context.type_environment)
    {
        return Ok(());
    }

    let has_reusable_evidence = trait_is_visible
        && context.evidence_target_is_visible(concrete_type_id)
        && evidence_for_type(
            concrete_type_id,
            trait_id,
            context.type_environment,
            evidence_environment,
        )
        .is_some();

    if has_reusable_evidence {
        return Ok(());
    }

    let trait_name = trait_environment
        .get(trait_id)
        .map(|definition| definition.name)
        .unwrap_or(parameter_name);
    let instance_name = context
        .type_environment
        .nominal_path(instance_type_id)
        .and_then(|path| path.name());

    Err(Box::new(CompilerDiagnostic::invalid_generic_instantiation(
        instance_name,
        InvalidGenericInstantiationReason::MissingNominalTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        },
        location.clone(),
    )))
}

pub(crate) fn evidence_target_is_visible(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    visible_source_names: Option<&FxHashMap<StringId, SourceDeclarationTarget>>,
    visible_type_alias_names: Option<&FxHashMap<StringId, SourceDeclarationTarget>>,
    visible_namespace_records: Option<&FxHashMap<StringId, NamespaceRecord>>,
    resolved_type_aliases: Option<&FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
) -> bool {
    if matches!(
        type_environment.get(type_id),
        Some(TypeDefinition::Builtin(_))
    ) {
        return true;
    }

    let no_file_visibility = visible_source_names.is_none()
        && visible_type_alias_names.is_none()
        && visible_namespace_records.is_none();
    if no_file_visibility {
        return true;
    }

    let target_matches = |target: &SourceDeclarationTarget| {
        source_target_resolves_to_type(target, type_id, type_environment, resolved_type_aliases)
    };

    if visible_source_names
        .into_iter()
        .flat_map(|names| names.values())
        .any(target_matches)
        || visible_type_alias_names
            .into_iter()
            .flat_map(|names| names.values())
            .any(target_matches)
    {
        return true;
    }

    visible_namespace_records
        .into_iter()
        .flat_map(|records| records.values())
        .flat_map(|record| record.type_members.values())
        .any(|member| match member {
            NamespaceTypeMember::SourceDeclaration(target) => target_matches(target),
            NamespaceTypeMember::ExternalSymbol(_) => false,
        })
}

fn source_target_resolves_to_type(
    target: &SourceDeclarationTarget,
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    resolved_type_aliases: Option<&FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
) -> bool {
    if let SourceDeclarationTarget::Imported { origin, .. } = target
        && let crate::compiler_frontend::semantic_identity::OriginDeclarationId::Type(origin) =
            origin
    {
        let source_nominal_matches = |candidate_type_id: TypeId| {
            matches!(
                type_environment.canonical_identity_for_type_id(candidate_type_id),
                Some(crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::SourceNominal(type_origin))
                    if type_origin == origin
            )
        };
        if source_nominal_matches(type_id)
            || matches!(
                type_environment.get(type_id),
                Some(TypeDefinition::GenericInstance(instance))
                    if type_environment
                        .type_id_for_nominal_id(instance.base)
                        .is_some_and(source_nominal_matches)
            )
        {
            return true;
        }
    }

    if type_environment
        .nominal_path(type_id)
        .is_some_and(|path| path == target.local_path())
    {
        return true;
    }

    resolved_type_aliases
        .and_then(|aliases| aliases.get(target.local_path()))
        .and_then(|annotation| annotation.type_id)
        == Some(type_id)
}

pub(crate) fn generic_parameter_declares_bound(
    concrete_type_id: TypeId,
    trait_id: TraitId,
    type_environment: &TypeEnvironment,
) -> bool {
    let Some(TypeDefinition::GenericParameter(parameter)) = type_environment.get(concrete_type_id)
    else {
        return false;
    };

    type_environment
        .trait_bounds_for_generic_parameter(parameter.id)
        .is_some_and(|bounds| bounds.contains(&trait_id))
}

fn trait_is_visible(
    trait_id: TraitId,
    trait_environment: &TraitEnvironment,
    visible_trait_names: Option<&FxHashMap<StringId, SourceDeclarationTarget>>,
) -> bool {
    let Some(trait_definition) = trait_environment.get(trait_id) else {
        return false;
    };

    if matches!(trait_definition.visibility, TraitVisibility::Core) {
        return true;
    }

    let Some(visible_trait_names) = visible_trait_names else {
        return true;
    };

    visible_trait_names
        .values()
        .any(|target| trait_environment.has_path(trait_id, target.local_path()))
}
