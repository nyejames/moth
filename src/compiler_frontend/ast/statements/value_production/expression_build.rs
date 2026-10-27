//! Completed value-control-flow construction. Branch leaves keep durable IDs; results retain ownership.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CompletedExpressionInput, ExprId, ExpressionConstructionFailure,
    ExpressionOp, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::failure_facts::ExpressionFailureFacts;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ProducedValues, ValueBlock, ValueIfBlock, ValueLexicalScope, ValueMatchBlock,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::value_mode::ValueMode;

pub(in crate::compiler_frontend::ast::statements::value_production) fn then_value_node(
    expressions: Vec<ExprId>,
    span: Option<SourceSpan>,
    scope: PathId,
) -> AstNode {
    AstNode {
        kind: NodeKind::ThenValue(ProducedValues { expressions, span }),
        span,
        scope,
    }
}

/// Commits the replacement for a folded Host call with zero or several results. The replacement is
/// one `LexicalScope` block holding a `Constant` row per result slot, in result order, under the
/// authored scope. A zero-result call gets an empty body, so its block stays a valid expression
/// statement. The block keeps the call's success shape, span, value mode, classification and
/// occurrence facts. `failure` is the authored eligibility of the call it replaces.
pub(crate) fn commit_constant_result_block(
    builder: &mut AstExpressionBuilder,
    call: ExprId,
    slots: &[ConstValueId],
    scope: PathId,
    failure: Option<ExpressionFailureFacts>,
) -> Result<OwnedExpr, ExpressionConstructionFailure> {
    let (success, span, value_mode, classification, occurrence, result_type_ids) = {
        let view = builder.view(call)?;
        (
            view.success_shape(),
            view.span(),
            view.value_mode(),
            view.classification(),
            view.occurrence_facts().cloned(),
            view.success_types().to_vec(),
        )
    };
    if slots.len() != result_type_ids.len() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "Folded call result slots disagree with the call's success types.",
        )
        .into());
    }
    let mut rows = Vec::with_capacity(slots.len());
    for (slot, type_id) in slots.iter().zip(&result_type_ids) {
        rows.push(builder.constant(*slot, *type_id, span, value_mode)?.id());
    }
    let body = if rows.is_empty() {
        Vec::new()
    } else {
        vec![then_value_node(rows, span, scope)]
    };
    let block = builder.append_value_block(
        ValueBlock::LexicalScope(ValueLexicalScope {
            body,
            scope,
            result_type_ids,
        }),
        span,
    )?;
    builder.commit_replacement(CompletedExpressionInput {
        operation: ExpressionOp::ValueBlock(block),
        success,
        error_type: None,
        span,
        value_mode,
        classification,
        failure,
        occurrence,
    })
}

pub(in crate::compiler_frontend::ast::statements::value_production) fn build_value_if_expression(
    value_if: ValueIfBlock,
    span: Option<SourceSpan>,
    expression_builder: &mut AstExpressionBuilder,
) -> Result<OwnedExpr, ExpressionParseError> {
    let success = expression_builder.success_from_slots(value_if.result_type_ids.clone(), span)?;
    Ok(expression_builder.value_block(
        ValueBlock::If(value_if),
        span,
        success,
        ValueMode::ImmutableOwned,
    )?)
}

pub(in crate::compiler_frontend::ast::statements::value_production) fn build_value_match_expression(
    value_match: ValueMatchBlock,
    span: Option<SourceSpan>,
    expression_builder: &mut AstExpressionBuilder,
) -> Result<OwnedExpr, ExpressionParseError> {
    let success =
        expression_builder.success_from_slots(value_match.result_type_ids.clone(), span)?;
    Ok(expression_builder.value_block(
        ValueBlock::Match(value_match),
        span,
        success,
        ValueMode::ImmutableOwned,
    )?)
}
