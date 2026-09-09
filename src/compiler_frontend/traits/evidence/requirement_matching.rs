//! Trait requirement signature compatibility and method lookup.
//!
//! WHAT: Matches trait requirement signatures against actual same-file receiver methods,
//!       performing parameter and return count/mode/type validation, mutability checks,
//!       and direct `This` type substitution.
//! WHY: Assures that a type conformant to a trait actually implements all trait requirements
//!      correctly at the binary/type level.

use super::diagnostics::{invalid_conformance, requirement_and_method_labels, requirement_label};
use super::environment::TraitRequirementEvidence;
use super::target_resolution::ConformanceTarget;
use crate::compiler_frontend::ast::statements::functions::ReturnSlot;
use crate::compiler_frontend::ast::{ReceiverMethodCatalog, ReceiverMethodEntry};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTraitConformanceReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitRequirement, TraitReceiverRequirement,
};

/// Result for the connected trait-requirement matching family.
///
/// Requirement lookup and signature validation keep diagnosed failures as plain
/// `CompilerDiagnostic` values through the evidence-validation boundary.
type RequirementValidationResult<T = ()> = Result<T, CompilerDiagnostic>;

pub(super) struct ImplementationMethod<'a> {
    pub(super) entry: &'a ReceiverMethodEntry,
    pub(super) receiver_type_id: TypeId,
}

pub(super) struct RequirementValidationContext<'a, 'strings> {
    pub(super) receiver_methods: &'a ReceiverMethodCatalog,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) target_name: StringId,
    pub(super) trait_name: StringId,
    pub(super) conformance_span: Option<SourceSpan>,
    pub(super) string_table: &'strings mut StringTable,
}

pub(super) fn validate_requirements(
    trait_definition: &ResolvedTraitDefinition,
    target: &ConformanceTarget,
    conformance_source_file: &InternedPath,
    context: &mut RequirementValidationContext<'_, '_>,
) -> RequirementValidationResult<Vec<TraitRequirementEvidence>> {
    let mut requirement_methods = Vec::with_capacity(trait_definition.requirements.len());

    for requirement in &trait_definition.requirements {
        let method = find_same_file_method(
            context.receiver_methods,
            target,
            requirement.name,
            conformance_source_file,
            context.type_environment,
        )
        .ok_or_else(|| {
            invalid_conformance(
                context.target_name,
                Some(context.trait_name),
                InvalidTraitConformanceReason::MissingMethod {
                    requirement_name: requirement.name,
                },
                context.conformance_span,
                requirement_label(requirement, context.string_table),
            )
        })?;

        validate_requirement_signature(requirement, trait_definition.this_type, &method, context)?;

        requirement_methods.push(TraitRequirementEvidence {
            requirement_id: requirement.id,
            method_path: method.entry.function_path.clone(),
        });
    }

    Ok(requirement_methods)
}

fn find_same_file_method<'a>(
    receiver_methods: &'a ReceiverMethodCatalog,
    target: &ConformanceTarget,
    method_name: StringId,
    conformance_source_file: &InternedPath,
    type_environment: &TypeEnvironment,
) -> Option<ImplementationMethod<'a>> {
    let entries = receiver_methods
        .by_receiver_and_name
        .get(&(target.receiver_key.clone(), method_name))?;

    for entry in entries {
        if entry.source_file != *conformance_source_file {
            continue;
        }

        let Some(receiver_parameter) = entry.signature.parameters.first() else {
            continue;
        };
        let receiver_type_id = receiver_parameter.value.type_id;
        if receiver_type_matches_target(receiver_type_id, target, type_environment) {
            return Some(ImplementationMethod {
                entry,
                receiver_type_id,
            });
        }
    }

    None
}

fn receiver_type_matches_target(
    receiver_type_id: TypeId,
    target: &ConformanceTarget,
    type_environment: &TypeEnvironment,
) -> bool {
    if receiver_type_id == target.type_id {
        return true;
    }

    if !target.is_generic_constructor {
        return false;
    }

    let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(receiver_type_id)
    else {
        return false;
    };
    type_environment
        .nominal_path_by_id(instance.base)
        .is_some_and(|base_path| target.path.as_ref().is_some_and(|path| base_path == path))
}

fn validate_requirement_signature(
    requirement: &ResolvedTraitRequirement,
    trait_this_type: TypeId,
    method: &ImplementationMethod<'_>,
    context: &mut RequirementValidationContext<'_, '_>,
) -> RequirementValidationResult {
    let required_receiver_mutable = match requirement.receiver {
        TraitReceiverRequirement::Immutable { .. } => false,
        TraitReceiverRequirement::Mutable { .. } => true,
    };

    if required_receiver_mutable != method.entry.receiver_mutable {
        return Err(invalid_conformance(
            context.target_name,
            Some(context.trait_name),
            InvalidTraitConformanceReason::ReceiverMutabilityMismatch {
                requirement_name: requirement.name,
            },
            context.conformance_span,
            requirement_and_method_labels(requirement, method.entry, context.string_table),
        )
        .into());
    }

    validate_parameters(requirement, trait_this_type, method, context)?;

    validate_returns(requirement, trait_this_type, method, context)
}

fn validate_parameters(
    requirement: &ResolvedTraitRequirement,
    trait_this_type: TypeId,
    method: &ImplementationMethod<'_>,
    context: &mut RequirementValidationContext<'_, '_>,
) -> RequirementValidationResult {
    let method_parameters = method
        .entry
        .signature
        .parameters
        .iter()
        .skip(1)
        .collect::<Vec<_>>();
    if requirement.parameters.len() != method_parameters.len() {
        return Err(invalid_conformance(
            context.target_name,
            Some(context.trait_name),
            InvalidTraitConformanceReason::ParameterCountMismatch {
                requirement_name: requirement.name,
                expected: requirement.parameters.len(),
                found: method_parameters.len(),
            },
            context.conformance_span,
            requirement_and_method_labels(requirement, method.entry, context.string_table),
        )
        .into());
    }

    for (index, (required, actual)) in requirement
        .parameters
        .iter()
        .zip(method_parameters.iter())
        .enumerate()
    {
        if required.value_mode != actual.value.value_mode {
            return Err(invalid_conformance(
                context.target_name,
                Some(context.trait_name),
                InvalidTraitConformanceReason::ParameterModeMismatch {
                    requirement_name: requirement.name,
                    parameter_index: index + 1,
                },
                context.conformance_span,
                requirement_and_method_labels(requirement, method.entry, context.string_table),
            )
            .into());
        }

        let expected_type =
            replace_trait_this(required.type_id, trait_this_type, method.receiver_type_id);
        if expected_type != actual.value.type_id {
            return Err(invalid_conformance(
                context.target_name,
                Some(context.trait_name),
                InvalidTraitConformanceReason::ParameterTypeMismatch {
                    requirement_name: requirement.name,
                    parameter_index: index + 1,
                    expected_type,
                    found_type: actual.value.type_id,
                },
                context.conformance_span,
                requirement_and_method_labels(requirement, method.entry, context.string_table),
            )
            .into());
        }
    }

    Ok(())
}

fn validate_returns(
    requirement: &ResolvedTraitRequirement,
    trait_this_type: TypeId,
    method: &ImplementationMethod<'_>,
    context: &mut RequirementValidationContext<'_, '_>,
) -> RequirementValidationResult {
    let method_returns = &method.entry.signature.returns;
    if requirement.returns.len() != method_returns.len() {
        return Err(invalid_conformance(
            context.target_name,
            Some(context.trait_name),
            InvalidTraitConformanceReason::ReturnCountMismatch {
                requirement_name: requirement.name,
                expected: requirement.returns.len(),
                found: method_returns.len(),
            },
            context.conformance_span,
            requirement_and_method_labels(requirement, method.entry, context.string_table),
        )
        .into());
    }

    for (index, (required, actual)) in requirement.returns.iter().zip(method_returns).enumerate() {
        if required.channel != actual.channel {
            return Err(invalid_conformance(
                context.target_name,
                Some(context.trait_name),
                InvalidTraitConformanceReason::ReturnChannelMismatch {
                    requirement_name: requirement.name,
                    return_index: index + 1,
                },
                context.conformance_span,
                requirement_and_method_labels(requirement, method.entry, context.string_table),
            )
            .into());
        }

        let Some(actual_type) = return_type_id(actual) else {
            return Err(invalid_conformance(
                context.target_name,
                Some(context.trait_name),
                InvalidTraitConformanceReason::ReturnTypeMismatch {
                    requirement_name: requirement.name,
                    return_index: index + 1,
                    expected_type: replace_trait_this(
                        required.type_id,
                        trait_this_type,
                        method.receiver_type_id,
                    ),
                    found_type: method.receiver_type_id,
                },
                context.conformance_span,
                requirement_and_method_labels(requirement, method.entry, context.string_table),
            )
            .into());
        };

        let expected_type =
            replace_trait_this(required.type_id, trait_this_type, method.receiver_type_id);
        if expected_type != actual_type {
            return Err(invalid_conformance(
                context.target_name,
                Some(context.trait_name),
                InvalidTraitConformanceReason::ReturnTypeMismatch {
                    requirement_name: requirement.name,
                    return_index: index + 1,
                    expected_type,
                    found_type: actual_type,
                },
                context.conformance_span,
                requirement_and_method_labels(requirement, method.entry, context.string_table),
            )
            .into());
        }
    }

    Ok(())
}

fn return_type_id(return_slot: &ReturnSlot) -> Option<TypeId> {
    return_slot.type_id
}

fn replace_trait_this(
    type_id: TypeId,
    trait_this_type: TypeId,
    receiver_type_id: TypeId,
) -> TypeId {
    if type_id == trait_this_type {
        receiver_type_id
    } else {
        type_id
    }
}
