//! Branch result-type inference and coercion for value-producing control flow.
//!
//! WHAT: unifies then/else branch types, coerces individual expressions, and infers
//! result types for block-form value-if and full value-match receivers.
//! WHY: receiver sites need contextual compatibility checks on canonical `TypeId`s;
//! `DataType` must not be used for semantic decisions once type IDs exist.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::completeness::visit_reachable_then_values;
use crate::compiler_frontend::ast::statements::value_production::types::ValueReceiverKind;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;

/// File-local result alias for value-producing receiver type inference.
///
/// Diagnosed failures remain plain `CompilerDiagnostic` values. The receiver
/// owner converts them at the same accumulation boundary as its other checks.
type ResultTypeResult<T> = Result<T, ExpressionParseError>;

/// Maps a receiver kind to the diagnostic context used when branch types mismatch.
pub(super) fn receiver_type_mismatch_context(kind: ValueReceiverKind) -> TypeMismatchContext {
    match kind {
        ValueReceiverKind::Return => TypeMismatchContext::ReturnValue,
        ValueReceiverKind::Declaration => TypeMismatchContext::Declaration,
        _ => TypeMismatchContext::Assignment,
    }
}

/// Unifies the types of two inline branch expressions.
///
/// WHAT: when the expected type is known, validates both branches are compatible
/// and returns it. When inferred, ensures both branches agree and returns the shared type.
pub(super) fn infer_inline_result_type(
    then_type: TypeId,
    else_type: TypeId,
    expected_type_id: Option<TypeId>,
    type_interner: &mut AstTypeInterner<'_>,
    span: Option<SourceSpan>,
    receiver_kind: ValueReceiverKind,
) -> ResultTypeResult<TypeId> {
    let context = receiver_type_mismatch_context(receiver_kind);

    if let Some(expected) = expected_type_id {
        let env = type_interner.environment();

        if !is_type_compatible(expected, then_type, env) {
            return Err(
                CompilerDiagnostic::type_mismatch(expected, then_type, context, span).into(),
            );
        }

        if !is_type_compatible(expected, else_type, env) {
            return Err(
                CompilerDiagnostic::type_mismatch(expected, else_type, context, span).into(),
            );
        }

        return Ok(expected);
    }

    if then_type != else_type {
        return Err(CompilerDiagnostic::type_mismatch(then_type, else_type, context, span).into());
    }

    Ok(then_type)
}

/// Infers the result type from block-form branch bodies.
///
/// Known receivers retain their ordered slots. Inferred single-value declarations inspect
/// every reachable producing path before reading its value type.
/// WHY: nested control flow can produce on several paths; the first `ThenValue` is not
/// enough to prove the inferred type.
pub(super) fn infer_block_if_result_type(
    then_body: &[AstNode],
    else_body: &[AstNode],
    expected_result_type_ids: &[TypeId],
    span: Option<SourceSpan>,
    receiver_kind: ValueReceiverKind,
    expression_builder: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> ResultTypeResult<Vec<TypeId>> {
    if !expected_result_type_ids.is_empty() {
        return Ok(expected_result_type_ids.to_vec());
    }

    unify_single_produced_types(
        [
            collect_reachable_single_produced_types(then_body, expression_builder, constants)?,
            collect_reachable_single_produced_types(else_body, expression_builder, constants)?,
        ]
        .concat(),
        span,
        receiver_kind,
    )
    .map(|single| vec![single])
}

/// Infers the result type for a full value-producing match.
///
/// Known receivers retain their slots. Inferred single receivers unify all producing arms.
pub(super) fn infer_value_match_result_type(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
    expected_result_type_ids: &[TypeId],
    span: Option<SourceSpan>,
    receiver_kind: ValueReceiverKind,
    expression_builder: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> ResultTypeResult<Vec<TypeId>> {
    if !expected_result_type_ids.is_empty() {
        return Ok(expected_result_type_ids.to_vec());
    }

    unify_single_produced_types(
        collect_value_match_single_produced_types(arms, default, expression_builder, constants)?,
        span,
        receiver_kind,
    )
    .map(|single| vec![single])
}

/// Collects the produced types from every reachable arm and optional default body.
fn collect_value_match_single_produced_types(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
    expression_builder: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> ResultTypeResult<Vec<(TypeId, Option<SourceSpan>)>> {
    let mut produced_types = Vec::new();

    for arm in arms {
        produced_types.extend(collect_reachable_single_produced_types(
            &arm.body,
            expression_builder,
            constants,
        )?);
    }

    if let Some(default_body) = default {
        produced_types.extend(collect_reachable_single_produced_types(
            default_body,
            expression_builder,
            constants,
        )?);
    }

    Ok(produced_types)
}

fn unify_single_produced_types(
    produced_types: Vec<(TypeId, Option<SourceSpan>)>,
    span: Option<SourceSpan>,
    receiver_kind: ValueReceiverKind,
) -> ResultTypeResult<TypeId> {
    let Some((first_type, _)) = produced_types.first().cloned() else {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            span,
        )
        .into());
    };

    let context = receiver_type_mismatch_context(receiver_kind);
    for (produced_type, produced_span) in produced_types.into_iter().skip(1) {
        if produced_type != first_type {
            return Err(CompilerDiagnostic::type_mismatch(
                first_type,
                produced_type,
                context,
                produced_span,
            )
            .into());
        }
    }

    Ok(first_type)
}

fn collect_reachable_single_produced_types(
    body: &[AstNode],
    expression_builder: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> ResultTypeResult<Vec<(TypeId, Option<SourceSpan>)>> {
    let mut produced_types = Vec::new();

    visit_reachable_then_values::<ExpressionParseError>(
        body,
        &mut |produced_values| {
            if let [expression] = produced_values.expressions.as_slice() {
                let type_id = expression_builder
                    .view(*expression)?
                    .require_single(ValueReceiverContext::Then)?;
                produced_types.push((type_id, expression_builder.view(*expression)?.span()));
            }
            Ok(())
        },
        expression_builder,
        constants,
    )?;

    Ok(produced_types)
}
