//! Deep Collection-chain fixture shared by finalisation traversal stack tests.
//!
//! The constant Boolean leaf and 4096 parent rows give each traversal the same depth on a
//! one MiB thread without tying the fixture to any traversal entry point.

use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::const_values::store::ConstValuePayload;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOp,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) fn collection_chain() -> (AstExpressionBuilder, ConstValueStore, ExprId) {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let leaf = values
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("leaf");
    let mut root = builder
        .append_expression(
            ExpressionOp::Constant(leaf),
            None,
            builtin_type_ids::BOOL,
            ValueMode::ImmutableOwned,
        )
        .expect("leaf expression")
        .id();
    for _ in 0..4096 {
        let children = builder
            .append_children(vec![builder.own(root)], None)
            .expect("children");
        root = builder
            .append_expression(
                ExpressionOp::Collection(children),
                None,
                builtin_type_ids::BOOL,
                ValueMode::ImmutableOwned,
            )
            .expect("collection")
            .id();
    }
    (builder, values, root)
}
