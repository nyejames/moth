//! Typed expression store invariants.
//!
//! WHAT: checks the one-based `ExprId` layout and handle sizes. Identity and success slots must
//! survive growth, and side appends are atomic under capacity limits. Multi-valued and range
//! edges are validated before a row is appended, and malformed child edges are rejected before
//! any cold fact changes.
//! WHY: the store is the only owner of expression rows and side payloads, so its append-time
//! checks must hold without going through full source parsing.

use super::*;
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;

#[test]
fn compact_choice_and_parameter_slot_guards_reject_before_storage_changes() {
    use crate::compiler_frontend::ast::expressions::call_argument::{
        CallAccessMode, CallArgument, ParameterSlot,
    };
    use crate::compiler_frontend::ast::expressions::expression::ChoiceConstructInput;
    use crate::compiler_frontend::compiler_messages::{
        AstCapacityResource, CapacityResource, DiagnosticPayload,
    };
    use crate::compiler_frontend::source::{LocalSpan, SourceId};
    use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_frontend::tests::ast_fixture_support::symbol;

    let span = Some(SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::source_start(),
    ));
    let mut builder = AstExpressionBuilder::new().unwrap();
    let root = builder.append_completed(input(constant())).unwrap();
    let before = builder.store().storage_statistics();
    let too_large = u32::MAX as usize + 1;
    let choice = builder.choice_construct(ChoiceConstructInput {
        tag: too_large,
        fields: Vec::<ConstructorField<OwnedExpr>>::new(),
        type_id: builtin_type_ids::BOOL,
        span,
        value_mode: ValueMode::ImmutableOwned,
    });
    let mut argument = CallArgument::positional(root, CallAccessMode::Shared, span);
    argument.parameter_slot = Some(ParameterSlot::new(too_large));
    let name = symbol(
        "compact_slot",
        &mut PathInternerFork::empty(),
        &mut StringTable::new(),
    );
    let call = builder.function_call_with_typed_arguments(
        name,
        vec![argument],
        vec![builtin_type_ids::BOOL],
        span,
    );
    for (result, resource) in [
        (choice, AstCapacityResource::ChoiceTags),
        (call, AstCapacityResource::ExpressionSideEntries),
    ] {
        let Err(ExpressionConstructionFailure::Diagnostic(diagnostic)) = result else {
            panic!("expected compact-index capacity rejection");
        };
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::CompilerCapacityExceeded {
                resource: CapacityResource::Ast(actual),
            } if actual == resource
        ));
        assert_eq!(diagnostic.primary_span, span);
    }
    let after = builder.store().storage_statistics();
    assert_eq!(after.rows, before.rows);
    assert_eq!(after.side_entries, before.side_entries);
    assert_eq!(
        builder.view_owned(root).unwrap().success_types(),
        &[builtin_type_ids::BOOL]
    );
}
fn input(value: ConstValueId) -> CompletedExpressionInput {
    CompletedExpressionInput {
        operation: ExpressionOp::Constant(value),
        success: SuccessShape::One(builtin_type_ids::BOOL),
        error_type: None,
        span: None,
        value_mode: ValueMode::ImmutableOwned,
        classification: ExpressionClassification {
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
        },
        failure: None,
        occurrence: None,
    }
}
fn constant() -> ConstValueId {
    ConstValueStore::default()
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .unwrap()
}
#[test]
fn one_based_identity_and_option_use_four_bytes() {
    assert_eq!(std::mem::size_of::<ExprId>(), 4);
    assert_eq!(std::mem::size_of::<Option<ExprId>>(), 4);
    assert_eq!(ExprId::at(0).0.get(), 1);
    assert_eq!(ExprId::at(u32::MAX as usize - 1).0.get(), u32::MAX);
}
#[test]
fn view_and_store_ref_are_pointer_sized_handles() {
    assert!(std::mem::size_of::<ExpressionView>() <= 16);
    assert!(std::mem::size_of::<&ExpressionStore>() <= 8);
}
#[test]
fn growth_keeps_bound_identity_and_success_slots() {
    let value = constant();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let first = builder.append_completed(input(value)).unwrap();
    for _ in 0..128 {
        builder.append_completed(input(value)).unwrap();
    }
    assert_eq!(
        builder.view_owned(first).unwrap().success_types(),
        &[builtin_type_ids::BOOL]
    );
    assert!(builder.view(ExprId::at(builder.len())).is_err());
    let frozen = builder.storage.freeze().unwrap();
    assert_eq!(frozen.view(first.id()).unwrap().id(), first.id());
    let stats = frozen.storage_statistics();
    assert_eq!(stats.rows, stats.row_capacity);
    assert_eq!(stats.side_entries, stats.side_capacity);
}
#[test]
fn multiple_and_ranges_validate_before_row_append() {
    let value = constant();
    let mut builder = AstExpressionBuilder::new().unwrap();
    for types in [vec![], vec![builtin_type_ids::BOOL]] {
        let range = builder.append_types(types, None).unwrap();
        let mut row = input(value);
        row.success = SuccessShape::Multiple(range);
        assert!(matches!(
            builder.append_completed(row),
            Err(ExpressionConstructionFailure::Invariant(_))
        ));
        assert_eq!(builder.len(), 0);
    }
    let range = builder
        .append_types(vec![builtin_type_ids::BOOL, builtin_type_ids::BOOL], None)
        .unwrap();
    let mut row = input(value);
    row.success = SuccessShape::Multiple(range);
    let root = builder.append_completed(row).unwrap();
    assert_eq!(
        builder.view_owned(root).unwrap().success_types(),
        &[builtin_type_ids::BOOL, builtin_type_ids::BOOL]
    );
    assert!(
        TypeRange {
            start: u32::MAX,
            len: 1
        }
        .bounds(usize::MAX)
        .is_err()
    );
    assert!(TypeRange { start: 0, len: 2 }.bounds(1).is_err());
    assert!(
        ExpressionRange { start: 0, len: 0 }
            .bounds(0)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn source_capacity_is_diagnostic_and_side_append_is_atomic() {
    let mut builder = AstExpressionBuilder {
        limit: 1,
        ..AstExpressionBuilder::new().unwrap()
    };
    let before = builder.storage.result_types.len();
    assert!(matches!(
        builder.append_types(vec![builtin_type_ids::BOOL; 2], None),
        Err(ExpressionConstructionFailure::Diagnostic(_))
    ));
    assert_eq!(builder.storage.result_types.len(), before);
    let value = constant();
    builder.append_completed(input(value)).unwrap();
    assert!(matches!(
        builder.append_completed(input(value)),
        Err(ExpressionConstructionFailure::Diagnostic(_))
    ));
    assert_eq!(builder.len(), 1);
    assert!(matches!(
        checked_extent(u32::MAX as usize, 1, u32::MAX, None, false),
        Err(ExpressionConstructionFailure::Diagnostic(_))
    ));
}
#[test]
fn capacity_failure_is_classified_for_speculative_probes() {
    let mut builder = AstExpressionBuilder {
        limit: 1,
        ..AstExpressionBuilder::new().unwrap()
    };
    let Err(ExpressionConstructionFailure::Diagnostic(capacity)) =
        builder.append_types(vec![builtin_type_ids::BOOL; 2], None)
    else {
        panic!("expected a capacity diagnostic from the side append");
    };
    assert!(capacity.is_compiler_capacity_exceeded());
    assert!(builder.storage.result_types.is_empty());
}
#[test]
fn row_capacity_failure_after_side_append_is_not_rolled_back() {
    let mut builder = AstExpressionBuilder {
        limit: 1,
        ..AstExpressionBuilder::new().unwrap()
    };
    builder.append_completed(input(constant())).unwrap();
    let side_before = builder.storage.result_types.len();
    builder
        .append_types(vec![builtin_type_ids::BOOL], None)
        .unwrap();
    let Err(ExpressionConstructionFailure::Diagnostic(capacity)) =
        builder.append_completed(input(constant()))
    else {
        panic!("expected the row append to report capacity");
    };
    assert!(capacity.is_compiler_capacity_exceeded());
    assert_eq!(builder.len(), 1);
    assert_eq!(builder.storage.result_types.len(), side_before + 1);
}
#[test]
fn malformed_child_rejects_before_cold_fact_mutation() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut row = input(constant());
    row.operation = ExpressionOp::LogicalNot {
        operand: builder.own(ExprId::at(0)),
    };
    row.failure = Some(ExpressionFailureFacts::default());
    assert!(matches!(
        builder.append_completed(row),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert!(builder.storage.rows.is_empty());
    assert!(builder.storage.failures.is_empty());
}
