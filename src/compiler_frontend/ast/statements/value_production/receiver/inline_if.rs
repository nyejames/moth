//! Inline Bool condition value-if parser.
//!
//! WHAT: parses `if <Bool> then <expr> else <expr>` at a closed receiver.
//! WHY: this is the simplest value-producing block form; it delegates all
//! then/else structural validation to `inline_then_else.rs`.

use super::ValueIfParseInput;
use super::inline_then_else::{InlineThenElseInput, parse_inline_then_else};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::value_production::expression_build::{
    build_value_if_expression, then_value_node,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueIfBlock;

/// Inline branch expression parsing can encounter a retained-data lifecycle failure, so this
/// private join uses the AST expression error lane rather than pre-rendering it as a diagnostic.
type InlineIfResult<T> = Result<T, ExpressionParseError>;

/// Parses an inline Bool value-if after the condition has been parsed.
///
/// WHAT: assumes the current token is `then`. Parses then/else branches and
/// builds a `ValueBlock::If` expression.
/// WHY: kept separate from inline match so it does not know about option/choice
/// single-predicate matching.
pub(super) fn parse_inline_value_if(
    input: ValueIfParseInput<'_, '_>,
) -> InlineIfResult<Expression> {
    let ValueIfParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        condition,
        location,
    } = input;

    let output = parse_inline_then_else(InlineThenElseInput {
        token_stream,
        then_context: context,
        else_context: context,
        type_interner,
        target,
        string_table,
    })?;

    let then_body = vec![then_value_node(
        output.then_values,
        location.clone(),
        context.scope.clone(),
    )];
    let else_body = vec![then_value_node(
        output.else_values,
        location.clone(),
        context.scope.clone(),
    )];

    let value_if = ValueIfBlock {
        condition,
        then_body,
        else_body,
        location: location.clone(),
        result_type_ids: output.result_type_ids,
    };

    Ok(build_value_if_expression(
        value_if,
        output.result_type_id,
        type_interner.environment(),
    ))
}
