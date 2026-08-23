//! Inline single-predicate value-match body parsing and assembly.
//!
//! WHAT: consumes a committed single-predicate header and parses
//! `then ... else ...` into a `ValueMatchBlock`.
//! WHY: header classification, scrutinee parsing and pattern eligibility live
//! in the shared single-predicate owner so this file does not rescan `if`.

use super::inline_then_else::{InlineThenElseInput, parse_inline_then_else, same_logical_line};
use super::single_predicate::{SinglePredicateHeaderInput, try_parse_single_predicate_header};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::if_headers::{
    IfHeaderClassification, IfHeaderDelimiter,
};
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::expression_build::{
    build_value_match_expression, then_value_node,
};
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

/// Input for the inline single-predicate body parser after `if` has been consumed.
pub(super) struct InlineSinglePredicateParseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) expected_result_type_ids: &'a [TypeId],
    pub(super) receiver_kind: ValueReceiverKind,
    pub(super) string_table: &'a mut StringTable,
    pub(super) location: SourceLocation,
    pub(super) classification: IfHeaderClassification,
}

/// Attempts to parse an inline single-predicate value match after `if`.
///
/// WHAT: consumes the shared header parser, then requires same-line `then`.
/// Returns `None` if the header is not a committed option/choice predicate so
/// the caller can fall back to Bool condition parsing.
pub(super) fn try_parse_inline_single_predicate_value_match(
    input: InlineSinglePredicateParseInput<'_, '_>,
) -> Option<Result<Expression, ExpressionParseError>> {
    let InlineSinglePredicateParseInput {
        token_stream,
        context,
        type_interner,
        expected_result_type_ids,
        receiver_kind,
        string_table,
        location,
        classification,
    } = input;

    let header = match try_parse_single_predicate_header(SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        string_table,
        classification,
    }) {
        Some(Ok(header)) => header,
        Some(Err(error)) => return Some(Err(error)),
        None => return None,
    };

    if header.body_delimiter != IfHeaderDelimiter::InlineThen
        || token_stream.current_token_kind() != &TokenKind::Then
    {
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

    Some(parse_inline_value_match(InlineValueMatchParseInput {
        token_stream,
        context,
        then_context: &header.then_context,
        type_interner,
        expected_result_type_ids,
        receiver_kind,
        string_table,
        scrutinee: header.scrutinee,
        pattern: header.pattern,
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
