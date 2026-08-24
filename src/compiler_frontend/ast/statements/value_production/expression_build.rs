//! AST node and expression construction for value-producing control flow.
//!
//! WHAT: builds `ThenValue` nodes, `ValueIfBlock`, `ValueMatchBlock`, and the
//! wrapping `ExpressionKind::ValueBlock` that HIR lowering consumes.
//! WHY: receiver and multi-bind parsers share one construction owner so they do
//! not build a temporary block and then overwrite its bodies.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ProducedValues, ValueBlock, ValueIfBlock, ValueMatchBlock,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

/// Builds a `ThenValue` AST node from produced branch expressions.
pub(in crate::compiler_frontend::ast::statements::value_production) fn then_value_node(
    expressions: Vec<Expression>,
    location: SourceLocation,
    scope: InternedPath,
) -> AstNode {
    AstNode {
        kind: NodeKind::ThenValue(ProducedValues {
            expressions,
            location: location.clone(),
        }),
        location,
        scope,
    }
}

/// Builds a `ValueBlock::If` expression from a completed `ValueIfBlock`.
///
/// `value_if.result_type_ids` must already be the final inferred or explicit
/// slot IDs. `result_type_id` is the expression type, including a tuple for
/// multi-slot receivers. Mixed multi-bind must not call this until slots are final.
pub(in crate::compiler_frontend::ast::statements::value_production) fn build_value_if_expression(
    value_if: ValueIfBlock,
    result_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Expression {
    let location = value_if.location.clone();

    Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::If(value_if)),
        },
        location,
        result_type_id,
        diagnostic_type_spelling(result_type_id, type_environment),
        ValueMode::ImmutableOwned,
    )
}

/// Builds a `ValueBlock::Match` expression from a completed `ValueMatchBlock`.
pub(in crate::compiler_frontend::ast::statements::value_production) fn build_value_match_expression(
    value_match: ValueMatchBlock,
    result_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Expression {
    let location = value_match.location.clone();

    Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::Match(value_match)),
        },
        location,
        result_type_id,
        diagnostic_type_spelling(result_type_id, type_environment),
        ValueMode::ImmutableOwned,
    )
}
