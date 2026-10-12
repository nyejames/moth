//! Full value-producing match parser.
//!
//! WHAT: parses `if <scrutinee> is: <arms> else => ...` at a closed receiver.
//! WHY: reuses the statement match parser (`parse_match_block`) under an active
//! value target so arms can contain `then` statements; this module does not own
//! statement match parsing itself.

use super::emit_collected_warnings;
use super::result_type::infer_value_match_result_type;
use super::retained::{DeferredValueBlock, DeferredValueBlockForm, RetainedValueBlock};
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::branching::{ParsedMatchBlock, parse_match_block};
use crate::compiler_frontend::ast::statements::match_headers::parse_scrutinee_until_is;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
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

/// Full value matches recurse into statement match bodies, so their result retains internal
/// frozen-token-table failures for the expression parser boundary.
type FullMatchResult<T> = Result<T, ExpressionParseError>;

/// Input for `parse_value_match_at_receiver`.
pub(super) struct ValueMatchParseInput<'a, 'b, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) span: Option<SourceSpan>,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Parses a full value-producing match at a closed receiver.
///
/// WHAT: parses the scrutinee, consumes `is`, delegates to `parse_match_block`,
/// validates completeness, infers the result type, and builds the expression.
pub(super) fn parse_value_match_at_receiver(
    input: ValueMatchParseInput<'_, '_, '_>,
) -> FullMatchResult<ParsedReceiverValue> {
    let ValueMatchParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        span,
        path_fork,
    } = input;

    let scrutinee_context =
        context.new_child_control_flow(ContextKind::Condition, string_table, path_fork);
    let scrutinee = parse_scrutinee_until_is(
        token_stream,
        &scrutinee_context,
        type_interner,
        string_table,
        path_fork,
    )?;

    if token_stream.current_tag() != TokenTag::IS {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
            Some(token_stream.current_span()),
        )
        .into());
    }
    token_stream.advance();

    let receiver_kind = target.receiver_kind;
    let expected_result_type_ids = target.result_type_ids.clone();
    let needs_slot_inference = target.needs_slot_inference();
    let mut warnings = Vec::new();
    let parsed_match = parse_match_block(
        scrutinee,
        token_stream,
        context,
        type_interner,
        &mut warnings,
        Some(target.clone()),
        string_table,
        path_fork,
    )?;
    emit_collected_warnings(context, warnings);

    let ready = match parsed_match {
        ParsedMatchBlock::Ready(ready) => ready,
        // A pending subject, pattern, guard or arm body keeps the whole match until the body root
        // has selected every numeric origin's type; the value finish then runs once with the
        // resolved inputs.
        ParsedMatchBlock::Retained(match_block) => {
            return Ok(ParsedReceiverValue::Retained(Box::new(
                RetainedValueBlock::value_block(
                    DeferredValueBlock {
                        form: DeferredValueBlockForm::FullMatch {
                            match_block: *match_block,
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

    finish_full_value_match(ValueMatchFinishInput {
        scrutinee: ready.scrutinee,
        arms: ready.arms,
        default: ready.default,
        exhaustiveness: ready.exhaustiveness,
        span,
        expected_result_type_ids,
        receiver_kind,
        needs_slot_inference,
        type_interner,
    })
}

/// Input for finishing a full value match whose arm and default bodies are already parsed.
///
/// WHAT: the parsed scrutinee, every authored arm body, the optional default, the match
///       exhaustiveness contract and the receiving routing needed for slot inference.
/// WHY: exhaustiveness, result slots and expression construction must run in one place, whether
///      the bodies came from this parse or from retained bodies resolved later at the enclosing
///      body root.
pub(super) struct ValueMatchFinishInput<'a, 'b> {
    pub(super) scrutinee: Expression,
    pub(super) arms: Vec<MatchArm>,
    pub(super) default: Option<Vec<AstNode>>,
    pub(super) exhaustiveness: MatchExhaustiveness,
    pub(super) span: Option<SourceSpan>,
    pub(super) expected_result_type_ids: Vec<TypeId>,
    pub(super) receiver_kind: ValueReceiverKind,
    pub(super) needs_slot_inference: bool,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
}

/// Finishes a full value match from parsed arm and default bodies.
///
/// WHAT: validates match completeness, infers the result slots, and builds the value-block
///       expression.
/// WHY: the parse path and a resumed retained body must share one construction owner, so no
///      second inference or validation path can disagree with the authored parse.
pub(super) fn finish_full_value_match(
    input: ValueMatchFinishInput<'_, '_>,
) -> FullMatchResult<ParsedReceiverValue> {
    let ValueMatchFinishInput {
        scrutinee,
        arms,
        default,
        exhaustiveness,
        span,
        expected_result_type_ids,
        receiver_kind,
        needs_slot_inference,
        type_interner,
    } = input;

    validate_value_match_completeness(&arms, default.as_deref(), span)?;

    if needs_slot_inference {
        return Ok(ParsedReceiverValue::NeedsSlotInference {
            block: ValueBlock::Match(ValueMatchBlock {
                scrutinee,
                arms,
                default,
                exhaustiveness,
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
    let result_type_ids = if expected_result_type_ids.is_empty() {
        vec![result_type_id]
    } else {
        expected_result_type_ids
    };

    Ok(ParsedReceiverValue::Complete(build_value_match_expression(
        ValueMatchBlock {
            scrutinee,
            arms,
            default,
            exhaustiveness,
            span,
            result_type_ids,
        },
        span,
        result_type_id,
        type_interner.environment(),
    )))
}
