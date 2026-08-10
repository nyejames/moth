//! Static generic-bound receiver method dispatch.
//!
//! WHAT: discovers generic parameter IDs that could carry trait bounds, looks up
//!       matching requirement candidates, resolves bound evidence, and reports
//!       ambiguity.
//! WHY: generic-bound dispatch is a separate semantic path from concrete source
//!      methods; isolating it keeps the bound-lookup complexity in one place.

use super::shared::{
    TraitSurfaceReceiverMethod, method_path_from_evidence, requirement_receiver_is_mutable,
    signature_from_trait_requirement,
};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::generic_bounds::evidence_for_type;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidReceiverCallReason};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitRequirement,
};
use crate::compiler_frontend::traits::evidence::TraitEvidenceDefinition;
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::FxHashSet;

struct GenericBoundRequirementCandidate<'a> {
    trait_definition: &'a ResolvedTraitDefinition,
    requirement: &'a ResolvedTraitRequirement,
}

pub(super) fn generic_parameter_id_for_receiver_type(
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Option<GenericParameterId> {
    match type_environment.get(receiver_type_id) {
        Some(TypeDefinition::GenericParameter(parameter)) => Some(parameter.id),
        _ => None,
    }
}

fn generic_parameter_ids_for_concrete_receiver(
    receiver_type_id: TypeId,
    scope_context: &ScopeContext,
) -> Vec<GenericParameterId> {
    let Some(active_context) = scope_context.active_generic_type_context() else {
        return Vec::new();
    };
    let Some(substitutions) = active_context.substitutions.as_ref() else {
        return Vec::new();
    };

    substitutions
        .iter()
        .filter_map(|(parameter_id, concrete_type_id)| {
            (*concrete_type_id == receiver_type_id).then_some(*parameter_id)
        })
        .collect()
}

fn generic_parameter_id_for_receiver_reference(
    receiver_node: &AstNode,
    scope_context: &ScopeContext,
) -> Option<GenericParameterId> {
    let active_context = scope_context.active_generic_type_context()?;

    let NodeKind::ExpressionStatement(expression) = &receiver_node.kind else {
        return None;
    };

    let ExpressionKind::Reference(reference_path) = &expression.kind else {
        return None;
    };

    active_context
        .source_parameter_by_rebased_path
        .get(reference_path)
        .copied()
}

fn generic_bound_requirement_candidates<'a>(
    parameter_ids: &[GenericParameterId],
    member_name: StringId,
    scope_context: &'a ScopeContext,
    type_environment: &'a TypeEnvironment,
) -> Vec<GenericBoundRequirementCandidate<'a>> {
    let mut candidates = Vec::new();
    let mut seen_traits = FxHashSet::default();

    for parameter_id in parameter_ids {
        let Some(bounds) = type_environment.trait_bounds_for_generic_parameter(*parameter_id)
        else {
            continue;
        };

        for trait_id in bounds {
            if !seen_traits.insert(*trait_id) || !scope_context.trait_id_is_visible(*trait_id) {
                continue;
            }

            let Some(trait_definition) = scope_context.trait_environment().get(*trait_id) else {
                continue;
            };

            for requirement in &trait_definition.requirements {
                if requirement.name == member_name {
                    candidates.push(GenericBoundRequirementCandidate {
                        trait_definition,
                        requirement,
                    });
                }
            }
        }
    }

    candidates
}

fn evidence_for_bound_method<'a>(
    trait_id: TraitId,
    receiver_type_id: TypeId,
    scope_context: &'a ScopeContext,
    type_environment: &TypeEnvironment,
) -> Result<Option<&'a TraitEvidenceDefinition>, ExpressionParseError> {
    let evidence_environment = scope_context.trait_evidence_environment();
    Ok(evidence_for_type(
        receiver_type_id,
        trait_id,
        type_environment,
        evidence_environment,
    )
    .and_then(|evidence_id| evidence_environment.get(evidence_id)))
}

pub(super) fn lookup_generic_bound_receiver_method(
    scope_context: &ScopeContext,
    receiver_node: &AstNode,
    receiver_type_id: TypeId,
    member_name: StringId,
    member_location: &SourceLocation,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
) -> Result<Option<TraitSurfaceReceiverMethod>, ExpressionParseError> {
    let mut parameter_ids = Vec::new();
    let receiver_is_unresolved_generic = if let Some(parameter_id) =
        generic_parameter_id_for_receiver_type(receiver_type_id, type_environment)
    {
        parameter_ids.push(parameter_id);
        true
    } else if let Some(parameter_id) =
        generic_parameter_id_for_receiver_reference(receiver_node, scope_context)
    {
        parameter_ids.push(parameter_id);
        false
    } else {
        parameter_ids.extend(generic_parameter_ids_for_concrete_receiver(
            receiver_type_id,
            scope_context,
        ));
        false
    };

    if parameter_ids.is_empty() {
        return Ok(None);
    }

    let candidates = generic_bound_requirement_candidates(
        &parameter_ids,
        member_name,
        scope_context,
        type_environment,
    );
    match candidates.as_slice() {
        [] => Ok(None),
        [_first, _second, ..] => Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::AmbiguousGenericBoundMethod,
            None,
            Some(member_name),
            None,
            None,
            member_location.clone(),
        )
        .into()),
        [candidate] => {
            let method_path = if receiver_is_unresolved_generic {
                // Validation-only generic bodies have no concrete evidence method yet. The
                // synthetic path is discarded with the validation AST nodes; concrete instances
                // reparse the same call and select the evidence method below.
                candidate
                    .trait_definition
                    .canonical_path
                    .append(candidate.requirement.name)
            } else {
                let Some(evidence) = evidence_for_bound_method(
                    candidate.trait_definition.id,
                    receiver_type_id,
                    scope_context,
                    type_environment,
                )?
                else {
                    return Ok(None);
                };

                let Some(method_path) = method_path_from_evidence(evidence, candidate.requirement)
                else {
                    return Ok(None);
                };
                method_path
            };

            let signature = signature_from_trait_requirement(
                &method_path,
                candidate.trait_definition,
                candidate.requirement,
                receiver_type_id,
                type_environment,
                string_table,
            );

            Ok(Some(TraitSurfaceReceiverMethod {
                method_path,
                signature,
                receiver_mutable: requirement_receiver_is_mutable(candidate.requirement),
            }))
        }
    }
}
