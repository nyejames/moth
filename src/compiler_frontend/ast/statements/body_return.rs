//! Function-body return statement parsing.
//!
//! WHAT: parses `return` and `return!` statements and emits validated AST return nodes.
//! WHY: return handling is signature-sensitive (arity, channels, coercion), so isolating this
//! logic keeps body dispatch simple and prevents return rules from leaking across modules.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, NumericContinuation, ReturnContinuation, StatementExpression,
    deferred_numeric_expression,
};
use crate::compiler_frontend::ast::statements::value_production::types::ParsedReceiverValue;
use crate::compiler_frontend::ast::statements::value_production::{
    ValueReceiverKind, try_parse_value_block_at_receiver,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidControlFlowStatementReason,
    InvalidReturnShapeReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Whether the tag ends a return statement with no expression following.
fn is_return_terminator(tag: TokenTag) -> bool {
    matches!(tag, TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF)
}

// --------------------------
//  Return statement parsing
// --------------------------

/// Parse one return-channel expression, retaining a numeric-dependent fragment.
///
/// WHAT: a return value whose fragment still holds construction-only numeric items becomes a
///       retained slot; every other value stays an ordinary expression.
/// WHY: a return slot is a receiving site, so an unresolved origin must keep its authored
///      fragment until the body root has selected the origin's single static type; the resumed
///      slot then applies the ordinary return-boundary coercion.
fn parse_return_channel_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected: ExpectedType,
    cast_target: CastTargetContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<StatementExpression, ExpressionParseError> {
    let mut expected = expected;
    let mut cast_target = cast_target;
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expected_type: &mut expected,
            cast_target_context: &mut cast_target,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
            path_fork,
        },
        false,
    )
    .with_numeric_deferral_owner();
    match create_expression_with_trailing_newline_policy(input) {
        Ok(expression) => Ok(StatementExpression::Built(expression)),
        Err(error) => {
            let deferral = error.take_numeric_deferral("a return value")?;
            Ok(StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                deferral.expected,
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            )))
        }
    }
}

pub(crate) fn parse_return_statement(
    token_stream: &mut AstCursor,
    ast: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    if !context.inside_authored_function
        && !matches!(
            context.kind,
            ContextKind::Function | ContextKind::CatchHandler
        )
    {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ReturnOutsideFunction,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let return_span = Some(token_stream.current_span());
    let is_error_return = token_stream.current_tag() == TokenTag::RETURN_BANG;
    token_stream.advance();

    // --------------------------
    //  Error return (return!)
    // --------------------------

    if is_error_return {
        let Some(expected_error_type_id) = context.expected_error_type else {
            return Err(CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ReturnBangOutsideErrorFunction,
                Some(token_stream.current_span()),
            )
            .into());
        };

        if is_return_terminator(token_stream.current_tag()) {
            return Err(CompilerDiagnostic::invalid_return_shape(
                InvalidReturnShapeReason::MissingReturnBangValue,
                Some(token_stream.current_span()),
            )
            .into());
        }

        let returned_error = parse_return_channel_expression(
            token_stream,
            context,
            type_interner,
            ExpectedType::Known(expected_error_type_id),
            CastTargetContext::None,
            string_table,
            path_fork,
        )?;

        return finish_error_return(
            ast,
            context,
            type_interner,
            returned_error,
            expected_error_type_id,
            return_span,
        );
    }

    if token_stream.current_tag() == TokenTag::BANG {
        return Err(CompilerDiagnostic::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(TokenTag::BANG),
            Some(token_stream.current_span()),
        )
        .into());
    }

    // --------------------------
    //  Value-producing return if
    // --------------------------

    // A return receives values for the enclosing function contract, not for the local
    // receiver it may be nested in, such as a value-producing declaration initializer.
    // Keep the statement kind so return-site `catch` stays permitted.
    let function_return_type_ids = &context.current_function_return_type_ids;
    let mut return_context = context.new_child_expression(function_return_type_ids.clone());
    return_context.kind = context.kind.clone();

    if token_stream.current_tag() == TokenTag::IF && !function_return_type_ids.is_empty() {
        let value_block_expr = match try_parse_value_block_at_receiver(
            token_stream,
            &return_context,
            type_interner,
            function_return_type_ids,
            ValueReceiverKind::Return,
            string_table,
            path_fork,
        ) {
            Some(Ok(ParsedReceiverValue::Complete(expression))) => expression,
            // The block's bodies still hold numeric continuations, so the block is this return's
            // whole value and only the body root can rebuild it. Keep the authored block as the
            // return's single retained slot: the resumed slot completes through the block's own
            // constructor (which applies the receiver's per-slot coercion) and the return
            // boundary then applies the same final guard the completed path applies.
            Some(Ok(ParsedReceiverValue::Retained(retained))) => {
                let ExpressionParseError::NumericDeferral(deferral) =
                    retained.into_deferral(&ValueMode::ImmutableOwned)
                else {
                    return Err(CompilerError::compiler_error(
                        "a retained value-block return lost its numeric deferral",
                    )
                    .into());
                };
                let slot = StatementExpression::Retained(deferred_numeric_expression(
                    deferral.items,
                    deferral.expected,
                    deferral.value_mode,
                    deferral.span,
                    deferral.suffix,
                ));
                let continuation = context.register_numeric_continuation(
                    NumericContinuation::Return(ReturnContinuation {
                        slots: vec![slot],
                        is_error_return: false,
                        span: return_span,
                        scope: context.scope,
                    }),
                )?;
                ast.push_pending(continuation);
                return Ok(());
            }
            Some(Ok(ParsedReceiverValue::NeedsSlotInference { .. })) => unreachable!(
                "a return receiver knows its result types; a value block cannot ask for slot inference"
            ),
            Some(Err(error)) => return Err(error),
            None => {
                // Token was `if` but parsing failed at a deeper level.
                // The helper has already advanced past `if` and reported
                // its own diagnostic, so we should not fall through to
                // normal return parsing which would produce a secondary error.
                return Err(CompilerDiagnostic::invalid_control_flow_statement(
                    InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                    Some(token_stream.current_span()),
                )
                .into());
            }
        };

        // For single-result returns, apply a final coercion guard to preserve
        // existing behaviour (e.g. optional wrapping). For multi-result, the
        // value-block parser already validated and coerced each slot.
        let return_expr = if let &[expected_type_id] = function_return_type_ids.as_slice() {
            coerce_expression_to_explicit_type_boundary(
                value_block_expr,
                expected_type_id,
                type_interner.environment(),
                TypeMismatchContext::ReturnValue,
            )?
        } else {
            value_block_expr
        };

        ast.push_node(AstNode {
            kind: NodeKind::Return(vec![return_expr]),
            span: return_span,
            scope: context.scope,
        });

        return Ok(());
    }

    // --------------------------
    //  Normal return values
    // --------------------------

    if function_return_type_ids.is_empty() {
        if !is_return_terminator(token_stream.current_tag()) {
            return Err(CompilerDiagnostic::invalid_return_shape(
                InvalidReturnShapeReason::ReturnValuesWithBareSignature,
                Some(token_stream.current_span()),
            )
            .into());
        }
        ast.push_node(AstNode {
            kind: NodeKind::Return(Vec::new()),
            span: return_span,
            scope: context.scope,
        });
        return Ok(());
    }

    if is_return_terminator(token_stream.current_tag()) {
        let expected_count = function_return_type_ids.len();
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::BareReturnWithExpectedValues { expected_count },
            Some(token_stream.current_span()),
        )
        .into());
    }

    let mut slots: Vec<StatementExpression> = Vec::with_capacity(function_return_type_ids.len());
    let mut retained = false;
    for (type_index, expected_type_id) in function_return_type_ids.iter().enumerate() {
        let expected =
            parse_expectation_for_type_id(*expected_type_id, type_interner.environment());
        let cast_target = cast_target_context_for_type_id(
            *expected_type_id,
            type_interner.environment(),
            string_table,
            path_fork,
        );
        let slot = parse_return_channel_expression(
            token_stream,
            &return_context,
            type_interner,
            expected,
            cast_target,
            string_table,
            path_fork,
        )?;
        retained |= matches!(slot, StatementExpression::Retained(_));
        slots.push(slot);

        // Every slot except the last must be followed by a comma, exactly like the shared
        // multi-expression list owner.
        if type_index + 1 < function_return_type_ids.len() {
            if token_stream.current_tag() != TokenTag::COMMA {
                let expected_count = function_return_type_ids.len();
                return Err(CompilerDiagnostic::invalid_return_shape(
                    InvalidReturnShapeReason::TooFewReturnValues {
                        expected_count,
                        provided_count: slots.len(),
                    },
                    Some(token_stream.current_span()),
                )
                .into());
            }

            token_stream.advance();
        }
    }

    if token_stream.current_tag() == TokenTag::COMMA {
        let expected_count = function_return_type_ids.len();
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues { expected_count },
            Some(token_stream.current_span()),
        )
        .into());
    }

    if retained {
        let continuation = context.register_numeric_continuation(NumericContinuation::Return(
            ReturnContinuation {
                slots,
                is_error_return: false,
                span: return_span,
                scope: context.scope,
            },
        ))?;
        ast.push_pending(continuation);
        return Ok(());
    }

    let mut coerced_values: Vec<Expression> = Vec::with_capacity(slots.len());
    for (slot, expected_type_id) in slots.into_iter().zip(function_return_type_ids.iter()) {
        let StatementExpression::Built(returned_value) = slot else {
            return Err(CompilerError::compiler_error(
                "a return slot list without retained fragments held a deferred value",
            )
            .into());
        };
        coerced_values.push(coerce_expression_to_explicit_type_boundary(
            returned_value,
            *expected_type_id,
            type_interner.environment(),
            TypeMismatchContext::ReturnValue,
        )?);
    }

    ast.push_node(AstNode {
        kind: NodeKind::Return(coerced_values),
        span: return_span,
        scope: context.scope,
    });

    Ok(())
}

/// Place one error-channel return value, retaining it when its fragment needs numeric resolution.
fn finish_error_return(
    ast: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    returned_error: StatementExpression,
    expected_error_type_id: TypeId,
    return_span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    match returned_error {
        StatementExpression::Built(value) => {
            let value = coerce_expression_to_explicit_type_boundary(
                value,
                expected_error_type_id,
                type_interner.environment(),
                TypeMismatchContext::ReturnValue,
            )?;
            ast.push_node(AstNode {
                kind: NodeKind::ReturnError(value),
                span: return_span,
                scope: context.scope,
            });
            Ok(())
        }
        StatementExpression::Retained(deferred) => {
            let continuation = context.register_numeric_continuation(
                NumericContinuation::Return(ReturnContinuation {
                    slots: vec![StatementExpression::Retained(deferred)],
                    is_error_return: true,
                    span: return_span,
                    scope: context.scope,
                }),
            )?;
            ast.push_pending(continuation);
            Ok(())
        }
    }
}
