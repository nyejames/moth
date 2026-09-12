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
    ActiveValueProductionTarget, ParsedReceiverValue, ValueBlock, ValueMatchBlock,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

/// Input for the inline single-predicate body parser after `if` has been consumed.
pub(super) struct InlineSinglePredicateParseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) header_index: usize,
    pub(super) span: Option<SourceSpan>,
    pub(super) classification: IfHeaderClassification,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Attempts to parse an inline single-predicate value match after `if`.
///
/// WHAT: consumes the shared header parser, then requires same-line `then`.
pub(super) fn try_parse_inline_single_predicate_value_match(
    input: InlineSinglePredicateParseInput<'_, '_>,
) -> Option<Result<ParsedReceiverValue, ExpressionParseError>> {
    let InlineSinglePredicateParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        header_index,
        span,
        classification,
        path_fork,
    } = input;

    let header = match try_parse_single_predicate_header(SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        string_table,
        classification,
        path_fork,
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
            Some(token_stream.current_span()),
        )
        .into()));
    }

    if !same_logical_line(token_stream, header_index, token_stream.index) {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            Some(token_stream.current_span()),
        )
        .into()));
    }

    Some(parse_inline_value_match(InlineValueMatchParseInput {
        token_stream,
        context,
        then_context: &header.then_context,
        type_interner,
        target,
        string_table,
        scrutinee: header.scrutinee,
        pattern: header.pattern,
        span,
        path_fork,
    }))
}

struct InlineValueMatchParseInput<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    then_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    target: ActiveValueProductionTarget,
    string_table: &'a mut StringTable,
    scrutinee: Expression,
    pattern: MatchPattern,
    span: Option<SourceSpan>,
    path_fork: &'a mut PathInternerFork,
}

/// The speculative outer parser may discard only authored diagnostics. Once a match shape is
/// accepted, the inner parser retains `CompilerError` until AST emission.
type InlineValueMatchResult<T> = Result<T, ExpressionParseError>;

fn parse_inline_value_match(
    input: InlineValueMatchParseInput<'_, '_>,
) -> InlineValueMatchResult<ParsedReceiverValue> {
    let InlineValueMatchParseInput {
        token_stream,
        context,
        then_context,
        type_interner,
        target,
        string_table,
        scrutinee,
        pattern,
        span,
        path_fork,
    } = input;

    let output = parse_inline_then_else(InlineThenElseInput {
        token_stream,
        then_context,
        else_context: context,
        type_interner,
        target,
        string_table,
        path_fork,
    })?;

    let then_body = vec![then_value_node(
        output.then_values,
        output.then_span,
        then_context.scope.clone(),
    )];
    let else_body = vec![then_value_node(
        output.else_values,
        output.else_span,
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
        span,
        result_type_ids: output.result_type_ids,
    };

    Ok(match output.result_type_id {
        Some(result_type_id) => ParsedReceiverValue::Complete(build_value_match_expression(
            value_match,
            span,
            result_type_id,
            type_interner.environment(),
        )),
        None => ParsedReceiverValue::NeedsSlotInference {
            block: ValueBlock::Match(value_match),
            span,
        },
    })
}
