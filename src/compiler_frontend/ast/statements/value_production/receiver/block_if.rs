//! Block-form value-if parser.
//!
//! WHAT: parses `if <condition>: <then-body> else <else-body>` at a closed receiver,
//! validates all-path completeness, and infers the result type from every producing path.
//! WHY: block form is the most general value-producing `if`; body parsing is shared
//! so match and multi-bind do not duplicate `else` and warning forwarding.

use super::ValueIfParseInput;
use super::block_body::{BlockBodyParseInput, parse_value_block_bodies};
use super::result_type::{final_slot_type_ids, infer_block_if_result_type};
use super::retained::{DeferredValueBlock, DeferredValueBlockForm, RetainedValueBlock};
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::numeric_construction::StatementExpression;
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    analyze_branch_exits, validate_closed_branch_pair,
};
use crate::compiler_frontend::ast::statements::value_production::expression_build::build_value_if_expression;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ParsedReceiverValue, ValueBlock, ValueIfBlock, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

/// Block value-if bodies recurse into the AST body parser and therefore preserve its two lanes.
type BlockIfResult<T> = Result<T, ExpressionParseError>;

/// Parses a block-form value-if after the condition has been parsed and `:` is current.
pub(super) fn parse_block_value_if(
    input: ValueIfParseInput<'_, '_, '_>,
) -> BlockIfResult<ParsedReceiverValue> {
    let ValueIfParseInput {
        token_stream,
        context,
        type_interner,
        target,
        string_table,
        condition,
        span,
        path_fork,
    } = input;

    let receiver_kind = target.receiver_kind;
    let expected_result_type_ids = target.result_type_ids.clone();
    let needs_slot_inference = target.needs_slot_inference();
    let bodies = parse_value_block_bodies(BlockBodyParseInput {
        token_stream,
        outer_context: context,
        then_parent: context,
        else_parent: context,
        type_interner,
        string_table,
        active_target: target.clone(),
        path_fork,
    })?;

    let condition = match condition {
        StatementExpression::Built(condition) if !bodies.has_pending() => condition,
        // A pending condition, or a branch still holding numeric continuations, keeps the whole block
        // for the enclosing body root, which resolves the condition and constructs the block once.
        pending_condition => {
            return Ok(ParsedReceiverValue::Retained(Box::new(
                RetainedValueBlock::value_block(
                    DeferredValueBlock {
                        form: DeferredValueBlockForm::If {
                            condition: pending_condition,
                            then_body: bodies.then_body,
                            else_body: bodies.else_body,
                            generic_request_ranges: bodies.generic_request_ranges,
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

    let then_scope = bodies.then_body.scope();
    let else_scope = bodies.else_body.scope();
    let then_body = bodies.then_body.into_nodes("a value-if then branch")?;
    let else_body = bodies.else_body.into_nodes("a value-if else branch")?;

    finish_block_value_if(BlockValueIfFinishInput {
        condition,
        then_body,
        else_body,
        then_scope,
        else_scope,
        span,
        generic_request_ranges: bodies.generic_request_ranges,
        expected_result_type_ids,
        receiver_kind,
        needs_slot_inference,
        type_interner,
    })
}

/// Input for finishing a block value-if whose branch bodies are already concrete.
///
/// WHAT: the parsed condition, both authored branch bodies with their scopes, and the receiving
///       routing needed for slot inference.
/// WHY: branch-closure validation, result-slot inference and expression construction must run in
///      one place, whether the bodies came from this parse or from a retained body the enclosing
///      body root resolved later.
pub(super) struct BlockValueIfFinishInput<'a, 'b> {
    pub(super) condition: Expression,
    pub(super) then_body: Vec<AstNode>,
    pub(super) else_body: Vec<AstNode>,
    pub(super) then_scope: PathId,
    pub(super) else_scope: PathId,
    pub(super) span: Option<SourceSpan>,
    pub(super) generic_request_ranges: IfGenericRequestRanges,
    pub(super) expected_result_type_ids: Vec<TypeId>,
    pub(super) receiver_kind: ValueReceiverKind,
    pub(super) needs_slot_inference: bool,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
}

/// Finishes a block value-if from concrete branch bodies.
///
/// WHAT: analyses both branch exits, validates all-path closure, infers the result slots from the
///       authored producing paths (or keeps them for later multi-slot inference), and builds the
///       value-block expression.
/// WHY: the parse path and a resumed retained body must share one construction owner, so no
///      second inference or validation path can disagree with the authored parse.
pub(super) fn finish_block_value_if(
    input: BlockValueIfFinishInput<'_, '_>,
) -> BlockIfResult<ParsedReceiverValue> {
    let BlockValueIfFinishInput {
        condition,
        then_body,
        else_body,
        then_scope,
        else_scope,
        span,
        generic_request_ranges,
        expected_result_type_ids,
        receiver_kind,
        needs_slot_inference,
        type_interner,
    } = input;

    let then_exits = analyze_branch_exits(&then_body);
    let else_exits = analyze_branch_exits(&else_body);
    validate_closed_branch_pair(then_exits, else_exits, span)?;

    if needs_slot_inference {
        return Ok(ParsedReceiverValue::NeedsSlotInference {
            block: ValueBlock::If(ValueIfBlock {
                condition,
                then_body,
                else_body,
                then_scope,
                else_scope,
                span,
                generic_request_ranges,
                result_type_ids: Vec::new(),
            }),
            span,
        });
    }

    let result_type_id = infer_block_if_result_type(
        &then_body,
        &else_body,
        &expected_result_type_ids,
        type_interner,
        span,
        receiver_kind,
    )?;
    let result_type_ids = final_slot_type_ids(&expected_result_type_ids, result_type_id);

    Ok(ParsedReceiverValue::Complete(build_value_if_expression(
        ValueIfBlock {
            condition,
            then_body,
            else_body,
            then_scope,
            else_scope,
            span,
            generic_request_ranges,
            result_type_ids,
        },
        span,
        result_type_id,
        type_interner.environment(),
    )))
}
