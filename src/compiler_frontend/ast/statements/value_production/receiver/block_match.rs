//! Block-form single-predicate value-match assembly.
//!
//! WHAT: consumes a committed option or choice header and parses
//! `: <then-body> else <else-body>` into one-arm `ValueMatchBlock`.
//! WHY: header classification, scrutinee parsing and pattern/capture
//! ownership live in shared owners. This file must not rescan `if`,
//! parse patterns, or run an independent completeness algorithm.

use super::block_body::{BlockBodyParseInput, parse_value_block_bodies};
use super::result_type::{final_slot_type_ids, infer_value_match_result_type};
use super::single_predicate::{SinglePredicateHeaderInput, try_parse_single_predicate_header};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::if_headers::{
    IfHeaderClassification, IfHeaderDelimiter,
};
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
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

/// Input for the block single-predicate body parser after `if` has been consumed.
pub(super) struct BlockSinglePredicateParseInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) location: SourceLocation,
    pub(super) classification: IfHeaderClassification,
}

/// Attempts to parse a block single-predicate value match after `if`.
///
/// WHAT: consumes the shared header parser, then requires `:`.
/// Returns `None` if the header is not a committed option/choice predicate so
/// the caller can fall back to Bool condition parsing.
pub(super) fn try_parse_block_single_predicate_value_match(
    input: BlockSinglePredicateParseInput<'_, '_>,
) -> Option<Result<Expression, ExpressionParseError>> {
    let BlockSinglePredicateParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        location,
        classification,
    } = input;

    let header = match try_parse_single_predicate_header(SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        string_table,
        classification,
    }) {
        Some(Ok(header)) => header,
        Some(Err(error)) => return Some(Err(error)),
        None => return None,
    };

    token_stream.skip_newlines();

    if header.body_delimiter != IfHeaderDelimiter::Colon
        || token_stream.current_token_kind() != &TokenKind::Colon
    {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            token_stream.current_location(),
        )
        .into()));
    }

    Some(parse_block_value_match(BlockValueMatchParseInput {
        token_stream,
        context,
        then_parent: &header.then_context,
        type_interner,
        target,
        string_table,
        scrutinee: header.scrutinee,
        pattern: header.pattern,
        location,
    }))
}

struct BlockValueMatchParseInput<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    then_parent: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    target: ActiveValueProductionTarget,
    string_table: &'a mut StringTable,
    scrutinee: Expression,
    pattern: MatchPattern,
    location: SourceLocation,
}

type BlockValueMatchResult<T> = Result<T, ExpressionParseError>;

fn parse_block_value_match(
    input: BlockValueMatchParseInput<'_, '_>,
) -> BlockValueMatchResult<Expression> {
    let BlockValueMatchParseInput {
        token_stream,
        context,
        then_parent,
        type_interner,
        target,
        string_table,
        scrutinee,
        pattern,
        location,
    } = input;

    let receiver_kind = target.receiver_kind;
    let expected_result_type_ids = target.result_type_ids.clone();
    let bodies = parse_value_block_bodies(BlockBodyParseInput {
        token_stream,
        outer_context: context,
        then_parent,
        else_parent: context,
        type_interner,
        string_table,
        active_target: target,
    })?;

    let arms = vec![MatchArm {
        pattern,
        guard: None,
        body: bodies.then_body,
    }];
    let default = Some(bodies.else_body);

    validate_value_match_completeness(&arms, default.as_deref(), &location)?;

    let result_type_id = infer_value_match_result_type(
        &arms,
        default.as_deref(),
        &expected_result_type_ids,
        type_interner,
        &location,
        receiver_kind,
    )?;
    let result_type_ids = final_slot_type_ids(&expected_result_type_ids, result_type_id);

    let value_match = ValueMatchBlock {
        scrutinee,
        arms,
        default,
        exhaustiveness: MatchExhaustiveness::HasDefault,
        location: location.clone(),
        result_type_ids,
    };

    Ok(build_value_match_expression(
        value_match,
        result_type_id,
        type_interner.environment(),
    ))
}
