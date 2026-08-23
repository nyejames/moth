//! Receiving-site parser entrypoint for value-producing control flow.
//!
//! WHAT: consumes `if` at closed receiver sites and routes through the shared
//! `if_headers` classifier to inline bool, inline single-predicate match, block
//! if, or full match forms.
//! WHY: this is the only place where `if` is permitted in expression position;
//! general expression parsing continues to reject bare `if` everywhere else.
//!
//! This module must not make value blocks general expressions.

use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_until;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::condition_validation::{
    ensure_if_statement_condition, if_condition_is_missing,
};
use crate::compiler_frontend::ast::statements::if_headers::{IfHeaderShape, classify_if_header};
use crate::compiler_frontend::ast::statements::value_production::types::ValueReceiverKind;
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

mod block_if;
mod expression_build;
mod full_match;
mod inline_if;
mod inline_match;
mod inline_then_else;
mod result_type;
mod single_predicate;
mod token_checkpoint;

// Shared receiver helpers consumed by sibling value-production parsers.
pub(super) use crate::compiler_frontend::ast::statements::value_production::completeness::validate_value_match_completeness;
pub(super) use inline_then_else::same_logical_line;

/// Forwards accumulated parser warnings into the outer scope.
///
/// WHAT: drains a local warning vec and emits each warning through the scope context.
/// WHY: branch-local parsing (e.g. `function_body_to_ast`) may produce warnings
/// that belong to the enclosing receiver site.
pub(super) fn emit_collected_warnings(context: &ScopeContext, warnings: Vec<CompilerDiagnostic>) {
    for warning in warnings {
        context.emit_warning(warning);
    }
}

/// Shared input for inline and block value-if parsers.
///
/// WHAT: bundles the common state needed after the condition has been parsed.
pub(super) struct ValueIfParseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) expected_result_type_ids: &'a [TypeId],
    pub(super) receiver_kind: ValueReceiverKind,
    pub(super) string_table: &'a mut StringTable,
    pub(super) condition: Expression,
    pub(super) location: SourceLocation,
}

/// Attempts to parse a value-producing block when the current token is `if` at a
/// closed receiving site.
///
/// WHAT: returns `None` if the current token is not `If`, otherwise parses the value
/// block and returns the resulting expression. The error preserves authored diagnostics and
/// retained-data infrastructure failures until the enclosing AST emission boundary.
/// WHY: this is the only place where `if` is permitted in expression position;
/// `create_expression` continues to reject it everywhere else.
pub fn try_parse_value_block_at_receiver(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_result_type_ids: &[TypeId],
    receiver_kind: ValueReceiverKind,
    string_table: &mut StringTable,
) -> Option<Result<Expression, ExpressionParseError>> {
    if token_stream.current_token_kind() != &TokenKind::If {
        return None;
    }

    let location = token_stream.current_location();
    token_stream.advance();

    let classification = classify_if_header(token_stream);

    if let Some(reason) = single_predicate::unsupported_optional_single_predicate_reason(
        token_stream,
        context,
        type_interner.environment(),
        classification,
    ) {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            reason,
            token_stream.current_location(),
        )
        .into()));
    }

    match classification.shape {
        IfHeaderShape::FullMatch => Some(full_match::parse_value_match_at_receiver(
            full_match::ValueMatchParseInput {
                token_stream,
                context,
                type_interner,
                expected_result_type_ids,
                receiver_kind,
                string_table,
                location,
            },
        )),

        IfHeaderShape::PotentialInlineSinglePredicate => {
            if let Some(result) = inline_match::try_parse_inline_single_predicate_value_match(
                inline_match::InlineSinglePredicateParseInput {
                    token_stream,
                    context,
                    type_interner,
                    expected_result_type_ids,
                    receiver_kind,
                    string_table,
                    location: location.clone(),
                    classification,
                },
            ) {
                return Some(result);
            }

            Some(parse_bool_value_if_after_condition(
                token_stream,
                context,
                type_interner,
                expected_result_type_ids,
                receiver_kind,
                string_table,
                location,
            ))
        }

        IfHeaderShape::OrdinaryBool | IfHeaderShape::PotentialBlockSinglePredicate => {
            Some(parse_bool_value_if_after_condition(
                token_stream,
                context,
                type_interner,
                expected_result_type_ids,
                receiver_kind,
                string_table,
                location,
            ))
        }
    }
}

/// The receiver parser is the local join between expression parsing and recursive body parsing.
/// It therefore carries the shared two-lane error type instead of collapsing an internal frozen
/// syntax failure into a source diagnostic before the emitter can report it correctly.
type ReceiverResult<T> = Result<T, ExpressionParseError>;

/// Parses a Bool condition value-if after the `if` keyword has been consumed.
///
/// WHAT: parses the condition expression, then routes to inline or block form
/// depending on whether the next token is `then` or `:`.
fn parse_bool_value_if_after_condition(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_result_type_ids: &[TypeId],
    receiver_kind: ValueReceiverKind,
    string_table: &mut StringTable,
    location: SourceLocation,
) -> ReceiverResult<Expression> {
    if if_condition_is_missing(token_stream) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedConditionAfterIf,
            token_stream.current_location(),
        )
        .into());
    }

    let mut condition_type = ExpectedType::Infer;
    let condition_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &condition_context,
        type_interner,
        expected_type: &mut condition_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let condition = create_expression_until(input, &[TokenKind::Then, TokenKind::Colon])?;

    ensure_if_statement_condition(&condition, type_interner.environment())?;

    if token_stream.current_token_kind() == &TokenKind::Then {
        if !same_logical_line(&location, &token_stream.current_location()) {
            return Err(CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::InlineValueIfMultiline,
                token_stream.current_location(),
            )
            .into());
        }

        return inline_if::parse_inline_value_if(ValueIfParseInput {
            token_stream,
            context,
            type_interner,
            expected_result_type_ids,
            receiver_kind,
            string_table,
            condition,
            location,
        });
    }

    if token_stream.current_token_kind() == &TokenKind::Colon {
        return block_if::parse_block_value_if(ValueIfParseInput {
            token_stream,
            context,
            type_interner,
            expected_result_type_ids,
            receiver_kind,
            string_table,
            condition,
            location,
        });
    }

    Err(CompilerDiagnostic::invalid_control_flow_statement(
        InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
        token_stream.current_location(),
    )
    .into())
}
