//! Evaluator escape guard: pending expression syntax that reaches resolved-operation construction
//! is a compiler invariant failure, never a source diagnostic.

use super::ExpressionTypingError;
use super::evaluator::expression_from_resolved_items;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn group_marker_reaching_construction_is_an_infrastructure_error() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();

    let result = expression_from_resolved_items(
        vec![ExpressionRpnItem::GroupOpen { anchor_span: None }],
        &mut expressions,
        &mut const_values,
        &mut string_table,
        None,
        ValueMode::ImmutableOwned,
    );

    assert!(
        matches!(result, Err(ExpressionTypingError::Infrastructure(_))),
        "a group marker must reach the escape guard as a compiler error"
    );
}
