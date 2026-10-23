//! Inline single-predicate value-match body parsing and assembly.
//!
//! WHAT: consumes a committed single-predicate header and parses
//! `then ... else ...` into a `ValueMatchBlock`.
//! WHY: header classification, scrutinee parsing and pattern eligibility live
//! in the shared single-predicate owner so this file does not rescan `if`.

use super::block_body::ValueBlockBranchBody;
use super::inline_then_else::{
    InlineThenElseInput, InlineThenElseOutcome, parse_inline_then_else, same_logical_line,
};
use super::retained::{DeferredValueBlock, DeferredValueBlockForm, RetainedValueBlock};
use super::single_predicate::{
    ClassifiedSinglePredicateHeader, SinglePredicateHeaderInput, SinglePredicateParseOutcome,
    classify_single_predicate_header,
};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, StatementExpression,
};
use crate::compiler_frontend::ast::statements::if_headers::{
    IfHeaderClassification, IfHeaderDelimiter,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::expression_build::{
    build_value_match_expression, then_value_node,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ParsedReceiverValue, ValueBlock, ValueMatchBlock,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::InvalidControlFlowStatementReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Input for the inline single-predicate body parser after `if` has been consumed.
pub(super) struct InlineSinglePredicateParseInput<'a, 'b, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) header_index: usize,
    pub(super) span: Option<SourceSpan>,
    pub(super) classification: IfHeaderClassification,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Attempts to parse an inline single-predicate value match after `if`.
///
/// WHAT: consumes the shared header parser, then requires same-line `then`.
pub(super) fn try_parse_inline_single_predicate_value_match(
    input: InlineSinglePredicateParseInput<'_, '_, '_>,
) -> SinglePredicateParseOutcome {
    let InlineSinglePredicateParseInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        target,
        string_table,
        header_index,
        span,
        classification,
        path_fork,
    } = input;

    let header = match classify_single_predicate_header(SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        classification,
        path_fork,
    }) {
        Some(Ok(ClassifiedSinglePredicateHeader::Predicate(header))) => *header,
        Some(Ok(ClassifiedSinglePredicateHeader::OrdinarySubject(seed))) => {
            return SinglePredicateParseOutcome::OrdinarySubject(seed);
        }
        Some(Err(error)) => return SinglePredicateParseOutcome::Decided(Err(error)),
        None => return SinglePredicateParseOutcome::Absent,
    };

    if header.body_delimiter != IfHeaderDelimiter::InlineThen
        || token_stream.current_tag() != TokenTag::THEN
    {
        return SinglePredicateParseOutcome::Decided(Err(
            choices_control_flow::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                Some(token_stream.current_span()),
            )
            .into(),
        ));
    }

    if !same_logical_line(token_stream, header_index, token_stream.position()) {
        return SinglePredicateParseOutcome::Decided(Err(
            choices_control_flow::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::InlineValueIfMultiline,
                Some(token_stream.current_span()),
            )
            .into(),
        ));
    }

    SinglePredicateParseOutcome::Decided(parse_inline_value_match(InlineValueMatchParseInput {
        token_stream,
        context,
        then_context: &header.then_context,
        type_interner,
        expression_builder,
        const_values,
        target,
        string_table,
        scrutinee: header.scrutinee,
        pattern: header.pattern,
        span,
        path_fork,
    }))
}

struct InlineValueMatchParseInput<'a, 'b, 'tokens> {
    token_stream: &'a mut AstCursor<'tokens>,
    context: &'a ScopeContext,
    then_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    target: ActiveValueProductionTarget,
    string_table: &'a mut StringTable,
    scrutinee: StatementExpression,
    pattern: DeferredArmPattern,
    span: Option<SourceSpan>,
    path_fork: &'a mut PathInternerFork,
}

/// The speculative outer parser may discard only authored diagnostics. Once a match shape is
/// accepted, the inner parser retains `CompilerError` until AST emission.
type InlineValueMatchResult<T> = Result<T, ExpressionParseError>;

fn parse_inline_value_match(
    input: InlineValueMatchParseInput<'_, '_, '_>,
) -> InlineValueMatchResult<ParsedReceiverValue> {
    let InlineValueMatchParseInput {
        token_stream,
        context,
        then_context,
        type_interner,
        expression_builder,
        const_values,
        target,
        string_table,
        scrutinee,
        pattern,
        span,
        path_fork,
    } = input;

    let output = match parse_inline_then_else(InlineThenElseInput {
        token_stream,
        then_context,
        else_context: context,
        type_interner,
        expression_builder,
        const_values,
        // The target still rides on the retained form when the subject or pattern has to resolve,
        // so the authored inline bodies parse against a copy of the same receiving context.
        target: target.clone(),
        string_table,
        path_fork,
    })? {
        InlineThenElseOutcome::Complete(output) => output,
        InlineThenElseOutcome::Retained {
            then_body,
            else_body,
            generic_request_ranges: _,
        } => {
            // A branch still depends on an unresolved numeric origin, so the committed header and
            // the authored inline bodies travel together to the enclosing body root.
            return Ok(ParsedReceiverValue::Retained(Box::new(
                RetainedValueBlock::value_block(
                    DeferredValueBlock {
                        form: DeferredValueBlockForm::SinglePredicate {
                            scrutinee,
                            pattern,
                            then_body: then_body.into_value_block_body(),
                            else_body: else_body.into_value_block_body(),
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
        then_context.scope,
    )];
    let else_body = vec![then_value_node(
        output.else_values,
        output.else_span,
        context.scope,
    )];

    // A subject or pattern that still holds numeric continuations cannot build the match here:
    // the committed header, the authored inline bodies and the match's own form travel to the
    // enclosing body root, where the block's constructor runs once with the resolved inputs.
    let (scrutinee, pattern) = match (scrutinee, pattern) {
        (StatementExpression::Built(scrutinee), DeferredArmPattern::Ready(pattern)) => {
            (scrutinee, pattern)
        }
        (scrutinee, pattern) => {
            return Ok(ParsedReceiverValue::Retained(Box::new(
                RetainedValueBlock::value_block(
                    DeferredValueBlock {
                        form: DeferredValueBlockForm::SinglePredicate {
                            scrutinee,
                            pattern,
                            then_body: ValueBlockBranchBody::Nodes {
                                nodes: then_body,
                                scope: then_context.scope,
                            },
                            else_body: ValueBlockBranchBody::Nodes {
                                nodes: else_body,
                                scope: context.scope,
                            },
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

    let value_match = ValueMatchBlock {
        scrutinee,
        arms: vec![MatchArm {
            pattern,
            guard: None,
            body: then_body,
        }],
        default: Some(else_body),
        exhaustiveness: MatchExhaustiveness::HasDefault,
        span,
        result_type_ids: output.result_type_ids,
    };

    Ok(match output.needs_slot_inference {
        false => ParsedReceiverValue::Complete(build_value_match_expression(
            value_match,
            span,
            expression_builder,
        )?),
        true => ParsedReceiverValue::NeedsSlotInference {
            block: ValueBlock::Match(value_match),
            span,
        },
    })
}
