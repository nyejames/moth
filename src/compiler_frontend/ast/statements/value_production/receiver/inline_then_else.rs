//! Shared inline `then` / `else` parsing for value-producing receivers.
//!
//! WHAT: consumes the shared `then ... else ...` shape that appears in both
//! inline Bool value-if and inline single-predicate value-match.
//! WHY: these two syntactic forms previously duplicated structural validation
//! (newline rejection, `else then` rejection, same-line checks, and coercion).

use super::result_type::{infer_inline_result_type, receiver_type_mismatch_context};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::value_production::parse_values::{
    ProducedValuesParseInput, is_missing_produced_value_boundary, parse_produced_values_typed,
};
use crate::compiler_frontend::ast::statements::value_production::types::ActiveValueProductionTarget;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id,
};
use crate::compiler_frontend::utilities::token_scan::find_expression_end_index;
use crate::compiler_frontend::value_mode::ValueMode;

/// Input for the shared inline then/else parser.
pub(super) struct InlineThenElseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) then_context: &'a ScopeContext,
    pub(super) else_context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
}

/// Output of the shared inline then/else parser.
pub(super) struct InlineThenElseOutput {
    pub(super) then_values: Vec<Expression>,
    pub(super) else_values: Vec<Expression>,
    pub(super) result_type_id: Option<TypeId>,
    pub(super) result_type_ids: Vec<TypeId>,
}

/// Returns `true` when both source locations are on the same logical line.
///
/// WHAT: used to enforce that inline value-if/match arms stay on one line.
pub(in crate::compiler_frontend::ast::statements::value_production) fn same_logical_line(
    left: &SourceLocation,
    right: &SourceLocation,
) -> bool {
    left.start_pos.line_number == right.start_pos.line_number
}

/// Inline branch expressions share the AST body parser's two-lane error boundary. This avoids
/// converting a retained-token lifecycle fault into a source diagnostic mid-parse.
type InlineThenElseResult<T> = Result<T, ExpressionParseError>;

/// Parses inline `then` / `else` values against an active production target.
///
/// WHAT: assumes the current token is `then`. Consumes the shared one-line shape
/// and reads each branch through `parse_produced_values_typed`.
/// WHY: known multi-value receivers and inferred multi-bind share this grammar;
/// slot inference stays with the multi-bind owner after these values are collected.
pub(in crate::compiler_frontend::ast::statements::value_production) fn parse_inline_then_else_with_target(
    token_stream: &mut FileTokens,
    then_context: &ScopeContext,
    else_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    target: &ActiveValueProductionTarget,
    string_table: &mut StringTable,
) -> InlineThenElseResult<(Vec<Expression>, Vec<Expression>)> {
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
    // the branch has no value and must not reach expression evaluation.
    if is_missing_produced_value_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterThen,
            token_stream.current_location(),
        )
        .into());
    }

    let then_values = parse_produced_values_typed(ProducedValuesParseInput {
        token_stream,
        context: then_context,
        type_interner,
        target,
        label: "then branch",
        string_table,
    })?;

    require_else_inline(token_stream, &then_location)?;
    token_stream.advance(); // consume `else`

    reject_else_then(token_stream)?;
    reject_newline_after_else(token_stream)?;
    reject_empty_value_after_else(token_stream)?;

    let else_values = parse_produced_values_typed(ProducedValuesParseInput {
        token_stream,
        context: else_context,
        type_interner,
        target,
        label: "else branch",
        string_table,
    })?;

    Ok((then_values, else_values))
}

/// Parses the shared `then <branch> else <branch>` inline shape.
///
/// WHAT: assumes the current token is `then`. Consumes it, parses then and else
/// branches, validates same-line constraints, infers/coerces the result type.
/// WHY: consolidates duplicated logic from inline Bool value-if and inline
/// single-predicate value-match.
pub(super) fn parse_inline_then_else(
    input: InlineThenElseInput<'_, '_>,
) -> InlineThenElseResult<InlineThenElseOutput> {
    let InlineThenElseInput {
        token_stream,
        then_context,
        else_context,
        type_interner,
        target,
        string_table,
    } = input;

    let expected_result_type_ids = target.result_type_ids.clone();
    let receiver_kind = target.receiver_kind;

    if expected_result_type_ids.len() > 1 || target.expected_arity.is_some_and(|arity| arity > 1) {
        let (then_values, else_values) = parse_inline_then_else_with_target(
            token_stream,
            then_context,
            else_context,
            type_interner,
            &target,
            string_table,
        )?;
        if target.needs_slot_inference() {
            return Ok(InlineThenElseOutput {
                then_values,
                else_values,
                result_type_id: None,
                result_type_ids: Vec::new(),
            });
        }

        let result_type_ids = expected_result_type_ids;
        let result_type_id = type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(result_type_ids.clone());

        return Ok(InlineThenElseOutput {
            then_values,
            else_values,
            result_type_id: Some(result_type_id),
            result_type_ids,
        });
    }

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
    // the branch has no value and must not reach expression evaluation.
    if is_missing_produced_value_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterThen,
            token_stream.current_location(),
        )
        .into());
    }

    // Single-value inline form (preserves existing single-result behavior).
    let expected_type_id = expected_result_type_ids.first().copied();
    let mut then_expr_type = expected_type_id
        .map(ExpectedType::Known)
        .unwrap_or(ExpectedType::Infer);
    let mut then_cast_target_context =
        cast_target_context_for_inline_branch(expected_type_id, type_interner, string_table);

    // An authored `else` keeps the existing bounded branch parse. Without one,
    // stop at the first receiving boundary so the missing-keyword diagnostic
    // retains that boundary's real source location.
    let else_follows = inline_else_follows_before_statement_end(token_stream);

    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: then_context,
        type_interner,
        expected_type: &mut then_expr_type,
        cast_target_context: &mut then_cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let then_expr = if else_follows {
        create_expression_until(input, &[TokenKind::Else])?
    } else {
        create_expression_until(
            input,
            &[
                TokenKind::Newline,
                TokenKind::End,
                TokenKind::Eof,
                TokenKind::Comma,
                TokenKind::CloseParenthesis,
                TokenKind::CloseCurly,
            ],
        )?
    };

    require_else_inline(token_stream, &then_location)?;
    token_stream.advance(); // consume `else`

    reject_else_then(token_stream)?;
    reject_newline_after_else(token_stream)?;

    reject_empty_value_after_else(token_stream)?;

    let mut else_expr_type = expected_type_id
        .map(ExpectedType::Known)
        .unwrap_or(ExpectedType::Infer);
    let mut else_cast_target_context =
        cast_target_context_for_inline_branch(expected_type_id, type_interner, string_table);
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: else_context,
            type_interner,
            expected_type: &mut else_expr_type,
            cast_target_context: &mut else_cast_target_context,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
        },
        false,
    );
    let else_expr = create_expression_with_trailing_newline_policy(input)?;

    if !same_logical_line(&then_location, &else_expr.location) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            else_expr.location.clone(),
        )
        .into());
    }

    let result_type_id = if let Some(expected_type_id) = expected_type_id {
        expected_type_id
    } else {
        infer_inline_result_type(
            then_expr.type_id,
            else_expr.type_id,
            None,
            type_interner,
            &then_expr.location,
            receiver_kind,
        )?
    };

    let mismatch_context = receiver_type_mismatch_context(receiver_kind);
    let then_expr = coerce_expression_to_explicit_type_boundary(
        then_expr,
        result_type_id,
        type_interner.environment(),
        then_context,
        mismatch_context,
    )?;
    let else_expr = coerce_expression_to_explicit_type_boundary(
        else_expr,
        result_type_id,
        type_interner.environment(),
        else_context,
        mismatch_context,
    )?;

    Ok(InlineThenElseOutput {
        then_values: vec![then_expr],
        else_values: vec![else_expr],
        result_type_id: Some(result_type_id),
        result_type_ids: if expected_result_type_ids.is_empty() {
            vec![result_type_id]
        } else {
            expected_result_type_ids.to_vec()
        },
    })
}

fn cast_target_context_for_inline_branch(
    expected_type_id: Option<TypeId>,
    type_interner: &AstTypeInterner<'_>,
    string_table: &StringTable,
) -> CastTargetContext {
    expected_type_id
        .map(|type_id| {
            cast_target_context_for_type_id(type_id, type_interner.environment(), string_table)
        })
        .unwrap_or(CastTargetContext::None)
}

/// Returns whether the current inline branch authors `else` before its statement ends.
///
/// WHAT: follows only expression-continuation newlines, while still recognising
/// a directly authored multiline `else`.
/// WHY: a later statement's unrelated `else` must not capture this value-if branch.
fn inline_else_follows_before_statement_end(token_stream: &FileTokens) -> bool {
    let stop_tokens = [
        TokenKind::Else,
        TokenKind::Newline,
        TokenKind::End,
        TokenKind::Eof,
        TokenKind::Comma,
        TokenKind::CloseParenthesis,
        TokenKind::CloseCurly,
    ];
    let mut scan_start = token_stream.index;

    loop {
        let boundary_index =
            find_expression_end_index(&token_stream.tokens, scan_start, &stop_tokens);
        let Some(boundary) = token_stream.tokens.get(boundary_index) else {
            return false;
        };

        match boundary.kind {
            TokenKind::Else => return true,
            TokenKind::Newline => {
                let previous_continues = boundary_index
                    .checked_sub(1)
                    .filter(|previous_index| *previous_index >= scan_start)
                    .is_some_and(|previous_index| {
                        token_stream.tokens[previous_index]
                            .kind
                            .continues_expression()
                    });
                let next_non_newline_index = token_stream
                    .tokens
                    .iter()
                    .enumerate()
                    .skip(boundary_index + 1)
                    .find(|(_, token)| token.kind != TokenKind::Newline)
                    .map(|(index, _)| index);
                let Some(next_non_newline_index) = next_non_newline_index else {
                    return false;
                };
                let next_kind = &token_stream.tokens[next_non_newline_index].kind;

                if next_kind == &TokenKind::Else {
                    return true;
                }
                if !previous_continues && !next_kind.continues_expression() {
                    return false;
                }

                scan_start = next_non_newline_index;
            }
            _ => return false,
        }
    }
}

/// Requires that the current token is `else` and that it is on the same logical line.
fn require_else_inline(
    token_stream: &FileTokens,
    then_location: &SourceLocation,
) -> InlineThenElseResult<()> {
    if token_stream.current_token_kind() != &TokenKind::Else {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfMissingElse,
            token_stream.current_location(),
        )
        .into());
    }

    if !same_logical_line(then_location, &token_stream.current_location()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(())
}

/// Rejects `else then`, which is never valid in inline value-producing `if`.
fn reject_else_then(token_stream: &FileTokens) -> InlineThenElseResult<()> {
    if token_stream.current_token_kind() == &TokenKind::Then {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfElseThen,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(())
}

/// Rejects a newline immediately after `else` in inline form.
fn reject_newline_after_else(token_stream: &FileTokens) -> InlineThenElseResult<()> {
    if token_stream.current_token_kind() == &TokenKind::Newline {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(())
}

/// Rejects an empty `else` branch at its first definite boundary.
fn reject_empty_value_after_else(token_stream: &FileTokens) -> InlineThenElseResult<()> {
    if is_missing_produced_value_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterElse,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(())
}
