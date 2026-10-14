//! Result shape, checked receiving and immutable handling invariants in the expression owner.

use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOp, ExpressionOperationView, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleExpressionHandling;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, ValueReceiverContext};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn slots_preserve_zero_one_and_multiple_shapes() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let value = values
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("constant content");
    for slots in [
        Vec::new(),
        vec![builtin_type_ids::BOOL],
        vec![
            builtin_type_ids::BOOL,
            builtin_type_ids::STRING,
            builtin_type_ids::BOOL,
        ],
    ] {
        let shape = expressions
            .success_from_slots(slots.clone(), None)
            .expect("checked result range");
        let root = expressions
            .append_with_success_shape(
                ExpressionOp::Constant(value),
                None,
                shape,
                ValueMode::ImmutableOwned,
            )
            .expect("shape row");
        let view = expressions.view_owned(root).expect("bound shape");
        assert_eq!(view.success_types(), slots);
        match slots.len() {
            0 => assert!(matches!(view.success_shape(), SuccessShape::Zero)),
            1 => assert!(matches!(view.success_shape(), SuccessShape::One(_))),
            _ => assert!(matches!(view.success_shape(), SuccessShape::Multiple(_))),
        }
    }
}

#[test]
fn multiple_payload_rejects_zero_and_one_slot_ranges() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let value = values
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("constant content");
    for slots in [Vec::new(), vec![builtin_type_ids::BOOL]] {
        let range = expressions
            .append_types(slots, None)
            .expect("ordinary ranges allow short lists");
        assert!(
            expressions
                .append_with_success_shape(
                    ExpressionOp::Constant(value),
                    None,
                    SuccessShape::Multiple(range),
                    ValueMode::ImmutableOwned
                )
                .is_err()
        );
    }
}

#[test]
fn single_value_boundary_reports_actual_success_arity() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let value = values
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("constant content");
    for slots in [
        Vec::new(),
        vec![builtin_type_ids::BOOL, builtin_type_ids::STRING],
    ] {
        let produced_count = slots.len();
        let shape = expressions.success_from_slots(slots, None).expect("shape");
        let root = expressions
            .append_with_success_shape(
                ExpressionOp::Constant(value),
                None,
                shape,
                ValueMode::ImmutableOwned,
            )
            .expect("shape row");
        let view = expressions.view_owned(root).expect("bound shape");
        let diagnostic = view
            .require_single(ValueReceiverContext::Declaration)
            .expect_err("scalar receiver rejects arity");
        assert!(
            matches!(diagnostic.payload, DiagnosticPayload::InvalidValueReceiver { receiver: ValueReceiverContext::Declaration, target_count: 1, produced_count: found } if found == produced_count)
        );
        assert!(
            view.value_type_id().is_err(),
            "internal single-value access also rejects arity"
        );
    }
    let root = expressions
        .bool(&mut values, true, None, ValueMode::ImmutableOwned)
        .expect("literal");
    assert_eq!(
        expressions
            .view_owned(root)
            .expect("literal view")
            .require_single(ValueReceiverContext::Condition)
            .expect("single slot"),
        builtin_type_ids::BOOL
    );
}

#[test]
fn handling_keeps_success_slots_and_failure_evidence_but_closes_own_error_channel() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let value = values
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("constant content");
    let shape = expressions
        .success_from_slots(vec![builtin_type_ids::BOOL, builtin_type_ids::STRING], None)
        .expect("shape");
    let producer = expressions
        .append_with_success_shape(
            ExpressionOp::Constant(value),
            None,
            shape,
            ValueMode::ImmutableOwned,
        )
        .expect("producer");
    let producer = expressions
        .with_typed_error_producer(producer, builtin_type_ids::STRING, None)
        .expect("typed channel");
    let handled = expressions
        .handled_result(producer, FallibleExpressionHandling::Propagate, None)
        .expect("handling");
    let view = expressions.view_owned(handled).expect("handled view");
    assert_eq!(view.success_shape(), shape);
    assert_eq!(view.error_type(), None);
    assert!(
        view.failure_facts()
            .is_none_or(|facts| facts.summary.first_typed.is_none())
    );
    let ExpressionOperationView::HandleFallible { value: child, .. } = view.operation() else {
        panic!("handling retains its producer")
    };
    assert_eq!(child.error_type(), Some(builtin_type_ids::STRING));
    assert!(
        child
            .failure_facts()
            .expect("producer facts")
            .typed_error
            .is_some()
    );
    let handled = expressions
        .with_typed_error_producer(handled, builtin_type_ids::STRING, None)
        .expect("authored evidence");
    let view = expressions.view_owned(handled).expect("handled view");
    assert_eq!(view.error_type(), None);
    assert!(
        view.failure_facts()
            .expect("authored facts")
            .typed_error
            .is_some()
    );
}

#[test]
fn occurrence_replacements_are_immutable_and_reuse_unchanged_roots() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let original = expressions
        .bool(&mut values, true, None, ValueMode::ImmutableOwned)
        .expect("literal");
    let row_count = expressions.len();
    let unchanged = expressions
        .with_regular_division_provenance(original, false)
        .expect("unchanged fact");
    assert_eq!(unchanged, original);
    assert_eq!(expressions.len(), row_count);

    let replacement = expressions
        .with_regular_division_provenance(original, true)
        .expect("scratch replacement");
    assert_ne!(replacement, original);
    assert!(
        !expressions
            .view_owned(original)
            .expect("original view")
            .classification()
            .contains_regular_division
    );
    assert!(
        expressions
            .view_owned(replacement)
            .expect("replacement view")
            .classification()
            .contains_regular_division
    );
    assert_eq!(
        expressions
            .view_owned(original)
            .expect("original view")
            .success_types(),
        expressions
            .view_owned(replacement)
            .expect("replacement view")
            .success_types()
    );
}

#[test]
fn present_empty_failure_records_survive_replacement() {
    use crate::compiler_frontend::ast::expressions::failure_facts::ExpressionFailureFacts;

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let original = expressions
        .bool(&mut values, false, None, ValueMode::ImmutableOwned)
        .expect("literal");
    assert!(
        expressions
            .view_owned(original)
            .expect("original view")
            .failure_facts()
            .is_none()
    );
    let replacement = expressions
        .with_failure_facts(original, ExpressionFailureFacts::default())
        .expect("present record");
    assert!(
        expressions
            .view_owned(replacement)
            .expect("replacement view")
            .failure_facts()
            .is_some()
    );
    let row_count = expressions.len();
    assert_eq!(
        expressions
            .with_failure_facts(replacement, ExpressionFailureFacts::default())
            .expect("same record"),
        replacement
    );
    assert_eq!(expressions.len(), row_count);
}
