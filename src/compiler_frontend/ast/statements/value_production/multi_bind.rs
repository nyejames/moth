//! Multi-bind receiving-site support for value-producing control-flow blocks.
//!
//! WHAT: parses `if`/match value blocks used as the RHS of multi-bind statements, including
//! cases where one or more target slot types must be inferred from produced branch values.
//! WHY: multi-bind inference is specific to closed assignment/declaration receivers and would make
//! the ordinary declaration/assignment value-block parser harder to follow if left inline.

use super::expression_build::{
    build_value_if_expression, build_value_match_expression, then_value_node,
};
use super::parse_values::is_missing_produced_value_boundary;
use super::receiver::{
    BlockBodyParseInput, emit_collected_warnings, parse_value_block_bodies, same_logical_line,
    try_parse_value_block_at_receiver, validate_value_match_completeness,
};
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_until;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::branching::parse_match_block;
use crate::compiler_frontend::ast::statements::condition_validation::{
    ensure_if_statement_condition, if_condition_is_missing,
};
use crate::compiler_frontend::ast::statements::if_headers::{IfHeaderShape, classify_if_header};
use crate::compiler_frontend::ast::statements::match_headers::parse_scrutinee_until_is;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    validate_closed_branch_pair, visit_reachable_then_values, visit_reachable_then_values_mut,
};
use crate::compiler_frontend::ast::statements::value_production::parse_values::parse_fixed_arity_inferred_values;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueIfBlock, ValueMatchBlock, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, InvalidReturnShapeReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::is_declaration_compatible;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// Value-producing multi-bind bodies recurse into the AST body parser, so they preserve the
/// source-diagnostic and retained-data-infrastructure lanes for expression callers.
type MultiBindValueResult<T> = Result<T, ExpressionParseError>;

// ----------------------------
//  Multi-bind value blocks
// ----------------------------

/// Attempts to parse an `if`-headed value-producing block for multi-bind.
///
/// WHAT: when the current token is `if` and the receiver is a multi-bind site,
/// parses inline boolean `if`, block boolean `if`, or full-match forms, validates
/// arity, and returns a `ValueBlock` expression whose type is an internal tuple
/// with one slot per target.
/// WHY: multi-bind target inference means some slot types may not be known before
/// the RHS is parsed, so the standard `try_parse_value_block_at_receiver` (which
/// requires all expected types upfront) cannot handle every case.
pub fn try_parse_multi_bind_value_block(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    target_count: usize,
    known_slot_types: &[Option<TypeId>],
    string_table: &mut StringTable,
) -> Option<MultiBindValueResult<Expression>> {
    if token_stream.current_token_kind() != &TokenKind::If {
        return None;
    }

    if let Some(expected_types) = collect_known_slot_types(known_slot_types) {
        return try_parse_value_block_at_receiver(
            token_stream,
            context,
            type_interner,
            &expected_types,
            ValueReceiverKind::MultiBind,
            string_table,
        );
    }

    Some(parse_inferred_multi_bind_value_block(
        token_stream,
        context,
        type_interner,
        target_count,
        known_slot_types,
        string_table,
    ))
}

fn parse_inferred_multi_bind_value_block(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    target_count: usize,
    known_slot_types: &[Option<TypeId>],
    string_table: &mut StringTable,
) -> MultiBindValueResult<Expression> {
    let location = token_stream.current_location();
    token_stream.advance(); // consume `if`

    let classification = classify_if_header(token_stream);

    if classification.shape == IfHeaderShape::FullMatch {
        return parse_inferred_multi_bind_value_match(InferredMultiBindValueMatchInput {
            token_stream,
            context,
            type_interner,
            target_count,
            known_slot_types,
            string_table,
            location,
        });
    }

    if if_condition_is_missing(token_stream) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedConditionAfterIf,
            token_stream.current_location(),
        )
        .into());
    }

    let mut condition_type = ExpectedType::Infer;
    let condition_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &condition_context,
        type_interner,
        expected_type: &mut condition_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let condition = create_expression_until(input, &[TokenKind::Then, TokenKind::Colon])?;
    ensure_if_statement_condition(&condition, type_interner.environment())?;

    if token_stream.current_token_kind() == &TokenKind::Then {
        return parse_inferred_inline_multi_bind_value_if(InferredMultiBindValueIfInput {
            token_stream,
            context,
            type_interner,
            target_count,
            known_slot_types,
            string_table,
            condition,
            location,
        });
    }

    if token_stream.current_token_kind() == &TokenKind::Colon {
        return parse_inferred_block_multi_bind_value_if(InferredMultiBindValueIfInput {
            token_stream,
            context,
            type_interner,
            target_count,
            known_slot_types,
            string_table,
            condition,
            location,
        });
    }

    Err(CompilerDiagnostic::invalid_control_flow_statement(
        InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
        token_stream.current_location(),
    )
    .into())
}

fn collect_known_slot_types(known_slot_types: &[Option<TypeId>]) -> Option<Vec<TypeId>> {
    let mut expected_types = Vec::with_capacity(known_slot_types.len());

    for slot_type in known_slot_types {
        expected_types.push((*slot_type)?);
    }

    Some(expected_types)
}

struct InferredMultiBindValueIfInput<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    target_count: usize,
    known_slot_types: &'a [Option<TypeId>],
    string_table: &'a mut StringTable,
    condition: Expression,
    location: SourceLocation,
}

struct InferredMultiBindValueMatchInput<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    target_count: usize,
    known_slot_types: &'a [Option<TypeId>],
    string_table: &'a mut StringTable,
    location: SourceLocation,
}

fn parse_inferred_multi_bind_value_match(
    input: InferredMultiBindValueMatchInput<'_, '_>,
) -> MultiBindValueResult<Expression> {
    let InferredMultiBindValueMatchInput {
        token_stream,
        context,
        type_interner,
        target_count,
        known_slot_types,
        string_table,
        location,
    } = input;

    let scrutinee_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let scrutinee = parse_scrutinee_until_is(
        token_stream,
        &scrutinee_context,
        type_interner,
        string_table,
    )?;

    if token_stream.current_token_kind() != &TokenKind::Is {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            token_stream.current_location(),
        )
        .into());
    }
    token_stream.advance();

    let active_target = ActiveValueProductionTarget {
        result_type_ids: vec![],
        receiver_kind: ValueReceiverKind::MultiBind,
        expected_arity: Some(target_count),
    };
    let mut warnings = Vec::new();
    let mut parsed_match = parse_match_block(
        scrutinee,
        token_stream,
        context,
        type_interner,
        &mut warnings,
        Some(active_target),
        string_table,
    )?;
    emit_collected_warnings(context, warnings);

    validate_value_match_completeness(
        &parsed_match.arms,
        parsed_match.default.as_deref(),
        &location,
    )?;

    let produced_value_sets =
        collect_match_multi_produced_values(&parsed_match.arms, parsed_match.default.as_deref());
    if produced_value_sets.is_empty() {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            location.clone(),
        )
        .into());
    }

    for values in &produced_value_sets {
        validate_optional_produced_arity(Some(values), target_count, &location)?;
    }

    let result_type_ids = infer_multi_bind_match_result_slots(
        &produced_value_sets,
        known_slot_types,
        type_interner.environment(),
        &location,
    )?;

    for arm in &mut parsed_match.arms {
        coerce_produced_values_in_body(
            &mut arm.body,
            &result_type_ids,
            type_interner.environment(),
        )?;
    }
    if let Some(default_body) = &mut parsed_match.default {
        coerce_produced_values_in_body(
            default_body,
            &result_type_ids,
            type_interner.environment(),
        )?;
    }

    let result_type_id = intern_multi_bind_result_type(&result_type_ids, type_interner);
    let value_match = ValueMatchBlock {
        scrutinee: parsed_match.scrutinee,
        arms: parsed_match.arms,
        default: parsed_match.default,
        exhaustiveness: parsed_match.exhaustiveness,
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_match_expression(
        value_match,
        result_type_id,
        type_interner.environment(),
    ))
}

fn parse_inferred_inline_multi_bind_value_if(
    input: InferredMultiBindValueIfInput<'_, '_>,
) -> MultiBindValueResult<Expression> {
    let InferredMultiBindValueIfInput {
        token_stream,
        context,
        type_interner,
        target_count,
        known_slot_types,
        string_table,
        condition,
        location,
    } = input;

    let then_location = token_stream.current_location();
    token_stream.advance(); // consume `then`

    if token_stream.current_token_kind() == &TokenKind::Newline {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            token_stream.current_location(),
        )
        .into());
    }

    // A retained newline is a multiline form. Every other definite boundary means
    // the branch has no first value.
    if is_missing_produced_value_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterThen,
            token_stream.current_location(),
        )
        .into());
    }

    let then_values = parse_fixed_arity_inferred_values(
        token_stream,
        context,
        type_interner,
        target_count,
        string_table,
    )?;

    if token_stream.current_token_kind() != &TokenKind::Else {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfMissingElse,
            token_stream.current_location(),
        )
        .into());
    }
    if !same_logical_line(&then_location, &token_stream.current_location()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            token_stream.current_location(),
        )
        .into());
    }

    token_stream.advance(); // consume `else`

    if token_stream.current_token_kind() == &TokenKind::Then {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfElseThen,
            token_stream.current_location(),
        )
        .into());
    }
    if token_stream.current_token_kind() == &TokenKind::Newline {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            token_stream.current_location(),
        )
        .into());
    }

    if is_missing_produced_value_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterElse,
            token_stream.current_location(),
        )
        .into());
    }

    let else_values = parse_fixed_arity_inferred_values(
        token_stream,
        context,
        type_interner,
        target_count,
        string_table,
    )?;

    let result_type_ids = unify_and_validate_inferred_slots(
        &then_values,
        &else_values,
        known_slot_types,
        type_interner.environment(),
        &location,
    )?;

    let coerced_then =
        apply_coercion_to_values(then_values, &result_type_ids, type_interner.environment());
    let coerced_else =
        apply_coercion_to_values(else_values, &result_type_ids, type_interner.environment());

    let result_type_id = intern_multi_bind_result_type(&result_type_ids, type_interner);
    let value_if = ValueIfBlock {
        condition,
        then_body: vec![then_value_node(
            coerced_then,
            location.clone(),
            context.scope.clone(),
        )],
        else_body: vec![then_value_node(
            coerced_else,
            location.clone(),
            context.scope.clone(),
        )],
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_if_expression(
        value_if,
        result_type_id,
        type_interner.environment(),
    ))
}

fn parse_inferred_block_multi_bind_value_if(
    input: InferredMultiBindValueIfInput<'_, '_>,
) -> MultiBindValueResult<Expression> {
    let InferredMultiBindValueIfInput {
        token_stream,
        context,
        type_interner,
        target_count,
        known_slot_types,
        string_table,
        condition,
        location,
    } = input;

    let mut bodies = parse_value_block_bodies(BlockBodyParseInput {
        token_stream,
        outer_context: context,
        then_parent: context,
        else_parent: context,
        type_interner,
        string_table,
        active_target: ActiveValueProductionTarget {
            result_type_ids: vec![],
            receiver_kind: ValueReceiverKind::MultiBind,
            expected_arity: Some(target_count),
        },
    })?;

    validate_closed_branch_pair(bodies.then_exits, bodies.else_exits, &location)?;

    let mut produced_value_sets = collect_reachable_produced_groups(&bodies.then_body);
    produced_value_sets.extend(collect_reachable_produced_groups(&bodies.else_body));
    if produced_value_sets.is_empty() {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            location.clone(),
        )
        .into());
    }

    for values in &produced_value_sets {
        validate_optional_produced_arity(Some(values), target_count, &location)?;
    }

    let result_type_ids = infer_multi_bind_match_result_slots(
        &produced_value_sets,
        known_slot_types,
        type_interner.environment(),
        &location,
    )?;

    coerce_produced_values_in_body(
        &mut bodies.then_body,
        &result_type_ids,
        type_interner.environment(),
    )?;
    coerce_produced_values_in_body(
        &mut bodies.else_body,
        &result_type_ids,
        type_interner.environment(),
    )?;

    let result_type_id = intern_multi_bind_result_type(&result_type_ids, type_interner);
    let value_if = ValueIfBlock {
        condition,
        then_body: bodies.then_body,
        else_body: bodies.else_body,
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_if_expression(
        value_if,
        result_type_id,
        type_interner.environment(),
    ))
}

/// Derives slot types from branch expressions and validates them against known slots.
fn unify_and_validate_inferred_slots(
    then_values: &[Expression],
    else_values: &[Expression],
    known_slot_types: &[Option<TypeId>],
    type_environment: &TypeEnvironment,
    location: &SourceLocation,
) -> MultiBindValueResult<Vec<TypeId>> {
    let mut result_types = Vec::with_capacity(known_slot_types.len());

    for ((then_expr, else_expr), known_type) in then_values
        .iter()
        .zip(else_values.iter())
        .zip(known_slot_types.iter())
    {
        let slot_type = if let Some(known) = known_type {
            if then_expr.type_id != *known
                && !is_declaration_compatible(*known, then_expr.type_id, type_environment)
            {
                return Err(CompilerDiagnostic::type_mismatch(
                    *known,
                    then_expr.type_id,
                    TypeMismatchContext::Assignment,
                    then_expr.location.clone(),
                )
                .into());
            }
            if else_expr.type_id != *known
                && !is_declaration_compatible(*known, else_expr.type_id, type_environment)
            {
                return Err(CompilerDiagnostic::type_mismatch(
                    *known,
                    else_expr.type_id,
                    TypeMismatchContext::Assignment,
                    else_expr.location.clone(),
                )
                .into());
            }
            *known
        } else {
            if then_expr.type_id != else_expr.type_id {
                return Err(CompilerDiagnostic::type_mismatch(
                    then_expr.type_id,
                    else_expr.type_id,
                    TypeMismatchContext::Assignment,
                    location.clone(),
                )
                .into());
            }
            then_expr.type_id
        };
        result_types.push(slot_type);
    }

    Ok(result_types)
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

fn infer_multi_bind_match_result_slots(
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
            infer_unknown_match_slot_type(produced_value_sets, slot_index, location)?
        };

        result_types.push(slot_type);
    }

    Ok(result_types)
}

fn infer_unknown_match_slot_type(
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

/// Wraps expressions in `Coerced` nodes where the target type differs from the natural type.
fn apply_coercion_to_values(
    values: Vec<Expression>,
    target_types: &[TypeId],
    type_environment: &TypeEnvironment,
) -> Vec<Expression> {
    values
        .into_iter()
        .zip(target_types.iter())
        .map(|(expr, target_type)| {
            if expr.type_id != *target_type
                && is_declaration_compatible(*target_type, expr.type_id, type_environment)
            {
                return Expression::coerced(expr, *target_type);
            }
            expr
        })
        .collect()
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
