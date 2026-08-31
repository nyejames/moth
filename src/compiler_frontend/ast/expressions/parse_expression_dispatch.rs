//! Expression token dispatch helpers.
//!
//! WHAT: routes one token at a time through expression-position parsing.
//! WHY: keeps delimiter/grammar ownership explicit while specialized helpers own detailed token families.

use super::error::ExpressionParseError;
use super::eval_expression::evaluate_expression;
use super::expression::{Expression, ExpressionKind, Operator};
use super::expression_rpn::ExpressionRpnItem;
use super::option_propagation::parse_option_propagation_suffix_for_expression;
use super::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use super::parse_expression_identifiers::parse_identifier_or_call;
use super::parse_expression_input::{ExpressionParseInput, ExpressionParseResources};
use super::parse_expression_literals::{LiteralParseState, parse_literal_expression};
use super::parse_expression_places::{
    parse_copy_place_expression, parse_mutable_receiver_expression,
};
use super::parse_expression_templates::parse_template_expression;
use crate::ast_log;
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::field_access::{
    PostfixChainAccess, parse_postfix_chain_expression,
};
use crate::compiler_frontend::ast::file_value_resolution::resolve_file_value;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    CastCatchSite, fallible_catch_allowed_in_context, parse_cast_catch_handling_suffix,
    parse_fallible_handling_suffix_for_expression, wrap_catch_expression,
};
use crate::compiler_frontend::ast::statements::match_arm_boundaries::current_token_starts_match_arm_header;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::builtins::casts::resolution::{
    CastResolutionInput, resolve_cast_expression,
};
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::builtins::expression_parsing::parse_curly_literal_expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidBuiltinCallReason, InvalidCastReason,
    InvalidControlFlowStatementReason, InvalidExpressionReason, InvalidTemplateStructureReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::expression_position::check_expression_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::utilities::token_scan::find_expression_end_index;
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
    pub(super) wrapper_location: SourceLocation,
}

/// Reports adjacent operands at the second expression without guessing the missing operator.
fn reject_adjacent_operand(
    expression: &[ExpressionRpnItem],
    second_expression_location: &SourceLocation,
) -> Result<(), ExpressionParseError> {
    let previous_is_operand = matches!(expression.last(), Some(ExpressionRpnItem::Operand(_)));

    if previous_is_operand {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            second_expression_location.clone(),
        )
        .into());
    }

    Ok(())
}

fn is_value_operand_start_token(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::Path(_)
            | TokenKind::NumericLiteral(_)
            | TokenKind::StringSliceLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::NoneLiteral
            | TokenKind::Symbol(_)
            | TokenKind::This
            | TokenKind::Mutable
            | TokenKind::OpenParenthesis
            | TokenKind::OpenCurly
            | TokenKind::Copy
    )
}

/// Skips value-less comment templates before checking what follows a value template.
fn reject_second_operand_after_value_template(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    consume_closing_parenthesis: bool,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    while token_stream.current_token_kind() == &TokenKind::TemplateHead {
        let next_template_start = token_stream.current_location();
        let next_template = parse_template_expression(
            token_stream,
            context,
            type_interner,
            consume_closing_parenthesis,
            value_mode,
            string_table,
        )?;

        if next_template.is_some() {
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::ExpectedOperatorBeforeExpression,
                next_template_start,
            )
            .into());
        }
    }

    if is_value_operand_start_token(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(())
}

fn push_expression_after_suffixes(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    expression_after_postfix: Expression,
) -> Result<(), ExpressionParseError> {
    // ----------------------------
    //  Common mistake: `!=`
    // ----------------------------
    // Detect `!=` (Bang + Assign) before treating `!` as a result-handling suffix.
    if token_stream.index < token_stream.length
        && token_stream.current_token_kind() == &TokenKind::Bang
        && token_stream.peek_next_token() == Some(&TokenKind::Assign)
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
    let expression_after_fallible = if token_stream.index < token_stream.length
        && (token_stream.current_token_kind() == &TokenKind::Bang
            || token_stream.current_token_kind() == &TokenKind::Catch
            || (matches!(token_stream.current_token_kind(), TokenKind::Symbol(_))
                && token_stream.peek_next_token() == Some(&TokenKind::Bang)))
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
        )?
    } else {
        expression_after_postfix
    };

    // ----------------------------
    //  Option propagation suffix
    // ----------------------------
    let expression_after_option_propagation = if token_stream.index < token_stream.length
        && token_stream.current_token_kind() == &TokenKind::QuestionMark
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
        let record_name =
            const_record_expression_name(&expression_after_option_propagation, string_table);
        return Err(CompilerDiagnostic::const_record_used_as_value(
            record_name,
            expression_after_option_propagation.location.clone(),
        )
        .into());
    }

    expression.push(ExpressionRpnItem::Operand(
        expression_after_option_propagation,
    ));
    Ok(())
}

pub(super) fn push_expression_operand(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    operand: Expression,
) -> Result<(), ExpressionParseError> {
    let wrapper_location = operand.location.clone();
    push_expression_operand_at_location(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        ExpressionOperandInput {
            operand,
            wrapper_location,
        },
    )
}

/// Push an expression operand while preserving a distinct wrapper location for suffix parsing.
///
/// WHAT: keeps postfix/fallible/option handling on the existing dispatch path without requiring
/// callers to construct `NodeKind::ExpressionStatement` themselves.
/// WHY: constant references may carry declaration-origin expression locations, but diagnostics
/// for suffixes and const-record misuse should still point at the source use site.
pub(super) fn push_expression_operand_at_location(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    operand_input: ExpressionOperandInput,
) -> Result<(), ExpressionParseError> {
    let expression_after_postfix = if token_stream.index < token_stream.length
        && token_stream.current_token_kind() == &TokenKind::Dot
    {
        parse_postfix_chain_expression(
            token_stream,
            operand_input.operand,
            operand_input.wrapper_location,
            PostfixChainAccess::shared(),
            context,
            type_interner,
            string_table,
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
    )
}

/// Extracts a display name for a const-record diagnostic from an expression.
///
/// WHAT: walks field-access chains back to the root identifier so the
/// diagnostic example points at the record, not an intermediate field.
fn const_record_expression_name(
    expression: &Expression,
    string_table: &mut StringTable,
) -> StringId {
    match &expression.kind {
        ExpressionKind::FieldAccess { base, .. } => {
            const_record_expression_name(base, string_table)
        }

        ExpressionKind::Reference(path) => {
            path.name().unwrap_or_else(|| string_table.intern("record"))
        }

        _ => string_table.intern("record"),
    }
}

/// Pushes a unary operator item onto the expression stack when the current token
/// is `Negative` or `Not`. Returns `true` when an operator was consumed.
fn parse_unary_operator(
    token_stream: &FileTokens,
    _context: &ScopeContext,
    expression: &mut Vec<ExpressionRpnItem>,
    next_number_negative: &mut bool,
) -> bool {
    match token_stream.current_token_kind() {
        TokenKind::Negative => {
            if matches!(
                token_stream.peek_next_token(),
                Some(TokenKind::NumericLiteral(_))
            ) {
                *next_number_negative = true;
            } else {
                expression.push(ExpressionRpnItem::Operator {
                    operator: Operator::Negate,
                    location: token_stream.current_location(),
                });
            }
            true
        }
        TokenKind::Not => {
            expression.push(ExpressionRpnItem::Operator {
                operator: Operator::Not,
                location: token_stream.current_location(),
            });
            true
        }
        _ => false,
    }
}

fn push_operator_item(
    expression: &mut Vec<ExpressionRpnItem>,
    _context: &ScopeContext,
    location: SourceLocation,
    operator: Operator,
) {
    expression.push(ExpressionRpnItem::Operator { operator, location });
}

/// Convenience for the common match arm that pushes an operator and advances.
fn advance_with_operator(
    expression: &mut Vec<ExpressionRpnItem>,
    context: &ScopeContext,
    location: SourceLocation,
    operator: Operator,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    push_operator_item(expression, context, location, operator);
    Ok(ExpressionTokenStep::Advance)
}

pub(super) fn dispatch_expression_token(
    token: TokenKind,
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    // Reject definite adjacency before semantic name, call or constructor parsing.
    if is_value_operand_start_token(&token) {
        reject_adjacent_operand(state.expression, &token_stream.current_location())?;
    }

    // This state machine is intentionally flat: each token either appends one AST node, advances
    // past a nested parse, or signals the caller that the surrounding grammar owns the delimiter.
    match token {
        // -------------------------------
        //  Delimiters and terminators
        // -------------------------------
        TokenKind::CloseCurly
        | TokenKind::Comma
        | TokenKind::Eof
        | TokenKind::TemplateClose
        | TokenKind::Arrow
        | TokenKind::StartTemplateBody
        | TokenKind::Colon
        | TokenKind::Else
        | TokenKind::End => dispatch_delimiter_token(token, token_stream, state, string_table),

        // -------------------------------
        //  Parentheses and grouping
        // -------------------------------
        TokenKind::CloseParenthesis => dispatch_close_parenthesis(token_stream, state),

        TokenKind::OpenParenthesis => {
            token_stream.advance();
            // A grouped expression is no longer the immediate receiving boundary.
            // This keeps `(cast value)` from acting as an operator operand while
            // still allowing `cast (left + right)` to narrow the cast operand.
            // Keep the parse-time literal context available to the group, but do not let the
            // group's natural result type become the outer expression's expected type. The
            // grouped expression remains an operand in the surrounding RPN stream, whose
            // operator policy owns the final result type.
            let mut grouped_expected_type = *state.expected_type;
            let mut grouped_cast_target_context = CastTargetContext::None;
            let grouped_input =
                ExpressionParseInput::grouped_without_cast_target(ExpressionParseResources {
                    token_stream,
                    scope_context: context,
                    type_interner,
                    expected_type: &mut grouped_expected_type,
                    cast_target_context: &mut grouped_cast_target_context,
                    value_mode: state.value_mode,
                    string_table,
                });
            let value = create_expression_with_trailing_newline_policy(grouped_input)?;

            push_expression_operand_at_location(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                ExpressionOperandInput {
                    operand: value,
                    wrapper_location: token_stream.current_location(),
                },
            )?;

            Ok(ExpressionTokenStep::Continue)
        }

        TokenKind::DatatypeInt
        | TokenKind::DatatypeFloat
        | TokenKind::DatatypeBool
        | TokenKind::DatatypeString
        | TokenKind::DatatypeChar => {
            if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis) {
                let cast_name = match token {
                    TokenKind::DatatypeInt => Some(string_table.intern("Int")),
                    TokenKind::DatatypeFloat => Some(string_table.intern("Float")),
                    TokenKind::DatatypeBool => Some(string_table.intern("Bool")),
                    TokenKind::DatatypeString => Some(string_table.intern("String")),
                    TokenKind::DatatypeChar => Some(string_table.intern("Char")),
                    _ => None,
                };
                return Err(CompilerDiagnostic::invalid_builtin_call(
                    InvalidBuiltinCallReason::ScalarConstructorRemoved,
                    cast_name,
                    token_stream.current_location(),
                )
                .into());
            }

            if let Some(error) =
                check_expression_common_mistake(token_stream, state.expression.is_empty())
            {
                return Err(error.into());
            }

            Err(CompilerDiagnostic::unexpected_token(token, token_stream.current_location()).into())
        }

        TokenKind::OpenCurly => {
            parse_curly_literal_expression(
                token_stream,
                context,
                type_interner,
                state.expected_type,
                state.value_mode,
                state.expression,
                string_table,
            )?;
            Ok(ExpressionTokenStep::Advance)
        }

        TokenKind::Newline => dispatch_newline(token_stream, context, state),

        // -------------------------------
        //  Primary expressions
        // -------------------------------
        TokenKind::Symbol(..) | TokenKind::This => {
            parse_identifier_or_call(
                token_stream,
                context,
                type_interner,
                state.expression,
                state.allow_boundary_catch,
                state.allow_expected_result_evidence,
                string_table,
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenKind::Mutable => {
            parse_mutable_receiver_expression(
                token_stream,
                context,
                type_interner,
                state.expression,
                state.allow_boundary_catch,
                string_table,
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenKind::NumericLiteral(_)
        | TokenKind::StringSliceLiteral(_)
        | TokenKind::BoolLiteral(_)
        | TokenKind::CharLiteral(_)
        | TokenKind::NoneLiteral => {
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
            )?;
            Ok(ExpressionTokenStep::Continue)
        }
        TokenKind::Path(path_syntax) => {
            let path_location = token_stream.current_location();
            let operand = resolve_file_value(
                path_syntax,
                token_stream,
                context,
                type_interner,
                state.value_mode,
                string_table,
            )?;
            token_stream.advance();
            push_expression_operand_at_location(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                ExpressionOperandInput {
                    operand,
                    wrapper_location: path_location,
                },
            )?;
            Ok(ExpressionTokenStep::Continue)
        }

        TokenKind::TemplateHead => {
            let template_start_location = token_stream.current_location();
            let template_expression = parse_template_expression(
                token_stream,
                context,
                type_interner,
                state.consume_closing_parenthesis,
                state.value_mode,
                string_table,
            )?;

            let Some(template_expression) = template_expression else {
                return Ok(ExpressionTokenStep::Continue);
            };

            reject_adjacent_operand(state.expression, &template_start_location)?;

            reject_second_operand_after_value_template(
                token_stream,
                context,
                type_interner,
                state.consume_closing_parenthesis,
                state.value_mode,
                string_table,
            )?;

            Ok(ExpressionTokenStep::Return(Box::new(template_expression)))
        }

        TokenKind::Copy => {
            let copy_location = token_stream.current_location();
            token_stream.advance();

            let copied_place =
                parse_copy_place_expression(token_stream, context, type_interner, string_table)?;

            let copy_expression = Expression::copy_with_type_id(
                copied_place.place,
                copied_place.diagnostic_type,
                copied_place.type_id,
                copy_location.clone(),
                state.value_mode.to_owned(),
            );

            state
                .expression
                .push(ExpressionRpnItem::Operand(copy_expression));

            Ok(ExpressionTokenStep::Continue)
        }

        // -------------------------------
        //  Reserved / invalid tokens
        // -------------------------------
        TokenKind::If => Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueBlockOutsideReceiver,
            token_stream.current_location(),
        )
        .into()),

        TokenKind::Assert => Err(CompilerDiagnostic::invalid_builtin_call(
            InvalidBuiltinCallReason::ExpressionPositionNotAllowed,
            Some(string_table.intern("assert")),
            token_stream.current_location(),
        )
        .into()),

        TokenKind::Must | TokenKind::TraitThis => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                token_stream.current_token_kind(),
                token_stream.current_location(),
                "Expression Parsing",
                "expression parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, token_stream.current_location()).into())
        }

        TokenKind::Hash => {
            if token_stream.peek_next_token() != Some(&TokenKind::TemplateHead) {
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::Hash,
                    token_stream.current_location(),
                )
                .into());
            }

            Ok(ExpressionTokenStep::Advance)
        }

        TokenKind::Reactive
            if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis) =>
        {
            Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::ReactiveSubscriptionOutsideTemplate,
                token_stream.current_location(),
            )
            .into())
        }

        TokenKind::Cast | TokenKind::CastBang => {
            parse_cast_expression(token_stream, context, type_interner, state, string_table)
        }

        TokenKind::Negative | TokenKind::Not => {
            let _ = parse_unary_operator(
                token_stream,
                context,
                state.expression,
                state.next_number_negative,
            );
            Ok(ExpressionTokenStep::Advance)
        }

        // -------------------------------
        //  Arithmetic operators
        // -------------------------------
        TokenKind::Add => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Add,
        ),
        TokenKind::Subtract => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Subtract,
        ),
        TokenKind::Multiply => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Multiply,
        ),
        TokenKind::Divide => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Divide,
        ),
        TokenKind::IntDivide => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::IntDivide,
        ),
        TokenKind::Exponent => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Exponent,
        ),
        TokenKind::Modulus => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Modulus,
        ),

        // -------------------------------
        //  Comparison operators
        // -------------------------------
        TokenKind::Is => {
            dispatch_is_token(token_stream, context, type_interner, state, string_table)
        }

        TokenKind::LessThan => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::LessThan,
        ),
        TokenKind::LessThanOrEqual => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::LessThanOrEqual,
        ),
        TokenKind::GreaterThan => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::GreaterThan,
        ),
        TokenKind::GreaterThanOrEqual => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::GreaterThanOrEqual,
        ),
        TokenKind::And => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::And,
        ),
        TokenKind::Or => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Or,
        ),

        TokenKind::ExclusiveRange => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Range,
        ),

        // -------------------------------
        //  Unexpected tokens
        // -------------------------------
        TokenKind::Wildcard => Err(CompilerDiagnostic::unexpected_token(
            TokenKind::Wildcard,
            token_stream.current_location(),
        )
        .into()),

        TokenKind::TypeParameterBracket => {
            if let Some(error) =
                check_expression_common_mistake(token_stream, state.expression.is_empty())
            {
                return Err(error.into());
            }

            Err(CompilerDiagnostic::unexpected_token(
                TokenKind::TypeParameterBracket,
                token_stream.current_location(),
            )
            .into())
        }

        TokenKind::AddAssign => Err(CompilerDiagnostic::unexpected_token(
            TokenKind::AddAssign,
            token_stream.current_location(),
        )
        .into()),

        _ => {
            if let Some(error) =
                check_expression_common_mistake(token_stream, state.expression.is_empty())
            {
                return Err(error.into());
            }

            Err(CompilerDiagnostic::unexpected_token(token, token_stream.current_location()).into())
        }
    }
}

// -------------------------------
//  Delimiter and terminator tokens
// -------------------------------
fn dispatch_delimiter_token(
    token: TokenKind,
    token_stream: &mut FileTokens,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    if state.expression.is_empty() {
        match token {
            TokenKind::Comma => {
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::Comma,
                    token_stream.current_location(),
                )
                .into());
            }

            TokenKind::Arrow => {
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::Arrow,
                    token_stream.current_location(),
                )
                .into());
            }

            _ => {}
        }
    }

    if state.consume_closing_parenthesis {
        return Err(CompilerDiagnostic::missing_closing_delimiter(
            string_table.intern(")"),
            token_stream.current_location(),
        )
        .into());
    }

    Ok(ExpressionTokenStep::Break)
}

// -------------------------------
//  Close parenthesis
// -------------------------------
fn dispatch_close_parenthesis(
    token_stream: &mut FileTokens,
    state: &mut ExpressionDispatchState<'_>,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    if state.consume_closing_parenthesis {
        token_stream.advance();
    }

    if state.expression.is_empty() {
        return Err(CompilerDiagnostic::unexpected_token(
            TokenKind::CloseParenthesis,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(ExpressionTokenStep::Break)
}

// -------------------------------
//  Newline
// -------------------------------
fn dispatch_newline(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    state: &mut ExpressionDispatchState<'_>,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    // When at the very start of the stream, treat the virtual previous token as a newline
    // so expression continuation logic does not fire.
    let previous_token = if token_stream.index == 0 {
        &TokenKind::Newline
    } else {
        token_stream.previous_token()
    };
    if state.consume_closing_parenthesis
        || (previous_token.continues_expression() && !matches!(previous_token, TokenKind::End))
    {
        token_stream.skip_newlines();
        return Ok(ExpressionTokenStep::Continue);
    }

    // ----------------------------
    //  Lookahead for continuation
    // ----------------------------
    // Look ahead past newlines to find the next meaningful token.
    // If that token continues the expression, skip newlines and keep parsing.
    let saved_index = token_stream.index;
    token_stream.skip_newlines();
    if context.kind == ContextKind::MatchArm
        && current_token_starts_match_arm_header(token_stream).is_some()
    {
        token_stream.index = saved_index;
        return Ok(ExpressionTokenStep::Break);
    }

    if token_stream.index < token_stream.length
        && token_stream.current_token_kind().continues_expression()
    {
        return Ok(ExpressionTokenStep::Continue);
    }
    token_stream.index = saved_index;

    ast_log!("Breaking out of expression with newline");
    Ok(ExpressionTokenStep::Break)
}

// -------------------------------
//  Is operator
// -------------------------------
fn dispatch_is_token(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    match token_stream.peek_next_token() {
        // `is not` → inequality operator.
        Some(TokenKind::Not) => {
            token_stream.advance();
            advance_with_operator(
                state.expression,
                context,
                token_stream.current_location(),
                Operator::NotEqual,
            )
        }

        // `is:` → type guard in a match arm. The left-hand side must be a single expression.
        Some(TokenKind::Colon) => {
            if state.expression.len() > 1 {
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::Colon,
                    token_stream.current_location(),
                )
                .into());
            }

            let value = evaluate_expression(
                context,
                std::mem::take(state.expression),
                type_interner,
                state.expected_type,
                state.value_mode,
                string_table,
            )?;
            Ok(ExpressionTokenStep::Return(Box::new(value)))
        }

        // Bare `is` → equality operator.
        _ => advance_with_operator(
            state.expression,
            context,
            token_stream.current_location(),
            Operator::Equality,
        ),
    }
}

/// Parses an explicit `cast` / `cast!` / `cast ... catch:` expression at a typed boundary.
///
/// WHAT: validates that `cast` starts the expression and that an explicit builtin target was
///      supplied by the boundary, parses the operand and any handling suffix, resolves evidence,
///      and pushes a resolved `ExpressionKind::Cast` node onto the expression stack.
/// WHY: cast is a prefix keyword whose meaning depends on the receiver type, so it is handled
///      directly by the dispatcher rather than the general operator or call machinery.
fn parse_cast_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut ExpressionDispatchState<'_>,
    string_table: &mut StringTable,
) -> Result<ExpressionTokenStep, ExpressionParseError> {
    // `cast` is only valid as the leading token of an expression at an explicit boundary.
    if !state.expression.is_empty() {
        let token = token_stream.current_token_kind().clone();
        return Err(
            CompilerDiagnostic::unexpected_token(token, token_stream.current_location()).into(),
        );
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
                token_stream.current_location(),
            )
            .into());
        }

        CastTargetContext::TargetNotBuiltin { target_type_id } => {
            return Err(CompilerDiagnostic::invalid_cast(
                InvalidCastReason::TargetNotBuiltin,
                None,
                Some(target_type_id),
                token_stream.current_location(),
            )
            .into());
        }

        CastTargetContext::None => {
            return Err(CompilerDiagnostic::invalid_cast(
                InvalidCastReason::MissingExplicitTarget,
                None,
                None,
                token_stream.current_location(),
            )
            .into());
        }
    };

    let cast_location = token_stream.current_location();
    let propagate = token_stream.current_token_kind() == &TokenKind::CastBang;
    token_stream.advance();

    // Attached `cast!` is a lexical token. A standalone `!` after `cast` is a
    // separated spelling and must not be treated as propagation.
    if token_stream.current_token_kind() == &TokenKind::Bang {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::BangMustAttachToCast,
            None,
            None,
            token_stream.current_location(),
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
    )?;

    // `cast!` and `cast ... catch:` are mutually exclusive.
    if propagate && token_stream.current_token_kind() == &TokenKind::Catch {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::PropagationAndRecoveryConflict,
            None,
            None,
            token_stream.current_location(),
        )
        .into());
    }

    // Determine the handling form. Catch handlers need the cast failure error type, which
    // is always the builtin `Error` type for the supported cast evidence catalogue.
    let mut catch_handler = None;
    let handling = if propagate {
        CastHandling::Propagate
    } else if token_stream.current_token_kind() == &TokenKind::Catch {
        let error_type_id =
            resolve_builtin_error_type_typed(context, &operand.location, string_table)?.type_id;
        catch_handler = Some(parse_cast_catch_handling_suffix(
            token_stream,
            context,
            type_interner,
            CastCatchSite {
                success_type_id: target_type_id,
                error_type_id,
                value_required_location: operand.location.clone(),
                allow_boundary_catch: state.allow_boundary_catch,
            },
            string_table,
        )?);
        CastHandling::Recover
    } else {
        CastHandling::Infallible
    };

    // For propagation, validate that the enclosing function can receive the error value.
    if propagate {
        let error_type_id =
            resolve_builtin_error_type_typed(context, &operand.location, string_table)?.type_id;
        let Some(expected_error_type_id) = context.expected_error_type else {
            return Err(CompilerDiagnostic::invalid_cast(
                InvalidCastReason::PropagationRequiresErrorReturn,
                None,
                None,
                token_stream.current_location(),
            )
            .into());
        };

        if !is_postfix_error_compatible(
            expected_error_type_id,
            error_type_id,
            type_interner.environment(),
        ) {
            return Err(CompilerDiagnostic::type_mismatch(
                expected_error_type_id,
                error_type_id,
                TypeMismatchContext::ErrorReturn,
                token_stream.current_location(),
            )
            .into());
        }
    }

    let mut cast_expression = resolve_cast_expression(CastResolutionInput {
        source: operand,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        handling,
        trait_environment: context.trait_environment(),
        trait_evidence_environment: context.trait_evidence_environment(),
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
        active_generic_type_context: context.active_generic_type_context(),
        location: cast_location,
    })?;

    if let Some(handler) = catch_handler {
        cast_expression = wrap_catch_expression(cast_expression, handler, vec![target_type_id]);
    }

    state
        .expression
        .push(ExpressionRpnItem::Operand(cast_expression));

    Ok(ExpressionTokenStep::Continue)
}

fn parse_cast_operand_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let catch_index = find_expression_end_index(
        &token_stream.tokens,
        token_stream.index,
        &[TokenKind::Catch],
    );
    let catch_is_cast_suffix = catch_index < token_stream.length
        && token_stream.tokens[catch_index].kind == TokenKind::Catch;

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
            },
            false,
        );
        return create_expression_until(input, &[TokenKind::Catch]);
    }

    let input = ExpressionParseInput::without_boundary_catch(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode,
            string_table,
        },
        consume_closing_parenthesis,
    );
    create_expression_with_trailing_newline_policy(input)
}

#[cfg(test)]
#[path = "tests/expression_dispatch_tests.rs"]
mod expression_dispatch_tests;
