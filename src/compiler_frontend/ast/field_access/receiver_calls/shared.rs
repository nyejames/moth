//! Shared helpers for receiver-call dispatch submodules.
//!
//! WHAT: result-type construction, trait-requirement signature lowering, and small
//!       declaration-building utilities used by multiple dispatch paths.
//! WHY: call-argument resolution and result handling are structurally similar across
//!      source and generic-bound calls; extracting the
//!      common pieces prevents drift and keeps each dispatch file focused on its
//!      lookup logic.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    call_success_is_optional, non_fallible_handler_reason,
    token_stream_starts_fallible_handling_suffix,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitRequirement, TraitReceiverRequirement,
};
use crate::compiler_frontend::traits::evidence::TraitEvidenceDefinition;
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) struct TraitSurfaceReceiverMethod {
    pub(super) method_path: InternedPath,
    pub(super) signature: FunctionSignature,
    pub(super) receiver_mutable: bool,
}

fn fallible_receiver_result_type_ids(
    success_return_type_ids: Vec<TypeId>,
    error_return_type_id: TypeId,
    type_interner: &mut AstTypeInterner<'_>,
) -> Vec<TypeId> {
    let success_type_id = match success_return_type_ids.as_slice() {
        [] => type_interner.builtins().none,
        [single] => *single,
        multiple => type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(multiple.to_vec()),
    };

    vec![type_interner.intern_fallible_carrier(success_type_id, error_return_type_id)]
}

pub(super) fn receiver_result_type_ids_for_call(
    success_return_type_ids: Vec<TypeId>,
    error_return_type_id: Option<TypeId>,
    token_stream: &mut FileTokens,
    type_interner: &mut AstTypeInterner<'_>,
) -> Result<Vec<TypeId>, ExpressionParseError> {
    if let Some(error_return_type_id) = error_return_type_id {
        if !token_stream_starts_fallible_handling_suffix(token_stream) {
            return Err(CompilerDiagnostic::invalid_fallible_handling(
                InvalidFallibleHandlingReason::UnhandledErrorReturn,
                token_stream.current_location(),
            )
            .into());
        }

        return Ok(fallible_receiver_result_type_ids(
            success_return_type_ids,
            error_return_type_id,
            type_interner,
        ));
    }

    if matches!(
        token_stream.current_token_kind(),
        TokenKind::Bang | TokenKind::Catch
    ) {
        let operand_is_optional = call_success_is_optional(
            success_return_type_ids.as_slice(),
            type_interner.environment(),
        );
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            non_fallible_handler_reason(token_stream.current_token_kind(), operand_is_optional),
            token_stream.current_location(),
        )
        .into());
    }

    Ok(success_return_type_ids)
}

pub(super) fn replace_trait_this_type(
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

pub(super) fn requirement_receiver_is_mutable(requirement: &ResolvedTraitRequirement) -> bool {
    matches!(
        requirement.receiver,
        TraitReceiverRequirement::Mutable { .. }
    )
}

pub(super) fn method_path_from_evidence(
    evidence: &TraitEvidenceDefinition,
    requirement: &ResolvedTraitRequirement,
) -> Option<InternedPath> {
    evidence
        .requirements
        .iter()
        .find(|requirement_evidence| requirement_evidence.requirement_id == requirement.id)
        .map(|requirement_evidence| requirement_evidence.method_path.clone())
}

fn declaration_for_trait_bound_parameter(
    id: InternedPath,
    type_id: TypeId,
    diagnostic_type: DataType,
    value_mode: ValueMode,
    location: SourceLocation,
) -> Declaration {
    Declaration {
        id,
        value: Expression::new(
            ExpressionKind::NoValue,
            location,
            type_id,
            diagnostic_type,
            value_mode,
        ),
        config_qualifier: None,
    }
}

pub(super) fn signature_from_trait_requirement(
    method_path: &InternedPath,
    trait_definition: &ResolvedTraitDefinition,
    requirement: &ResolvedTraitRequirement,
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
) -> FunctionSignature {
    let receiver_mutable = requirement_receiver_is_mutable(requirement);
    let receiver_mode = if receiver_mutable {
        ValueMode::MutableReference
    } else {
        ValueMode::ImmutableReference
    };
    let mut parameters = Vec::with_capacity(requirement.parameters.len() + 1);
    let receiver_name = method_path.join_str("__trait_bound_receiver", string_table);
    parameters.push(declaration_for_trait_bound_parameter(
        receiver_name,
        receiver_type_id,
        diagnostic_type_spelling(receiver_type_id, type_environment),
        receiver_mode,
        requirement.location.clone(),
    ));

    for parameter in &requirement.parameters {
        let type_id = replace_trait_this_type(
            parameter.type_id,
            trait_definition.this_type,
            receiver_type_id,
        );
        parameters.push(declaration_for_trait_bound_parameter(
            parameter.name.clone(),
            type_id,
            diagnostic_type_spelling(type_id, type_environment),
            parameter.value_mode.clone(),
            parameter.location.clone(),
        ));
    }

    let returns = requirement
        .returns
        .iter()
        .map(|return_slot| {
            let type_id = replace_trait_this_type(
                return_slot.type_id,
                trait_definition.this_type,
                receiver_type_id,
            );

            ReturnSlot {
                value: diagnostic_type_spelling(type_id, type_environment),
                type_id: Some(type_id),
                reactive_template: None,
                channel: return_slot.channel,
            }
        })
        .collect();

    FunctionSignature {
        parameters,
        returns,
    }
}
