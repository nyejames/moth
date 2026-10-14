//! Inline Bool condition value-if parser.
//!
//! WHAT: parses `if <Bool> then <expr> else <expr>` at a closed receiver.
//! WHY: this is the simplest value-producing block form; it delegates all
//! then/else structural validation to `inline_then_else.rs`.

use super::ValueIfParseInput;
use super::block_body::ValueBlockBranchBody;
use super::inline_then_else::{InlineThenElseInput, InlineThenElseOutcome, parse_inline_then_else};
use super::retained::{DeferredValueBlock, DeferredValueBlockForm, RetainedValueBlock};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::numeric_construction::StatementExpression;
use crate::compiler_frontend::ast::statements::value_production::expression_build::{
    build_value_if_expression, then_value_node,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ParsedReceiverValue, ValueBlock, ValueIfBlock,
};

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
    input: ValueIfParseInput<'_, '_, '_>,
) -> InlineIfResult<ParsedReceiverValue> {
    let ValueIfParseInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        target,
        string_table,
        condition,
        span,
        path_fork,
    } = input;

    let output = match parse_inline_then_else(InlineThenElseInput {
        token_stream,
        then_context: context,
        else_context: context,
        type_interner,
        expression_builder,
        const_values,
        // The authored block keeps the same active production target when a branch has to
        // resolve at the body root, so the parse runs against a copy of that exact target.
        target: target.clone(),
        string_table,
        path_fork,
    })? {
        InlineThenElseOutcome::Complete(output) => output,
        InlineThenElseOutcome::Retained {
            then_body,
            else_body,
            generic_request_ranges,
        } => {
            // A branch still depends on an unresolved numeric origin, so the whole inline value-if
            // travels to the enclosing body root, which resolves both branches and builds this
            // block once — never a placeholder value or a second parse.
            return Ok(ParsedReceiverValue::Retained(Box::new(
                RetainedValueBlock::value_block(
                    DeferredValueBlock {
                        form: DeferredValueBlockForm::If {
                            condition,
                            then_body: then_body.into_value_block_body(),
                            else_body: else_body.into_value_block_body(),
                            generic_request_ranges,
                        },
                        span,
                        target,
                        resolved_slot_types: None,
                    },
                    span,
                ),
            )));
        }
    };

    let condition = match condition {
        StatementExpression::Built(condition) => condition,
        // Both branches completed, but the condition still depends on a pending origin, so the whole
        // value-if waits for the body root and keeps its authored branch nodes unchanged.
        pending_condition => {
            return Ok(ParsedReceiverValue::Retained(Box::new(
                RetainedValueBlock::value_block(
                    DeferredValueBlock {
                        form: DeferredValueBlockForm::If {
                            condition: pending_condition,
                            then_body: ValueBlockBranchBody::Nodes {
                                nodes: vec![then_value_node(
                                    output.then_values,
                                    output.then_span,
                                    context.scope,
                                )],
                                scope: context.scope,
                            },
                            else_body: ValueBlockBranchBody::Nodes {
                                nodes: vec![then_value_node(
                                    output.else_values,
                                    output.else_span,
                                    context.scope,
                                )],
                                scope: context.scope,
                            },
                            generic_request_ranges: output.generic_request_ranges,
                        },
                        span,
                        target,
                        resolved_slot_types: None,
                    },
                    span,
                ),
            )));
        }
    };

    let then_body = vec![then_value_node(
        output.then_values,
        output.then_span,
        context.scope,
    )];
    let else_body = vec![then_value_node(
        output.else_values,
        output.else_span,
        context.scope,
    )];

    let value_if = ValueIfBlock {
        condition,
        then_body,
        else_body,
        then_scope: context.scope,
        else_scope: context.scope,
        span,
        generic_request_ranges: output.generic_request_ranges,
        result_type_ids: output.result_type_ids,
    };

    Ok(match output.needs_slot_inference {
        false => ParsedReceiverValue::Complete(build_value_if_expression(
            value_if,
            span,
            expression_builder,
        )?),
        true => ParsedReceiverValue::NeedsSlotInference {
            block: ValueBlock::If(value_if),
            span,
        },
    })
}
