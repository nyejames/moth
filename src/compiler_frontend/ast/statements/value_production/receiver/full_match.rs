//! Full value-producing match parser.
//!
//! WHAT: parses `if <scrutinee> is: <arms> else => ...` at a closed receiver.
//! WHY: reuses the statement match parser (`parse_match_block`) under an active
//! value target so arms can contain `then` statements; this module does not own
//! statement match parsing itself.

use super::emit_collected_warnings;
use super::result_type::infer_value_match_result_type;
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::branching::parse_match_block;
use crate::compiler_frontend::ast::statements::match_headers::parse_scrutinee_until_is;
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_value_match_completeness;
use crate::compiler_frontend::ast::statements::value_production::expression_build::build_value_match_expression;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueMatchBlock,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

/// Full value matches recurse into statement match bodies, so their result retains internal
/// frozen-token-table failures for the expression parser boundary.
type FullMatchResult<T> = Result<T, ExpressionParseError>;

/// Input for `parse_value_match_at_receiver`.
pub(super) struct ValueMatchParseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) location: SourceLocation,
}

/// Parses a full value-producing match at a closed receiver.
///
/// WHAT: parses the scrutinee, consumes `is`, delegates to `parse_match_block`,
/// validates completeness, infers the result type, and builds the expression.
pub(super) fn parse_value_match_at_receiver(
    input: ValueMatchParseInput<'_, '_>,
) -> FullMatchResult<Expression> {
    let ValueMatchParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        location,
    } = input;

    let scrutinee_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let scrutinee = parse_scrutinee_until_is(
        token_stream,
        &scrutinee_context,
        type_interner,
        string_table,
    )?;

    if token_stream.current_token_kind() != &TokenKind::Is {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            token_stream.current_location(),
        )
        .into());
    }
    token_stream.advance();

    let receiver_kind = target.receiver_kind;
    let expected_result_type_ids = target.result_type_ids.clone();
    let mut warnings = Vec::new();
    let parsed_match = parse_match_block(
        scrutinee,
        token_stream,
        context,
        type_interner,
        &mut warnings,
        Some(target),
        string_table,
    )?;
    emit_collected_warnings(context, warnings);

    validate_value_match_completeness(
        &parsed_match.arms,
        parsed_match.default.as_deref(),
        &location,
    )?;

    let result_type_id = infer_value_match_result_type(
        &parsed_match.arms,
        parsed_match.default.as_deref(),
        &expected_result_type_ids,
        type_interner,
        &location,
        receiver_kind,
    )?;
    let result_type_ids = if expected_result_type_ids.is_empty() {
        vec![result_type_id]
    } else {
        expected_result_type_ids
    };

    let value_match = ValueMatchBlock {
        scrutinee: parsed_match.scrutinee,
        arms: parsed_match.arms,
        default: parsed_match.default,
        exhaustiveness: parsed_match.exhaustiveness,
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_match_expression(
        value_match,
        result_type_id,
        type_interner.environment(),
    ))
}
