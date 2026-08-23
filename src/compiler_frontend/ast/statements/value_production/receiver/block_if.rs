//! Block-form value-if parser.
//!
//! WHAT: parses `if <condition>: <then-body> else <else-body>` at a closed receiver,
//! validates all-path completeness, and infers the result type from every producing path.
//! WHY: block form is the most general value-producing `if`; it uses
//! `function_body_to_ast` so nested control flow and multiple statements are
//! permitted inside each branch.

use super::expression_build::build_value_if_expression;
use super::result_type::{final_slot_type_ids, infer_block_if_result_type};
use super::{ValueIfParseInput, emit_collected_warnings};
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::body_dispatch::parse_function_body_statements;
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_value_if_completeness;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueIfBlock,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;

/// Block value-if bodies recurse into the AST body parser and therefore preserve its two lanes.
type BlockIfResult<T> = Result<T, ExpressionParseError>;

/// Parses a block-form value-if after the condition has been parsed and `:` consumed.
///
/// WHAT: sets up active value targets for both branches, parses bodies, validates
/// branch flow, infers the result type, and builds the `ValueBlock::If` expression.
pub(super) fn parse_block_value_if(input: ValueIfParseInput<'_, '_>) -> BlockIfResult<Expression> {
    let ValueIfParseInput {
        token_stream,
        context,
        type_interner,
        expected_result_type_ids,
        receiver_kind,
        string_table,
        condition,
        location,
    } = input;

    token_stream.advance(); // consume `:`

    let active_target = ActiveValueProductionTarget {
        result_type_ids: expected_result_type_ids.to_vec(),
        receiver_kind,
        expected_arity: None,
    };

    let mut then_context = context.new_child_control_flow(ContextKind::Branch, string_table);
    then_context.active_value_target = Some(active_target.clone());
    let mut then_warnings = Vec::new();
    let then_body = parse_function_body_statements(
        token_stream,
        then_context,
        type_interner,
        &mut then_warnings,
        string_table,
    )?;
    emit_collected_warnings(context, then_warnings);

    if token_stream.current_token_kind() != &TokenKind::Else {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfMissingElse,
            token_stream.current_location(),
        )
        .into());
    }
    token_stream.advance(); // consume `else`

    let mut else_context = context.new_child_control_flow(ContextKind::Branch, string_table);
    else_context.active_value_target = Some(active_target);
    let mut else_warnings = Vec::new();
    let else_body = parse_function_body_statements(
        token_stream,
        else_context,
        type_interner,
        &mut else_warnings,
        string_table,
    )?;
    emit_collected_warnings(context, else_warnings);

    validate_value_if_completeness(&then_body, &else_body, &location)?;

    let result_type_id = infer_block_if_result_type(
        &then_body,
        &else_body,
        expected_result_type_ids,
        type_interner,
        &location,
        receiver_kind,
    )?;
    let result_type_ids = final_slot_type_ids(expected_result_type_ids, result_type_id);

    let value_if = ValueIfBlock {
        condition,
        then_body,
        else_body,
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_if_expression(
        value_if,
        result_type_id,
        type_interner.environment(),
    ))
}
