//! Completed value-control-flow construction. Branch leaves keep durable IDs; results retain ownership.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ProducedValues, ValueBlock, ValueIfBlock, ValueMatchBlock,
};
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
