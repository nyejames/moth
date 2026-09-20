//! Shared inline `then` / `else` parsing for value-producing receivers.
//!
//! WHAT: consumes the shared `then ... else ...` shape that appears in both
//! inline Bool value-if and inline single-predicate value-match.
//! WHY: these two syntactic forms previously duplicated structural validation
//! (newline rejection, `else then` rejection, same-line checks, and coercion).

use super::result_type::{infer_inline_result_type, receiver_type_mismatch_context};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestRange, IfGenericRequestRanges,
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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id,
};
use crate::compiler_frontend::utilities::token_scan::ExpressionBoundaryDepth;
use crate::compiler_frontend::value_mode::ValueMode;

/// Input for the shared inline then/else parser.
pub(super) struct InlineThenElseInput<'a, 'b, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) then_context: &'a ScopeContext,
    pub(super) else_context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}
/// Output of the shared inline then/else parser.
pub(super) struct InlineThenElseOutput {
    pub(super) then_span: Option<SourceSpan>,
    pub(super) else_span: Option<SourceSpan>,
    pub(super) then_values: Vec<Expression>,
    pub(super) else_values: Vec<Expression>,
    pub(super) result_type_id: Option<TypeId>,
    pub(super) result_type_ids: Vec<TypeId>,
    pub(super) generic_request_ranges: IfGenericRequestRanges,
}

struct ParsedInlineBranchValues {
    then_span: Option<SourceSpan>,
    else_span: Option<SourceSpan>,
    then_values: Vec<Expression>,
    else_values: Vec<Expression>,
    generic_request_ranges: IfGenericRequestRanges,
}

/// Returns `true` when both source locations are on the same logical line.
///
/// Returns `true` when no explicit newline token separates two token positions.
///
/// WHAT: used to enforce that inline value-if/match arms stay on one line.
/// WHY: token spans intentionally carry byte ranges only; newline tokens are the
/// parser's retained physical-line boundary and avoid reconstructing locations.
pub(in crate::compiler_frontend::ast::statements::value_production) fn same_logical_line(
    token_stream: &AstCursor,
    left_index: usize,
    right_index: usize,
) -> bool {
    let (start, end) = if left_index <= right_index {
        (left_index, right_index)
    } else {
        (right_index, left_index)
    };
    (start..=end).all(|index| {
        token_stream
            .token_ref_at(index)
            .is_some_and(|token| token.tag() != TokenTag::NEWLINE)
    })
}
/// converting a retained-token lifecycle fault into a source diagnostic mid-parse.
type InlineThenElseResult<T> = Result<T, ExpressionParseError>;

/// Parses inline `then` / `else` values against an active production target.
///
/// WHAT: assumes the current token is `then`. Consumes the shared one-line shape
/// and reads each branch through `parse_produced_values_typed`.
/// WHY: known multi-value receivers and inferred multi-bind share this grammar;
/// slot inference stays with the multi-bind owner after these values are collected.
fn parse_inline_then_else_with_target(
    token_stream: &mut AstCursor,
    then_context: &ScopeContext,
    else_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    target: &ActiveValueProductionTarget,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> InlineThenElseResult<ParsedInlineBranchValues> {
    let then_request_start = then_context.generic_request_checkpoint();
    let then_index = token_stream.position();
    let then_span = Some(token_stream.current_span());
    token_stream.advance(); // consume `then`

    if token_stream.current_tag() == TokenTag::NEWLINE {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // A retained newline is a multiline form. Every other definite boundary means
    // the branch has no value and must not reach expression evaluation.
    if is_missing_produced_value_boundary(token_stream.current_tag()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterThen,
            Some(token_stream.current_span()),
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
        path_fork,
    })?;
    let then_request_end = then_context.generic_request_checkpoint();

    require_else_inline(token_stream, then_index)?;
    let else_span = Some(token_stream.current_span());
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
        path_fork,
    })?;
    let else_request_end = else_context.generic_request_checkpoint();

    Ok(ParsedInlineBranchValues {
        then_values,
        else_values,
        then_span,
        else_span,
        generic_request_ranges: IfGenericRequestRanges {
            then_branch: GenericRequestRange::new(then_request_start, then_request_end),
            else_branch: GenericRequestRange::new(then_request_end, else_request_end),
        },
    })
}

/// Parses the shared `then <branch> else <branch>` inline shape.
///
/// WHAT: assumes the current token is `then`. Consumes it, parses then and else
/// branches, validates same-line constraints, infers/coerces the result type.
/// WHY: consolidates duplicated logic from inline Bool value-if and inline
/// single-predicate value-match.
pub(super) fn parse_inline_then_else(
    input: InlineThenElseInput<'_, '_, '_>,
) -> InlineThenElseResult<InlineThenElseOutput> {
    let InlineThenElseInput {
        token_stream,
        then_context,
        else_context,
        type_interner,
        target,
        string_table,
        path_fork,
    } = input;
    let expected_result_type_ids = target.result_type_ids.clone();
    let receiver_kind = target.receiver_kind;

    if expected_result_type_ids.len() > 1 || target.expected_arity.is_some_and(|arity| arity > 1) {
        let parsed = parse_inline_then_else_with_target(
            token_stream,
            then_context,
            else_context,
            type_interner,
            &target,
            string_table,
            path_fork,
        )?;
        if target.needs_slot_inference() {
            return Ok(InlineThenElseOutput {
                then_span: parsed.then_span,
                else_span: parsed.else_span,
                then_values: parsed.then_values,
                else_values: parsed.else_values,
                result_type_id: None,
                result_type_ids: Vec::new(),
                generic_request_ranges: parsed.generic_request_ranges,
            });
        }

        let result_type_ids = expected_result_type_ids;
        let result_type_id = type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(result_type_ids.clone());

        return Ok(InlineThenElseOutput {
            then_span: parsed.then_span,
            else_span: parsed.else_span,
            then_values: parsed.then_values,
            else_values: parsed.else_values,
            result_type_id: Some(result_type_id),
            result_type_ids,
            generic_request_ranges: parsed.generic_request_ranges,
        });
    }

    let then_index = token_stream.position();
    let then_span = Some(token_stream.current_span());
    let then_request_start = then_context.generic_request_checkpoint();
    token_stream.advance(); // consume `then`

    if token_stream.current_tag() == TokenTag::NEWLINE {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // A retained newline is a multiline form. Every other definite boundary means
    // the branch has no value and must not reach expression evaluation.
    if is_missing_produced_value_boundary(token_stream.current_tag()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterThen,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Single-value inline form (preserves existing single-result behavior).
    let expected_type_id = expected_result_type_ids.first().copied();
    let mut then_expr_type = expected_type_id
        .map(ExpectedType::Known)
        .unwrap_or(ExpectedType::Infer);
    let mut then_cast_target_context = cast_target_context_for_inline_branch(
        expected_type_id,
        type_interner,
        string_table,
        path_fork,
    );

    // An authored `else` keeps the existing bounded branch parse. Without one,
    // stop at the first receiving boundary so the missing-keyword diagnostic
    let else_follows = inline_else_follows_before_statement_end(token_stream);

    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: then_context,
        type_interner,
        expected_type: &mut then_expr_type,
        cast_target_context: &mut then_cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
        path_fork,
    });
    let then_expr = if else_follows {
        create_expression_until(input, &[TokenTag::ELSE])?
    } else {
        create_expression_until(
            input,
            &[
                TokenTag::NEWLINE,
                TokenTag::END,
                TokenTag::EOF,
                TokenTag::COMMA,
                TokenTag::CLOSE_PARENTHESIS,
                TokenTag::CLOSE_CURLY,
            ],
        )?
    };
    let then_request_end = then_context.generic_request_checkpoint();

    require_else_inline(token_stream, then_index)?;
    let else_span = Some(token_stream.current_span());
    token_stream.advance(); // consume `else`

    reject_else_then(token_stream)?;
    reject_newline_after_else(token_stream)?;

    reject_empty_value_after_else(token_stream)?;

    let mut else_expr_type = expected_type_id
        .map(ExpectedType::Known)
        .unwrap_or(ExpectedType::Infer);
    let mut else_cast_target_context = cast_target_context_for_inline_branch(
        expected_type_id,
        type_interner,
        string_table,
        path_fork,
    );
    let else_expression_start_index = token_stream.position();
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: else_context,
            type_interner,
            expected_type: &mut else_expr_type,
            cast_target_context: &mut else_cast_target_context,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
            path_fork,
        },
        false,
    );
    let else_expr = create_expression_with_trailing_newline_policy(input)?;
    let else_request_end = else_context.generic_request_checkpoint();

    if !same_logical_line(token_stream, then_index, else_expression_start_index) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            else_expr.span,
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
            then_expr.span,
            receiver_kind,
        )?
    };

    let mismatch_context = receiver_type_mismatch_context(receiver_kind);
    let then_expr = coerce_expression_to_explicit_type_boundary(
        then_expr,
        result_type_id,
        type_interner.environment(),
        mismatch_context,
    )?;
    let else_expr = coerce_expression_to_explicit_type_boundary(
        else_expr,
        result_type_id,
        type_interner.environment(),
        mismatch_context,
    )?;

    Ok(InlineThenElseOutput {
        then_span,
        else_span,
        then_values: vec![then_expr],
        else_values: vec![else_expr],
        result_type_id: Some(result_type_id),
        result_type_ids: if expected_result_type_ids.is_empty() {
            vec![result_type_id]
        } else {
            expected_result_type_ids.to_vec()
        },
        generic_request_ranges: IfGenericRequestRanges {
            then_branch: GenericRequestRange::new(then_request_start, then_request_end),
            else_branch: GenericRequestRange::new(then_request_end, else_request_end),
        },
    })
}

fn cast_target_context_for_inline_branch(
    expected_type_id: Option<TypeId>,
    type_interner: &AstTypeInterner<'_>,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> CastTargetContext {
    expected_type_id
        .map(|type_id| {
            cast_target_context_for_type_id(
                type_id,
                type_interner.environment(),
                string_table,
                path_fork,
            )
        })
        .unwrap_or(CastTargetContext::None)
}

/// Returns whether the current inline branch authors `else` before its statement ends.
///
/// WHAT: follows only expression-continuation newlines, while still recognising
/// a directly authored multiline `else`.
/// WHY: a later statement's unrelated `else` must not capture this value-if branch.
fn inline_else_follows_before_statement_end(token_stream: &mut AstCursor) -> bool {
    let stop_tokens = [
        TokenTag::ELSE,
        TokenTag::NEWLINE,
        TokenTag::END,
        TokenTag::EOF,
        TokenTag::COMMA,
        TokenTag::CLOSE_PARENTHESIS,
        TokenTag::CLOSE_CURLY,
    ];
    let resume = token_stream.position();
    let follows = inline_else_follows_from_position(token_stream, &stop_tokens);
    token_stream
        .set_position(resume)
        .expect("inline else scan resume stays inside the active parser view");
    follows
}

/// Walk the live cursor forward across expression-continuation newlines.
///
/// The caller owns the single position restore.
fn inline_else_follows_from_position(
    token_stream: &mut AstCursor,
    stop_tokens: &[TokenTag],
) -> bool {
    loop {
        let scan_start = token_stream.position();
        let mut depth = ExpressionBoundaryDepth::default();
        while !token_stream.is_at_end() {
            if depth.is_top_level() && stop_tokens.contains(&token_stream.current_tag()) {
                break;
            }
            depth.step_tag(token_stream.current_tag());
            if token_stream.current_tag() == TokenTag::EOF {
                break;
            }
            token_stream.advance();
        }
        if token_stream.is_at_end() {
            return false;
        }

        match token_stream.current_tag() {
            TokenTag::ELSE => return true,
            TokenTag::NEWLINE => {}
            _ => return false,
        }

        // The branch survives this newline only when the expression continues
        // across it, either from the token before or the one after.
        let previous_continues = token_stream.position() > scan_start
            && token_stream
                .previous()
                .is_some_and(|token| token.tag().continues_expression());

        token_stream.advance();
        token_stream.skip_newlines();
        if token_stream.is_at_end() {
            return false;
        }

        let next_tag = token_stream.current_tag();
        if next_tag == TokenTag::ELSE {
            return true;
        }
        if !previous_continues && !next_tag.continues_expression() {
            return false;
        }
    }
}

/// Requires that the current token is `else` and that it is on the same logical line.
fn require_else_inline(token_stream: &AstCursor, then_index: usize) -> InlineThenElseResult<()> {
    if token_stream.current_tag() != TokenTag::ELSE {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfMissingElse,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if !same_logical_line(token_stream, then_index, token_stream.position()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

/// Rejects `else then`, which is never valid in inline value-producing `if`.
fn reject_else_then(token_stream: &AstCursor) -> InlineThenElseResult<()> {
    if token_stream.current_tag() == TokenTag::THEN {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfElseThen,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

/// Rejects a newline immediately after `else` in inline form.
fn reject_newline_after_else(token_stream: &AstCursor) -> InlineThenElseResult<()> {
    if token_stream.current_tag() == TokenTag::NEWLINE {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

/// Rejects an empty `else` branch at its first definite boundary.
fn reject_empty_value_after_else(token_stream: &AstCursor) -> InlineThenElseResult<()> {
    if is_missing_produced_value_boundary(token_stream.current_tag()) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedValueAfterElse,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}
