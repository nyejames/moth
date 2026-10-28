//! Finalization regression tests for HIR-boundary type ownership.
//!
//! WHAT: documents that typed constructors own canonical call `TypeId` data before
//! finalization.
//! WHY: finalization validates expression `TypeId`s against the canonical environment;
//! build expressions against the canonical AST type environment.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathId;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn typed_call_constructor_sets_authoritative_success_shape() {
    let int_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut _constants = ConstValueStore::default();

    let expression = expressions
        .function_call_with_typed_arguments(PathId::ROOT, vec![], vec![int_type_id], None)
        .expect("fixture call fits store");

    assert_eq!(
        expressions
            .view_owned(expression)
            .expect("fixture expression belongs to builder")
            .success_types(),
        [int_type_id]
    );
    assert!(matches!(
        expressions
            .view_owned(expression)
            .expect("fixture expression belongs to builder")
            .operation(),
        ExpressionOperationView::Call(_)
    ));
}
