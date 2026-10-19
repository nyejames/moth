//! Expression token dispatch helpers.
//!
//! WHAT: routes one token at a time through expression-position parsing.
//! WHY: keeps delimiter/grammar ownership explicit while specialized helpers own detailed token families.

use super::anonymous_const_record::parse_parenthesized_anonymous_const_record_expression;
use super::call_arguments::{ParenthesizedExpressionKind, classify_parenthesized_expression};
use super::error::ExpressionParseError;
use super::eval_expression::evaluate_expression;
use super::expression_kind::Operator;
use super::expression_rpn::{ExpressionRpnItem, trailing_operand_start};
use super::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, ExpressionView, OwnedExpr,
};
use super::option_propagation::parse_option_propagation_suffix_for_expression;
use super::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use super::parse_expression_identifiers::parse_identifier_or_call;
use super::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources, RouteArgumentContext,
};
use super::parse_expression_literals::{
    LiteralParseState, parse_literal_expression, typed_suffix_follows,
};
use super::parse_expression_places::{
    ParsedCopyPlace, parse_copy_place_expression, parse_mutable_receiver_expression,
};
use super::parse_expression_templates::parse_template_expression;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use crate::ast_log;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::field_access::{
    ParsedPostfixExpression, PostfixChainAccess, RetainedOperandInput, grouped_evaluation_expected,
    parse_postfix_chain_expression, retain_operand_call_propagation,
    retain_operand_handling_suffix, retain_pending_operand_member_access,
};
use crate::compiler_frontend::ast::file_value_resolution::resolve_file_value;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredExpressionSuffix, NumericBindingId, items_contain_unresolvable_numeric,
};
use crate::compiler_frontend::ast::statements::fallible_handling::{
    CastCatchSite, compatible_expression_error_type, fallible_catch_allowed_in_context,
    parse_cast_catch_handling_suffix, parse_fallible_handling_suffix_for_expression,
};
use crate::compiler_frontend::ast::statements::match_arm_boundaries::token_index_starts_match_arm_header;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::builtins::casts::resolution::{
    CastResolutionInput, resolve_cast_expression,
};
use crate::compiler_frontend::builtins::expression_parsing::parse_curly_literal_expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CompilerDiagnostic, DeferredFeatureReason, DiagnosticToken,
    InvalidBuiltinCallReason, InvalidCastReason, InvalidControlFlowStatementReason,
    InvalidExpressionReason, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::declaration_syntax::type_syntax::builtin_scalar_type_name_for_tag;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::expression_position::check_expression_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::utilities::token_scan::ExpressionBoundaryDepth;
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) enum ExpressionTokenStep {
    Continue,
    Advance,
    /// The current token closes the innermost group. The caller pops that group's frame.
    CloseGroup,
    Break,
    Return(OwnedExpr),
}

/// One open parenthesised group in the flat expression stream.
///
/// WHAT: records where the group's `GroupOpen` marker sits and the parent's inference state. The
///       close restores that state in place, so no group needs its own copy of the stream.
pub(super) struct GroupFrame {
    /// Index of the `GroupOpen` marker. The group's items start one slot after it.
    pub(super) open_index: usize,
    pub(super) opening_span: SourceSpan,
    pub(super) parent_expected_type: ExpectedType,
    pub(super) parent_cast_target_context: CastTargetContext,
    pub(super) parent_next_number_negative: bool,
    /// Set when a closed child group in this frame holds a construction-only numeric item. Each
    /// child close records it here, so this group's own close never rescans nested interiors.
    pub(super) child_unresolvable: bool,
}

impl GroupFrame {
    pub(super) fn item_start(&self) -> usize {
        self.open_index + 1
    }
}

pub(super) struct ExpressionDispatchState<'a> {
    pub(super) expected_type: &'a mut ExpectedType,
    pub(super) cast_target_context: &'a mut CastTargetContext,
    pub(super) value_mode: &'a ValueMode,
    pub(super) consume_closing_parenthesis: bool,
    pub(super) allow_boundary_catch: bool,
    pub(super) allow_expected_result_evidence: bool,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) next_number_negative: &'a mut bool,
    pub(super) retain_pending_numeric_fragment: bool,
    pub(super) group_frames: &'a mut Vec<GroupFrame>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) route_argument_context: Option<RouteArgumentContext>,
}

impl ExpressionDispatchState<'_> {
    /// Whether the parser is inside a parenthesised group.
    pub(super) fn in_group(&self) -> bool {
        !self.group_frames.is_empty()
    }
}

/// Items of the innermost group, or the whole fragment at the root. These take the two fields
/// rather than the whole state, so a caller can hold a disjoint mutable borrow of the builder.
fn frame_items<'a>(
    expression: &'a [ExpressionRpnItem],
    group_frames: &[GroupFrame],
) -> &'a [ExpressionRpnItem] {
    let start = group_frames.last().map_or(0, GroupFrame::item_start);
    expression.get(start..).unwrap_or_default()
}

/// Whether the innermost group, or the fragment at the root, has no items yet.
fn frame_is_empty(expression: &[ExpressionRpnItem], group_frames: &[GroupFrame]) -> bool {
    frame_items(expression, group_frames).is_empty()
}

/// Whether no operand or operator follows the innermost open group, or the root stream is empty.
///
/// WHAT: a group's `GroupOpen` is the last item exactly while the group's frame has no items,
///       because the group's items are appended after its `GroupOpen`.
/// WHY: identifier and namespace leaf resolution receive the flat stream without the frame stack.
///      For the same stream this gives the same answer as `frame_is_empty`.
pub(super) fn stream_is_boundary_leading(expression: &[ExpressionRpnItem]) -> bool {
    matches!(
        expression.last(),
        None | Some(ExpressionRpnItem::GroupOpen { .. })
    )
}

pub(super) struct ExpressionOperandInput {
    pub(super) operand: OwnedExpr,
    pub(super) wrapper_span: Option<SourceSpan>,
}

/// Reports adjacent operands at the second expression without guessing the missing operator.
fn reject_adjacent_operand(
    expression: &[ExpressionRpnItem],
    second_expression_span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    let previous_is_operand = expression
        .last()
        .is_some_and(ExpressionRpnItem::is_operand_shape);

    if previous_is_operand {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            second_expression_span,
        )
        .into());
    }

    Ok(())
}

fn unexpected_token_at_current(
    token_stream: &AstCursor,
    fallback: TokenTag,
    string_table: &mut StringTable,
) -> Result<CompilerDiagnostic, ExpressionParseError> {
    let span = Some(token_stream.current_span());
    let found = token_stream
        .current_diagnostic_token(string_table)
        .map_err(|error| {
            CompilerDiagnostic::token_view_invariant_error(
                error,
                "expression unexpected-token diagnostic",
            )
        })?;
    Ok(match found {
        Some(found) => CompilerDiagnostic::unexpected_token_from_tag(found, span),
        None => CompilerDiagnostic::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(fallback),
            span,
        ),
    })
}

fn is_value_operand_start_token(tag: TokenTag) -> bool {
    // The schema class is the canonical operand-start authority; `RAW_STRING_LITERAL` keeps its
    // legacy exclusion because raw strings never appear in value-operand position here.
    tag.is_operand_start() && tag != TokenTag::RAW_STRING_LITERAL
}
/// Skips value-less comment templates before checking what follows a value template.
fn reject_second_operand_after_value_template(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    while token_stream.current_tag() == TokenTag::TEMPLATE_HEAD {
        let next_template_start = Some(token_stream.current_span());
        let next_template = parse_template_expression(
            token_stream,
            context,
            type_interner,
            state.expression_builder,
            state.const_values,
            state.consume_closing_parenthesis,
            state.value_mode,
            string_table,
            path_fork,
        )?;

        if next_template.is_some() {
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::ExpectedOperatorBeforeExpression,
                next_template_start,
            )
            .into());
        }
    }

    if is_value_operand_start_token(token_stream.current_tag()) {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "suffix dispatch keeps the token stream, scope, mutable interner/string/rpn/path state, catch policy, and the postfix expression as separate borrows"
)]
fn push_expression_after_suffixes(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    expression_after_postfix: OwnedExpr,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    // ----------------------------
    //  Common mistake: `!=`
    // ----------------------------
    // Detect `!=` (Bang + Assign) before treating `!` as a result-handling suffix.
    if token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::BANG
        && token_stream.peek_next_tag() == Some(TokenTag::ASSIGN)
    {
        if let Some(error) = check_expression_common_mistake(token_stream, false) {
            return Err(error.into());
        }
        // Invariant: the condition above guarantees Bang+Assign, which
        // check_expression_common_mistake always matches. Reaching here is a compiler bug.
        return Err(CompilerError::compiler_error(
            "Bang+Assign pattern did not produce expected error",
        )
        .into());
    }

    // ----------------------------
    //  Fallible handling suffix
    // ----------------------------
    let expression_after_fallible = if token_stream.position() < token_stream.length()
        && (token_stream.current_tag() == TokenTag::BANG
            || (token_stream.current_tag() == TokenTag::SYMBOL
                && token_stream.peek_next_tag() == Some(TokenTag::BANG)))
    {
        let value_required = !expression_builder
            .view_owned(expression_after_postfix)?
            .success_types()
            .is_empty();
        parse_fallible_handling_suffix_for_expression(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            expression_after_postfix,
            value_required,
            allow_boundary_catch
                && expression.is_empty()
                && fallible_catch_allowed_in_context(context),
            string_table,
            path_fork,
        )?
    } else {
        expression_after_postfix
    };

    // ----------------------------
    //  Option propagation suffix
    // ----------------------------
    let expression_after_option_propagation = if token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::QUESTION_MARK
    {
        parse_option_propagation_suffix_for_expression(
            token_stream,
            context,
            type_interner,
            expression_builder,
            expression_after_fallible,
        )?
    } else {
        expression_after_fallible
    };

    // ----------------------------
    //  Const record validation
    // ----------------------------
    // Const records are field-access-only. After postfix parsing and fallible
    // handling, reject any expression that resolves to a const-record value in a
    // runtime context. Identifier-level parsing already catches bare names; this
    // catches field chains whose final step is itself a const record.
    if !context.kind.is_constant_context()
        && expression_builder
            .view_owned(expression_after_option_propagation)?
            .is_const_record_value()
    {
        let record_name = const_record_expression_name(
            expression_builder.view_owned(expression_after_option_propagation)?,
            string_table,
            path_fork,
        );
        return Err(CompilerDiagnostic::const_record_used_as_value(
            record_name,
            expression_builder
                .view_owned(expression_after_option_propagation)?
                .span(),
        )
        .into());
    }

    expression.push(ExpressionRpnItem::Operand(
        expression_after_option_propagation.id(),
    ));
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "operand dispatch keeps the token stream, scope, mutable interner/string/rpn/path state, catch policy, and the operand expression as separate borrows"
)]
pub(super) fn push_expression_operand(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    operand: OwnedExpr,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let wrapper_span = expression_builder.view_owned(operand)?.span();
    push_expression_operand_with_span(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        expression,
        allow_boundary_catch,
        ExpressionOperandInput {
            operand,
            wrapper_span,
        },
        path_fork,
    )
}
/// Push an expression operand while preserving a distinct wrapper span for suffix parsing.
///
/// WHAT: keeps postfix/fallible/option handling on the existing dispatch path without requiring
/// callers to construct `NodeKind::ExpressionStatement` themselves.
/// WHY: constant references may carry declaration-origin expression spans, but diagnostics
/// for suffixes and const-record misuse should still point at the source use site.
#[allow(
    clippy::too_many_arguments,
    reason = "operand dispatch keeps the token stream, scope, mutable interner/string/rpn/path state, catch policy, and the spanned operand input as separate borrows"
)]
pub(super) fn push_expression_operand_with_span(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    operand_input: ExpressionOperandInput,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    expression_builder.view_owned(operand_input.operand)?;
    let expression_after_postfix = if token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::DOT
    {
        parse_postfix_chain_expression(
            token_stream,
            operand_input.operand,
            operand_input.wrapper_span,
            PostfixChainAccess::shared(),
            allow_boundary_catch,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?
    } else {
        ParsedPostfixExpression::Value(operand_input.operand)
    };

    // A retained receiver call takes its authored operand position as one pending item; the
    // enclosing fragment either completes where it was parsed (when its own receiving boundary
    // can) or defers to the body root with this item in place, so the call resumes exactly once.
    let expression_after_postfix = match expression_after_postfix {
        ParsedPostfixExpression::Value(value) => value,
        ParsedPostfixExpression::RetainedOperand(item) => {
            expression.push(item);
            return Ok(());
        }
    };

    push_expression_after_suffixes(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        expression,
        allow_boundary_catch,
        expression_after_postfix,
        path_fork,
    )
}

/// Extracts a display name for a const-record diagnostic from an expression.
///
/// WHAT: walks field-access chains back to the root identifier so the
/// diagnostic example points at the record, not an intermediate field.
fn const_record_expression_name(
    expression: ExpressionView<'_>,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> StringId {
    match expression.operation() {
        ExpressionOperationView::FieldAccess { base, .. } => {
            const_record_expression_name(base, string_table, path_fork)
        }

        ExpressionOperationView::Reference(path) => path_fork
            .component(path)
            .unwrap_or_else(|| string_table.intern("record")),

        _ => string_table.intern("record"),
    }
}

/// Pushes a unary operator item onto the expression stack when the current token
/// is `Negative` or `Not`. Returns `true` when an operator was consumed.
fn parse_unary_operator(
    token_stream: &AstCursor,
    _context: &ScopeContext,
    expression: &mut Vec<ExpressionRpnItem>,
    next_number_negative: &mut bool,
) -> bool {
    match token_stream.current_tag() {
        TokenTag::NEGATIVE => {
            if token_stream.peek_next_tag() == Some(TokenTag::NUMERIC_LITERAL) {
                *next_number_negative = true;
            } else {
                // Token-local postfix span comes from the AstCursor view.
                let span = Some(token_stream.current_postfix_operator_span());
                expression.push(ExpressionRpnItem::Operator {
                    operator: Operator::Negate,
                    span,
                });
            }
            true
        }
        TokenTag::NOT => {
            let span = Some(token_stream.current_postfix_operator_span());
            expression.push(ExpressionRpnItem::Operator {
                operator: Operator::Not,
                span,
            });
            true
        }
        _ => false,
    }
}

fn push_operator_item(
    expression: &mut Vec<ExpressionRpnItem>,
    _context: &ScopeContext,
    span: Option<SourceSpan>,
    operator: Operator,
) {
    expression.push(ExpressionRpnItem::Operator { operator, span });
}

/// Convenience for the common match arm that pushes an operator and advances.
fn advance_with_operator(
    expression: &mut Vec<ExpressionRpnItem>,
    context: &ScopeContext,
    token_stream: &AstCursor,
    operator: Operator,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    let span = Some(token_stream.current_postfix_operator_span());
    push_operator_item(expression, context, span, operator);
    Ok(ExpressionTokenStep::Advance)
}

pub(super) fn dispatch_expression_token(
    token: TokenTag,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    // Reject definite adjacency before semantic name, call or constructor parsing.
    if is_value_operand_start_token(token) {
        reject_adjacent_operand(state.expression, Some(token_stream.current_span()))?;
    }

    // Opening a group only pushes a marker and a frame, so it never recurses.
    if token == TokenTag::OPEN_PARENTHESIS {
        return dispatch_parenthesized_expression(
            token_stream,
            context,
            type_interner,
            state,
            string_table,
            path_fork,
        );
    }

    dispatch_non_group_expression_token(
        token,
        token_stream,
        context,
        type_interner,
        state,
        string_table,
        path_fork,
    )
}

#[inline(never)]
fn dispatch_non_group_expression_token(
    token: TokenTag,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    let expression_builder = &mut *state.expression_builder;
    let const_values = &mut *state.const_values;
    // This state machine is intentionally flat: each token either appends one AST node, advances
    // past a nested parse, or signals the caller that the surrounding grammar owns the delimiter.
    match token {
        // Recovery selects the completed ordinary expression, never the last primary.
        TokenTag::CATCH => {
            if !state.allow_boundary_catch || !fallible_catch_allowed_in_context(context) {
                return Err(CompilerDiagnostic::invalid_fallible_handling(
                    InvalidFallibleHandlingReason::CatchOutsideBoundary,
                    Some(token_stream.current_span()),
                )
                .into());
            }
            Ok(ExpressionTokenStep::Break)
        }

        TokenTag::CLOSE_CURLY
        | TokenTag::COMMA
        | TokenTag::EOF
        | TokenTag::TEMPLATE_CLOSE
        | TokenTag::ARROW
        | TokenTag::START_TEMPLATE_BODY
        | TokenTag::COLON
        | TokenTag::ELSE
        | TokenTag::END => dispatch_delimiter_token(token, token_stream, state, string_table),

        TokenTag::CLOSE_PARENTHESIS => {
            dispatch_close_parenthesis(token, token_stream, state, string_table)
        }

        tag if tag.is_builtin_scalar_type_name() => {
            if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
                let cast_name = builtin_scalar_type_name_for_tag(token)
                    .map(|spelling| string_table.intern(spelling));
                return Err(CompilerDiagnostic::invalid_builtin_call(
                    InvalidBuiltinCallReason::ScalarConstructorRemoved,
                    cast_name,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            if let Some(error) = check_expression_common_mistake(
                token_stream,
                frame_is_empty(state.expression, state.group_frames),
            ) {
                return Err(error.into());
            }

            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        TokenTag::OPEN_CURLY => {
            parse_curly_literal_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                state.expected_type,
                state.value_mode,
                state.retain_pending_numeric_fragment,
                state.expression,
                string_table,
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Advance)
        }

        TokenTag::NEWLINE => dispatch_newline(token_stream, context, state),

        TokenTag::SYMBOL | TokenTag::THIS => {
            parse_identifier_or_call(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                state.expression,
                state.allow_boundary_catch,
                state.allow_expected_result_evidence,
                state.route_argument_context,
                string_table,
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenTag::MUTABLE => {
            parse_mutable_receiver_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                state.expression,
                state.allow_boundary_catch,
                string_table,
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenTag::NUMERIC_LITERAL
        | TokenTag::STRING_SLICE_LITERAL
        | TokenTag::BOOL_LITERAL
        | TokenTag::CHAR_LITERAL
        | TokenTag::NONE_LITERAL => {
            let mut literal_state = LiteralParseState {
                expected_type: state.expected_type,
                value_mode: state.value_mode,
                expression: state.expression,
                next_number_negative: state.next_number_negative,
                allow_boundary_catch: state.allow_boundary_catch,
            };
            parse_literal_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                &mut literal_state,
                string_table,
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenTag::PATH => {
            let path_span = Some(token_stream.current_postfix_operator_span());
            let path_syntax = token_stream
                .current()
                .and_then(|token| token.path_syntax_id())
                .ok_or_else(|| CompilerError::compiler_error("path token had no payload"))?;
            token_stream.current_path_syntax()?.ok_or_else(|| {
                CompilerError::compiler_error("path token had no validated syntax row")
            })?;
            let operand = resolve_file_value(
                path_syntax,
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                state.value_mode,
                string_table,
                path_fork,
            )?;
            token_stream.advance();
            push_expression_operand_with_span(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                ExpressionOperandInput {
                    operand,
                    wrapper_span: path_span,
                },
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenTag::TEMPLATE_HEAD => {
            let template_expression = parse_template_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                state.consume_closing_parenthesis,
                state.value_mode,
                string_table,
                path_fork,
            )?;

            let Some(template_expression) = template_expression else {
                return Ok(ExpressionTokenStep::Continue);
            };

            reject_adjacent_operand(state.expression, Some(token_stream.current_span()))?;
            reject_second_operand_after_value_template(
                token_stream,
                context,
                type_interner,
                state,
                string_table,
                path_fork,
            )?;

            Ok(ExpressionTokenStep::Return(template_expression))
        }

        TokenTag::COPY => {
            let copy_span = Some(token_stream.current_span());
            token_stream.advance();

            match parse_copy_place_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )? {
                ParsedCopyPlace::Ready { place, type_id } => {
                    let copy_expression = expression_builder.copy_with_type_id(
                        place,
                        type_id,
                        copy_span,
                        *state.value_mode,
                    )?;

                    state
                        .expression
                        .push(ExpressionRpnItem::Operand(copy_expression.id()));
                }

                ParsedCopyPlace::PendingNumeric { binding, span } => {
                    // Retain the authored `copy` of a pending numeric binding: body completion
                    // builds the value copy once the binding's single static type is selected.
                    push_pending_numeric_binding(context, state.expression, binding, span, true)?;
                }
            }

            Ok(ExpressionTokenStep::Continue)
        }

        TokenTag::IF => Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueBlockOutsideReceiver,
            Some(token_stream.current_span()),
        )
        .into()),

        TokenTag::ASSERT => Err(CompilerDiagnostic::invalid_builtin_call(
            InvalidBuiltinCallReason::ExpressionPositionNotAllowed,
            Some(string_table.intern("assert")),
            Some(token_stream.current_span()),
        )
        .into()),

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                token_stream.current_tag(),
                Some(token_stream.current_span()),
                "Expression Parsing",
                "expression parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, Some(token_stream.current_span())).into())
        }

        TokenTag::HASH => {
            if token_stream.peek_next_tag() != Some(TokenTag::TEMPLATE_HEAD) {
                return Err(unexpected_token_at_current(token_stream, token, string_table)?.into());
            }

            Ok(ExpressionTokenStep::Advance)
        }
        TokenTag::CAST => parse_cast_expression(
            token,
            token_stream,
            context,
            type_interner,
            state,
            string_table,
            path_fork,
        ),

        TokenTag::NEGATIVE | TokenTag::NOT => {
            let _ = parse_unary_operator(
                token_stream,
                context,
                state.expression,
                state.next_number_negative,
            );
            Ok(ExpressionTokenStep::Advance)
        }

        TokenTag::ADD => {
            advance_with_operator(state.expression, context, token_stream, Operator::Add)
        }
        TokenTag::SUBTRACT => {
            advance_with_operator(state.expression, context, token_stream, Operator::Subtract)
        }
        TokenTag::MULTIPLY => {
            advance_with_operator(state.expression, context, token_stream, Operator::Multiply)
        }
        TokenTag::DIVIDE => {
            advance_with_operator(state.expression, context, token_stream, Operator::Divide)
        }
        TokenTag::INT_DIVIDE => {
            advance_with_operator(state.expression, context, token_stream, Operator::IntDivide)
        }
        TokenTag::EXPONENT => {
            advance_with_operator(state.expression, context, token_stream, Operator::Exponent)
        }
        TokenTag::MODULUS => {
            advance_with_operator(state.expression, context, token_stream, Operator::Modulus)
        }

        TokenTag::IS => dispatch_is_token(
            token,
            token_stream,
            context,
            type_interner,
            state,
            string_table,
            path_fork,
        ),

        TokenTag::LESS_THAN => {
            advance_with_operator(state.expression, context, token_stream, Operator::LessThan)
        }
        TokenTag::LESS_THAN_OR_EQUAL => advance_with_operator(
            state.expression,
            context,
            token_stream,
            Operator::LessThanOrEqual,
        ),
        TokenTag::GREATER_THAN => advance_with_operator(
            state.expression,
            context,
            token_stream,
            Operator::GreaterThan,
        ),
        TokenTag::GREATER_THAN_OR_EQUAL => advance_with_operator(
            state.expression,
            context,
            token_stream,
            Operator::GreaterThanOrEqual,
        ),
        TokenTag::AND => {
            advance_with_operator(state.expression, context, token_stream, Operator::And)
        }
        TokenTag::OR => {
            advance_with_operator(state.expression, context, token_stream, Operator::Or)
        }

        TokenTag::EXCLUSIVE_RANGE => {
            advance_with_operator(state.expression, context, token_stream, Operator::Range)
        }

        TokenTag::WILDCARD => {
            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        TokenTag::TYPE_PARAMETER_BRACKET => {
            // A complete operand precedes `|`: there is no binary `|` operator. Runtime
            // `||` is the C-family `or` mistake.
            if state
                .expression
                .last()
                .is_some_and(ExpressionRpnItem::is_operand_shape)
            {
                if !context.kind.is_constant_context()
                    && let Some(error) = check_expression_common_mistake(
                        token_stream,
                        frame_is_empty(state.expression, state.group_frames),
                    )
                {
                    return Err(error.into());
                }
                return Ok(ExpressionTokenStep::Break);
            }

            if let Some(error) = check_expression_common_mistake(
                token_stream,
                frame_is_empty(state.expression, state.group_frames),
            ) {
                return Err(error.into());
            }

            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        TokenTag::ADD_ASSIGN => {
            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        _ => {
            // A fragment that must defer to the body root keeps its authored continuation: an
            // authored `!`/`?` on a retained operand stays at its operand position (the suffix
            // cannot apply before the value exists, while the enclosing expression must keep
            // collecting its authored operators), and member access resolves against known
            // receiver structure or records its delayed scalar-member occurrence. Any other
            // continuation stops the fragment and the entry that raises the deferral records it.
            if items_contain_unresolvable_numeric(frame_items(state.expression, state.group_frames))
            {
                match token {
                    TokenTag::BANG
                        if token_stream.peek_next_tag() != Some(TokenTag::ASSIGN)
                            && last_item_is_retained_numeric_operand(frame_items(
                                state.expression,
                                state.group_frames,
                            ))? =>
                    {
                        // A `!` whose operand is exactly one retained call belongs to that call's
                        // declared failure: the shared fragment entry validates and consumes it
                        // against the call's own signature facts, and the call's completion builds
                        // the propagated value once, so the authored operators after the operand
                        // keep their positions instead of needing a carrier-typed value here.
                        if retain_operand_call_propagation(
                            token_stream,
                            &mut state.expression[..],
                            context,
                            type_interner,
                            expression_builder,
                            const_values,
                            string_table,
                            path_fork,
                            state.allow_boundary_catch,
                        )? {
                            return Ok(ExpressionTokenStep::Continue);
                        }
                        let span = Some(token_stream.current_postfix_operator_span());
                        token_stream.advance();
                        retain_operand_handling_suffix(
                            state.expression,
                            DeferredExpressionSuffix::Propagate { span },
                            *state.expected_type,
                            state.value_mode,
                            expression_builder,
                        )?;
                        return Ok(ExpressionTokenStep::Continue);
                    }
                    TokenTag::QUESTION_MARK
                        if last_item_is_retained_numeric_operand(frame_items(
                            state.expression,
                            state.group_frames,
                        ))? =>
                    {
                        // `?` immediately followed by an authored catch is the completed path's
                        // recovery-conflict mistake; the suffix itself applies through the shared
                        // option-propagation constructor once the operand resolves.
                        if token_stream.peek_next_tag() == Some(TokenTag::CATCH) {
                            let conflict_span =
                                token_stream.span_at(token_stream.position().saturating_add(1));
                            return Err(CompilerDiagnostic::invalid_fallible_handling(
                                InvalidFallibleHandlingReason::OptionPropagationCatchConflict,
                                conflict_span,
                            )
                            .into());
                        }
                        let span = Some(token_stream.current_postfix_operator_span());
                        token_stream.advance();
                        retain_operand_handling_suffix(
                            state.expression,
                            DeferredExpressionSuffix::OptionalPropagate { span },
                            *state.expected_type,
                            state.value_mode,
                            expression_builder,
                        )?;
                        return Ok(ExpressionTokenStep::Continue);
                    }
                    TokenTag::BANG if token_stream.peek_next_tag() != Some(TokenTag::ASSIGN) => {
                        return Ok(ExpressionTokenStep::Break);
                    }
                    TokenTag::QUESTION_MARK => return Ok(ExpressionTokenStep::Break),
                    TokenTag::DOT
                        if last_item_is_retained_numeric_operand(frame_items(
                            state.expression,
                            state.group_frames,
                        ))? =>
                    {
                        retain_pending_operand_member_access(
                            token_stream,
                            state.expression,
                            RetainedOperandInput {
                                chain_access: PostfixChainAccess::shared(),
                                expected: *state.expected_type,
                                value_mode: state.value_mode,
                            },
                            context,
                            type_interner,
                            expression_builder,
                            const_values,
                            string_table,
                            path_fork,
                        )?;
                        return Ok(ExpressionTokenStep::Continue);
                    }
                    _ => {}
                }
            }

            if let Some(error) = check_expression_common_mistake(
                token_stream,
                frame_is_empty(state.expression, state.group_frames),
            ) {
                return Err(error.into());
            }

            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }
    }
}

// A group opens with one `GroupOpen` marker and a frame that saves the parent's inference state.
// Its items stay in the flat stream until the matching `)` closes the frame.
fn dispatch_parenthesized_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    let parenthesized_kind = classify_parenthesized_expression(token_stream);
    if !matches!(parenthesized_kind, ParenthesizedExpressionKind::Group) {
        return dispatch_parenthesized_record(
            token_stream,
            context,
            type_interner,
            state,
            string_table,
            path_fork,
            parenthesized_kind,
        );
    }

    // A group is not a receiving boundary. Its items see the parent's expected type, with a
    // numeric literal widened to `Infer`, and the parent's state returns when the frame closes.
    state.group_frames.push(GroupFrame {
        open_index: state.expression.len(),
        opening_span: token_stream.current_postfix_operator_span(),
        parent_expected_type: *state.expected_type,
        parent_cast_target_context: std::mem::replace(
            state.cast_target_context,
            CastTargetContext::None,
        ),
        parent_next_number_negative: std::mem::replace(state.next_number_negative, false),
        child_unresolvable: false,
    });
    state
        .expression
        .push(ExpressionRpnItem::GroupOpen { anchor_span: None });
    *state.expected_type = grouped_evaluation_expected(*state.expected_type);
    Ok(ExpressionTokenStep::Advance)
}

#[inline(never)]
fn dispatch_parenthesized_record(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    parenthesized_kind: ParenthesizedExpressionKind,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    let expression_builder = &mut *state.expression_builder;
    let const_values = &mut *state.const_values;
    let group_span = Some(token_stream.current_postfix_operator_span());
    if !context.kind.is_constant_context() {
        if matches!(parenthesized_kind, ParenthesizedExpressionKind::Empty) {
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::EmptyRuntimeAnonymousRecord,
                Some(token_stream.current_span()),
            )
            .into());
        }

        return Err(CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::RuntimeAnonymousRecord,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let record = parse_parenthesized_anonymous_const_record_expression(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;
    push_expression_operand_with_span(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        state.expression,
        state.allow_boundary_catch,
        ExpressionOperandInput {
            operand: record,
            wrapper_span: group_span,
        },
        path_fork,
    )?;
    Ok(ExpressionTokenStep::Continue)
}

/// Closes the innermost group at its `)`, at a break that ends the group, or at a value-template
/// return that stands for the whole group.
///
/// WHAT: a deferred group keeps its items in the flat stream between its markers. Its anchor span
///       is written into the `GroupOpen` and mirrored on the `GroupClose`. Any other group is
///       split off the stream, evaluated now, and replaced by one operand. Either way the parent's
///       inference state returns.
/// WHY: the close reads only its direct items. A nested group is jumped over through its
///      `open_distance`, and its unresolved flag was carried up by its own close, so each item
///      is read by one close only.
#[allow(
    clippy::too_many_arguments,
    reason = "the close needs the cursor, scope, type and string state, plus the early value and root policy"
)]
pub(super) fn close_innermost_group(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    early_value: Option<OwnedExpr>,
    root_allow_boundary_catch: bool,
) -> Result<(), ExpressionParseError> {
    let Some(frame) = state.group_frames.pop() else {
        return Err(CompilerError::compiler_error(
            "grouped expression closed without an open group frame",
        )
        .into());
    };
    let mut group_expected_type = *state.expected_type;
    *state.expected_type = frame.parent_expected_type;
    *state.cast_target_context = frame.parent_cast_target_context;
    *state.next_number_negative = frame.parent_next_number_negative;
    // A group nested inside another group never keeps a boundary catch, because the enclosing
    // group already has none. Only a group that closes at the root takes the root's policy.
    let parent_allow_boundary_catch = root_allow_boundary_catch && !state.in_group();
    let item_start = frame.item_start();

    if let Some(value) = early_value {
        // A value-template return stands for the whole group and has already consumed the group's
        // `)`. The partial items are dropped and the returned value replaces the group.
        state.expression.truncate(frame.open_index);
        push_expression_operand_with_span(
            token_stream,
            context,
            type_interner,
            &mut *state.expression_builder,
            &mut *state.const_values,
            string_table,
            state.expression,
            parent_allow_boundary_catch,
            ExpressionOperandInput {
                operand: value,
                wrapper_span: Some(frame.opening_span),
            },
            path_fork,
        )?;
        return Ok(());
    }

    // Nested unresolved items were recorded by their own closes, so only direct items are read.
    let facts = group_content_facts(state.expression, item_start)?;
    let subtree_unresolvable = facts.unresolvable || frame.child_unresolvable;
    if let Some(parent) = state.group_frames.last_mut() {
        parent.child_unresolvable |= subtree_unresolvable;
    }
    // Typed suffixes need a completed operand. Plain `catch` is not one: dispatch stops the whole
    // outer fragment and its receiving owner completes recovery after evaluation. Well-formed groups
    // defer so receivers and decimal/unsigned peers reach their leaves. Malformed groups evaluate
    // eagerly so the diagnostic stays anchored at the group's own operator.
    //
    // A group whose own items contain construction-only numeric items can never be evaluated at
    // parse time, so it defers even when a typed suffix follows or its shape is malformed.
    let deferred = (!typed_suffix_follows(token_stream, 0) && facts.has_balanced_operands())
        || subtree_unresolvable;
    if deferred {
        let anchor_index = facts.first_operand.unwrap_or(item_start);
        let anchor_span = match state.expression.get(anchor_index) {
            Some(item) => item.source_span(&*state.expression_builder)?,
            None => {
                return Err(CompilerError::compiler_error(
                    "No nodes found in expression. This should never happen.",
                )
                .into());
            }
        };
        let close_index = state.expression.len();
        let open_distance = group_open_distance(close_index, frame.open_index, anchor_span)?;
        state.expression[frame.open_index] = ExpressionRpnItem::GroupOpen { anchor_span };
        state.expression.push(ExpressionRpnItem::GroupClose {
            open_distance,
            anchor_span,
        });
        return Ok(());
    }

    let content = state.expression.split_off(item_start);
    state.expression.truncate(frame.open_index);
    let value = evaluate_expression(
        context,
        content,
        type_interner,
        &mut *state.expression_builder,
        &mut *state.const_values,
        &mut group_expected_type,
        state.value_mode,
        string_table,
        path_fork,
    )?;
    push_expression_operand_with_span(
        token_stream,
        context,
        type_interner,
        &mut *state.expression_builder,
        &mut *state.const_values,
        string_table,
        state.expression,
        parent_allow_boundary_catch,
        ExpressionOperandInput {
            operand: value,
            wrapper_span: Some(frame.opening_span),
        },
        path_fork,
    )?;
    Ok(())
}

/// Encode a marker distance, distinguishing exhausted capacity from an invalid stream.
fn group_open_distance(
    close_index: usize,
    open_index: usize,
    span: Option<SourceSpan>,
) -> Result<u32, ExpressionParseError> {
    let distance = close_index
        .checked_sub(open_index)
        .filter(|distance| *distance > 0)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "grouped expression close does not follow its open marker",
            )
        })?;
    u32::try_from(distance).map_err(|_| {
        CompilerDiagnostic::compiler_capacity_exceeded(AstCapacityResource::GroupDistances, span)
            .into()
    })
}

/// True when the last item is an operand whose construction only body completion can finish.
///
/// WHAT: pending bindings, casts and calls are always retained, and a group is retained when its
///       own nodes contain unresolvable numeric items.
/// WHY: a member step on such a receiver cannot resolve before the body root selects its type, so
///      the ordinary member diagnostic replaces a field, method or trait search against a type
///      that does not exist yet. Literal-only operands keep their existing diagnostics.
fn last_item_is_retained_numeric_operand(
    items: &[ExpressionRpnItem],
) -> Result<bool, CompilerError> {
    Ok(match items.last() {
        Some(ExpressionRpnItem::PendingNumericBinding { .. })
        | Some(ExpressionRpnItem::PendingNumericCast { .. })
        | Some(ExpressionRpnItem::PendingNumericCall { .. })
        | Some(ExpressionRpnItem::PendingNumericExpression { .. }) => true,

        // A closed group keeps its items in the flat stream, so its own items run from the
        // `GroupOpen` marker through the `GroupClose`.
        Some(ExpressionRpnItem::GroupClose { .. }) => trailing_operand_start(items)?
            .is_some_and(|start| items_contain_unresolvable_numeric(&items[start..])),

        _ => false,
    })
}

/// Push one pending numeric binding reference as a construction-only operand item.
///
/// WHAT: reads the binding's authored value mode from its own live construction domain and
///       pushes the `PendingNumericBinding` item that body completion replaces with the
///       canonical operand.
/// WHY: a pending binding has no declaration, `TypeId` or default, so every reference position
///      (a plain read and a `copy` place) must retain the authored shape instead of fabricating
///      a value. `None` from the domain-checked lookup is a broken compiler invariant, never an
///      immutable binding.
pub(super) fn push_pending_numeric_binding(
    context: &ScopeContext,
    expression: &mut Vec<ExpressionRpnItem>,
    binding: NumericBindingId,
    span: Option<SourceSpan>,
    is_copy: bool,
) -> Result<(), ExpressionParseError> {
    let value_mode = context.pending_numeric_value_mode(binding).ok_or_else(|| {
        CompilerError::compiler_error(
            "a pending numeric binding reference had no live row in its own construction domain",
        )
    })?;

    expression.push(ExpressionRpnItem::PendingNumericBinding {
        binding,
        span,
        value_mode,
        is_copy,
    });

    Ok(())
}

fn dispatch_delimiter_token(
    token_tag: TokenTag,
    token_stream: &mut AstCursor,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    if frame_is_empty(state.expression, state.group_frames) {
        match token_tag {
            TokenTag::COMMA | TokenTag::ARROW => {
                return Err(
                    unexpected_token_at_current(token_stream, token_tag, string_table)?.into(),
                );
            }

            _ => {}
        }
    }

    if state.consume_closing_parenthesis {
        return Err(CompilerDiagnostic::missing_closing_delimiter(
            string_table.intern(")"),
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(ExpressionTokenStep::Break)
}

// -------------------------------
//  Close parenthesis
// -------------------------------
fn dispatch_close_parenthesis(
    token: TokenTag,
    token_stream: &mut AstCursor,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    if state.consume_closing_parenthesis {
        token_stream.advance();
    }

    // Inside a group the `)` closes its innermost frame. The loop performs the close, so the group
    // becomes one operand of its parent, which then continues from the next token.
    if state.in_group() {
        if frame_is_empty(state.expression, state.group_frames) {
            return Err(unexpected_token_at_current(token_stream, token, string_table)?.into());
        }
        return Ok(ExpressionTokenStep::CloseGroup);
    }

    if state.expression.is_empty() {
        return Err(unexpected_token_at_current(token_stream, token, string_table)?.into());
    }

    Ok(ExpressionTokenStep::Break)
}

// -------------------------------
//  Newline
// -------------------------------

/// Whether the expression continues past the newline at the cursor.
///
/// WHAT: the single owner of the logical-line continuation rule.
/// WHY: newline dispatch and the cast-operand `catch` lookahead must agree on where an
///      ordinary expression ends. A lookahead that ignored continuation would stop on a
///      legal multiline operand (`x +\n y`, `x\n + y`), and a lookahead that ignored the
///      boundary would adopt a `catch` from a later statement.
///
/// PRECONDITION: the cursor is positioned on the newline being tested, and `previous_tag` is
///      the tag of the token immediately before it.
fn newline_continues_expression(
    token_stream: &AstCursor,
    previous_tag: TokenTag,
    consume_closing_parenthesis: bool,
    match_arm_boundary: bool,
) -> bool {
    // An incomplete expression carries its own continuation, and inside a consumed
    // parenthesis list newlines never end the expression at all.
    if consume_closing_parenthesis
        || (previous_tag.continues_expression()
            && !matches!(
                previous_tag,
                TokenTag::END | TokenTag::TYPE_PARAMETER_BRACKET
            ))
    {
        return true;
    }

    // Look past the newline run without moving the caller's cursor: a match-arm header owns
    // the next line, while a leading operator or call continues the expression.
    let mut next_index = token_stream.position();
    while token_stream
        .token_ref_at(next_index)
        .is_some_and(|token| token.tag() == TokenTag::NEWLINE)
    {
        next_index = next_index.saturating_add(1);
    }

    if match_arm_boundary
        && token_index_starts_match_arm_header(token_stream, next_index, None).is_some()
    {
        return false;
    }

    token_stream
        .token_ref_at(next_index)
        .is_some_and(|token| token.tag().continues_expression())
}

fn dispatch_newline(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    state: &mut ExpressionDispatchState<'_>,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    let previous_tag = token_stream
        .previous()
        .map(|token| token.tag())
        .unwrap_or(TokenTag::NEWLINE);
    if newline_continues_expression(
        token_stream,
        previous_tag,
        state.consume_closing_parenthesis,
        context.match_arm_boundary,
    ) {
        token_stream.skip_newlines();
        return Ok(ExpressionTokenStep::Continue);
    }

    ast_log!("Breaking out of expression with newline");
    Ok(ExpressionTokenStep::Break)
}

fn dispatch_is_token(
    token: TokenTag,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    match token_stream.peek_next_tag() {
        // `is not` → inequality operator.
        Some(TokenTag::NOT) => {
            token_stream.advance();
            advance_with_operator(state.expression, context, token_stream, Operator::NotEqual)
        }

        // `is:` → type guard in a match arm. The left side must be one operand: the whole fragment
        // at the root, or the innermost group's single operand.
        Some(TokenTag::COLON) => {
            let items = frame_items(state.expression, state.group_frames);
            let single_item_or_group = items.len() <= 1
                || (items
                    .last()
                    .is_some_and(ExpressionRpnItem::is_operand_shape)
                    && trailing_operand_start(items)? == Some(0));
            if !single_item_or_group {
                return Err(unexpected_token_at_current(token_stream, token, string_table)?.into());
            }

            // Inside a group the `is` belongs to the enclosing expression. The group closes as one
            // operand, and the parent's `is:` return evaluates it.
            if state.in_group() {
                return Ok(ExpressionTokenStep::Break);
            }

            let value = evaluate_expression(
                context,
                std::mem::take(state.expression),
                type_interner,
                &mut *state.expression_builder,
                &mut *state.const_values,
                state.expected_type,
                state.value_mode,
                string_table,
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Return(value))
        }

        // Bare `is` → equality operator.
        _ => advance_with_operator(state.expression, context, token_stream, Operator::Equality),
    }
}

/// Parses an explicit `cast` or `cast ... catch:` expression at a typed boundary.
///
/// WHAT: validates that `cast` starts the expression and that an explicit builtin target was
///      supplied by the boundary, parses the operand and any handling suffix, resolves evidence,
///      and pushes the resolved cast's ID onto the transient expression stack.
/// WHY: cast is a prefix keyword whose meaning depends on the receiver type, so it is handled
///      directly by the dispatcher rather than the general operator or call machinery.
fn parse_cast_expression(
    token: TokenTag,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    let expression_builder = &mut *state.expression_builder;
    let const_values = &mut *state.const_values;
    // `cast` is only valid as the leading token of an expression at an explicit boundary.
    if !frame_is_empty(state.expression, state.group_frames) {
        return Err(unexpected_token_at_current(token_stream, token, string_table)?.into());
    }

    let cast_target_context = *state.cast_target_context;
    let (target_type_id, target, requires_optional_wrap_after_cast) = match cast_target_context {
        CastTargetContext::ExplicitBoundary {
            target_type_id,
            target,
            requires_optional_wrap_after_cast,
        } => (target_type_id, target, requires_optional_wrap_after_cast),

        CastTargetContext::TargetIsGenericParameter { target_type_id } => {
            return Err(CompilerDiagnostic::invalid_cast(
                InvalidCastReason::TargetIsGenericParameter,
                None,
                Some(target_type_id),
                Some(token_stream.current_span()),
            )
            .into());
        }

        CastTargetContext::TargetNotBuiltin { target_type_id } => {
            return Err(CompilerDiagnostic::invalid_cast(
                InvalidCastReason::TargetNotBuiltin,
                None,
                Some(target_type_id),
                Some(token_stream.current_span()),
            )
            .into());
        }

        CastTargetContext::None => {
            return Err(CompilerDiagnostic::invalid_cast(
                InvalidCastReason::MissingExplicitTarget,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    let cast_span = Some(token_stream.current_span());
    token_stream.advance();

    // `cast!` was removed. Its separate bang remains an ordinary token so the
    // diagnostic can point to the exact byte instead of silently changing meaning.
    if token_stream.current_tag() == TokenTag::BANG {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::CastPropagationRemoved,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Parse the operand without inheriting the cast target, so nested `cast` is rejected
    // and literals resolve to their natural type rather than the boundary target.
    let mut operand_expected_type = ExpectedType::Infer;
    let operand_result = parse_cast_operand_expression(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        &mut operand_expected_type,
        state.value_mode,
        state.consume_closing_parenthesis,
        string_table,
        path_fork,
    );
    // Recovery ownership belongs to the authored suffix, even when source typing is retained.
    let has_catch = token_stream.current_tag() == TokenTag::CATCH;
    let handling = if has_catch {
        CastHandling::Recover
    } else {
        CastHandling::Implicit
    };
    let operand = match operand_result {
        Ok(operand) => operand,
        Err(error) => {
            // The operand is a complete bounded fragment: retain its ordered syntax inside the
            // cast item and resume source resolution and pair classification at the body root.
            let deferral = error.take_numeric_deferral("the source of an authored `cast`")?;
            state
                .expression
                .push(ExpressionRpnItem::PendingNumericCast {
                    target_type_id,
                    target,
                    requires_optional_wrap_after_cast,
                    handling,
                    source: deferral.items,
                    span: cast_span,
                });
            return Ok(ExpressionTokenStep::Continue);
        }
    };

    let mut cast_expression = resolve_cast_expression(CastResolutionInput {
        source: operand,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        handling,
        scope_context: context,
        trait_environment: context.trait_environment(),
        trait_evidence_environment: context.trait_evidence_environment(),
        type_interner: &mut *type_interner,
        expression_builder: &mut *expression_builder,
        string_table: &mut *string_table,
        path_fork: &mut *path_fork,
        active_generic_type_context: context.active_generic_type_context(),
        span: cast_span,
    })?;

    if has_catch {
        let error_type_id = compatible_expression_error_type(
            &mut cast_expression,
            expression_builder,
            context,
            type_interner.environment(),
            string_table,
            Some(token_stream.current_span()),
        )?;
        let value_required_span = expression_builder.view_owned(cast_expression)?.span();
        cast_expression = parse_cast_catch_handling_suffix(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            CastCatchSite {
                success_type_id: target_type_id,
                error_type_id,
                value_required_span,
                // Explicit constant/default receivers retain cast recovery; const evaluation
                // owns whether the handler's value-producing shape can actually fold.
                allow_boundary_catch: state.allow_boundary_catch
                    && (fallible_catch_allowed_in_context(context)
                        || matches!(
                            context.kind,
                            ContextKind::Constant | ContextKind::ConstantHeader
                        )),
            },
            cast_expression,
            string_table,
            path_fork,
        )?;
    }

    state
        .expression
        .push(ExpressionRpnItem::Operand(cast_expression.id()));

    Ok(ExpressionTokenStep::Continue)
}

#[allow(
    clippy::too_many_arguments,
    reason = "cast operand parsing keeps the token stream, scope, mutable interner/expected-type/string/path state, value mode, and parenthesis policy as separate borrows"
)]
fn parse_cast_operand_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<OwnedExpr, ExpressionParseError> {
    // The pre-scan walks the live cursor and restores the entry position:
    // indexed reads re-scan segment prefixes on every step.
    let resume = token_stream.position();
    let mut depth = ExpressionBoundaryDepth::default();
    let mut catch_is_cast_suffix = false;
    while !token_stream.is_at_end() {
        let tag = token_stream.current_tag();
        if depth.is_top_level() {
            if tag == TokenTag::CATCH {
                catch_is_cast_suffix = true;
                break;
            }

            // A closer whose opener sits outside this operand ends the cast's own window: the
            // enclosing call-argument list, group or literal owns it. A `catch` beyond that
            // closer belongs to the enclosing receiving boundary, so an argument cast must not
            // treat the handler of its enclosing call as its own. Saturating depth cannot report
            // the boundary, so test it before stepping past the closer.
            if matches!(tag, TokenTag::CLOSE_PARENTHESIS | TokenTag::CLOSE_CURLY) {
                break;
            }

            // The operand ends exactly where the ordinary expression ends: at a statement
            // terminator, at an argument separator, or at a newline that does not continue the
            // expression. Scanning past that boundary would treat a later statement's `catch`
            // as this cast's suffix and swallow every statement in between.
            if matches!(tag, TokenTag::END | TokenTag::COMMA) {
                break;
            }
            if tag == TokenTag::NEWLINE {
                let previous_tag = token_stream
                    .previous()
                    .map(|token| token.tag())
                    .unwrap_or(TokenTag::NEWLINE);
                if !newline_continues_expression(
                    token_stream,
                    previous_tag,
                    consume_closing_parenthesis,
                    context.match_arm_boundary,
                ) {
                    break;
                }
                token_stream.skip_newlines();
                continue;
            }
        }
        depth.step_tag(tag);
        // A malformed payload is surfaced by typed readers at the consuming parser boundary.
        if tag == TokenTag::EOF {
            break;
        }
        token_stream.advance();
    }
    token_stream
        .set_position(resume)
        .expect("catch pre-scan resume stays inside the active parser view");

    let mut cast_target_context = CastTargetContext::None;

    if catch_is_cast_suffix {
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream,
                scope_context: context,
                type_interner,
                expression_builder,
                const_values,
                expected_type,
                cast_target_context: &mut cast_target_context,
                value_mode,
                string_table,
                path_fork,
            },
            false,
        )
        // The bounded operand owns its whole fragment; a `catch` beyond the operand belongs to
        // the enclosing receiving boundary and is never part of this source.
        .with_numeric_deferral_owner();
        return create_expression_until(input, &[TokenTag::CATCH]);
    }

    let mut input = ExpressionParseInput::without_boundary_catch(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expression_builder,
            const_values,
            expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode,
            string_table,
            path_fork,
        },
        consume_closing_parenthesis,
    );
    // The operand is parsed while its enclosing expression loop is still running, so the
    // operand must hand the loop back at the operand's own end. Skipping the terminating
    // newline here would move the loop onto the next statement's first token and report it as
    // an adjacent operand. The enclosing parse owns the ordinary trailing-newline skip.
    input.trailing_policy.skip_trailing_newlines = false;
    create_expression_with_trailing_newline_policy(input.with_numeric_deferral_owner())
}

/// A group's direct-level shape. A nested group is seen only through its `GroupClose`.
struct GroupContentFacts {
    /// Operand-shaped direct items. A nested group counts once, through its `GroupClose`.
    operands: usize,
    /// Direct binary operators.
    binary_operators: usize,
    /// A construction-only numeric item sits directly in this group.
    unresolvable: bool,
    /// The earliest operand-shaped direct item. Its span is the group's anchor.
    first_operand: Option<usize>,
    /// A resolved operator reached this group, which an infix group never holds.
    resolved_operator: bool,
}

impl GroupContentFacts {
    /// Shunting-yard diagnostics belong to the malformed group when its operand shape says so.
    /// A dangling inner operator placed in the outer stream would report at the wrong anchor.
    fn has_balanced_operands(&self) -> bool {
        !self.resolved_operator && self.operands == self.binary_operators + 1
    }
}

/// Reads a group's direct items from right to left, without entering any nested group.
///
/// WHAT: a nested `GroupClose` is one operand here. Its `open_distance` jumps the walk back
///       to the nested `GroupOpen`, so none of the nested items are visited.
/// WHY: each item is read only by its direct parent's close, so the walk is linear in the
///      group's direct width. A nested unresolved flag reaches this group via `child_unresolvable`.
fn group_content_facts(
    expression: &[ExpressionRpnItem],
    item_start: usize,
) -> Result<GroupContentFacts, CompilerError> {
    let mut facts = GroupContentFacts {
        operands: 0,
        binary_operators: 0,
        unresolvable: false,
        first_operand: None,
        resolved_operator: false,
    };
    let mut index = expression.len();
    while index > item_start {
        index -= 1;
        let item = &expression[index];
        facts.unresolvable |= items_contain_unresolvable_numeric(std::slice::from_ref(item));
        match item {
            ExpressionRpnItem::GroupClose { open_distance, .. } => {
                let open_index = usize::try_from(*open_distance)
                    .ok()
                    .and_then(|distance| index.checked_sub(distance))
                    .filter(|open| {
                        *open >= item_start
                            && matches!(expression[*open], ExpressionRpnItem::GroupOpen { .. })
                    })
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "grouped expression close does not match its open marker",
                        )
                    })?;
                facts.operands += 1;
                facts.first_operand = Some(index);
                index = open_index;
            }
            ExpressionRpnItem::GroupOpen { .. } => {
                return Err(CompilerError::compiler_error(
                    "grouped expression open marker has no matching close",
                ));
            }
            ExpressionRpnItem::Operand(_)
            | ExpressionRpnItem::PendingNumericLiteral { .. }
            | ExpressionRpnItem::PendingNumericBinding { .. }
            | ExpressionRpnItem::PendingNumericCall { .. }
            | ExpressionRpnItem::PendingNumericCast { .. }
            | ExpressionRpnItem::PendingNumericExpression { .. } => {
                facts.operands += 1;
                facts.first_operand = Some(index);
            }
            ExpressionRpnItem::Operator { operator, .. } => {
                if operator.required_values() == 2 {
                    facts.binary_operators += 1;
                }
            }
            ExpressionRpnItem::ResolvedOperator(_) => {
                debug_assert!(false, "resolved operation reached an infix group");
                facts.resolved_operator = true;
            }
        }
    }
    Ok(facts)
}

#[cfg(test)]
#[path = "tests/expression_dispatch_tests.rs"]
mod expression_dispatch_tests;
