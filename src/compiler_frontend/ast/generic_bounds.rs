//! Static trait-bound validation for concrete nominal generic instances.
//!
//! WHAT: validates declaration-site trait bounds on concrete `Struct of T` and `Choice of T`
//! instantiations once concrete type arguments are known.
//! WHY: nominal generic instances are keyed only by constructor plus type arguments, so
//! only reusable canonical/compiler-owned evidence may satisfy those bounds.
use crate::compiler_frontend::builtins::casts::evidence::builtin_cast_proves_core_trait;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;

use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use rustc_hash::FxHashSet;

type GenericBoundValidationResult<T> = Result<T, CompilerDiagnostic>;

/// Evidence selected for a concrete trait bound in the requester context.
///
/// Only registered evidence carries an evidence ID. An on-demand builtin proof retains the
/// exact requester-local source and trait pair so request freezing can canonicalize it without
/// mutating the evidence registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundEvidenceSelection {
    Registered(TraitEvidenceId),
    BuiltinCastProof {
        source_type_id: TypeId,
        trait_id: TraitId,
    },
}

pub(crate) struct GenericBoundEvidenceContext<'a> {
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) trait_environment: Option<&'a TraitEnvironment>,
    pub(crate) trait_evidence_environment: Option<&'a TraitEvidenceEnvironment>,
}

pub(crate) fn validate_nominal_generic_bound_evidence(
    type_id: TypeId,
    instance_name: Option<StringId>,
    span: Option<SourceSpan>,
    context: &GenericBoundEvidenceContext<'_>,
) -> GenericBoundValidationResult<()> {
    let mut visited = FxHashSet::default();
    validate_type_recursive(type_id, instance_name, span, context, &mut visited)
}

/// Resolve reusable evidence for one concrete type, including evidence declared on a generic
/// nominal constructor, then consider a matching on-demand builtin cast proof.
///
/// WHAT: preserves exact registered evidence precedence and the generic constructor fallback.
///      Only after those registered rows are absent can the compiler-owned cast pair policy prove
///      a builtin core cast trait on demand.
/// WHY: the public interface carries constructor identity while consumers validate a concrete
///      instance. Pair-scoped builtin selection leaves the requester's evidence registry unchanged.
pub(crate) fn evidence_for_type(
    type_id: TypeId,
    trait_id: TraitId,
    type_environment: &TypeEnvironment,
    trait_environment: &TraitEnvironment,
    evidence_environment: &TraitEvidenceEnvironment,
) -> Option<BoundEvidenceSelection> {
    let exact = evidence_environment
        .builtin_for(type_id, trait_id)
        .or_else(|| evidence_environment.canonical_for(type_id, trait_id));
    if let Some(evidence_id) = exact {
        return Some(BoundEvidenceSelection::Registered(evidence_id));
    }

    if let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(type_id) {
        let base_type_id = type_environment.type_id_for_nominal_id(instance.base)?;
        if let Some(evidence_id) = evidence_environment
            .builtin_for(base_type_id, trait_id)
            .or_else(|| evidence_environment.canonical_for(base_type_id, trait_id))
        {
            return Some(BoundEvidenceSelection::Registered(evidence_id));
        }
    }

    builtin_cast_proves_core_trait(type_id, trait_id, type_environment, trait_environment)
        .then_some(BoundEvidenceSelection::BuiltinCastProof {
            source_type_id: type_id,
            trait_id,
        })
}

fn validate_type_recursive(
    type_id: TypeId,
    instance_name: Option<StringId>,
    span: Option<SourceSpan>,
    context: &GenericBoundEvidenceContext<'_>,
    visited: &mut FxHashSet<TypeId>,
) -> GenericBoundValidationResult<()> {
    if !visited.insert(type_id) {
        return Ok(());
    }

    match context.type_environment.get(type_id) {
        Some(TypeDefinition::GenericInstance(instance)) => {
            validate_instance_bounds(type_id, instance_name, span, context)?;

            for argument in &instance.arguments {
                validate_type_recursive(*argument, None, span, context, visited)?;
            }
        }

        Some(TypeDefinition::Constructed(definition)) => {
            for argument in &definition.arguments {
                validate_type_recursive(*argument, None, span, context, visited)?;
            }
        }

        Some(TypeDefinition::Function(definition)) => {
            for parameter in &definition.parameters {
                validate_type_recursive(parameter.type_id, None, span, context, visited)?;
            }

            for return_type in &definition.returns {
                validate_type_recursive(*return_type, None, span, context, visited)?;
            }

            if let Some(error_type) = definition.error_return {
                validate_type_recursive(error_type, None, span, context, visited)?;
            }
        }

        Some(
            TypeDefinition::Builtin(..)
            | TypeDefinition::Struct(..)
            | TypeDefinition::Choice(..)
            | TypeDefinition::External(..)
            | TypeDefinition::GenericParameter(..)
            | TypeDefinition::AnonymousConstRecordMarker,
        )
        | None => {}
    }

    Ok(())
}

fn validate_instance_bounds(
    instance_type_id: TypeId,
    instance_name: Option<StringId>,
    span: Option<SourceSpan>,
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
                instance_name,
                parameter.name,
                *concrete_type_id,
                *trait_id,
                span,
                context,
            )?;
        }
    }

    Ok(())
}

fn validate_single_bound(
    instance_name: Option<StringId>,
    parameter_name: StringId,
    concrete_type_id: TypeId,
    trait_id: TraitId,
    span: Option<SourceSpan>,
    context: &GenericBoundEvidenceContext<'_>,
) -> GenericBoundValidationResult<()> {
    let Some(trait_environment) = context.trait_environment else {
        return Ok(());
    };
    let Some(evidence_environment) = context.trait_evidence_environment else {
        return Ok(());
    };

    // A concrete generic-parameter argument that declares the bound satisfies it from the
    // parameter's own declaration, so it needs no evidence row.
    if generic_parameter_declares_bound(concrete_type_id, trait_id, context.type_environment) {
        return Ok(());
    }

    // WHY: names never gate reuse. A conformance reaches this requester only through evidence
    // its imports or declarations carried in, so the selected evidence row is the whole check.
    let has_reusable_evidence = evidence_for_type(
        concrete_type_id,
        trait_id,
        context.type_environment,
        trait_environment,
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

    Err(CompilerDiagnostic::invalid_generic_instantiation(
        instance_name,
        InvalidGenericInstantiationReason::MissingNominalTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        },
        span,
    ))
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
