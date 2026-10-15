//! Foreign transient handles must fail before expression storage changes.

use super::scratch::tests::{constant, seed};
use super::*;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleExpressionHandling;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::symbol;

fn storage_state(builder: &AstExpressionBuilder) -> [usize; 8] {
    let statistics = builder.store().storage_statistics();
    [
        statistics.rows,
        statistics.row_capacity,
        statistics.row_bytes,
        statistics.row_capacity_bytes,
        statistics.side_entries,
        statistics.side_capacity,
        statistics.side_bytes,
        statistics.side_capacity_bytes,
    ]
}

#[test]
fn foreign_call_receiver_and_arguments_are_rejected_before_mutation() {
    let value = constant();
    let mut first = AstExpressionBuilder::new().unwrap();
    let mut second = AstExpressionBuilder::new().unwrap();
    let first_root = seed(&mut first, value);
    let second_root = seed(&mut second, value);
    assert_eq!(first_root.id().index(), second_root.id().index());
    let mut paths = PathInternerFork::empty();
    let mut strings = StringTable::new();
    let method = symbol("owner_boundary_method", &mut paths, &mut strings);
    let before = storage_state(&second);

    assert!(matches!(
        second.method_call_with_typed_arguments(
            first_root,
            method,
            vec![CallArgument::positional(
                second_root,
                CallAccessMode::Shared,
                None
            )],
            vec![builtin_type_ids::BOOL; 2],
            None,
        ),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);

    assert!(matches!(
        second.method_call_with_typed_arguments(
            second_root,
            method,
            vec![CallArgument::positional(
                first_root,
                CallAccessMode::Shared,
                None
            )],
            vec![builtin_type_ids::BOOL; 2],
            None,
        ),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);

    let raw_arguments = [CallArgument::positional(
        first_root.id(),
        CallAccessMode::Shared,
        None,
    )];
    assert!(matches!(
        second.method_call_with_typed_arguments(
            second_root,
            method,
            first.own_slice(&raw_arguments),
            vec![builtin_type_ids::BOOL; 2],
            None,
        ),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);
}

#[test]
fn foreign_handle_with_overlapping_index_is_rejected_before_mutation() {
    let value = constant();
    let mut first = AstExpressionBuilder::new().unwrap();
    let mut second = AstExpressionBuilder::new().unwrap();
    let first_root = seed(&mut first, value);
    let second_root = seed(&mut second, value);
    assert_eq!(first_root.id().index(), second_root.id().index());
    let before = storage_state(&second);

    assert!(matches!(
        second.append_children(vec![second_root, first_root], None),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);
    let raw_children = [first_root.id()];
    assert!(matches!(
        second.append_children(first.own_slice(&raw_children), None),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);
    let mut replacement = second
        .view_owned(second_root)
        .unwrap()
        .to_completed_input(&second)
        .unwrap();
    replacement.operation = ExpressionOp::LogicalNot {
        operand: first_root,
    };
    let serial_before = second.candidate_serial;
    assert!(matches!(
        second.commit_replacement(replacement),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(second.candidate_serial, serial_before);
    assert_eq!(storage_state(&second), before);
    assert!(second.view_owned(first_root).is_err());
    assert_eq!(
        second.view_owned(second_root).unwrap().id(),
        second_root.id()
    );
}

#[test]
fn foreign_handle_is_rejected_by_completed_operation_install() {
    let value = constant();
    let mut first = AstExpressionBuilder::new().unwrap();
    let mut second = AstExpressionBuilder::new().unwrap();
    let first_root = seed(&mut first, value);
    let second_root = seed(&mut second, value);
    assert_eq!(first_root.id().index(), second_root.id().index());
    let before = storage_state(&second);
    let operation = CompletedOperation::HandleFallible {
        value: first_root,
        handling: HandlingRecord {
            handling: FallibleExpressionHandling::Propagate,
            propagation_span: None,
            error_type: Some(builtin_type_ids::STRING),
        },
    };

    // Installing this operation would append a handling side entry without the owner preflight.
    assert!(matches!(
        operation.install(&mut second, None),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);
    assert!(second.storage.handling.is_empty());
    assert_eq!(
        second.view_owned(second_root).unwrap().id(),
        second_root.id()
    );
}

#[test]
fn pre_freeze_handle_is_rejected_by_frozen_store_and_fresh_builder() {
    struct Root(ExprId);
    impl ExpressionFreezeRoots for Root {
        fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId)) {
            visitor(self.0);
        }
        fn visit_expression_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut ExprId)) {
            visitor(&mut self.0);
        }
        fn visit_place_ids(&self, _: &mut dyn FnMut(PlaceId)) {}
        fn visit_place_ids_mut(&mut self, _: &mut dyn FnMut(&mut PlaceId)) {}
    }

    let value = constant();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let before_freeze = seed(&mut builder, value);
    let mut root = Root(before_freeze.id());
    let frozen = builder.freeze(&mut root).unwrap();
    let mut fresh = AstExpressionBuilder::new().unwrap();
    let fresh_root = seed(&mut fresh, value);
    assert_eq!(before_freeze.id().index(), root.0.index());
    assert_eq!(before_freeze.id().index(), fresh_root.id().index());
    assert_eq!(frozen.view(root.0).unwrap().id(), root.0);
    assert_eq!(fresh.view_owned(fresh_root).unwrap().id(), fresh_root.id());

    // Both stores contain index zero so an index-only check would accept the stale handle.
    assert!(frozen.view_owned(before_freeze).is_err());
    assert!(fresh.view_owned(before_freeze).is_err());
}

#[test]
fn completed_input_rechecks_owners_before_any_edge_access() {
    let value = constant();
    let mut first = AstExpressionBuilder::new().unwrap();
    let mut second = AstExpressionBuilder::new().unwrap();
    let foreign = seed(&mut first, value);
    let local = seed(&mut second, value);
    let invalid_local = second.own(ExprId::at(second.len()));
    let before = storage_state(&second);

    let mut input = second
        .completed_input(
            ExpressionOp::LogicalNot { operand: local },
            SuccessShape::One(builtin_type_ids::BOOL),
            None,
            ValueMode::ImmutableOwned,
        )
        .unwrap();
    input.operation = ExpressionOp::LogicalBinary {
        left: invalid_local,
        right: foreign,
        operation: LogicalOperator::And,
    };
    let Err(ExpressionConstructionFailure::Invariant(error)) = second.append_completed(input)
    else {
        panic!("a mutated completed input must reject its foreign edge");
    };
    // A local out-of-range first edge must not conceal the foreign second edge.
    assert_eq!(error.primary.msg, "expression handle from another owner");
    assert_eq!(storage_state(&second), before);

    let mut input = second
        .view_owned(local)
        .unwrap()
        .to_completed_input(&second)
        .unwrap();
    input.operation = ExpressionOp::LogicalNot {
        operand: invalid_local,
    };
    assert!(matches!(
        second.append_completed(input),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&second), before);
}
