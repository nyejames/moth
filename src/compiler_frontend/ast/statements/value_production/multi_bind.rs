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
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    visit_authored_then_values_mut, visit_reachable_then_values,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ParsedReceiverValue, ProducedValues, ValueBlock, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data, choices_control_flow, types_borrows_targets,
};
use crate::compiler_frontend::compiler_messages::{
    InvalidControlFlowStatementReason, InvalidReturnShapeReason, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;
use crate::compiler_frontend::value_mode::ValueMode;

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
#[allow(
    clippy::too_many_arguments,
    reason = "multi-bind parsing keeps the cursor, scope, slot shape and mutable state as separate inputs"
)]
pub fn try_parse_multi_bind_value_block(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    target_count: usize,
    known_slot_types: &[Option<TypeId>],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Option<MultiBindValueResult<OwnedExpr>> {
    debug_assert_eq!(known_slot_types.len(), target_count);

    let parsed = try_parse_value_block_at_receiver_with_target(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        ActiveValueProductionTarget::mixed(known_slot_types, ValueReceiverKind::MultiBind),
        string_table,
        path_fork,
    )?;

    Some(parsed.and_then(|parsed| match parsed {
        ParsedReceiverValue::Complete(expression) => Ok(expression),
        ParsedReceiverValue::NeedsSlotInference { block, span } => finalize_inferred_value_block(
            block,
            span,
            known_slot_types,
            target_count,
            type_interner,
            expression_builder,
            const_values,
        ),
        // A branch body still holds numeric continuations, so the block cannot be finished here.
        // The block is this right-hand position's whole authored syntax, so it travels to the
        // enclosing body root through the shared retained splice, which rebuilds it and runs the
        // slot inference above exactly once.
        ParsedReceiverValue::Retained(retained) => {
            Err(retained.into_deferral(&ValueMode::ImmutableOwned))
        }
    }))
}

/// Complete one inferred mixed-slot value block into its multi-bind result expression.
///
/// WHAT: infers every unknown slot from the block's producing bodies, coerces each produced value
///       against the final slot list, and builds the value-block expression from those slots.
/// WHY `pub(crate)`: this is the single semantic finalizer for an inferred or mixed value block.
/// The retained mixed-slot resume in this module's subtree and the pending-target assignment
/// value-if lane both need it, so it stays reachable across the crate instead of being duplicated
/// or wrapped by a forwarding helper.
pub(crate) fn finalize_inferred_value_block(
    block: ValueBlock,
    span: Option<SourceSpan>,
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> MultiBindValueResult<OwnedExpr> {
    match block {
        ValueBlock::If(mut value_if) => {
            let result_type_ids = infer_and_coerce_if_slots(
                &mut value_if.then_body,
                &mut value_if.else_body,
                known_slot_types,
                target_count,
                type_interner,
                expression_builder,
                const_values,
                value_if.span,
            )?;
            value_if.result_type_ids = result_type_ids;
            Ok(build_value_if_expression(
                value_if,
                span,
                expression_builder,
            )?)
        }
        ValueBlock::Match(mut value_match) => {
            let result_type_ids = infer_and_coerce_match_slots(
                &mut value_match.arms,
                value_match.default.as_deref_mut(),
                known_slot_types,
                target_count,
                type_interner,
                expression_builder,
                const_values,
                value_match.span,
            )?;
            value_match.result_type_ids = result_type_ids;
            Ok(build_value_match_expression(
                value_match,
                span,
                expression_builder,
            )?)
        }
        ValueBlock::Catch(_) => crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "value-if receivers do not parse catch handlers; mixed-slot catch is not a multi-bind path"
        ),
        ValueBlock::LexicalScope(_) => crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "static value-if specialisation runs after mixed-slot inference is complete"
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "if slot inference keeps both branch bodies, slot shape and mutable state as separate inputs"
)]
fn infer_and_coerce_if_slots(
    then_body: &mut [AstNode],
    else_body: &mut [AstNode],
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    span: Option<SourceSpan>,
) -> MultiBindValueResult<Vec<TypeId>> {
    let result_type_ids = infer_slots_from_bodies(
        &[then_body, else_body],
        known_slot_types,
        target_count,
        expression_builder,
        const_values,
        type_interner.environment(),
        span,
    )?;
    coerce_produced_values_in_body(
        then_body,
        &result_type_ids,
        expression_builder,
        type_interner.environment(),
    )?;
    coerce_produced_values_in_body(
        else_body,
        &result_type_ids,
        expression_builder,
        type_interner.environment(),
    )?;
    Ok(result_type_ids)
}

#[allow(
    clippy::too_many_arguments,
    reason = "match slot inference keeps the arms, default, slot shape and mutable state as separate inputs"
)]
fn infer_and_coerce_match_slots(
    arms: &mut [MatchArm],
    default: Option<&mut [AstNode]>,
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    span: Option<SourceSpan>,
) -> MultiBindValueResult<Vec<TypeId>> {
    let mut slot_types = known_slot_types.to_vec();
    let mut saw_producing_path = false;

    for arm in arms.iter() {
        accumulate_slots_from_body(
            &arm.body,
            known_slot_types,
            &mut slot_types,
            target_count,
            expression_builder,
            const_values,
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
            expression_builder,
            const_values,
            type_interner.environment(),
            &mut saw_producing_path,
        )?;
    }

    let result_type_ids = finish_inferred_slots(slot_types, saw_producing_path, span)?;

    for arm in arms {
        coerce_produced_values_in_body(
            &mut arm.body,
            &result_type_ids,
            expression_builder,
            type_interner.environment(),
        )?;
    }
    if let Some(default_body) = default {
        coerce_produced_values_in_body(
            default_body,
            &result_type_ids,
            expression_builder,
            type_interner.environment(),
        )?;
    }

    Ok(result_type_ids)
}

fn infer_slots_from_bodies(
    bodies: &[&[AstNode]],
    known_slot_types: &[Option<TypeId>],
    target_count: usize,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    type_environment: &TypeEnvironment,
    span: Option<SourceSpan>,
) -> MultiBindValueResult<Vec<TypeId>> {
    let mut slot_types = known_slot_types.to_vec();
    let mut saw_producing_path = false;

    for body in bodies {
        accumulate_slots_from_body(
            body,
            known_slot_types,
            &mut slot_types,
            target_count,
            expression_builder,
            const_values,
            type_environment,
            &mut saw_producing_path,
        )?;
    }

    finish_inferred_slots(slot_types, saw_producing_path, span)
}

#[allow(
    clippy::too_many_arguments,
    reason = "slot accumulation keeps the body, slot shape, outputs and read-only state separate"
)]
fn accumulate_slots_from_body(
    body: &[AstNode],
    known_slot_types: &[Option<TypeId>],
    slot_types: &mut [Option<TypeId>],
    target_count: usize,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    type_environment: &TypeEnvironment,
    saw_producing_path: &mut bool,
) -> MultiBindValueResult<()> {
    visit_reachable_then_values(
        body,
        &mut |produced_values| {
            *saw_producing_path = true;
            accumulate_produced_group(
                produced_values,
                known_slot_types,
                slot_types,
                target_count,
                expression_builder,
                type_environment,
            )
        },
        expression_builder,
        const_values,
    )
}

fn accumulate_produced_group(
    produced_values: &ProducedValues,
    known_slot_types: &[Option<TypeId>],
    slot_types: &mut [Option<TypeId>],
    target_count: usize,
    expression_builder: &AstExpressionBuilder,
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
    validate_optional_produced_arity(
        Some(&produced_values.expressions),
        target_count,
        produced_values.span,
    )?;

    for (slot_index, expression) in produced_values.expressions.iter().enumerate() {
        let produced_type = expression_builder
            .view(*expression)?
            .require_single(ValueReceiverContext::Then)?;
        if let Some(known_type) = known_slot_types[slot_index] {
            validate_expression_against_slot(
                expression,
                known_type,
                expression_builder,
                type_environment,
            )?;
            slot_types[slot_index] = Some(known_type);
            continue;
        }

        if let Some(existing) = slot_types[slot_index] {
            if existing != produced_type {
                return Err(types_borrows_targets::type_mismatch(
                    existing,
                    produced_type,
                    TypeMismatchContext::Assignment,
                    expression_builder.view(*expression)?.span(),
                )
                .into());
            }
        } else {
            slot_types[slot_index] = Some(produced_type);
        }
    }

    Ok(())
}

fn finish_inferred_slots(
    slot_types: Vec<Option<TypeId>>,
    saw_producing_path: bool,
    span: Option<SourceSpan>,
) -> MultiBindValueResult<Vec<TypeId>> {
    if !saw_producing_path {
        return Err(choices_control_flow::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfNoProducingPath,
            span,
        )
        .into());
    }

    slot_types
        .into_iter()
        .map(|slot_type| {
            slot_type.ok_or_else(|| {
                choices_control_flow::invalid_control_flow_statement(
                    InvalidControlFlowStatementReason::ValueIfNoProducingPath,
                    span,
                )
                .into()
            })
        })
        .collect()
}

fn validate_expression_against_slot(
    expression: &ExprId,
    expected_type: TypeId,
    expression_builder: &AstExpressionBuilder,
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
    let produced_type = expression_builder
        .view(*expression)?
        .require_single(ValueReceiverContext::Then)?;
    if produced_type == expected_type
        || is_type_compatible(expected_type, produced_type, type_environment)
    {
        return Ok(());
    }

    Err(types_borrows_targets::type_mismatch(
        expected_type,
        produced_type,
        TypeMismatchContext::Assignment,
        expression_builder.view(*expression)?.span(),
    )
    .into())
}

fn validate_optional_produced_arity(
    values: Option<&[ExprId]>,
    target_count: usize,
    span: Option<SourceSpan>,
) -> MultiBindValueResult<()> {
    let Some(values) = values else {
        return Ok(());
    };

    if values.len() == target_count {
        return Ok(());
    }

    if values.len() > target_count {
        return Err(calls_data::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues {
                expected_count: target_count,
            },
            span,
        )
        .into());
    }

    Err(calls_data::invalid_return_shape(
        InvalidReturnShapeReason::TooFewReturnValues {
            expected_count: target_count,
            provided_count: values.len(),
        },
        span,
    )
    .into())
}

/// Mutates every authored `ThenValue` group in a body to apply coercion, including dead groups.
fn coerce_produced_values_in_body(
    body: &mut [AstNode],
    expected_types: &[TypeId],
    expression_builder: &mut AstExpressionBuilder,
    type_environment: &TypeEnvironment,
) -> MultiBindValueResult<()> {
    visit_authored_then_values_mut(body, &mut |produced_values| {
        if produced_values.expressions.len() != expected_types.len() {
            return validate_optional_produced_arity(
                Some(&produced_values.expressions),
                expected_types.len(),
                produced_values.span,
            );
        }

        for (expr, expected_type) in produced_values
            .expressions
            .iter_mut()
            .zip(expected_types.iter())
        {
            let produced_type = expression_builder
                .view(*expr)?
                .require_single(ValueReceiverContext::Then)?;
            if produced_type == *expected_type {
                continue;
            }

            if !is_type_compatible(*expected_type, produced_type, type_environment) {
                return Err(types_borrows_targets::type_mismatch(
                    *expected_type,
                    produced_type,
                    TypeMismatchContext::Assignment,
                    expression_builder.view(*expr)?.span(),
                )
                .into());
            }

            *expr = expression_builder
                .coerced(expression_builder.own(*expr), *expected_type)?
                .id();
        }

        Ok(())
    })
}
