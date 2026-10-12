//! Block-form single-predicate value-match assembly.
//!
//! WHAT: consumes a committed option or choice header and parses
//! `: <then-body> else <else-body>` into one-arm `ValueMatchBlock`.
//! WHY: header classification, scrutinee parsing and pattern/capture
//! ownership live in shared owners. This file must not rescan `if`,
//! parse patterns, or run an independent completeness algorithm.

use super::block_body::{BlockBodyParseInput, ParsedValueBlockBodies, parse_value_block_bodies};
use super::result_type::{final_slot_type_ids, infer_value_match_result_type};
use super::retained::{DeferredValueBlock, DeferredValueBlockForm, RetainedValueBlock};
use super::single_predicate::{SinglePredicateHeaderInput, try_parse_single_predicate_header};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, StatementExpression,
};
use crate::compiler_frontend::ast::statements::if_headers::{
    IfHeaderClassification, IfHeaderDelimiter,
};
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::completeness::validate_value_match_completeness;
use crate::compiler_frontend::ast::statements::value_production::expression_build::build_value_match_expression;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ParsedReceiverValue, ValueBlock, ValueMatchBlock,
    ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Input for the block single-predicate body parser after `if` has been consumed.
pub(super) struct BlockSinglePredicateParseInput<'a, 'b, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) span: Option<SourceSpan>,
    pub(super) classification: IfHeaderClassification,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Attempts to parse a block single-predicate value match after `if`.
///
/// WHAT: consumes the shared header parser, then requires `:`.
/// Returns `None` if the header is not a committed option/choice predicate so
pub(super) fn try_parse_block_single_predicate_value_match(
    input: BlockSinglePredicateParseInput<'_, '_, '_>,
) -> Option<Result<ParsedReceiverValue, ExpressionParseError>> {
    let BlockSinglePredicateParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        classification,
        span,
        path_fork,
    } = input;

    let header = match try_parse_single_predicate_header(SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        string_table,
        classification,
        path_fork,
    }) {
        Some(Ok(header)) => header,
        Some(Err(error)) => return Some(Err(error)),
        None => return None,
    };

    token_stream.skip_newlines();

    if header.body_delimiter != IfHeaderDelimiter::Colon
        || token_stream.current_tag() != TokenTag::COLON
    {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            Some(token_stream.current_span()),
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
        span,
        path_fork,
    }))
}

struct BlockValueMatchParseInput<'a, 'b, 'tokens> {
    token_stream: &'a mut AstCursor<'tokens>,
    context: &'a ScopeContext,
    then_parent: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    target: ActiveValueProductionTarget,
    string_table: &'a mut StringTable,
    scrutinee: StatementExpression,
    pattern: DeferredArmPattern,
    span: Option<SourceSpan>,
    path_fork: &'a mut PathInternerFork,
}

type BlockValueMatchResult<T> = Result<T, ExpressionParseError>;

fn parse_block_value_match(
    input: BlockValueMatchParseInput<'_, '_, '_>,
) -> BlockValueMatchResult<ParsedReceiverValue> {
    let BlockValueMatchParseInput {
        token_stream,
        context,
        then_parent,
        type_interner,
        target,
        string_table,
        scrutinee,
        pattern,
        span,
        path_fork,
    } = input;

    let receiver_kind = target.receiver_kind;
    let expected_result_type_ids = target.result_type_ids.clone();
    let needs_slot_inference = target.needs_slot_inference();
    let bodies = parse_value_block_bodies(BlockBodyParseInput {
        token_stream,
        outer_context: context,
        then_parent,
        else_parent: context,
        type_interner,
        string_table,
        active_target: target.clone(),
        path_fork,
    })?;

    // A subject, pattern or branch body that still holds numeric continuations cannot be built
    // here: the committed header and this match's own form travel to the enclosing body root,
    // where the constructor below runs once with the resolved inputs.
    let (scrutinee, pattern) = match (scrutinee, pattern) {
        (StatementExpression::Built(scrutinee), DeferredArmPattern::Ready(pattern)) => {
            (scrutinee, pattern)
        }
        (scrutinee, pattern) => {
            return Ok(retained_single_predicate_value_block(
                scrutinee, pattern, bodies, span, target,
            ));
        }
    };

    if bodies.has_pending() {
        // A branch body still holds numeric continuations, so this match cannot be built here.
        // The committed header and the block's own form travel to the enclosing body root, where
        // the constructor runs once with the resolved bodies.
        return Ok(retained_single_predicate_value_block(
            StatementExpression::Built(scrutinee),
            DeferredArmPattern::Ready(pattern),
            bodies,
            span,
            target,
        ));
    }

    let then_body = bodies
        .then_body
        .into_nodes("a single-predicate match then branch")?;
    let else_body = bodies
        .else_body
        .into_nodes("a single-predicate match else branch")?;

    finish_block_value_match(BlockValueMatchFinishInput {
        scrutinee,
        pattern,
        then_body,
        else_body,
        span,
        expected_result_type_ids,
        receiver_kind,
        needs_slot_inference,
        type_interner,
    })
}

/// Retains a single-predicate value match whose header or branch bodies still hold numeric
/// continuations.
///
/// WHAT: packs the authored scrutinee, the committed pattern and both branch bodies into the
///       block's own deferred form.
/// WHY: the enclosing body root owns numeric resolution, so the block travels intact instead of
///      publishing a half-resolved branch or replacing a retained statement with a placeholder.
fn retained_single_predicate_value_block(
    scrutinee: StatementExpression,
    pattern: DeferredArmPattern,
    bodies: ParsedValueBlockBodies,
    span: Option<SourceSpan>,
    target: ActiveValueProductionTarget,
) -> ParsedReceiverValue {
    ParsedReceiverValue::Retained(Box::new(RetainedValueBlock::value_block(
        DeferredValueBlock {
            form: DeferredValueBlockForm::SinglePredicate {
                scrutinee,
                pattern,
                then_body: bodies.then_body,
                else_body: bodies.else_body,
            },
            span,
            target,
            resolved_slot_types: None,
        },
        span,
    )))
}

/// Input for finishing a block single-predicate value match from concrete branch bodies.
///
/// WHAT: the parsed scrutinee, the committed pattern, both authored branch bodies and the
///       receiving routing needed for slot inference.
/// WHY: match completeness, result slots and expression construction must run in one place,
///      whether the bodies came from this parse or from retained bodies resolved later at the
///      enclosing body root.
pub(super) struct BlockValueMatchFinishInput<'a, 'b> {
    pub(super) scrutinee: Expression,
    pub(super) pattern: MatchPattern,
    pub(super) then_body: Vec<AstNode>,
    pub(super) else_body: Vec<AstNode>,
    pub(super) span: Option<SourceSpan>,
    pub(super) expected_result_type_ids: Vec<TypeId>,
    pub(super) receiver_kind: ValueReceiverKind,
    pub(super) needs_slot_inference: bool,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
}

/// Finishes a block single-predicate value match from concrete branch bodies.
///
/// WHAT: consumes the committed pattern as the only arm, validates match completeness, infers the
///       result slots and builds the value-block expression.
/// WHY: the parse path and a resumed retained body must share one construction owner, so no
///      second inference or validation path can disagree with the authored parse.
pub(super) fn finish_block_value_match(
    input: BlockValueMatchFinishInput<'_, '_>,
) -> BlockValueMatchResult<ParsedReceiverValue> {
    let BlockValueMatchFinishInput {
        scrutinee,
        pattern,
        then_body,
        else_body,
        span,
        expected_result_type_ids,
        receiver_kind,
        needs_slot_inference,
        type_interner,
    } = input;

    let arms = vec![MatchArm {
        pattern,
        guard: None,
        body: then_body,
    }];
    let default = Some(else_body);

    validate_value_match_completeness(&arms, default.as_deref(), span)?;

    if needs_slot_inference {
        return Ok(ParsedReceiverValue::NeedsSlotInference {
            block: ValueBlock::Match(ValueMatchBlock {
                scrutinee,
                arms,
                default,
                exhaustiveness: MatchExhaustiveness::HasDefault,
                span,
                result_type_ids: Vec::new(),
            }),
            span,
        });
    }

    let result_type_id = infer_value_match_result_type(
        &arms,
        default.as_deref(),
        &expected_result_type_ids,
        type_interner,
        span,
        receiver_kind,
    )?;
    let result_type_ids = final_slot_type_ids(&expected_result_type_ids, result_type_id);

    Ok(ParsedReceiverValue::Complete(build_value_match_expression(
        ValueMatchBlock {
            scrutinee,
            arms,
            default,
            exhaustiveness: MatchExhaustiveness::HasDefault,
            span,
            result_type_ids,
        },
        span,
        result_type_id,
        type_interner.environment(),
    )))
}
