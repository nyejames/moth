//! Inline single-predicate value-match parser.
//!
//! WHAT: parses option-present capture (`if opt is |v| then ... else ...`)
//! and choice single-predicate matches (`if status is Ready then ... else ...`).
//! WHY: these forms are syntactically similar to inline Bool value-if but need
//! speculative scrutinee parsing, pattern capture scopes, and `ValueMatchBlock`
//! construction instead of `ValueIfBlock`.

use super::expression_build::{build_value_match_expression, then_value_node};
use super::inline_then_else::{InlineThenElseInput, parse_inline_then_else, same_logical_line};
use super::token_checkpoint::TokenCheckpoint;
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_until;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::match_headers::parse_single_predicate_match_pattern;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueMatchBlock, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// Attempts to parse an inline single-predicate value match speculatively.
///
/// WHAT: tries to parse `if <scrutinee> is <pattern> then ... else ...`.
/// Returns `None` if the token stream does not match this form, restoring the
/// token index so the caller can fall back to Bool condition parsing.
/// WHY: the `if` keyword has already been consumed; speculation must not leave
/// the stream in a partial state.
pub(super) fn try_parse_inline_single_predicate_value_match(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_result_type_ids: &[TypeId],
    receiver_kind: ValueReceiverKind,
    string_table: &mut StringTable,
    location: SourceLocation,
) -> Option<Result<Expression, ExpressionParseError>> {
    let checkpoint = TokenCheckpoint::capture(token_stream);

    let mut scrutinee_type = ExpectedType::Infer;
    let scrutinee_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &scrutinee_context,
        type_interner,
        expected_type: &mut scrutinee_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let scrutinee = match create_expression_until(input, &[TokenKind::Is]) {
        Ok(expression) => expression,
        Err(ExpressionParseError::Diagnostic(_)) => {
            checkpoint.restore(token_stream);
            return None;
        }
        Err(error @ ExpressionParseError::Infrastructure(_)) => return Some(Err(error)),
    };

    if token_stream.current_token_kind() != &TokenKind::Is {
        checkpoint.restore(token_stream);
        return None;
    }

    let type_environment = type_interner.environment();
    let is_option_present_capture = type_environment
        .option_inner_type(scrutinee.type_id)
        .is_some()
        && super::detect::next_non_newline_index(token_stream, token_stream.index + 1).is_some_and(
            |index| token_stream.tokens[index].kind == TokenKind::TypeParameterBracket,
        );
    let is_choice_predicate = type_environment.variants_for(scrutinee.type_id).is_some();

    if !is_option_present_capture && !is_choice_predicate {
        checkpoint.restore(token_stream);
        return None;
    }

    token_stream.advance(); // consume `is`

    // Inline single-predicate arms still need an arm-local scope so captures such as
    // `name = if maybe is |name| then name else "guest"` do not reuse the receiving
    // declaration's path or leak into the else expression.
    let match_context = context.new_child_control_flow(ContextKind::Branch, string_table);
    let parsed_pattern = match parse_single_predicate_match_pattern(
        &scrutinee,
        token_stream,
        &match_context,
        type_interner,
        string_table,
    ) {
        Ok(pattern) => pattern,
        Err(error) => return Some(Err(error)),
    };

    if token_stream.current_token_kind() != &TokenKind::Then {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            token_stream.current_location(),
        )
        .into()));
    }

    if !same_logical_line(&location, &token_stream.current_location()) {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            token_stream.current_location(),
        )
        .into()));
    }

    checkpoint.commit();

    Some(parse_inline_value_match(InlineValueMatchParseInput {
        token_stream,
        context,
        then_context: &parsed_pattern.arm_scope,
        type_interner,
        expected_result_type_ids,
        receiver_kind,
        string_table,
        scrutinee,
        pattern: parsed_pattern.pattern,
        location,
    }))
}

struct InlineValueMatchParseInput<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    then_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    expected_result_type_ids: &'a [TypeId],
    receiver_kind: ValueReceiverKind,
    string_table: &'a mut StringTable,
    scrutinee: Expression,
    pattern: MatchPattern,
    location: SourceLocation,
}

/// The speculative outer parser may discard only authored diagnostics. Once a match shape is
/// accepted, the inner parser retains `CompilerError` until AST emission.
type InlineValueMatchResult<T> = Result<T, ExpressionParseError>;

fn parse_inline_value_match(
    input: InlineValueMatchParseInput<'_, '_>,
) -> InlineValueMatchResult<Expression> {
    let InlineValueMatchParseInput {
        token_stream,
        context,
        then_context,
        type_interner,
        expected_result_type_ids,
        receiver_kind,
        string_table,
        scrutinee,
        pattern,
        location,
    } = input;

    let output = parse_inline_then_else(InlineThenElseInput {
        token_stream,
        then_context,
        else_context: context,
        type_interner,
        expected_result_type_ids,
        receiver_kind,
        string_table,
    })?;

    let then_body = vec![then_value_node(
        output.then_values,
        location.clone(),
        then_context.scope.clone(),
    )];
    let else_body = vec![then_value_node(
        output.else_values,
        location.clone(),
        context.scope.clone(),
    )];

    let value_match = ValueMatchBlock {
        scrutinee,
        arms: vec![MatchArm {
            pattern,
            guard: None,
            body: then_body,
        }],
        default: Some(else_body),
        exhaustiveness: MatchExhaustiveness::HasDefault,
        location: location.clone(),
        result_type_ids: output.result_type_ids,
    };

    Ok(build_value_match_expression(
        value_match,
        output.result_type_id,
        type_interner.environment(),
    ))
}
