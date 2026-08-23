//! Multi-bind receiving-site support for value-producing control-flow blocks.
//!
//! WHAT: routes multi-bind `if`/match RHS forms through the shared header classifier, single-
//! predicate parser and block-body parser, then infers and coerces slot types.
//! WHY: known slots reuse the ordinary receiver. Partially inferred slots still need a
//! slot-inference owner, but they must not keep a second header or body grammar.

use super::receiver::try_parse_value_block_at_receiver_with_target;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    visit_reachable_then_values, visit_reachable_then_values_mut,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueBlock, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, InvalidReturnShapeReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::type_coercion::compatibility::is_declaration_compatible;

/// Value-producing multi-bind bodies recurse into the AST body parser, so they preserve the
/// source-diagnostic and retained-data-infrastructure lanes for expression callers.
type MultiBindValueResult<T> = Result<T, ExpressionParseError>;

// ----------------------------
//  Multi-bind value blocks
// ----------------------------

/// Attempts to parse an `if`-headed value-producing block for multi-bind.
///
/// WHAT: routes every multi-bind value `if` through the shared receiver dispatcher.
/// When any slot is unknown, the shared parser keeps known-slot receiving context
/// and this owner then infers and coerces every producing path.
/// WHY: partially inferred multi-bind must not keep a second header or body grammar.
pub fn try_parse_multi_bind_value_block(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    target_count: usize,
    known_slot_types: &[Option<TypeId>],
    string_table: &mut StringTable,
) -> Option<MultiBindValueResult<Expression>> {
    debug_assert_eq!(known_slot_types.len(), target_count);

    let target = ActiveValueProductionTarget::mixed(known_slot_types, ValueReceiverKind::MultiBind);
    let needs_slot_inference = target.needs_slot_inference();
    let parsed = try_parse_value_block_at_receiver_with_target(
        token_stream,
        context,
        type_interner,
        target,
        string_table,
    )?;

    Some(parsed.and_then(|mut expression| {
        if needs_slot_inference {
            finalize_inferred_value_block(
                &mut expression,
                known_slot_types,
                target_count,
                type_interner,
            )?;
        }
        Ok(expression)
    }))
}

fn finalize_inferred_value_block(
    expression: &mut Expression,
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
) -> MultiBindValueResult<()> {
    let ExpressionKind::ValueBlock { block } = &mut expression.kind else {
        return Ok(());
    };

    let location = expression.location.clone();
    let result_type_ids = match block.as_mut() {
        ValueBlock::If(value_if) => infer_and_coerce_if_slots(
            &mut value_if.then_body,
            &mut value_if.else_body,
            known_slot_types,
            target_count,
            type_interner,
            &location,
        )?,
        ValueBlock::Match(value_match) => infer_and_coerce_match_slots(
            &mut value_match.arms,
            value_match.default.as_deref_mut(),
            known_slot_types,
            target_count,
            type_interner,
            &location,
        )?,
        ValueBlock::Catch(_) => return Ok(()),
    };

    match block.as_mut() {
        ValueBlock::If(value_if) => value_if.result_type_ids = result_type_ids.clone(),
        ValueBlock::Match(value_match) => value_match.result_type_ids = result_type_ids.clone(),
        ValueBlock::Catch(_) => {}
    }

    let result_type_id = intern_multi_bind_result_type(&result_type_ids, type_interner);
    expression.type_id = result_type_id;
    expression.diagnostic_type =
        diagnostic_type_spelling(result_type_id, type_interner.environment());
    Ok(())
}

fn infer_and_coerce_if_slots(
    then_body: &mut [AstNode],
    else_body: &mut [AstNode],
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    let result_type_ids = infer_slots_from_bodies(
        &[then_body, else_body],
        known_slot_types,
        target_count,
        type_interner.environment(),
        location,
    )?;
    coerce_produced_values_in_body(then_body, &result_type_ids, type_interner.environment())?;
    coerce_produced_values_in_body(else_body, &result_type_ids, type_interner.environment())?;
    Ok(result_type_ids)
}

fn infer_and_coerce_match_slots(
    arms: &mut [MatchArm],
    default: Option<&mut [AstNode]>,
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    let produced_value_sets = collect_match_multi_produced_values(arms, default.as_deref());
    let result_type_ids = infer_slots_from_produced_groups(
        &produced_value_sets,
        known_slot_types,
        target_count,
        type_interner.environment(),
        location,
    )?;

    for arm in arms {
        coerce_produced_values_in_body(
            &mut arm.body,
            &result_type_ids,
            type_interner.environment(),
        )?;
    }
    if let Some(default_body) = default {
        coerce_produced_values_in_body(
            default_body,
            &result_type_ids,
            type_interner.environment(),
        )?;
    }

    Ok(result_type_ids)
}

fn infer_slots_from_bodies(
    bodies: &[&[AstNode]],
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_environment: &TypeEnvironment,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    let mut produced_value_sets = Vec::new();

    for body in bodies {
        produced_value_sets.extend(collect_reachable_produced_groups(body));
    }

    infer_slots_from_produced_groups(
        &produced_value_sets,
        known_slot_types,
        target_count,
        type_environment,
        location,
    )
}

fn infer_slots_from_produced_groups(
    produced_value_sets: &[Vec<Expression>],
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_environment: &TypeEnvironment,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    if produced_value_sets.is_empty() {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            location.clone(),
        )
        .into());
    }

    for values in produced_value_sets {
        validate_optional_produced_arity(Some(values), target_count, location)?;
    }

    infer_multi_bind_result_slots(
        produced_value_sets,
        known_slot_types,
        type_environment,
        location,
    )
}

fn collect_match_multi_produced_values(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
) -> Vec<Vec<Expression>> {
    let mut produced_value_sets = Vec::new();

    for arm in arms {
        produced_value_sets.extend(collect_reachable_produced_groups(&arm.body));
    }

    if let Some(default_body) = default {
        produced_value_sets.extend(collect_reachable_produced_groups(default_body));
    }

    produced_value_sets
}

fn collect_reachable_produced_groups(body: &[AstNode]) -> Vec<Vec<Expression>> {
    let mut produced_value_sets = Vec::new();

    visit_reachable_then_values(body, &mut |produced_values| {
        produced_value_sets.push(produced_values.expressions.clone());
    });

    produced_value_sets
}

fn infer_multi_bind_result_slots(
    produced_value_sets: &[Vec<Expression>],
    known_slot_types: &[Option<TypeId>],
    type_environment: &TypeEnvironment,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    let mut result_types = Vec::with_capacity(known_slot_types.len());

    for (slot_index, known_type) in known_slot_types.iter().enumerate() {
        let slot_type = if let Some(known_type) = known_type {
            for values in produced_value_sets {
                validate_expression_against_slot(
                    values.get(slot_index),
                    *known_type,
                    type_environment,
                )?;
            }
            *known_type
        } else {
            infer_unknown_slot_type(produced_value_sets, slot_index, location)?
        };

        result_types.push(slot_type);
    }

    Ok(result_types)
}

fn infer_unknown_slot_type(
    produced_value_sets: &[Vec<Expression>],
    slot_index: usize,
    location: &SourceLocation,
) -> MultiBindValueResult<TypeId> {
    let mut inferred_type: Option<TypeId> = None;

    for values in produced_value_sets {
        let Some(expression) = values.get(slot_index) else {
            return Err(CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ValueIfNoProducingPath,
                location.clone(),
            )
            .into());
        };

        if let Some(existing) = inferred_type {
            if existing != expression.type_id {
                return Err(CompilerDiagnostic::type_mismatch(
                    existing,
                    expression.type_id,
                    TypeMismatchContext::Assignment,
                    expression.location.clone(),
                )
                .into());
            }
        } else {
            inferred_type = Some(expression.type_id);
        }
    }

    inferred_type.ok_or_else(|| {
        CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            location.clone(),
        )
        .into()
    })
}

fn validate_expression_against_slot(
    expression: Option<&Expression>,
    expected_type: TypeId,
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
    let Some(expression) = expression else {
        return Ok(());
    };

    if expression.type_id == expected_type
        || is_declaration_compatible(expected_type, expression.type_id, type_environment)
    {
        return Ok(());
    }

    Err(CompilerDiagnostic::type_mismatch(
        expected_type,
        expression.type_id,
        TypeMismatchContext::Assignment,
        expression.location.clone(),
    )
    .into())
}

fn validate_optional_produced_arity(
    values: Option<&[Expression]>,
    target_count: usize,
    location: &SourceLocation,
) -> MultiBindValueResult<()> {
    let Some(values) = values else {
        return Ok(());
    };

    if values.len() == target_count {
        return Ok(());
    }

    if values.len() > target_count {
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues {
                expected_count: target_count,
            },
            location.clone(),
        )
        .into());
    }

    Err(CompilerDiagnostic::invalid_return_shape(
        InvalidReturnShapeReason::TooFewReturnValues {
            expected_count: target_count,
            provided_count: values.len(),
        },
        location.clone(),
    )
    .into())
}

/// Mutates reachable `ThenValue` expressions in a body to apply coercion when needed.
fn coerce_produced_values_in_body(
    body: &mut [AstNode],
    expected_types: &[TypeId],
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
    visit_reachable_then_values_mut(body, &mut |produced_values| {
        if produced_values.expressions.len() != expected_types.len() {
            return validate_optional_produced_arity(
                Some(&produced_values.expressions),
                expected_types.len(),
                &produced_values.location,
            );
        }

        for (expr, expected_type) in produced_values
            .expressions
            .iter_mut()
            .zip(expected_types.iter())
        {
            if expr.type_id == *expected_type {
                continue;
            }

            if !is_declaration_compatible(*expected_type, expr.type_id, type_environment) {
                return Err(CompilerDiagnostic::type_mismatch(
                    *expected_type,
                    expr.type_id,
                    TypeMismatchContext::Assignment,
                    expr.location.clone(),
                )
                .into());
            }

            *expr = Expression::coerced(expr.clone(), *expected_type);
        }

        Ok(())
    })
}

fn intern_multi_bind_result_type(
    result_type_ids: &[TypeId],
    type_interner: &mut AstTypeInterner<'_>,
) -> TypeId {
    type_interner
        .environment_mut_for_derived_types()
        .intern_tuple(result_type_ids.to_vec())
}
