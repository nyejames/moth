//! Shared helpers for receiver-call dispatch submodules.
//!
//! WHAT: result-type construction, pending typed producer completion, trait-requirement signature
//!       lowering and declaration-building utilities shared by receiver dispatch paths.
//! WHY: explicit propagation stays at the postfix owner, while pending receiver calls expose success
//!      types until whole-expression catch completion chooses their error delivery.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, FallibleExpressionHandling,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    call_success_is_optional, non_fallible_handler_reason,
    token_stream_starts_typed_propagation_suffix,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitRequirement, TraitReceiverRequirement,
};
use crate::compiler_frontend::traits::evidence::TraitEvidenceDefinition;
use crate::compiler_frontend::value_mode::ValueMode;
pub(super) struct TraitSurfaceReceiverMethod {
    pub(super) method_path: PathId,
    pub(super) signature: FunctionSignature,
    pub(super) receiver_mutable: bool,
}

pub(super) fn receiver_result_type_ids_for_call(
    success_return_type_ids: Vec<TypeId>,
    token_stream: &mut AstCursor<'_>,
    type_interner: &mut AstTypeInterner<'_>,
) -> Result<Vec<TypeId>, ExpressionParseError> {
    if token_stream.current_tag() == TokenTag::BANG {
        let operand_is_optional = call_success_is_optional(
            success_return_type_ids.as_slice(),
            type_interner.environment(),
        );
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            non_fallible_handler_reason(token_stream.current_tag(), operand_is_optional),
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(success_return_type_ids)
}

/// Retains typed receiver producers until the whole selected expression completes.
///
/// The existing carrier stays inside the handled node for HIR's error-channel lowering. The outer
/// node exposes only success types, so receiver and argument typing never sees a first-class result.
pub(in crate::compiler_frontend::ast::field_access) fn finish_pending_receiver_call_expression(
    mut expression: Expression,
    token_stream: &AstCursor<'_>,
    type_interner: &mut AstTypeInterner<'_>,
) -> Expression {
    if token_stream_starts_typed_propagation_suffix(token_stream) {
        return expression;
    }

    let Some((success_type_id, error_type_id)) = type_interner
        .environment()
        .fallible_carrier_slots(expression.type_id)
    else {
        return expression;
    };

    let span = expression.span;
    let pending_facts = std::mem::take(&mut expression.failure_facts);
    let diagnostic_type =
        diagnostic_type_spelling(success_type_id, type_interner.environment());
    let mut expression = Expression::handled_result_with_type_id(
        expression,
        FallibleExpressionHandling::Recover,
        success_type_id,
        diagnostic_type,
        span,
    );
    expression.failure_facts = pending_facts;
    expression.with_typed_error_producer(error_type_id)
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
) -> Option<PathId> {
    evidence
        .requirements
        .iter()
        .find(|requirement_evidence| requirement_evidence.requirement_id == requirement.id)
        .map(|requirement_evidence| requirement_evidence.method_path)
}

fn declaration_for_trait_bound_parameter(
    id: PathId,
    type_id: TypeId,
    diagnostic_type: DataType,
    value_mode: ValueMode,
    span: Option<SourceSpan>,
    binding_span: Option<SourceSpan>,
) -> Declaration {
    Declaration {
        id,
        value: Expression::new(
            ExpressionKind::NoValue,
            span,
            type_id,
            diagnostic_type,
            value_mode,
        ),
        binding_span,
        config_qualifier: None,
    }
}

pub(super) fn signature_from_trait_requirement(
    method_path: &PathId,
    trait_definition: &ResolvedTraitDefinition,
    requirement: &ResolvedTraitRequirement,
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> FunctionSignature {
    let receiver_mutable = requirement_receiver_is_mutable(requirement);
    let receiver_mode = if receiver_mutable {
        ValueMode::MutableReference
    } else {
        ValueMode::ImmutableReference
    };
    let mut parameters = Vec::with_capacity(requirement.parameters.len() + 1);
    let receiver_name = path_fork
        .try_intern_child(*method_path, string_table.intern("__trait_bound_receiver"))
        .expect("path table exhausted while interning synthetic trait receiver");
    parameters.push(declaration_for_trait_bound_parameter(
        receiver_name,
        receiver_type_id,
        diagnostic_type_spelling(receiver_type_id, type_environment),
        receiver_mode,
        requirement.span,
        // Synthetic receiver has no authored binding token.
        None,
    ));

    for parameter in &requirement.parameters {
        let type_id = replace_trait_this_type(
            parameter.type_id,
            trait_definition.this_type,
            receiver_type_id,
        );
        parameters.push(declaration_for_trait_bound_parameter(
            parameter.name,
            type_id,
            diagnostic_type_spelling(type_id, type_environment),
            parameter.value_mode.clone(),
            parameter.span,
            parameter.span,
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
