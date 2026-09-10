//! Finalization regression tests for HIR-boundary type ownership.
//!
//! WHAT: documents that typed constructors now own canonical call `TypeId` data before
//! finalization.
//! WHY: finalization validates expression `TypeId`s against the canonical environment;
//! build expressions against the canonical AST type environment.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::interned_path::InternedPath;

#[test]
fn typed_call_constructor_sets_expression_and_result_type_ids() {
    let mut type_environment = TypeEnvironment::new();
    let int_type_id = type_environment.builtins().int;

    let expression = Expression::function_call_with_typed_arguments(
        InternedPath::new(),
        vec![],
        vec![int_type_id],
        &mut type_environment,
        None,
    );

    assert_eq!(expression.type_id, int_type_id);
    assert!(
        matches!(
            &expression.kind,
            ExpressionKind::FunctionCall { result_type_ids, .. }
                if result_type_ids.as_slice() == [int_type_id]
        ),
        "call constructors must carry canonical result TypeIds before finalization"
    );
}
