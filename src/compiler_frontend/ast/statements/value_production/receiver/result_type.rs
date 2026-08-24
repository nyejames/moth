//! Branch result-type inference and coercion for value-producing control flow.
//!
//! WHAT: unifies then/else branch types, coerces individual expressions, and infers
//! result types for block-form value-if and full value-match receivers.
//! WHY: receiver sites need contextual compatibility checks on canonical `TypeId`s;
//! `DataType` must not be used for semantic decisions once type IDs exist.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::completeness::visit_reachable_then_values;
use crate::compiler_frontend::ast::statements::value_production::types::ValueReceiverKind;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::type_coercion::compatibility::is_declaration_compatible;

/// File-local boxed diagnostic result alias.
///
/// WHAT: every result-type inference function in this module returns
/// `Result<T, Box<CompilerDiagnostic>>` through this alias.
/// WHY: `CompilerDiagnostic` is large enough to trigger `clippy::result_large_err`
/// when stored directly in a `Result` variant. Boxing the error at this owner boundary
/// keeps the `Result` envelope small without changing `DiagnosticBag`, `CompilerMessages`,
/// or any shared error type. The already-boxed callers in `block_if.rs`,
/// `full_match.rs`, and `inline_then_else.rs` consume these results directly without
/// unbox/rebox churn; `receiver/mod.rs` unboxes once at the plain accumulation boundary.
type ResultTypeResult<T> = Result<T, Box<CompilerDiagnostic>>;

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
    location: &SourceLocation,
    receiver_kind: ValueReceiverKind,
) -> ResultTypeResult<TypeId> {
    let context = receiver_type_mismatch_context(receiver_kind);

    if let Some(expected) = expected_type_id {
        let env = type_interner.environment();

        if !is_declaration_compatible(expected, then_type, env) {
            return Err(Box::new(CompilerDiagnostic::type_mismatch(
                expected,
                then_type,
                context,
                location.clone(),
            )));
        }

        if !is_declaration_compatible(expected, else_type, env) {
            return Err(Box::new(CompilerDiagnostic::type_mismatch(
                expected,
                else_type,
                context,
                location.clone(),
            )));
        }

        return Ok(expected);
    }

    if then_type != else_type {
        return Err(Box::new(CompilerDiagnostic::type_mismatch(
            then_type,
            else_type,
            context,
            location.clone(),
        )));
    }

    Ok(then_type)
}

/// Slot types stored on the finished value block for HIR result-local allocation.
///
/// WHAT: keeps explicit receiver slots when they exist and otherwise stores the
/// inferred single result type.
/// WHY: HIR allocates one local per `result_type_ids` entry and must not see an
/// empty vector for an inferred block.
pub(super) fn final_slot_type_ids(
    expected_result_type_ids: &[TypeId],
    inferred_expression_type: TypeId,
) -> Vec<TypeId> {
    if expected_result_type_ids.is_empty() {
        vec![inferred_expression_type]
    } else {
        expected_result_type_ids.to_vec()
    }
}

/// Infers the result type from block-form branch bodies.
///
/// WHAT: when the receiver expects known types, returns the corresponding expression
/// type (single type or internal tuple type for multi-value). For inferred single-value
/// declarations, inspects every reachable producing path.
/// WHY: nested control flow can produce on several paths; the first `ThenValue` is not
/// enough to prove the inferred type.
pub(super) fn infer_block_if_result_type(
    then_body: &[AstNode],
    else_body: &[AstNode],
    expected_result_type_ids: &[TypeId],
    type_interner: &mut AstTypeInterner<'_>,
    location: &SourceLocation,
    receiver_kind: ValueReceiverKind,
) -> ResultTypeResult<TypeId> {
    if expected_result_type_ids.len() > 1 {
        return Ok(type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(expected_result_type_ids.to_vec()));
    }

    if let Some(expected) = expected_result_type_ids.first().copied() {
        return Ok(expected);
    }

    unify_single_produced_types(
        [
            collect_reachable_single_produced_types(then_body),
            collect_reachable_single_produced_types(else_body),
        ]
        .concat(),
        location,
        receiver_kind,
    )
}

/// Infers the result type for a full value-producing match.
///
/// WHAT: for multi-value receivers, returns the interned tuple type.
/// For single-value inferred receivers, collects produced types from all arms
/// and default and ensures they agree.
pub(super) fn infer_value_match_result_type(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
    expected_result_type_ids: &[TypeId],
    type_interner: &mut AstTypeInterner<'_>,
    location: &SourceLocation,
    receiver_kind: ValueReceiverKind,
) -> ResultTypeResult<TypeId> {
    if expected_result_type_ids.len() > 1 {
        return Ok(type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(expected_result_type_ids.to_vec()));
    }

    if let Some(expected) = expected_result_type_ids.first().copied() {
        return Ok(expected);
    }

    unify_single_produced_types(
        collect_value_match_single_produced_types(arms, default),
        location,
        receiver_kind,
    )
}

/// Collects the produced types from every reachable arm and optional default body.
fn collect_value_match_single_produced_types(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
) -> Vec<(TypeId, SourceLocation)> {
    let mut produced_types = Vec::new();

    for arm in arms {
        produced_types.extend(collect_reachable_single_produced_types(&arm.body));
    }

    if let Some(default_body) = default {
        produced_types.extend(collect_reachable_single_produced_types(default_body));
    }

    produced_types
}

fn unify_single_produced_types(
    produced_types: Vec<(TypeId, SourceLocation)>,
    location: &SourceLocation,
    receiver_kind: ValueReceiverKind,
) -> ResultTypeResult<TypeId> {
    let Some((first_type, _)) = produced_types.first().cloned() else {
        return Err(Box::new(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ValueIfNoProducingPath,
                location.clone(),
            ),
        ));
    };

    let context = receiver_type_mismatch_context(receiver_kind);
    for (produced_type, produced_location) in produced_types.into_iter().skip(1) {
        if produced_type != first_type {
            return Err(Box::new(CompilerDiagnostic::type_mismatch(
                first_type,
                produced_type,
                context,
                produced_location,
            )));
        }
    }

    Ok(first_type)
}

fn collect_reachable_single_produced_types(body: &[AstNode]) -> Vec<(TypeId, SourceLocation)> {
    let mut produced_types = Vec::new();

    visit_reachable_then_values::<()>(body, &mut |produced_values| {
        if produced_values.expressions.len() == 1 {
            produced_types.push((
                produced_values.expressions[0].type_id,
                produced_values.expressions[0].location.clone(),
            ));
        }
        Ok(())
    })
    .expect("single-produced-type collection is infallible");

    produced_types
}
