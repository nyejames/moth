//! Static trait-bound validation for concrete nominal generic instances.
//!
//! WHAT: validates declaration-site trait bounds on concrete `Struct of T` and `Choice of T`
//! instantiations once concrete type arguments are known.
//! WHY: nominal generic instances are keyed only by constructor plus type arguments. Until
//! only reusable canonical/compiler-owned evidence may satisfy those bounds.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::import_environment::{
    FileVisibility, SourceDeclarationTarget,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::{FxHashMap, FxHashSet};

type GenericBoundValidationResult<T> = Result<T, Box<CompilerDiagnostic>>;

pub(crate) struct GenericBoundEvidenceContext<'a> {
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) trait_environment: Option<&'a TraitEnvironment>,
    pub(crate) trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
    pub(crate) visible_trait_names: Option<&'a FxHashMap<StringId, SourceDeclarationTarget>>,
}

impl<'a> GenericBoundEvidenceContext<'a> {
    pub(crate) fn from_file_visibility(
        type_environment: &'a TypeEnvironment,
        trait_environment: &'a TraitEnvironment,
        trait_evidence_environment: &'a TraitEvidenceEnvironment,
        visibility: &'a FileVisibility,
        _source_file_scope: &'a InternedPath,
    ) -> Self {
        Self {
            type_environment,
            trait_environment: Some(trait_environment),
            trait_evidence_environment: Some(trait_evidence_environment),
            visible_trait_names: Some(&visibility.visible_trait_names),
        }
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
        && (evidence_environment
            .builtin_for(concrete_type_id, trait_id)
            .is_some()
            || evidence_environment
                .canonical_for(concrete_type_id, trait_id)
                .is_some());

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

fn generic_parameter_declares_bound(
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
        .any(|target| target.local_path() == &trait_definition.canonical_path)
}
