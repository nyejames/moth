//! AST expression parsing and expression-list helpers.
//!
//! WHAT: parses token streams into typed AST expressions before evaluation and lowering.
//! WHY: expression parsing centralizes precedence, call parsing, and place-expression rules in one pass.

use super::error::ExpressionParseError;
use super::eval_expression::evaluate_expression;
use super::expression::Expression;
use super::expression_rpn::ExpressionRpnItem;
use super::parse_expression_dispatch::{
    ExpressionDispatchState, ExpressionTokenStep, dispatch_expression_token,
};
use super::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources, ExpressionTrailingPolicy,
};
use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidReturnShapeReason};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::utilities::token_scan::find_expression_end_index;
use crate::compiler_frontend::value_mode::ValueMode;

// WHAT: parses a comma-separated expression list against already-known expected result types.
// WHY: function calls and multi-return contexts must preserve arity and per-slot type
//      expectations while still sharing the normal expression parser.
pub fn create_multiple_expressions(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    _context_label: &str,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<Expression>, ExpressionParseError> {
    create_multiple_expressions_inner(
        token_stream,
        context,
        type_interner,
        consume_closing_parenthesis,
        string_table,
        path_fork,
    )
}

fn create_multiple_expressions_inner(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<Expression>, ExpressionParseError> {
    let mut expressions: Vec<Expression> = Vec::new();

    for (type_index, expected_type) in context.expected_result_type_ids.iter().enumerate() {
        // Pass parse-time context only for context-sensitive literals. Other
        // expressions resolve their natural type here, and callers own
        // validation or coercion after this call returns.
        let mut expression_type =
            parse_expectation_for_type_id(*expected_type, type_interner.environment());
        let mut cast_target_context = cast_target_context_for_type_id(
            *expected_type,
            type_interner.environment(),
            string_table,
            &*path_fork,
        );
        let input = ExpressionParseInput::new(
            ExpressionParseResources {
                token_stream,
                scope_context: context,
                type_interner,
                expected_type: &mut expression_type,
                cast_target_context: &mut cast_target_context,
                value_mode: &ValueMode::ImmutableOwned,
                string_table,
                path_fork,
            },
            ExpressionTrailingPolicy {
                consume_closing_parenthesis,
                skip_trailing_newlines: false,
                allow_boundary_catch: !consume_closing_parenthesis,
                allow_expected_result_evidence: !consume_closing_parenthesis,
            },
        );
        let expression = create_expression_with_trailing_newline_policy(input)?;

        expressions.push(expression);

        // Newlines are expression terminators almost everywhere else. Only normalize
        // them here when we're inside a parenthesized list so multiline calls like
        // `io.line(\n value\n)` leave us positioned on the comma or `)`.
        if consume_closing_parenthesis && token_stream.current_token_kind() == &TokenKind::Newline {
            token_stream.skip_newlines();
        }

        // Comma check: every slot except the last must be followed by a comma.
        if type_index + 1 < context.expected_result_type_ids.len() {
            if token_stream.current_token_kind() != &TokenKind::Comma {
                return Err(CompilerDiagnostic::invalid_return_shape(
                    InvalidReturnShapeReason::TooFewReturnValues {
                        expected_count: context.expected_result_type_ids.len(),
                        provided_count: expressions.len(),
                    },
                    Some(token_stream.current_span()),
                )
                .into());
            }

            token_stream.advance();
        }
    }

    if consume_closing_parenthesis {
        if token_stream.current_token_kind() != &TokenKind::CloseParenthesis {
            return Err(CompilerDiagnostic::expected_token(
                TokenKind::CloseParenthesis,
                Some(token_stream.current_token_kind().to_owned()),
                Some(token_stream.current_span()),
            )
            .into());
        }

        token_stream.advance();
    }

    Ok(expressions)
}

// WHAT: parses one expression and evaluates the AST fragment into a typed expression node.
// WHY: expression parsing is the choke point where token structure, place rules, and expected
//      type information meet before later lowering stages.
pub fn create_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode,
            string_table,
            path_fork,
        },
        consume_closing_parenthesis,
    );
    create_expression_with_trailing_newline_policy(input)
}

// WHAT: parses a nested expression while preserving ordinary expression semantics.
// WHY: `catch` is procedural recovery syntax, so nested expression positions must reject it even
// when they reuse a function/body context that would allow `catch` at the outer statement boundary.
pub(crate) fn create_expression_without_boundary_catch(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::without_boundary_catch(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode,
            string_table,
            path_fork,
        },
        consume_closing_parenthesis,
    );
    create_expression_with_trailing_newline_policy(input)
}

// WHAT: shared expression parser entry with configurable trailing-newline behavior.
// WHY: callers parsing comma-separated lists outside parentheses (for example
//      fallback/return lists) must preserve line boundaries between statements.
pub(crate) fn create_expression_with_trailing_newline_policy(
    input: ExpressionParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let mut expression: Vec<ExpressionRpnItem> = Vec::new();

    ast_log!(
        "Parsing ",
        #input.value_mode,
        format!("{:?}", input.expected_type),
        " Expression"
    );

    // Build the flat infix AST fragment first. `evaluate_expression` is the stage that turns
    // this fragment into precedence-ordered RPN, resolves the final type, and folds constants.
    let mut next_number_negative = false;
    while input.token_stream.index < input.token_stream.length {
        let token = input.token_stream.current_token_kind().to_owned();
        ast_log!("Parsing expression: ", #token);
        let mut dispatch_state = ExpressionDispatchState {
            expected_type: input.expected_type,
            cast_target_context: input.cast_target_context,
            value_mode: input.value_mode,
            consume_closing_parenthesis: input.trailing_policy.consume_closing_parenthesis,
            allow_boundary_catch: input.trailing_policy.allow_boundary_catch,
            allow_expected_result_evidence: input.trailing_policy.allow_expected_result_evidence,
            expression: &mut expression,
            next_number_negative: &mut next_number_negative,
        };
        match dispatch_expression_token(
            token,
            input.token_stream,
            input.scope_context,
            input.type_interner,
            &mut dispatch_state,
            input.string_table,
            input.path_fork,
        )? {
            ExpressionTokenStep::Continue => continue,
            ExpressionTokenStep::Advance => input.token_stream.advance(),
            ExpressionTokenStep::Break => break,
            ExpressionTokenStep::Return(value) => return Ok(*value),
        }
    }

    if input.trailing_policy.skip_trailing_newlines {
        input.token_stream.skip_newlines();
    }

    evaluate_expression(
        input.scope_context,
        expression,
        input.type_interner,
        input.expected_type,
        input.value_mode,
        input.string_table,
        input.path_fork,
    )
    .map_err(ExpressionParseError::from)
}

// WHAT: parses an expression from a bounded token slice without consuming the stop token.
// WHY: some parent parsers need normal expression semantics while reserving a delimiter for the
//      surrounding grammar layer to inspect and consume itself.
pub(crate) fn create_expression_until(
    input: ExpressionParseInput<'_, '_>,
    stop_tokens: &[TokenKind],
) -> Result<Expression, ExpressionParseError> {
    create_expression_until_with_policy(input, stop_tokens)
}

fn create_expression_until_with_policy(
    input: ExpressionParseInput<'_, '_>,
    stop_tokens: &[TokenKind],
) -> Result<Expression, ExpressionParseError> {
    let allow_boundary_catch = input.trailing_policy.allow_boundary_catch;
    let allow_expected_result_evidence = input.trailing_policy.allow_expected_result_evidence;

    // Fast path: no stop tokens means an unbounded parse with the same boundary policy.
    if stop_tokens.is_empty() {
        let mut unbounded_input = input;
        unbounded_input.trailing_policy = ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: true,
            allow_boundary_catch,
            allow_expected_result_evidence,
        };

        return create_expression_with_trailing_newline_policy(unbounded_input);
    }

    // ------------------------
    //  Locate expression window
    // ------------------------
    let start_index = input.token_stream.index;
    let end_index = find_expression_end_index(&input.token_stream.tokens, start_index, stop_tokens);

    // ------------------------
    //  Validate window bounds
    // ------------------------
    if end_index >= input.token_stream.length {
        let formatted_stop_tokens: Vec<String> = stop_tokens
            .iter()
            .map(|token| format!("{token:?}"))
            .collect();
        let expected_tokens = formatted_stop_tokens.join(", ");

        let expected_delimiter = input.string_table.intern(&expected_tokens);
        return Err(CompilerDiagnostic::unexpected_end_of_file(
            Some(expected_delimiter),
            Some(input.token_stream.current_span()),
        )
        .into());
    }

    if end_index == start_index {
        return Err(CompilerDiagnostic::unexpected_token(
            input.token_stream.tokens[end_index].kind.to_owned(),
            Some(SourceSpan::new(
                input.token_stream.file_id,
                input.token_stream.tokens[end_index].span,
            )),
        )
        .into());
    }

    if !stop_tokens
        .iter()
        .any(|stop| input.token_stream.tokens[end_index].kind == *stop)
    {
        return Err(CompilerDiagnostic::unexpected_token(
            input.token_stream.tokens[end_index].kind.to_owned(),
            Some(SourceSpan::new(
                input.token_stream.file_id,
                input.token_stream.tokens[end_index].span,
            )),
        )
        .into());
    }

    // ------------------------
    //  Parse within window
    // ------------------------
    let window_token_count = end_index.saturating_sub(start_index);
    add_ast_counter(AstCounter::BoundedExpressionTokenWindows, 1);
    add_ast_counter(
        AstCounter::BoundedExpressionTokenCopiesAvoided,
        window_token_count,
    );

    // Narrow the visible token stream so the inner parser stops at the stop token
    // without needing to copy the slice. The original length is restored after parsing.
    let original_length = input.token_stream.length;
    input.token_stream.length = end_index;

    let inner_input = ExpressionParseInput::new(
        ExpressionParseResources {
            token_stream: input.token_stream,
            scope_context: input.scope_context,
            type_interner: input.type_interner,
            expected_type: input.expected_type,
            cast_target_context: input.cast_target_context,
            value_mode: input.value_mode,
            string_table: input.string_table,
            path_fork: input.path_fork,
        },
        ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: true,
            allow_boundary_catch,
            allow_expected_result_evidence,
        },
    );

    let result = create_expression_with_trailing_newline_policy(inner_input);

    // Restore the full stream and position the cursor on the stop token.
    // `input` was never moved, only its fields were reborrowed for `inner_input`,
    // so the token stream reference is still available here.
    input.token_stream.length = original_length;
    input.token_stream.index = end_index;
    result
}

#[cfg(test)]
#[path = "tests/bounded_expression_tests.rs"]
mod bounded_expression_tests;
