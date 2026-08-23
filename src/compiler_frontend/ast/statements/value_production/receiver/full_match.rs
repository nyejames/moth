//! Full value-producing match parser.
//!
//! WHAT: parses `if <scrutinee> is: <arms> else => ...` at a closed receiver.
//! WHY: reuses the statement match parser (`parse_match_block`) under an active
//! value target so arms can contain `then` statements; this module does not own
//! statement match parsing itself.

use super::emit_collected_warnings;
use super::expression_build::build_value_match_expression;
use super::result_type::infer_value_match_result_type;
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_until;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::branching::parse_match_block;
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_value_match_completeness;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueMatchBlock, ValueReceiverKind,
};
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

/// Full value matches recurse into statement match bodies, so their result retains internal
/// frozen-token-table failures for the expression parser boundary.
type FullMatchResult<T> = Result<T, ExpressionParseError>;

/// Input for `parse_value_match_at_receiver`.
pub(super) struct ValueMatchParseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) expected_result_type_ids: &'a [TypeId],
    pub(super) receiver_kind: ValueReceiverKind,
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
        expected_result_type_ids,
        receiver_kind,
        string_table,
        location,
    } = input;

    let mut scrutinee_type = ExpectedType::Infer;
    let scrutinee_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &scrutinee_context,
        type_interner,
        expected_type: &mut scrutinee_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let scrutinee = create_expression_until(input, &[TokenKind::Is])?;

    if token_stream.current_token_kind() != &TokenKind::Is {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            token_stream.current_location(),
        )
        .into());
    }
    token_stream.advance();

    let active_target = ActiveValueProductionTarget {
        result_type_ids: expected_result_type_ids.to_vec(),
        receiver_kind,
        expected_arity: None,
    };
    let mut warnings = Vec::new();
    let parsed_match = parse_match_block(
        scrutinee,
        token_stream,
        context,
        type_interner,
        &mut warnings,
        Some(active_target),
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
        expected_result_type_ids,
        type_interner,
        &location,
        receiver_kind,
    )?;
    let result_type_ids = if expected_result_type_ids.is_empty() {
        vec![result_type_id]
    } else {
        expected_result_type_ids.to_vec()
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
