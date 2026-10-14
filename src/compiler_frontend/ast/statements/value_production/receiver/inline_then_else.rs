//! Shared inline `then` / `else` parsing for value-producing receivers.
//!
//! WHAT: consumes the shared `then ... else ...` shape that appears in both
//! inline Bool value-if and inline single-predicate value-match.
//! WHY: these two syntactic forms previously duplicated structural validation
//! (newline rejection, `else then` rejection, same-line checks, and coercion).

use super::block_body::ValueBlockBranchBody;
use super::result_type::{infer_inline_result_type, receiver_type_mismatch_context};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestRange, IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, DeferredBody, NumericContinuation, StatementExpression, deferred_numeric_expression,
};
use crate::compiler_frontend::ast::statements::value_production::expression_build::then_value_node;
use crate::compiler_frontend::ast::statements::value_production::parse_values::{
    ProducedValuesParseInput, is_missing_produced_value_boundary, parse_retained_produced_values,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ThenValueContinuation, ThenValueSlot,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id,
};
use crate::compiler_frontend::utilities::token_scan::ExpressionBoundaryDepth;
use crate::compiler_frontend::value_mode::ValueMode;

/// True when every value of one inline branch completed at its parse position.
fn branch_values_are_ready(values: &[StatementExpression]) -> bool {
    values
        .iter()
        .all(|value| matches!(value, StatementExpression::Built(_)))
}

/// Input for the shared inline then/else parser.
pub(super) struct InlineThenElseInput<'a, 'b, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) then_context: &'a ScopeContext,
    pub(super) else_context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}
/// Output of the shared inline then/else parser.
pub(super) struct InlineThenElseOutput {
    pub(super) then_span: Option<SourceSpan>,
    pub(super) else_span: Option<SourceSpan>,
    pub(super) then_values: Vec<ExprId>,
    pub(super) else_values: Vec<ExprId>,
    pub(super) needs_slot_inference: bool,
    pub(super) result_type_ids: Vec<TypeId>,
    pub(super) generic_request_ranges: IfGenericRequestRanges,
}

struct ParsedInlineBranchValues {
    then_span: Option<SourceSpan>,
    else_span: Option<SourceSpan>,
    then_values: Vec<StatementExpression>,
    else_values: Vec<StatementExpression>,
    generic_request_ranges: IfGenericRequestRanges,
}

/// The result of parsing the shared inline `then`/`else` shape.
///
/// WHAT: either both branches completed where they were parsed, or at least one branch kept a
///       numeric-dependent value that only the enclosing body root can resolve.
/// WHY: an inline value-if or value-match is an expression over branch values, so a branch that
///      retained a numeric origin travels as the whole authored block instead of escaping as a
///      non-owning deferral that no construct can resume.
pub(super) enum InlineThenElseOutcome {
    /// Both branches completed at their parse position.
    Complete(InlineThenElseOutput),
    /// At least one branch holds values whose numeric origins the body root must select first.
    Retained {
        then_body: InlineBranchBody,
        else_body: InlineBranchBody,
        generic_request_ranges: IfGenericRequestRanges,
    },
}

/// One parsed inline branch body.
///
/// WHAT: the branch's authored `then` values, or the draft that keeps them for the body root.
/// WHY: the two shapes must stay distinct — a concrete branch publishes directly, while a
///      retained branch resumes through the session's single construction state.
pub(super) enum InlineBranchBody {
    /// The branch completed at its parse position; these values publish as one `then` node.
    Ready {
        values: Vec<ExprId>,
        span: Option<SourceSpan>,
        scope: PathId,
    },
    /// The branch retained numeric continuations for the enclosing body root.
    Retained(DeferredBody),
}

impl InlineBranchBody {
    /// The authored body this branch contributes to its value block.
    pub(super) fn into_value_block_body(self) -> ValueBlockBranchBody {
        match self {
            Self::Ready {
                values,
                span,
                scope,
            } => ValueBlockBranchBody::Nodes {
                nodes: vec![then_value_node(values, span, scope)],
                scope,
            },
            Self::Retained(body) => ValueBlockBranchBody::Draft(body),
        }
    }
}

/// Turns one branch's parsed values into its authored body.
///
/// WHAT: publishes a fully built branch directly, and registers a branch that holds any retained
///       value as one `then` continuation inside a fresh draft.
/// WHY: the enclosing body root owns numeric resolution, so a retained branch keeps its values,
///       scope and authored span until the session resolves them at the resolved receiving slots —
///       never materialised early and never escaped as a non-owning deferral.
fn inline_branch_body(
    values: Vec<StatementExpression>,
    span: Option<SourceSpan>,
    scope: PathId,
    context: &ScopeContext,
    target: &ActiveValueProductionTarget,
) -> InlineThenElseResult<InlineBranchBody> {
    if values
        .iter()
        .all(|value| matches!(value, StatementExpression::Built(_)))
    {
        return Ok(InlineBranchBody::Ready {
            values: into_built_values(values)?,
            span,
            scope,
        });
    }

    let continuation = context.register_numeric_continuation(NumericContinuation::ThenValue(
        ThenValueContinuation {
            values: values.into_iter().map(ThenValueSlot::Pending).collect(),
            slot_expected_types: target.slot_expected_types(),
            receiver_kind: target.receiver_kind,
            span,
            scope,
        },
    ))?;
    let mut draft = BodyDraft::new();
    draft.push_pending(continuation);

    Ok(InlineBranchBody::Retained(DeferredBody { draft, scope }))
}

/// Extracts the built expressions of a fully built branch value list.
fn into_built_values(values: Vec<StatementExpression>) -> InlineThenElseResult<Vec<ExprId>> {
    let mut built = Vec::with_capacity(values.len());
    for value in values {
        let StatementExpression::Built(expression) = value else {
            return Err(CompilerError::compiler_error(
                "an inline branch reported ready while holding a retained value",
            )
            .into());
        };
        built.push(expression);
    }

    Ok(built)
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
/// and reads each branch through `parse_retained_produced_values`.
/// WHY: known multi-value receivers and inferred multi-bind share this grammar, and each branch
/// owns its numeric deferral so a pending value travels to the body root rather than escaping;
/// slot inference stays with the multi-bind owner after these values are collected.
#[allow(
    clippy::too_many_arguments,
    reason = "inline branch parsing keeps the cursor, both scopes, target and mutable state separate"
)]
fn parse_inline_then_else_with_target(
    token_stream: &mut AstCursor,
    then_context: &ScopeContext,
    else_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
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

    let then_values = parse_retained_produced_values(ProducedValuesParseInput {
        token_stream,
        context: then_context,
        type_interner,
        expression_builder,
        const_values,
        target,
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

    let else_values = parse_retained_produced_values(ProducedValuesParseInput {
        token_stream,
        context: else_context,
        type_interner,
        expression_builder,
        const_values,
        target,
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
) -> InlineThenElseResult<InlineThenElseOutcome> {
    let InlineThenElseInput {
        token_stream,
        then_context,
        else_context,
        type_interner,
        expression_builder,
        const_values,
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
            expression_builder,
            const_values,
            &target,
            string_table,
            path_fork,
        )?;

        // A branch that retained a numeric origin keeps the whole inline block for the body root,
        // where this block's own constructor runs once with the resolved values.
        if !branch_values_are_ready(&parsed.then_values)
            || !branch_values_are_ready(&parsed.else_values)
        {
            let then_body = inline_branch_body(
                parsed.then_values,
                parsed.then_span,
                then_context.scope,
                then_context,
                &target,
            )?;
            let else_body = inline_branch_body(
                parsed.else_values,
                parsed.else_span,
                else_context.scope,
                else_context,
                &target,
            )?;

            return Ok(InlineThenElseOutcome::Retained {
                then_body,
                else_body,
                generic_request_ranges: parsed.generic_request_ranges,
            });
        }

        let then_values = into_built_values(parsed.then_values)?;
        let else_values = into_built_values(parsed.else_values)?;

        if target.needs_slot_inference() {
            return Ok(InlineThenElseOutcome::Complete(InlineThenElseOutput {
                then_span: parsed.then_span,
                else_span: parsed.else_span,
                then_values,
                else_values,
                needs_slot_inference: true,
                result_type_ids: Vec::new(),
                generic_request_ranges: parsed.generic_request_ranges,
            }));
        }

        let result_type_ids = expected_result_type_ids;

        return Ok(InlineThenElseOutcome::Complete(InlineThenElseOutput {
            then_span: parsed.then_span,
            else_span: parsed.else_span,
            then_values,
            else_values,
            needs_slot_inference: false,
            result_type_ids,
            generic_request_ranges: parsed.generic_request_ranges,
        }));
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
        expression_builder,
        const_values,
        expected_type: &mut then_expr_type,
        cast_target_context: &mut then_cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
        path_fork,
    });
    // This branch owns its numeric deferral: a value that still depends on an unresolved origin
    // is retained for the body root instead of escaping as a non-owning failure.
    let mut input = input.with_numeric_deferral_owner();
    if expected_type_id.is_none() {
        // An unresolved slot has no destination yet, so a literal-only branch must stay
        // unresolved too; a known slot materialises it through its own expectation.
        input = input.with_retained_pending_numeric_fragment();
    }
    let then_stops: &[TokenTag] = if else_follows {
        &[TokenTag::ELSE]
    } else {
        &[
            TokenTag::NEWLINE,
            TokenTag::END,
            TokenTag::EOF,
            TokenTag::COMMA,
            TokenTag::CLOSE_PARENTHESIS,
            TokenTag::CLOSE_CURLY,
        ]
    };
    let then_value =
        match create_expression_until(input, then_stops).map(|expression| expression.id()) {
            Ok(expression) => StatementExpression::Built(expression),
            Err(error) => {
                let deferral = error.take_numeric_deferral(
                    "an inline value-if branch whose receiving site is not resolved yet",
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
    let then_request_end = then_context.generic_request_checkpoint();

    if else_follows {
        // An authored `else` on a later line still belongs to this value-if: the scan above only
        // follows expression-continuation newlines, so the cursor rests on the newline before it.
        // Skipping it lets `require_else_inline` report the authored multiline form instead of a
        // missing `else` at the first receiving boundary.
        token_stream.skip_newlines();
    }
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
            expression_builder,
            const_values,
            expected_type: &mut else_expr_type,
            cast_target_context: &mut else_cast_target_context,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
            path_fork,
        },
        false,
    );
    // The else branch owns its numeric deferral for the same reason as the then branch.
    let mut input = input.with_numeric_deferral_owner();
    if expected_type_id.is_none() {
        input = input.with_retained_pending_numeric_fragment();
    }
    let else_value = match create_expression_with_trailing_newline_policy(input)
        .map(|expression| expression.id())
    {
        Ok(expression) => StatementExpression::Built(expression),
        Err(error) => {
            let deferral = error.take_numeric_deferral(
                "an inline value-if branch whose receiving site is not resolved yet",
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
    let else_request_end = else_context.generic_request_checkpoint();

    if !same_logical_line(token_stream, then_index, else_expression_start_index) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::InlineValueIfMultiline,
            else_span,
        )
        .into());
    }

    // A branch that still depends on an unresolved numeric origin keeps the whole inline block:
    // the enclosing body root resolves the values and re-runs this form's constructor once.
    if !matches!(then_value, StatementExpression::Built(_))
        || !matches!(else_value, StatementExpression::Built(_))
    {
        let then_body = inline_branch_body(
            vec![then_value],
            then_span,
            then_context.scope,
            then_context,
            &target,
        )?;
        let else_body = inline_branch_body(
            vec![else_value],
            else_span,
            else_context.scope,
            else_context,
            &target,
        )?;

        return Ok(InlineThenElseOutcome::Retained {
            then_body,
            else_body,
            generic_request_ranges: IfGenericRequestRanges {
                then_branch: GenericRequestRange::new(then_request_start, then_request_end),
                else_branch: GenericRequestRange::new(then_request_end, else_request_end),
            },
        });
    }

    let StatementExpression::Built(then_expr) = then_value else {
        return Err(CompilerError::compiler_error(
            "an inline then branch reported ready while holding a retained value",
        )
        .into());
    };
    let StatementExpression::Built(else_expr) = else_value else {
        return Err(CompilerError::compiler_error(
            "an inline else branch reported ready while holding a retained value",
        )
        .into());
    };

    let then_type = expression_builder
        .view(then_expr)?
        .require_single(ValueReceiverContext::Then)?;
    let else_type = expression_builder
        .view(else_expr)?
        .require_single(ValueReceiverContext::Then)?;

    let result_type_id = if let Some(expected_type_id) = expected_type_id {
        expected_type_id
    } else {
        infer_inline_result_type(
            then_type,
            else_type,
            None,
            type_interner,
            expression_builder.view(then_expr)?.span(),
            receiver_kind,
        )?
    };

    let mismatch_context = receiver_type_mismatch_context(receiver_kind);
    let then_expr = coerce_expression_to_explicit_type_boundary(
        expression_builder.own(then_expr),
        result_type_id,
        type_interner.environment(),
        mismatch_context,
        expression_builder,
    )
    .map(|expression| expression.id())?;
    let else_expr = coerce_expression_to_explicit_type_boundary(
        expression_builder.own(else_expr),
        result_type_id,
        type_interner.environment(),
        mismatch_context,
        expression_builder,
    )
    .map(|expression| expression.id())?;

    Ok(InlineThenElseOutcome::Complete(InlineThenElseOutput {
        then_span,
        else_span,
        then_values: vec![then_expr],
        else_values: vec![else_expr],
        needs_slot_inference: false,
        result_type_ids: if expected_result_type_ids.is_empty() {
            vec![result_type_id]
        } else {
            expected_result_type_ids.to_vec()
        },
        generic_request_ranges: IfGenericRequestRanges {
            then_branch: GenericRequestRange::new(then_request_start, then_request_end),
            else_branch: GenericRequestRange::new(then_request_end, else_request_end),
        },
    }))
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
