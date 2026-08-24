//! Multi-bind receiving-site support for value-producing control-flow blocks.
//!
//! WHAT: routes multi-bind `if`/match RHS forms through the shared header classifier, single-
//! predicate parser and block-body parser, then infers and coerces slot types.
//! WHY: known slots reuse the ordinary receiver. Partially inferred slots still need a
//! slot-inference owner, but they must not keep a second header or body grammar.

use super::expression_build::{build_value_if_expression, build_value_match_expression};
use super::receiver::try_parse_value_block_at_receiver_with_target;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    visit_reachable_then_values, visit_reachable_then_values_mut,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ParsedReceiverValue, ProducedValues, ValueBlock, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, InvalidReturnShapeReason,
    TypeMismatchContext,
};
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
/// Mixed slots parse a structural block; this owner infers and coerces, then wraps once.
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

    let parsed = try_parse_value_block_at_receiver_with_target(
        token_stream,
        context,
        type_interner,
        ActiveValueProductionTarget::mixed(known_slot_types, ValueReceiverKind::MultiBind),
        string_table,
    )?;

    Some(parsed.and_then(|parsed| match parsed {
        ParsedReceiverValue::Complete(expression) => Ok(expression),
        ParsedReceiverValue::NeedsSlotInference(block) => {
            finalize_inferred_value_block(block, known_slot_types, target_count, type_interner)
        }
    }))
}

fn finalize_inferred_value_block(
    block: ValueBlock,
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
) -> MultiBindValueResult<Expression> {
    match block {
        ValueBlock::If(mut value_if) => {
            let location = value_if.location.clone();
            let result_type_ids = infer_and_coerce_if_slots(
                &mut value_if.then_body,
                &mut value_if.else_body,
                known_slot_types,
                target_count,
                type_interner,
                &location,
            )?;
            value_if.result_type_ids = result_type_ids.clone();
            let result_type_id = intern_multi_bind_result_type(&result_type_ids, type_interner);
            Ok(build_value_if_expression(
                value_if,
                result_type_id,
                type_interner.environment(),
            ))
        }
        ValueBlock::Match(mut value_match) => {
            let location = value_match.location.clone();
            let result_type_ids = infer_and_coerce_match_slots(
                &mut value_match.arms,
                value_match.default.as_deref_mut(),
                known_slot_types,
                target_count,
                type_interner,
                &location,
            )?;
            value_match.result_type_ids = result_type_ids.clone();
            let result_type_id = intern_multi_bind_result_type(&result_type_ids, type_interner);
            Ok(build_value_match_expression(
                value_match,
                result_type_id,
                type_interner.environment(),
            ))
        }
        ValueBlock::Catch(_) => unreachable!(
            "value-if receivers do not parse catch handlers; mixed-slot catch is not a multi-bind path"
        ),
        ValueBlock::Scoped(_) => unreachable!(
            "static value-if specialisation runs after mixed-slot inference is complete"
        ),
    }
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
    let mut slot_types = known_slot_types.to_vec();
    let mut saw_producing_path = false;

    for arm in arms.iter() {
        accumulate_slots_from_body(
            &arm.body,
            known_slot_types,
            &mut slot_types,
            target_count,
            type_interner.environment(),
            &mut saw_producing_path,
        )?;
    }
    if let Some(default_body) = default.as_deref() {
        accumulate_slots_from_body(
            default_body,
            known_slot_types,
            &mut slot_types,
            target_count,
            type_interner.environment(),
            &mut saw_producing_path,
        )?;
    }

    let result_type_ids = finish_inferred_slots(slot_types, saw_producing_path, location)?;

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
    let mut slot_types = known_slot_types.to_vec();
    let mut saw_producing_path = false;

    for body in bodies {
        accumulate_slots_from_body(
            body,
            known_slot_types,
            &mut slot_types,
            target_count,
            type_environment,
            &mut saw_producing_path,
        )?;
    }

    finish_inferred_slots(slot_types, saw_producing_path, location)
}

fn accumulate_slots_from_body(
    body: &[AstNode],
    known_slot_types: &[Option<TypeId>],
    slot_types: &mut [Option<TypeId>],
    target_count: usize,
    type_environment: &TypeEnvironment,
    saw_producing_path: &mut bool,
) -> MultiBindValueResult<()> {
    visit_reachable_then_values(body, &mut |produced_values| {
        *saw_producing_path = true;
        accumulate_produced_group(
            produced_values,
            known_slot_types,
            slot_types,
            target_count,
            type_environment,
        )
    })
}

fn accumulate_produced_group(
    produced_values: &ProducedValues,
    known_slot_types: &[Option<TypeId>],
    slot_types: &mut [Option<TypeId>],
    target_count: usize,
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
    validate_optional_produced_arity(
        Some(&produced_values.expressions),
        target_count,
        &produced_values.location,
    )?;

    for (slot_index, expression) in produced_values.expressions.iter().enumerate() {
        if let Some(known_type) = known_slot_types[slot_index] {
            validate_expression_against_slot(expression, known_type, type_environment)?;
            slot_types[slot_index] = Some(known_type);
            continue;
        }

        if let Some(existing) = slot_types[slot_index] {
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
            slot_types[slot_index] = Some(expression.type_id);
        }
    }

    Ok(())
}

fn finish_inferred_slots(
    slot_types: Vec<Option<TypeId>>,
    saw_producing_path: bool,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    if !saw_producing_path {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            location.clone(),
        )
        .into());
    }

    slot_types
        .into_iter()
        .map(|slot_type| {
            slot_type.ok_or_else(|| {
                CompilerDiagnostic::invalid_control_flow_statement(
                    InvalidControlFlowStatementReason::ValueIfNoProducingPath,
                    location.clone(),
                )
                .into()
            })
        })
        .collect()
}

fn validate_expression_against_slot(
    expression: &Expression,
    expected_type: TypeId,
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
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
