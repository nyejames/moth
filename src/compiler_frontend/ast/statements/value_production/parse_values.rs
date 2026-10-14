//! Typed value-production parsing helpers.
//!
//! WHAT: parses one or more expressions after `then`, validating arity and applying
//! contextual coercion against the active value-production target.
//! WHY: this logic was previously catch-specific; generalising it lets value `if`,
//! match, and catch share one arity/coercion path.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources, ExpressionTrailingPolicy,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredExpressionCompletion, DeferredExpressionServices, StatementExpression,
    deferred_numeric_expression,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ProducedValues, ThenValueContinuation, ThenValueSlot,
    ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason, InvalidReturnShapeReason,
    TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Input bundle for `parse_retained_produced_values`.
///
/// WHAT: avoids a long parameter list by grouping everything the parser needs.
/// WHY: the caller already has all of these values on hand; a struct keeps call sites
/// readable.
pub struct ProducedValuesParseInput<'a, 'b, 'tokens> {
    pub token_stream: &'a mut AstCursor<'tokens>,
    pub context: &'a ScopeContext,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub expression_builder: &'a mut AstExpressionBuilder,
    pub const_values: &'a mut ConstValueStore,
    pub target: &'a ActiveValueProductionTarget,
    pub string_table: &'a mut StringTable,
    pub path_fork: &'a mut PathInternerFork,
}

/// Returns whether the current token proves that no produced value was authored.
pub(crate) fn is_missing_produced_value_boundary(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::ELSE
            | TokenTag::EOF
            | TokenTag::END
            | TokenTag::COMMA
            | TokenTag::CLOSE_PARENTHESIS
            | TokenTag::CLOSE_CURLY
    )
}

/// Parses a fixed number of `then` fallback values for their authored receiving contract.
///
/// WHAT: reads exactly the target's value count in expressions and returns them in slot order; a
///       numeric-dependent value is retained as its authored fragment instead of materialising at
///       the language default, and a fully built list is coerced at every slot whose receiver type
///       is already known.
/// WHY: every `then` position owns its numeric deferral, so a numeric-dependent fallback travels
///      to the body root instead of escaping as a nested non-owning failure; a known slot keeps
///      the ordinary receiver expectation and coercion, while an inferred slot stays unresolved
///      for the site (or the owning block's result-type inference) that resolves it later.
pub(crate) fn parse_retained_produced_values(
    input: ProducedValuesParseInput<'_, '_, '_>,
) -> Result<Vec<StatementExpression>, ExpressionParseError> {
    let ProducedValuesParseInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        target,
        string_table,
        path_fork,
    } = input;

    let slot_expected_types = target.slot_expected_types();
    let arity = target.expected_value_count();
    let expression_context = context.new_child_expression(vec![]);
    let mut values = Vec::with_capacity(arity);

    for index in 0..arity {
        let known_slot_type = slot_expected_types.get(index).copied().flatten();
        let mut expected_type = match known_slot_type {
            Some(slot_type_id) => {
                parse_expectation_for_type_id(slot_type_id, type_interner.environment())
            }
            None => ExpectedType::Infer,
        };
        let mut slot_cast_target = match known_slot_type {
            Some(slot_type_id) => cast_target_context_for_type_id(
                slot_type_id,
                type_interner.environment(),
                string_table,
                path_fork,
            ),
            None => CastTargetContext::None,
        };
        let mut input = ExpressionParseInput::new(
            ExpressionParseResources {
                token_stream,
                scope_context: &expression_context,
                type_interner,
                expression_builder,
                const_values,
                expected_type: &mut expected_type,
                cast_target_context: &mut slot_cast_target,
                value_mode: &ValueMode::ImmutableOwned,
                string_table,
                path_fork,
            },
            ExpressionTrailingPolicy {
                consume_closing_parenthesis: false,
                skip_trailing_newlines: false,
                allow_boundary_catch: true,
                allow_expected_result_evidence: false,
            },
        )
        .with_numeric_deferral_owner();
        if known_slot_type.is_none() {
            // An unresolved slot has no destination yet, so a literal-only fallback must stay
            // unresolved too; a known slot materialises it through its own expectation.
            input = input.with_retained_pending_numeric_fragment();
        }
        let value = match create_expression_with_trailing_newline_policy(input)
            .map(|expression| expression.id())
        {
            Ok(expression) => StatementExpression::Built(expression),
            Err(error) => {
                let deferral = error.take_numeric_deferral(
                    "a fallback value of a value-producing statement whose receiving site is not \
                     resolved yet",
                )?;
                StatementExpression::Retained(deferred_numeric_expression(
                    deferral.items,
                    deferral.expected,
                    deferral.value_mode,
                    deferral.span,
                    deferral.suffix,
                ))
            }
        };
        values.push(value);

        if index + 1 < arity {
            if token_stream.current_tag() != TokenTag::COMMA {
                // An expression start here means the author wrote more values than the receiver
                // accepts; the ordinary multi-value path reports the same reason for that shape.
                if is_expression_start_tag(token_stream.current_tag()) {
                    return Err(CompilerDiagnostic::invalid_return_shape(
                        InvalidReturnShapeReason::TooManyReturnValues {
                            expected_count: arity,
                        },
                        Some(token_stream.current_span()),
                    )
                    .into());
                }

                return Err(CompilerDiagnostic::invalid_return_shape(
                    InvalidReturnShapeReason::TooFewReturnValues {
                        expected_count: arity,
                        provided_count: values.len(),
                    },
                    Some(token_stream.current_span()),
                )
                .into());
            }
            token_stream.advance();
        }
    }

    if token_stream.current_tag() == TokenTag::COMMA {
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues {
                expected_count: arity,
            },
            Some(token_stream.current_span()),
        )
        .into());
    }

    if values
        .iter()
        .all(|value| matches!(value, StatementExpression::Built(_)))
    {
        // A fully built list keeps the ordinary eager path, including the per-slot receiver
        // coercion this boundary always applied. A list holding a retained value resumes at the
        // body root, where the receiver that resolves later coerces it.
        let mut expressions = Vec::with_capacity(values.len());
        for value in values {
            let StatementExpression::Built(expression) = value else {
                return Err(CompilerError::compiler_error(
                    "a produced-value list reported ready while holding a retained fallback",
                )
                .into());
            };
            expressions.push(expression);
        }

        let coerced = coerce_built_values_at_known_slots(
            expressions,
            &slot_expected_types,
            type_interner.environment(),
            target.receiver_kind,
            expression_builder,
        )?;
        return Ok(coerced
            .into_iter()
            .map(StatementExpression::Built)
            .collect());
    }

    Ok(values)
}

/// Applies the ordinary receiver coercion to each value whose slot type is already known.
///
/// WHAT: coerces a value at its known slot and leaves an inferred slot's value untouched.
/// WHY: a known receiver must coerce or reject a concrete fallback exactly as the ordinary
///      produced-values boundary does, while an inferred slot's value is typed by the site that
///      resolves later and must not be forced into a default.
fn coerce_built_values_at_known_slots(
    values: Vec<ExprId>,
    slot_expected_types: &[Option<TypeId>],
    type_environment: &TypeEnvironment,
    receiver_kind: ValueReceiverKind,
    expression_builder: &mut AstExpressionBuilder,
) -> Result<Vec<ExprId>, ExpressionParseError> {
    let mismatch_context = mismatch_context_for_receiver(receiver_kind);
    let mut coerced = Vec::with_capacity(values.len());

    for (value, slot_type_id) in values.into_iter().zip(slot_expected_types.iter()) {
        expression_builder
            .view(value)?
            .require_single(ValueReceiverContext::Then)?;
        match slot_type_id {
            Some(slot_type_id) => coerced.push(
                coerce_expression_to_explicit_type_boundary(
                    expression_builder.own(value),
                    *slot_type_id,
                    type_environment,
                    mismatch_context,
                    expression_builder,
                )
                .map(|expression| expression.id())?,
            ),
            None => coerced.push(value),
        }
    }

    Ok(coerced)
}

/// Rebuilds one retained `then` statement at the receiving site that resolved it.
///
/// WHAT: completes every retained fallback at its real receiver slot, applies the ordinary
///       receiver validation and coercion, and returns the authored `then` node.
/// WHY: the fallback was authored before its receiving contract existed, so it resumes here with
///      the resolved slots for a provisional site or the authored contract for a known one —
///      never a placeholder value, a defaulted literal, or a second parse.
pub(crate) fn complete_retained_then_value<'env>(
    continuation: ThenValueContinuation,
    resolved_slot_type_ids: Option<&[TypeId]>,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<AstNode, ExpressionParseError> {
    let ThenValueContinuation {
        values,
        slot_expected_types,
        receiver_kind,
        span,
        scope,
    } = continuation;

    // A provisional site pushes the slots its protected value resolved to, and a resumed inferred
    // value block pushes the slots its completed targets resolved to. A known site keeps the
    // contract the statement was authored against. An empty override is a real error-only contract
    // and is never conflated with an inferred slot.
    let effective_slot_types: Vec<Option<TypeId>> = match resolved_slot_type_ids {
        Some(resolved) => resolved.iter().copied().map(Some).collect(),
        None => slot_expected_types,
    };

    if effective_slot_types.is_empty() {
        // The contract carries no success values, so an authored fallback has nothing to produce:
        // the reason the parse-time path reports for an error-only receiver.
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
            span,
        )
        .into());
    }

    if effective_slot_types.len() != values.len() {
        return Err(CompilerDiagnostic::invalid_return_shape(
            if values.len() < effective_slot_types.len() {
                InvalidReturnShapeReason::TooFewReturnValues {
                    expected_count: effective_slot_types.len(),
                    provided_count: values.len(),
                }
            } else {
                InvalidReturnShapeReason::TooManyReturnValues {
                    expected_count: effective_slot_types.len(),
                }
            },
            span,
        )
        .into());
    }

    let expectations: Vec<Option<ExpectedType>> = {
        let services = completion.session_services();
        effective_slot_types
            .iter()
            .map(|slot_type_id| {
                slot_type_id.map(|slot_type_id| {
                    parse_expectation_for_type_id(
                        slot_type_id,
                        services.type_interner.environment(),
                    )
                })
            })
            .collect()
    };

    let mut expressions = Vec::with_capacity(values.len());
    for (value, expectation) in values.into_iter().zip(expectations) {
        let value = match value {
            ThenValueSlot::Pending(value) => match (value, expectation) {
                (StatementExpression::Built(expression), _) => {
                    StatementExpression::Built(expression)
                }
                (StatementExpression::Retained(mut retained), Some(expectation)) => {
                    // The resolved or known slot is this fallback's destination, so it carries the
                    // same expectation the ordinary value-production boundary derives for that
                    // receiver. Completion then runs through the ordinary statement-expression
                    // owner, which applies the authored suffix and closes the handler's own
                    // typed-error boundary.
                    retained.expected = expectation;
                    StatementExpression::Retained(retained)
                }
                // A genuinely inferred slot keeps the expectation the fallback was authored with:
                // its type comes from the completed body through the owning block's result-type
                // inference, never from a fabricated contract.
                (StatementExpression::Retained(retained), None) => {
                    StatementExpression::Retained(retained)
                }
            },
            ThenValueSlot::Completed(expression) => StatementExpression::Built(expression),
            ThenValueSlot::Taken => {
                return Err(CompilerError::compiler_error(
                    "a retained `then` value reached completion while a multi-bind producer still owned it",
                )
                .into());
            }
        };
        expressions.push(completion.resolve_statement_expression(value)?.id());
    }

    let expressions = {
        let services = completion.session_services();
        coerce_built_values_at_known_slots(
            expressions,
            &effective_slot_types,
            services.type_interner.environment(),
            receiver_kind,
            services.expression_builder,
        )?
    };

    Ok(AstNode {
        kind: NodeKind::ThenValue(ProducedValues { expressions, span }),
        span,
        scope,
    })
}

/// Applies the ordinary receiving boundary to one completed `then` group.
///
/// WHAT: requires the exact slot count and coerces every value to the resolved slot type.
/// WHY: a completed handler body can hold groups that were parsed before the protected slot was
///      known (or while the handler was still provisional), so completion re-applies the
///      boundary the authored site applies at parse time instead of letting a mismatched value
///      reach HIR as a write into the wrong carrier.
pub(crate) fn validate_and_coerce_completed_then_group(
    produced: &mut ProducedValues,
    slot_type_ids: &[TypeId],
    receiver_kind: ValueReceiverKind,
    services: &mut DeferredExpressionServices<'_, '_>,
) -> Result<(), ExpressionParseError> {
    if produced.expressions.len() != slot_type_ids.len() {
        return Err(CompilerDiagnostic::invalid_return_shape(
            if produced.expressions.len() < slot_type_ids.len() {
                InvalidReturnShapeReason::TooFewReturnValues {
                    expected_count: slot_type_ids.len(),
                    provided_count: produced.expressions.len(),
                }
            } else {
                InvalidReturnShapeReason::TooManyReturnValues {
                    expected_count: slot_type_ids.len(),
                }
            },
            produced.span,
        )
        .into());
    }

    let mismatch_context = mismatch_context_for_receiver(receiver_kind);
    let mut coerced = Vec::with_capacity(produced.expressions.len());
    for (value, slot_type_id) in produced.expressions.drain(..).zip(slot_type_ids.iter()) {
        services
            .expression_builder
            .view(value)?
            .require_single(ValueReceiverContext::Then)?;
        coerced.push(
            coerce_expression_to_explicit_type_boundary(
                services.expression_builder.own(value),
                *slot_type_id,
                services.type_interner.environment(),
                mismatch_context,
                services.expression_builder,
            )?
            .id(),
        );
    }
    produced.expressions = coerced;

    Ok(())
}

fn mismatch_context_for_receiver(receiver_kind: ValueReceiverKind) -> TypeMismatchContext {
    match receiver_kind {
        ValueReceiverKind::Declaration => TypeMismatchContext::Declaration,
        ValueReceiverKind::Return => TypeMismatchContext::ReturnValue,
        ValueReceiverKind::Assignment
        | ValueReceiverKind::MultiBind
        | ValueReceiverKind::CatchHandler => TypeMismatchContext::General,
    }
}

// WHAT: identifies tokens that can begin a new expression.
// WHY: when a fixed-arity value list stops early, the token at which it stopped may actually be
// the start of an extra produced value (user forgot a comma). Distinguishing expression starts
// from statement keywords or terminators lets us report TooManyReturnValues instead of the
// misleading TooFewReturnValues.
fn is_expression_start_tag(tag: TokenTag) -> bool {
    // Explicit-width scalars and `Byte` open a type-annotation expression attempt, matching the
    // builtin scalar spellings below.
    if tag.is_builtin_scalar_type_name() {
        return true;
    }

    matches!(
        tag,
        TokenTag::SYMBOL
            | TokenTag::NUMERIC_LITERAL
            | TokenTag::STRING_SLICE_LITERAL
            | TokenTag::RAW_STRING_LITERAL
            | TokenTag::CHAR_LITERAL
            | TokenTag::BOOL_LITERAL
            | TokenTag::NONE_LITERAL
            | TokenTag::OPEN_CURLY
            | TokenTag::OPEN_PARENTHESIS
            | TokenTag::TEMPLATE_HEAD
            | TokenTag::SUBTRACT
            | TokenTag::COPY
            | TokenTag::MUTABLE
    )
}
