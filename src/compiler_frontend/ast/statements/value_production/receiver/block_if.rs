//! Block-form value-if parser.
//!
//! WHAT: parses `if <condition>: <then-body> else <else-body>` at a closed receiver,
//! validates all-path completeness, and infers the result type from every producing path.
//! WHY: block form is the most general value-producing `if`; body parsing is shared
//! so match and multi-bind do not duplicate `else` and warning forwarding.

use super::ValueIfParseInput;
use super::block_body::{BlockBodyParseInput, parse_value_block_bodies};
use super::result_type::{final_slot_type_ids, infer_block_if_result_type};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_closed_branch_pair;
use crate::compiler_frontend::ast::statements::value_production::expression_build::build_value_if_expression;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueIfBlock,
};

/// Block value-if bodies recurse into the AST body parser and therefore preserve its two lanes.
type BlockIfResult<T> = Result<T, ExpressionParseError>;

/// Parses a block-form value-if after the condition has been parsed and `:` is current.
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

    let bodies = parse_value_block_bodies(BlockBodyParseInput {
        token_stream,
        outer_context: context,
        then_parent: context,
        else_parent: context,
        type_interner,
        string_table,
        active_target: ActiveValueProductionTarget {
            result_type_ids: expected_result_type_ids.to_vec(),
            receiver_kind,
            expected_arity: None,
        },
    })?;

    validate_closed_branch_pair(bodies.then_exits, bodies.else_exits, &location)?;

    let result_type_id = infer_block_if_result_type(
        &bodies.then_body,
        &bodies.else_body,
        expected_result_type_ids,
        type_interner,
        &location,
        receiver_kind,
    )?;
    let result_type_ids = final_slot_type_ids(expected_result_type_ids, result_type_id);

    let value_if = ValueIfBlock {
        condition,
        then_body: bodies.then_body,
        else_body: bodies.else_body,
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_if_expression(
        value_if,
        result_type_id,
        type_interner.environment(),
    ))
}
