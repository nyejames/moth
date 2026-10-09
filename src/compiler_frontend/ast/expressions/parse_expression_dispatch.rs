//! Expression token dispatch helpers.
//!
//! WHAT: routes one token at a time through expression-position parsing.
//! WHY: keeps delimiter/grammar ownership explicit while specialized helpers own detailed token families.

use super::anonymous_const_record::parse_parenthesized_anonymous_const_record_expression;
use super::call_arguments::{ParenthesizedExpressionKind, classify_parenthesized_expression};
use super::error::ExpressionParseError;
use super::eval_expression::{evaluate_expression, extract_expression_span};
use super::expression::{Expression, ExpressionKind, Operator};
use super::expression_rpn::ExpressionRpnItem;
use super::option_propagation::parse_option_propagation_suffix_for_expression;
use super::parse_expression::{
    ExpressionFragment, collect_expression_fragment, create_expression_until,
    create_expression_with_trailing_newline_policy,
};
use super::parse_expression_identifiers::parse_identifier_or_call;
use super::parse_expression_input::{ExpressionParseInput, ExpressionParseResources};
use super::parse_expression_literals::{
    LiteralParseState, parse_literal_expression, typed_suffix_follows,
};
use super::parse_expression_places::{
    parse_copy_place_expression, parse_mutable_receiver_expression,
};
use super::parse_expression_templates::parse_template_expression;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use crate::ast_log;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::field_access::{
    PostfixChainAccess, parse_postfix_chain_expression,
};
use crate::compiler_frontend::ast::file_value_resolution::resolve_file_value;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    CastCatchSite, compatible_expression_error_type, complete_catch_failure_fact,
    fallible_catch_allowed_in_context, parse_cast_catch_handling_suffix,
    parse_fallible_handling_suffix_for_expression, wrap_catch_expression,
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
    CompilerDiagnostic, DeferredFeatureReason, DiagnosticToken, InvalidBuiltinCallReason,
    InvalidCastReason, InvalidControlFlowStatementReason, InvalidExpressionReason,
    InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
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
    Break,
    Return(Box<Expression>),
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
}

pub(super) struct ExpressionOperandInput {
    pub(super) operand: Expression,
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
    consume_closing_parenthesis: bool,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    while token_stream.current_tag() == TokenTag::TEMPLATE_HEAD {
        let next_template_start = Some(token_stream.current_span());
        let next_template = parse_template_expression(
            token_stream,
            context,
            type_interner,
            consume_closing_parenthesis,
            value_mode,
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
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    expression_after_postfix: Expression,
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
        let value_required = expression_after_postfix.type_id != builtin_type_ids::NONE;
        parse_fallible_handling_suffix_for_expression(
            token_stream,
            context,
            type_interner,
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
        && expression_after_option_propagation.is_const_record_value()
    {
        let record_name = const_record_expression_name(
            &expression_after_option_propagation,
            string_table,
            path_fork,
        );
        return Err(CompilerDiagnostic::const_record_used_as_value(
            record_name,
            expression_after_option_propagation.span,
        )
        .into());
    }

    expression.push(ExpressionRpnItem::Operand(
        expression_after_option_propagation,
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
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    operand: Expression,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let wrapper_span = operand.span;
    push_expression_operand_with_span(
        token_stream,
        context,
        type_interner,
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
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    operand_input: ExpressionOperandInput,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let expression_after_postfix = if token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::DOT
    {
        parse_postfix_chain_expression(
            token_stream,
            operand_input.operand,
            operand_input.wrapper_span,
            PostfixChainAccess::shared(),
            context,
            type_interner,
            string_table,
            path_fork,
        )?
    } else {
        operand_input.operand
    };

    push_expression_after_suffixes(
        token_stream,
        context,
        type_interner,
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
    expression: &Expression,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> StringId {
    match &expression.kind {
        ExpressionKind::FieldAccess { base, .. } => {
            const_record_expression_name(base, string_table, path_fork)
        }

        ExpressionKind::Reference(path) => path_fork
            .component(*path)
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

        TokenTag::OPEN_PARENTHESIS => {
            let group_span = Some(token_stream.current_postfix_operator_span());
            let parenthesized_kind = classify_parenthesized_expression(token_stream);
            if !matches!(parenthesized_kind, ParenthesizedExpressionKind::Group) {
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
                    string_table,
                    path_fork,
                )?;
                push_expression_operand_with_span(
                    token_stream,
                    context,
                    type_interner,
                    string_table,
                    state.expression,
                    state.allow_boundary_catch,
                    ExpressionOperandInput {
                        operand: record,
                        wrapper_span: group_span,
                    },
                    path_fork,
                )?;
                return Ok(ExpressionTokenStep::Continue);
            }

            token_stream.advance();
            // A group is not a receiving boundary. Retain its raw syntax for the outer
            // evaluator, while typed completion boundaries use their own natural context.
            let mut grouped_expected_type = match *state.expected_type {
                ExpectedType::NumericLiteral(_) => ExpectedType::Infer,
                other => other,
            };
            let mut grouped_cast_target_context = CastTargetContext::None;
            let mut grouped_input =
                ExpressionParseInput::grouped_without_cast_target(ExpressionParseResources {
                    token_stream,
                    scope_context: context,
                    type_interner,
                    expected_type: &mut grouped_expected_type,
                    cast_target_context: &mut grouped_cast_target_context,
                    value_mode: state.value_mode,
                    path_fork,
                    string_table,
                });
            let value = match collect_expression_fragment(&mut grouped_input)? {
                ExpressionFragment::Value(value) => *value,

                // Typed suffixes need a completed operand. Plain `catch` is not one:
                // dispatch stops the whole outer fragment and its receiving owner completes
                // recovery after evaluation. Well-formed groups defer so receivers and Dec/Uint
                // peers reach their leaves; malformed groups evaluate eagerly so the
                // diagnostic stays anchored at the group's own operator.
                ExpressionFragment::Nodes(nodes)
                    if !typed_suffix_follows(token_stream, 0)
                        && group_nodes_have_balanced_operands(&nodes) =>
                {
                    let span = extract_expression_span(&nodes)?;
                    state
                        .expression
                        .push(ExpressionRpnItem::PendingGroup { nodes, span });
                    return Ok(ExpressionTokenStep::Continue);
                }
                ExpressionFragment::Nodes(nodes) => evaluate_expression(
                    context,
                    nodes,
                    type_interner,
                    &mut grouped_expected_type,
                    state.value_mode,
                    string_table,
                    path_fork,
                )?,
            };

            push_expression_operand_with_span(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                ExpressionOperandInput {
                    operand: value,
                    wrapper_span: group_span,
                },
                path_fork,
            )?;

            Ok(ExpressionTokenStep::Continue)
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

            if let Some(error) =
                check_expression_common_mistake(token_stream, state.expression.is_empty())
            {
                return Err(error.into());
            }

            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        TokenTag::OPEN_CURLY => {
            parse_curly_literal_expression(
                token_stream,
                context,
                type_interner,
                state.expected_type,
                state.value_mode,
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
                state.expression,
                state.allow_boundary_catch,
                state.allow_expected_result_evidence,
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
                state.value_mode,
                string_table,
                path_fork,
            )?;
            token_stream.advance();
            push_expression_operand_with_span(
                token_stream,
                context,
                type_interner,
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
                state.consume_closing_parenthesis,
                state.value_mode,
                string_table,
                path_fork,
            )?;

            Ok(ExpressionTokenStep::Return(Box::new(template_expression)))
        }

        TokenTag::COPY => {
            let copy_span = Some(token_stream.current_span());
            token_stream.advance();

            let copied_place = parse_copy_place_expression(
                token_stream,
                context,
                type_interner,
                string_table,
                path_fork,
            )?;

            let copy_expression = Expression::copy_with_type_id(
                copied_place.place,
                copied_place.diagnostic_type,
                copied_place.type_id,
                copy_span,
                state.value_mode.to_owned(),
            );

            state
                .expression
                .push(ExpressionRpnItem::Operand(copy_expression));

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
                    && let Some(error) =
                        check_expression_common_mistake(token_stream, state.expression.is_empty())
                {
                    return Err(error.into());
                }
                return Ok(ExpressionTokenStep::Break);
            }

            if let Some(error) =
                check_expression_common_mistake(token_stream, state.expression.is_empty())
            {
                return Err(error.into());
            }

            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        TokenTag::ADD_ASSIGN => {
            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }

        _ => {
            if let Some(error) =
                check_expression_common_mistake(token_stream, state.expression.is_empty())
            {
                return Err(error.into());
            }

            Err(unexpected_token_at_current(token_stream, token, string_table)?.into())
        }
    }
}

fn dispatch_delimiter_token(
    token_tag: TokenTag,
    token_stream: &mut AstCursor,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    if state.expression.is_empty() {
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

        // `is:` → type guard in a match arm. The left-hand side must be a single expression.
        Some(TokenTag::COLON) => {
            if state.expression.len() > 1 {
                return Err(unexpected_token_at_current(token_stream, token, string_table)?.into());
            }

            let value = evaluate_expression(
                context,
                std::mem::take(state.expression),
                type_interner,
                state.expected_type,
                state.value_mode,
                string_table,
                path_fork,
            )?;
            Ok(ExpressionTokenStep::Return(Box::new(value)))
        }

        // Bare `is` → equality operator.
        _ => advance_with_operator(state.expression, context, token_stream, Operator::Equality),
    }
}

/// Parses an explicit `cast` or `cast ... catch:` expression at a typed boundary.
///
/// WHAT: validates that `cast` starts the expression and that an explicit builtin target was
///      supplied by the boundary, parses the operand and any handling suffix, resolves evidence,
///      and pushes a resolved `ExpressionKind::Cast` node onto the expression stack.
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
    // `cast` is only valid as the leading token of an expression at an explicit boundary.
    if !state.expression.is_empty() {
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
    let operand = parse_cast_operand_expression(
        token_stream,
        context,
        type_interner,
        &mut operand_expected_type,
        state.value_mode,
        state.consume_closing_parenthesis,
        string_table,
        path_fork,
    )?;

    let has_catch = token_stream.current_tag() == TokenTag::CATCH;
    let handling = if has_catch {
        CastHandling::Recover
    } else {
        CastHandling::Implicit
    };

    let mut cast_expression = resolve_cast_expression(CastResolutionInput {
        source: operand,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        handling,
        numeric_profile: context.numeric_profile,
        scope_context: context,
        trait_environment: context.trait_environment(),
        trait_evidence_environment: context.trait_evidence_environment(),
        type_interner: &mut *type_interner,
        string_table: &mut *string_table,
        path_fork: &mut *path_fork,
        active_generic_type_context: context.active_generic_type_context(),
        span: cast_span,
    })?;

    if has_catch {
        let error_type_id = compatible_expression_error_type(
            &mut cast_expression,
            context,
            type_interner.environment(),
            string_table,
            Some(token_stream.current_span()),
        )?;
        let handler = parse_cast_catch_handling_suffix(
            token_stream,
            context,
            type_interner,
            CastCatchSite {
                success_type_id: target_type_id,
                error_type_id,
                value_required_span: cast_expression.span,
                // Explicit constant/default receivers retain cast recovery; const evaluation
                // owns whether the handler's value-producing shape can actually fold.
                allow_boundary_catch: state.allow_boundary_catch
                    && (fallible_catch_allowed_in_context(context)
                        || matches!(
                            context.kind,
                            ContextKind::Constant | ContextKind::ConstantHeader
                        )),
            },
            string_table,
            path_fork,
        )?;
        cast_expression = wrap_catch_expression(cast_expression, handler, vec![target_type_id]);
        complete_catch_failure_fact(&mut cast_expression, error_type_id, context)?;
    }

    state
        .expression
        .push(ExpressionRpnItem::Operand(cast_expression));

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
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
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
                expected_type,
                cast_target_context: &mut cast_target_context,
                value_mode,
                string_table,
                path_fork,
            },
            false,
        );
        return create_expression_until(input, &[TokenTag::CATCH]);
    }

    let mut input = ExpressionParseInput::without_boundary_catch(
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
    // The operand is parsed while its enclosing expression loop is still running, so the
    // operand must hand the loop back at the operand's own end. Skipping the terminating
    // newline here would move the loop onto the next statement's first token and report it as
    // an adjacent operand. The enclosing parse owns the ordinary trailing-newline skip.
    input.trailing_policy.skip_trailing_newlines = false;
    create_expression_with_trailing_newline_policy(input)
}

/// Check a group's own operand balance before deferring it into a pending group.
///
/// Shunting-yard diagnostics belong to the malformed group when its operand shape says so.
/// A dangling inner operator placed in the outer stream would report at the wrong anchor.
fn group_nodes_have_balanced_operands(nodes: &[ExpressionRpnItem]) -> bool {
    let mut operands = 0;
    let mut binary_operators = 0;
    for node in nodes {
        match node {
            ExpressionRpnItem::Operand(_)
            | ExpressionRpnItem::PendingNumericLiteral { .. }
            | ExpressionRpnItem::PendingGroup { .. } => operands += 1,
            ExpressionRpnItem::Operator { operator, .. } => {
                if operator.required_values() == 2 {
                    binary_operators += 1;
                }
            }
        }
    }
    operands == binary_operators + 1
}

#[cfg(test)]
#[path = "tests/expression_dispatch_tests.rs"]
mod expression_dispatch_tests;
