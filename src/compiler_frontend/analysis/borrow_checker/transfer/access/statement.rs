//! Statement-level borrow transfer rules.
//!
//! WHAT: applies borrow effects for HIR statements and records statement/value facts.
//! WHY: keeping statement transfer separate from traversal helpers makes the block transfer
//! entrypoint easier to inspect without changing the borrow-analysis model.

#[path = "call_results.rs"]
pub(in crate::compiler_frontend::analysis::borrow_checker) mod call_results;

use super::super::call_semantics::{TypedContinuation, classify_typed_continuation};
use super::conflicts::{
    OperandAccessIndex, Overlap, check_call_access, overlap, record_call_observations,
};
use super::layer::needs_layer_store_capture;
use super::*;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, AllocationRetirement, CapturedValue, Coverage, EntryFamilies, FieldKey,
    ReceiverDependency, ResolvedPlace, RootSet, SmallSet, TypedStore, ValueAlt,
    ValueKind as PayloadKind, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::CallReturns;
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{
    Demand, selected_members,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    CapturedCallBatch, CapturedDestination, CapturedDetachment, InlineVec, InvokeLane,
    InvokeLayerOp,
};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirMapOp;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::hir::terminators::HirFallibleOperation;
use crate::compiler_frontend::public_call_summary::typed::{SupportId, TypedCallAllocationFacts};

#[cfg(test)]
mod typed_argument_tests {
    use super::*;
    use crate::compiler_frontend::public_call_summary::typed::{
        Component, Coverage as TypedCoverage, Footprint, Lane, OutputPort, Post, Product, SupportId,
    };
    use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
    use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;

    #[test]
    fn pending_arguments_execute_without_settling_optional_transfer_or_a_lane() {
        let (ast, mut paths, mut strings) = parse_single_file_ast("x = 1");
        let packages = default_external_package_registry(&mut strings);
        let (module, types) = lower_ast(ast, &mut strings, &mut paths, &packages).unwrap();
        let context = BorrowTransferContext {
            external_package_registry: &packages,
            public_call_summaries: &Default::default(),
            imported_call_summaries: &module.imported_call_summaries,
            module_private_call_summaries: &module.module_private_call_summaries,
            generated_call_summaries: &module.generated_call_summaries,
            local_types: &Default::default(),
            structs: &module.structs,
            choices: &module.choices,
            expressions: &module.expressions,
            type_environment: &types,
            typed_call_cache: Default::default(),
            type_projection:
                crate::compiler_frontend::ast::generic_functions::registered_type_projection(
                    &types, &packages,
                ),
            diagnostics: BorrowDiagnostics::new(&module, &paths, &strings),
        };
        let view = OperationView {
            local_ids: &[],
            local_indices: &Default::default(),
            local_mutable: &[],
            local_has_in_place_mutation_surface: &[],
            block_successors: &Default::default(),
            local_first_write_order: &[],
            acyclic_entry_block: None,
            entry_roots: None,
        };
        let args = [CallArgumentTransfer {
            argument: HirValueId(0),
            effect: ArgEffect::MayConsumeShared,
        }];
        let mut state = BorrowState::new_uninitialized(0);
        let mut stats = BlockTransferStats::default();
        let pending = None;
        let bottom = TypedCallAllocationFacts {
            may_retain: vec![0],
            ..Default::default()
        };
        let unresolved = TypedCallAllocationFacts {
            success: Lane::Returning(
                Product::new(vec![Component {
                    support: SupportId(0),
                    alternatives: vec![Post {
                        coverage: TypedCoverage::Unresolved(Footprint::Locations(vec![
                            OutputPort::Result(0),
                        ])),
                        ..Default::default()
                    }],
                }])
                .unwrap(),
            ),
            ..Default::default()
        };
        let mut executed = Vec::new();
        {
            let mut input = CallTransferContext::new(
                &context,
                view,
                &mut state,
                BlockId(0),
                0,
                None,
                &mut stats,
                None,
            );
            for facts in [pending, Some(&bottom), Some(&unresolved)] {
                account_typed_call_arguments(
                    &mut input,
                    &mut CallActuals::new(&args),
                    facts,
                    CallStorage::User,
                    true,
                    |_| false,
                    |argument, effect, retains| executed.push((argument, effect, retains)),
                )
                .unwrap();
            }
        }
        assert_eq!(
            executed,
            [
                (HirValueId(0), None, false),
                (HirValueId(0), Some(ArgEffect::SharedBorrow), true),
                (HirValueId(0), Some(ArgEffect::SharedBorrow), false),
            ]
        );
        assert!(stats.invoke.is_empty() && stats.observations.is_empty() && !stats.diverges);
        for terminal in [false, true] {
            for error in [false, true] {
                let classify = |facts, mandatory| {
                    classify_typed_continuation(
                        facts,
                        terminal,
                        error,
                        mandatory,
                        &context.diagnostics,
                    )
                };
                assert!(matches!(
                    classify(pending, false),
                    Ok(TypedContinuation::Deferred)
                ));
                assert!(classify(pending, true).is_err());
                assert!(matches!(
                    classify(Some(&bottom), false),
                    Ok(TypedContinuation::NoContinuation)
                ));
                let selected = classify(Some(&unresolved), true).unwrap();
                if terminal || error {
                    assert!(matches!(selected, TypedContinuation::NoContinuation));
                } else {
                    assert!(
                        matches!(selected, TypedContinuation::Match(lane) if std::ptr::eq(lane, &unresolved.success))
                    );
                }
            }
        }
    }

    #[test]
    fn typed_lookup_captures_a_real_receiver_before_receiving() {
        check_typed_lookup(false);
        check_typed_lookup(true);
    }

    fn check_typed_lookup(copied: bool) {
        use crate::compiler_frontend::analysis::borrow_checker::{
            engine::apply_captured_call_batch, state::StoredValue, transfer::EdgeTransferView,
        };
        let source = if copied {
            "items = {1}\nobserved = (copy items).get(0) catch then 0\n"
        } else {
            "items = {1}\nobserved = items.get(0) catch then 0\n"
        };
        let (ast, mut paths, mut strings) = parse_single_file_ast(source);
        let packages = default_external_package_registry(&mut strings);
        let (module, types) = lower_ast(ast, &mut strings, &mut paths, &packages).unwrap();
        let (argument, receiver) = (0..module.expressions.expression_count())
            .find_map(|index| {
                let id = HirValueId(index as u32);
                match &module.expressions.expression(id).kind {
                    HirExpressionKind::Load(place) if !copied => Some((id, place.root)),
                    HirExpressionKind::Copy(place) if copied => Some((id, place.root)),
                    _ => None,
                }
            })
            .expect("lookup has a receiver load");
        let receiving = LocalId(u32::MAX);
        let locals = [receiver, receiving];
        let indices = [(receiver, 0), (receiving, 1)].into_iter().collect();
        let view = OperationView {
            local_ids: &locals,
            local_indices: &indices,
            local_mutable: &[true; 2],
            local_has_in_place_mutation_surface: &[true, false],
            block_successors: &Default::default(),
            local_first_write_order: &[0, 3],
            acyclic_entry_block: None,
            entry_roots: None,
        };
        let context = BorrowTransferContext {
            external_package_registry: &packages,
            public_call_summaries: &Default::default(),
            imported_call_summaries: &module.imported_call_summaries,
            module_private_call_summaries: &module.module_private_call_summaries,
            generated_call_summaries: &module.generated_call_summaries,
            local_types: &Default::default(),
            structs: &module.structs,
            choices: &module.choices,
            expressions: &module.expressions,
            type_environment: &types,
            typed_call_cache: Default::default(),
            type_projection:
                crate::compiler_frontend::ast::generic_functions::registered_type_projection(
                    &types, &packages,
                ),
            diagnostics: BorrowDiagnostics::new(&module, &paths, &strings),
        };
        let mut state = BorrowState::new_uninitialized(2);
        state.initialize_parameter(0);
        let mut derived = RootSet::empty(2);
        derived.insert(0);
        state.mark_slot_derivation(0, derived);
        let holder = state.local_alloc_ids(0).0[0];
        state.publish_store(
            &TypedStore::write_projected(
                [holder],
                FieldKey::Elem,
                Coverage::Partial,
                CapturedValue::new(
                    vec![StoredValue {
                        id: holder,
                        writable: true,
                    }],
                    Vec::new(),
                ),
                false,
            ),
            0,
            None,
        );
        let mut stats = BlockTransferStats::default();
        let args = [CallArgumentTransfer {
            argument,
            effect: ArgEffect::SharedBorrow,
        }];
        let batch = capture_typed_builtin_element_batch(
            &mut CallTransferContext::new(
                &context,
                view,
                &mut state,
                BlockId(0),
                3,
                None,
                &mut stats,
                None,
            ),
            &mut CallActuals::new(&args),
            Some(HirLocalDestination::Define(receiving)),
            CallStorage::Get,
        )
        .unwrap()
        .expect("populated lookup can return");
        apply_captured_call_batch(
            EdgeTransferView { operations: view },
            batch,
            [],
            3,
            &mut state,
        )
        .unwrap();
        assert_eq!(state.effective_roots(1).contains(0), !copied);
        assert_eq!(state.derived_roots_from_source(1).contains(0), !copied);
        let result = state.resolve_captured_allocs(&state.resolve_cells(1), None);
        let mut dependencies = result.alternatives.iter().flat_map(|alt| &alt.deps);
        assert!(
            dependencies
                .clone()
                .any(|dep| !dep.entries.as_slice().is_empty())
        );
        assert_eq!(
            dependencies.any(|dep| dep.entries.as_slice().contains(&holder)),
            !copied,
        );
    }
}
use crate::compiler_frontend::source::SourceSpan;
use call_results::{
    CallActuals, CallLane, MatchedLane, call_result_ops, capture_graph_values,
    capture_typed_actuals, match_call_actuals,
};

pub(crate) fn transfer_statement(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &mut BorrowState,
    block_id: BlockId,
    statement: &HirStatement,
    stats: &mut BlockTransferStats,
    value_fact_buffer: &mut ValueFactBuffer,
) -> Result<(), BorrowCheckError> {
    let mut tracker = StatementAccessTracker::new(layout.local_count());
    let conflicts_before = stats.conflicts_checked;
    let order = layout.statement_order_or_unknown(statement.id);
    let location = context.diagnostics.statement_error_span(statement);
    transfer_statement_operation(
        &mut CallTransferContext::new(
            context,
            OperationView::from_layout(layout),
            state,
            block_id,
            order,
            location,
            stats,
            Some(ValidationSink {
                layout,
                tracker: &mut tracker,
                value_facts: value_fact_buffer,
            }),
        ),
        &statement.kind,
    )?;

    check_aggregate_transfers(
        context,
        layout,
        state,
        block_id,
        statement,
        order,
        value_fact_buffer,
    )?;
    let statement_fact = StatementBorrowFact {
        shared_roots: roots_to_local_ids(layout, &tracker.shared_roots),
        mutable_roots: roots_to_local_ids(layout, &tracker.mutable_roots),
        written_roots: roots_to_local_ids(layout, &tracker.written_roots),
        conflicts_checked: stats.conflicts_checked - conflicts_before,
    };
    stats.statement_facts.push((statement.id, statement_fact));
    Ok(())
}

pub(in super::super) fn transfer_statement_operation(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    kind: &HirStatementKind,
) -> Result<(), BorrowCheckError> {
    let expressions = input.context.expressions;
    match kind {
        HirStatementKind::Write { target, value } => {
            transfer_statement_write(input, target, *value)
        }
        HirStatementKind::Call {
            target,
            args,
            results,
        } => transfer_immediate_call(input, target, expressions.values(*args), results),
        HirStatementKind::MapOp {
            op,
            receiver,
            args,
            result,
        } => transfer_call_arguments_and_result(
            input,
            &map_call_arguments(*op, *receiver, expressions.values(*args)),
            result.iter().copied(),
            CallReturns::Fresh,
            CallResultTransfer::Immediate,
            CallStorage::for_map_op(*op),
        ),
        HirStatementKind::FormatFloat { source, result, .. }
        | HirStatementKind::ValidateFloat { source, result, .. }
        | HirStatementKind::FastStoreConversion { source, result, .. } => transfer_plain_operation(
            input,
            [*source],
            std::iter::once(*result),
            CallResultTransfer::Immediate,
        ),
        HirStatementKind::NumericOp {
            operands, result, ..
        } => {
            let results = std::iter::once(*result);
            let transfer = CallResultTransfer::Immediate;
            match operands {
                HirNumericOperands::Unary { operand } => {
                    transfer_plain_operation(input, [*operand], results, transfer)
                }
                HirNumericOperands::Binary { left, right } => {
                    transfer_plain_operation(input, [*left, *right], results, transfer)
                }
            }
        }
        HirStatementKind::LoopStepFailure { result, .. } => transfer_plain_operation(
            input,
            [],
            std::iter::once(*result),
            CallResultTransfer::Immediate,
        ),
        HirStatementKind::RangeCandidate {
            current,
            step,
            end,
            candidate_result,
            in_range_result,
            ..
        } => {
            read_statement_operands(input, [*current, *step, *end])?;
            transfer_plain_operation(
                input,
                [],
                [*candidate_result, *in_range_result].into_iter(),
                CallResultTransfer::Immediate,
            )
        }
        HirStatementKind::Expr(value) | HirStatementKind::PushRuntimeFragment { value, .. } => {
            read_statement_operands(input, [*value])
        }
    }
}

fn transfer_statement_write(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    target: &HirWriteTarget,
    value: HirValueId,
) -> Result<(), BorrowCheckError> {
    let (place, is_definition) = match target {
        HirWriteTarget::DefineLocal(local) => (HirPlace::local(*local), true),
        HirWriteTarget::AssignPlace(place) => (*place, false),
    };
    let local = input.view.index_of(place.root);
    let pristine_definition = if is_definition {
        first_definition_index(
            input.view,
            input.state,
            input.block_id,
            input.current_order,
            || local,
        )
    } else {
        None
    };
    let location = input.location;
    let local_count = input.view.local_count();
    if input.validation.is_some() {
        let mut reads = input.shared_read_env(location);
        record_shared_reads_in_place_indices(
            &mut reads,
            &place,
            location,
            &mut RootSet::empty(local_count),
        )?;
        record_assignment_value_reads(&mut reads, &place, is_definition, value, location)?;
    }

    if input.validation.is_none() {
        let Some(local) = local else {
            return Err(input.context.diagnostics.internal_error(format!(
                "Assignment target local '{}' is not in the active function layout",
                input.context.diagnostics.local_name(place.root)
            )));
        };
        let values = LayerValueContext {
            context: input.context,
            view: input.view,
            block_id: input.block_id,
            materialise_contents: input.stats.sparse,
        };
        let plan = plan_assignment(&values, input.state, &place, value, is_definition, local)?;
        if !is_definition
            && plan.destination.as_ref().is_some_and(|place| {
                writes_entry_storage(input.view, input.state, plan.local, &place.address)
            })
            && !matches!(&input.context.expressions.expression(value).kind,
                HirExpressionKind::Load(source) if *source == place)
        {
            record_executed_retention(input, value);
        }
        return publish_assignment(
            &values,
            input.state,
            &mut input.stats.observations,
            input.current_order,
            value,
            plan,
            pristine_definition,
        );
    }

    let validation = input
        .validation
        .as_mut()
        .expect("statement writes always validate");
    transfer_assign_target(
        &mut AssignTransferContext {
            context: input.context,
            layout: validation.layout,
            state: input.state,
            block_id: input.block_id,
            current_order: input.current_order,
            tracker: validation.tracker,
            value_fact_buffer: validation.value_facts,
            location,
            span: location,
            stats: input.stats,
        },
        &place,
        value,
        is_definition,
        local,
        pristine_definition,
    )
}

pub(super) fn read_statement_operands(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    operands: impl IntoIterator<Item = HirValueId>,
) -> Result<(), BorrowCheckError> {
    if input.validation.is_none() {
        return Ok(());
    }
    let location = input.location;
    let local_count = input.view.local_count();
    let mut reads = input.shared_read_env(location);
    for operand in operands {
        record_shared_reads_in_expression(
            &mut reads,
            operand,
            location,
            &mut RootSet::empty(local_count),
        )?;
    }
    Ok(())
}

fn check_aggregate_transfers(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &BorrowState,
    block_id: BlockId,
    statement: &HirStatement,
    statement_order: i32,
    value_fact_buffer: &mut ValueFactBuffer,
) -> Result<(), BorrowCheckError> {
    let location = || context.diagnostics.statement_error_span(statement);
    let (value, span) = match &statement.kind {
        HirStatementKind::Write { value, .. } => (*value, location()),
        HirStatementKind::Expr(value)
        | HirStatementKind::PushRuntimeFragment { value, .. }
        | HirStatementKind::FormatFloat { source: value, .. }
        | HirStatementKind::ValidateFloat { source: value, .. }
        | HirStatementKind::FastStoreConversion { source: value, .. } => (
            *value,
            context.diagnostics.value_error_span(*value, location()),
        ),
        _ => return Ok(()),
    };
    transfer_aggregate_expression_ownership(
        layout,
        state,
        value,
        block_id,
        statement_order,
        span,
        &mut AggregateTransferContext {
            diagnostics: &context.diagnostics,
            expressions: context.expressions,
            value_fact_buffer,
        },
    )
}

/// Evaluate operation inputs before the outgoing edge retires the previous destinations.
pub(super) fn transfer_invoke(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    terminator: &HirTerminator,
) -> Result<(), BorrowCheckError> {
    let HirTerminator::Invoke {
        operation,
        success_results,
        error_result,
        ..
    } = terminator
    else {
        return Err(input
            .context
            .diagnostics
            .internal_error("Non-Invoke terminator reached operation transfer"));
    };
    let context = input.context;
    let results = success_results
        .iter()
        .copied()
        .map(HirLocalDestination::Define);
    match operation {
        HirFallibleOperation::Call { target, args } => transfer_target_call(
            input,
            target,
            context.expressions.values(*args),
            results,
            CallResultTransfer::Edge(*error_result),
        ),
        HirFallibleOperation::MapOp { op, receiver, args } => {
            let arguments = map_call_arguments(*op, *receiver, context.expressions.values(*args));
            transfer_call_arguments_and_result(
                input,
                &arguments,
                results,
                CallReturns::Fresh,
                CallResultTransfer::Edge(*error_result),
                CallStorage::for_map_op(*op),
            )
        }
        _ => {
            let transfer = CallResultTransfer::Edge(*error_result);
            let mut operands = operation.operands(context.expressions);
            // Non-call operations have at most two operands and need no argument buffer.
            let Some(first) = operands.next() else {
                return transfer_plain_operation(input, [], results, transfer);
            };
            match operands.next() {
                Some(second) => transfer_plain_operation(input, [first, second], results, transfer),
                None => transfer_plain_operation(input, [first], results, transfer),
            }
        }
    }
}

/// Apply one terminating call's argument effects before the block diverges.
///
/// WHAT: resolves the ordinary call-target contract and transfers every evaluated argument
///       with no result destinations.
/// WHY: a declared Never call has ordinary argument access and optional final-use effects but
///      produces no value, so no result local, return alias or post-call state may be created.
pub(super) fn transfer_never_call(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    target: &CallTarget,
    args: &[HirValueId],
) -> Result<(), BorrowCheckError> {
    transfer_target_call(
        input,
        target,
        args,
        std::iter::empty(),
        CallResultTransfer::Diverges,
    )
}

pub(super) fn transfer_immediate_call(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    target: &CallTarget,
    args: &[HirValueId],
    results: &[HirLocalDestination],
) -> Result<(), BorrowCheckError> {
    let result_transfer = CallResultTransfer::for_call_definition(
        input.view,
        input.state,
        input.block_id,
        input.current_order,
        results,
    );
    transfer_target_call(
        input,
        target,
        args,
        results.iter().copied(),
        result_transfer,
    )
}

fn transfer_target_call(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    target: &CallTarget,
    args: &[HirValueId],
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    result_transfer: CallResultTransfer,
) -> Result<(), BorrowCheckError> {
    let semantics = resolve_call_semantics(input.context, target, args.len())?;
    let arguments: InlineVec<_> = args
        .iter()
        .copied()
        .zip(semantics.arg_effects())
        .map(|(argument, effect)| CallArgumentTransfer { argument, effect })
        .collect();
    let returns = semantics.returns();
    if let CallReturns::Summary(summary) = returns {
        return transfer_typed_user_call(
            input,
            &arguments,
            results,
            summary.facts(),
            result_transfer,
        );
    }
    transfer_call_arguments_and_result(
        input,
        &arguments,
        results,
        returns,
        result_transfer,
        CallStorage::for_target(target),
    )
}

fn transfer_plain_operation<const N: usize>(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    operands: [HirValueId; N],
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    transfer: CallResultTransfer,
) -> Result<(), BorrowCheckError> {
    transfer_call_arguments_and_result(
        input,
        &operands.map(|argument| CallArgumentTransfer {
            argument,
            effect: ArgEffect::SharedBorrow,
        }),
        results,
        CallReturns::Fresh,
        transfer,
        CallStorage::Plain,
    )
}

fn map_call_arguments(
    op: HirMapOp,
    receiver: HirValueId,
    args: &[HirValueId],
) -> Vec<CallArgumentTransfer> {
    let mut arguments = Vec::with_capacity(args.len() + 1);
    arguments.push(CallArgumentTransfer {
        argument: receiver,
        effect: if op.requires_mutable_receiver() {
            ArgEffect::MutableBorrow
        } else {
            ArgEffect::SharedBorrow
        },
    });
    arguments.extend(args.iter().copied().enumerate().map(|(index, argument)| {
        CallArgumentTransfer {
            argument,
            effect: map_argument_effect(op, index),
        }
    }));
    arguments
}

pub(super) struct CallArgumentTransfer {
    pub(super) argument: HirValueId,
    effect: ArgEffect,
}

impl CallArgumentTransfer {
    pub(super) fn may_write(&self) -> bool {
        self.effect.may_write()
    }
}

/// How a call moves values between its arguments' allocations.
#[derive(Clone, Copy)]
enum CallStorage {
    /// A user function may write through any field a mutable argument reaches.
    User,
    Plain,
    Native,
    /// A builtin edits only its receiver's own contents.
    Builtin,
    /// A builtin stores these argument positions into its receiver, argument 0.
    Store(&'static [(usize, FieldKey)]),
    ClearMap,
    /// A builtin takes one of its receiver's elements out as its result.
    Remove {
        map: bool,
    },
    /// A builtin returns one of its receiver's elements.
    Get,
}

pub(in crate::compiler_frontend::analysis::borrow_checker) fn call_member_result(
    target: &CallTarget,
) -> Option<usize> {
    matches!(
        CallStorage::for_target(target),
        CallStorage::Get | CallStorage::Remove { .. }
    )
    .then_some(0)
}

pub(in crate::compiler_frontend::analysis::borrow_checker) fn call_member_writes(
    target: &CallTarget,
) -> &'static [crate::compiler_frontend::public_call_summary::CallField] {
    use crate::compiler_frontend::public_call_summary::CallField;
    match CallStorage::for_target(target) {
        CallStorage::Store(_) | CallStorage::Remove { .. } => &[CallField::Elem],
        _ => &[],
    }
}

impl CallStorage {
    fn for_target(target: &CallTarget) -> Self {
        let CallTarget::External(id) = target else {
            return Self::User;
        };
        match CollectionBuiltinOp::from_external_function_id(*id) {
            Some(CollectionBuiltinOp::PushGrowable | CollectionBuiltinOp::PushFixed) => {
                Self::Store(&[(1, FieldKey::Elem)])
            }
            Some(CollectionBuiltinOp::Set) => Self::Store(&[(2, FieldKey::Elem)]),
            Some(CollectionBuiltinOp::Remove) => Self::Remove { map: false },
            Some(CollectionBuiltinOp::Get) => Self::Get,
            Some(_) => Self::Builtin,
            None => Self::Native,
        }
    }

    fn for_map_op(op: HirMapOp) -> Self {
        match op {
            // Arguments follow the receiver: a key, then a value.
            HirMapOp::Set => Self::Store(&[(1, FieldKey::MapKey), (2, FieldKey::Elem)]),
            HirMapOp::Clear => Self::ClearMap,
            HirMapOp::Remove => Self::Remove { map: true },
            HirMapOp::Get => Self::Get,
            _ => Self::Builtin,
        }
    }

    fn reaches_children(self) -> bool {
        matches!(self, Self::User)
    }
}

pub(in super::super) struct CallTransferContext<'a, 'module, 'state, 'tracker, 'stats, 'facts> {
    context: &'a BorrowTransferContext<'module>,
    view: OperationView<'a>,
    state: &'state mut BorrowState,
    block_id: BlockId,
    current_order: i32,
    location: Option<SourceSpan>,
    pub(in super::super) call_components: Option<&'a [SupportId]>,
    stats: &'stats mut BlockTransferStats,
    validation: Option<ValidationSink<'a, 'tracker, 'facts>>,
}

impl<'a, 'module, 'state, 'tracker, 'stats, 'facts>
    CallTransferContext<'a, 'module, 'state, 'tracker, 'stats, 'facts>
{
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        context: &'a BorrowTransferContext<'module>,
        view: OperationView<'a>,
        state: &'state mut BorrowState,
        block_id: BlockId,
        current_order: i32,
        location: Option<SourceSpan>,
        stats: &'stats mut BlockTransferStats,
        validation: Option<ValidationSink<'a, 'tracker, 'facts>>,
    ) -> Self {
        Self {
            context,
            view,
            state,
            block_id,
            current_order,
            location,
            call_components: None,
            stats,
            validation,
        }
    }

    pub(super) fn read_match_pattern(&mut self, arm: &HirMatchArm) -> Result<(), BorrowCheckError> {
        if self.validation.is_some() {
            let location = self.location;
            record_shared_reads_in_pattern(&mut self.shared_read_env(location), arm)?;
        }
        Ok(())
    }

    fn shared_read_env(&mut self, location: Option<SourceSpan>) -> SharedReadEnv<'_, 'module> {
        let validation = self
            .validation
            .as_mut()
            .expect("shared reads always validate");
        SharedReadEnv {
            context: self.context,
            layout: validation.layout,
            state: self.state,
            block_id: self.block_id,
            tracker: validation.tracker,
            location,
            current_order: self.current_order,
            stats: self.stats,
            value_fact_buffer: validation.value_facts,
        }
    }
}

#[derive(Clone, Copy)]
enum CallResultTransfer {
    Immediate,
    FirstDefinition(usize),
    Edge(crate::compiler_frontend::hir::ids::LocalId),
    Diverges,
}

impl CallResultTransfer {
    fn for_call_definition(
        view: OperationView<'_>,
        state: &BorrowState,
        block: BlockId,
        order: i32,
        results: &[HirLocalDestination],
    ) -> Self {
        let [HirLocalDestination::Define(local)] = results else {
            return Self::Immediate;
        };
        first_definition_index(view, state, block, order, || view.index_of(*local))
            .map_or(Self::Immediate, Self::FirstDefinition)
    }
}

fn transfer_call_arguments_and_result(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    returns: CallReturns<'_>,
    result_transfer: CallResultTransfer,
    storage: CallStorage,
) -> Result<(), BorrowCheckError> {
    let any_mutable = args.iter().any(CallArgumentTransfer::may_write);
    if any_mutable {
        input.stats.mutable_call_sites += 1;
    }

    let error = match result_transfer {
        CallResultTransfer::Edge(local) => Some(HirLocalDestination::Define(local)),
        _ => None,
    };
    let success_lane = if matches!(storage, CallStorage::Get) {
        CallLane::Lookup
    } else {
        CallLane::new(returns, InvokeLane::Success)
    };
    let error_returns = if matches!(storage, CallStorage::User | CallStorage::Native) {
        returns
    } else {
        CallReturns::Fresh
    };
    let error_lane = CallLane::new(error_returns, InvokeLane::Error);
    let retains_success = results.clone().next().is_some() && success_lane.retains_result();
    let retains_error = error.is_some() && error_lane.retains_result();
    let mut actuals = CallActuals::new(args);
    let plans = plan_executed_call_arguments(input, &mut actuals, storage)?;

    // Result provenance comes from the argument values, not from every root their evaluation
    // reads. `copy place` reads the source place but produces an independent graph, so a
    // retargeted alias into the copied value must not keep the source root alive.
    //
    // WHAT: only a retained result that may alias an argument reads these roots.
    // WHY: `Fresh` results and calls without a result never inspect them, so building the sets
    //      would walk every argument expression for no consumer.
    let mut arg_value_roots = InlineVec::Empty;
    if retains_success || retains_error {
        actuals.capture_derivations(input);
        for arg in args {
            let argument_location = input
                .context
                .diagnostics
                .value_error_span(arg.argument, input.location);
            arg_value_roots.push(call_argument_value_roots(
                input.context,
                input.view,
                input.validation.as_ref().map(|sink| sink.layout),
                input.state,
                arg.argument,
                argument_location,
            )?);
        }
    }

    if let CallStorage::Store(positions) = storage
        && input.view.entry_roots.is_some()
        && let Some(receiver) = args.first()
        && let HirExpressionKind::Load(place) =
            &input.context.expressions.expression(receiver.argument).kind
        && let Some(resolved) =
            resolve_place(input.view, input.state, input.context.expressions, place)
        && let Some(local) = input.view.index_of(place.root)
        && writes_entry_storage(input.view, input.state, local, &resolved.address)
    {
        for &(index, _) in positions {
            if let Some(arg) = args.get(index) {
                record_executed_retention(input, arg.argument);
            }
        }
    }
    check_call_arguments(
        input,
        &mut actuals,
        plans,
        storage,
        |arg_index| {
            (retains_success && success_lane.may_retain(arg_index, &arg_value_roots))
                || (retains_error && error_lane.may_retain(arg_index, &arg_value_roots))
        },
        error.is_some(),
    )?;
    if storage.reaches_children() && any_mutable {
        input.state.invalidate_detachments();
    }

    if matches!(result_transfer, CallResultTransfer::Diverges) {
        return Ok(());
    }
    publish_call_continuations(
        input,
        &mut actuals,
        CallContinuations {
            results,
            error,
            success_lane,
            error_lane,
        },
        &arg_value_roots,
        storage,
        result_transfer,
    )
}

fn plan_executed_call_arguments(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    storage: CallStorage,
) -> Result<InlineVec<Option<CallAccessPlan>>, BorrowCheckError> {
    let mut plans = InlineVec::Empty;
    if input.validation.is_some() {
        let args = actuals.arguments();
        for (index, arg) in args.iter().enumerate() {
            increment_frontend_counter(FrontendCounter::BorrowArgumentPlansScanned);
            let location = input
                .context
                .diagnostics
                .value_error_span(arg.argument, input.location);
            plans.push(plan_call_actual(
                input, actuals, index, arg, location, storage,
            )?);
        }
    }
    // Index and copy-source reads evaluated the operands. They are not call loans.
    if let Some(validation) = &mut input.validation {
        validation.tracker.clear_reservations();
    }
    Ok(plans)
}

fn writes_entry_storage(
    view: OperationView<'_>,
    state: &BorrowState,
    local: usize,
    addresses: &AddressSet,
) -> bool {
    view.entry_roots.is_some_and(|entry| {
        entry.contains(local)
            || state
                .effective_roots(local)
                .iter_ones()
                .any(|root| entry.contains(root))
            || addresses.cells().iter().any(|cell| match *cell {
                CellId::Slot(wrapper) => !wrapper.is_call_owned() && entry.contains(wrapper.local),
                CellId::Projected(holder, _) => state
                    .alloc_origin_local(holder)
                    .is_some_and(|root| entry.contains(root)),
            })
    })
}

fn record_executed_retention(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    value: HirValueId,
) {
    layer::visit_retained_locals(input.context.expressions, value, &mut |source| {
        if let Some(local) = input.view.index_of(source) {
            input.stats.executed_retention.insert(local);
            input
                .stats
                .executed_retention
                .extend(input.state.effective_roots(local).iter_ones());
        }
    });
}

/// Record argument execution once, not once per feasible continuation. For a
/// deferred call only operand evaluation is known; no optional transfer is final.
fn account_typed_call_arguments(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    facts: Option<&TypedCallAllocationFacts>,
    storage: CallStorage,
    on_edge: bool,
    mut selected_retention: impl FnMut(usize) -> bool,
    mut executed: impl FnMut(HirValueId, Option<ArgEffect>, bool),
) -> Result<(), BorrowCheckError> {
    let settled = classify_typed_continuation(
        facts,
        false,
        false,
        input.validation.is_some(),
        &input.context.diagnostics,
    )?;
    let plans = plan_executed_call_arguments(input, actuals, storage)?;
    let args = actuals.arguments();
    for (index, arg) in args.iter().enumerate() {
        let effect = (!matches!(settled, TypedContinuation::Deferred))
            .then(|| fallback_call_argument_effect(arg.effect));
        let retained = facts.is_some_and(|facts| facts.executed_may_retain(index));
        if input.stats.sparse && retained {
            record_executed_retention(input, arg.argument);
        }
        executed(arg.argument, effect, retained);
    }
    if let Some(facts) = facts {
        check_call_arguments(
            input,
            actuals,
            plans,
            storage,
            |index| facts.executed_may_retain(index) || selected_retention(index),
            on_edge,
        )?;
        if args.iter().any(CallArgumentTransfer::may_write) {
            input.stats.mutable_call_sites += 1;
            if storage.reaches_children() {
                input.state.invalidate_detachments();
            }
        }
    }
    Ok(())
}

fn transfer_typed_user_call(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    facts: Option<&TypedCallAllocationFacts>,
    transfer: CallResultTransfer,
) -> Result<(), BorrowCheckError> {
    use crate::compiler_frontend::analysis::borrow_checker::engine::apply_captured_call_batch;
    use crate::compiler_frontend::public_call_summary::typed::EntryPort;

    let error = match transfer {
        CallResultTransfer::Edge(local) => Some(HirLocalDestination::Define(local)),
        _ => None,
    };
    let terminal = matches!(transfer, CallResultTransfer::Diverges);
    let order = input.current_order;
    if facts.is_some()
        && !terminal
        && input.state.has_recent_call_storage(order)
        && input
            .state
            .block_may_repeat(input.block_id, input.view.block_successors)
    {
        // Carry the preceding invocation's storage forward before capturing
        // actuals. The batch's newly published objects use the recent keys.
        let before = input.stats.observations.before_rekey(input.state);
        input.state.age_call_storage(order);
        input.stats.observations.after_rekey(
            before,
            input.state,
            |wrapper| input.state.aged_call_wrapper(wrapper, order),
            |id| input.state.aged_call_allocation(id, order),
        );
    }
    let mut actuals = CallActuals::new(args);
    let mut retained = None;
    if let Some(facts) = facts
        && !terminal
        && input.validation.is_some()
    {
        let mut mark_retained = |parameter| {
            let slots: &mut InlineVec<bool> =
                retained.get_or_insert_with(|| (0..args.len()).map(|_| false).collect());
            slots[parameter] = true;
        };
        let mut uses_opaque_sources = false;
        for lane in std::iter::once(&facts.success).chain(error.iter().map(|_| &facts.error)) {
            lane.visit_compatible(|_| Ok::<_, BorrowCheckError>(None), |_, product| {
                for post in product.components().iter().flat_map(|component| &component.alternatives) {
                    uses_opaque_sources |= matches!(
                        &post.coverage,
                        crate::compiler_frontend::public_call_summary::typed::Coverage::Unresolved(
                            crate::compiler_frontend::public_call_summary::typed::Footprint::Locations(_)
                        )
                    );
                    post.visit_entry_ports(false, &mut |port| {
                        let mut port = &facts.ports[port.0 as usize];
                        if let EntryPort::Edge(holder, _) = port {
                            port = &facts.ports[holder.0 as usize];
                        }
                        let parameter = match port {
                            EntryPort::Wrapper(index) | EntryPort::Immediate(index) | EntryPort::Terminal(index) => *index,
                            EntryPort::Value { parameter, .. } => *parameter,
                            EntryPort::Edge(..) => unreachable!("typed edge holder is an entry value"),
                        };
                        mark_retained(parameter);
                    });
                }
                Ok(())
            })?;
        }
        if uses_opaque_sources {
            // A bounded opaque suffix substitutes possible predecessor values
            // even when its known prefix does not name a source explicitly.
            for port in &facts.ports {
                if let EntryPort::Value { parameter, .. } = *port {
                    mark_retained(parameter);
                }
            }
        }
    }
    account_typed_call_arguments(
        input,
        &mut actuals,
        facts,
        CallStorage::User,
        error.is_some(),
        |index| retained.as_ref().is_some_and(|slots| slots[index]),
        |_, _, _| {},
    )?;
    let Some(facts) = facts else {
        input.stats.deferred = true;
        input.stats.invoke.success.unreachable = true;
        input.stats.invoke.error.unreachable = true;
        return Ok(());
    };
    if terminal {
        input.stats.diverges = true;
        return Ok(());
    }
    let mut entries = capture_typed_actuals(input, &mut actuals, &facts.ports)?;
    let parameters: InlineVec<_> = args
        .iter()
        .map(|actual| input.context.expressions.expression(actual.argument).ty)
        .collect();
    if let Some(retained) = retained {
        let mut roots: Option<InlineVec<Option<RootSet>>> = None;
        for (port, capture) in facts.ports.iter().zip(&mut entries) {
            let parameter = match port {
                EntryPort::Wrapper(parameter)
                | EntryPort::Immediate(parameter)
                | EntryPort::Terminal(parameter)
                | EntryPort::Value { parameter, .. } => *parameter,
                EntryPort::Edge(..) => continue,
            };
            if !retained[parameter] {
                continue;
            }
            let roots = roots.get_or_insert_with(|| (0..args.len()).map(|_| None).collect());
            let argument = args[parameter].argument;
            if roots[parameter].is_none() {
                roots[parameter] = Some(call_argument_value_roots(
                    input.context,
                    input.view,
                    input.validation.as_ref().map(|sink| sink.layout),
                    input.state,
                    argument,
                    input
                        .context
                        .diagnostics
                        .value_error_span(argument, input.location),
                )?);
            }
            capture.roots.clone_from(&roots[parameter]);
        }
    }
    let checkpoints = std::mem::take(&mut input.stats.invoke.success.checkpoints);
    let success = capture_typed_user_lane(
        input,
        &entries,
        &parameters,
        facts,
        results,
        false,
        checkpoints,
    )?;
    if let Some(error) = error {
        let failure = capture_typed_user_lane(
            input,
            &entries,
            &parameters,
            facts,
            std::iter::once(error),
            true,
            Vec::new(),
        )?;
        input.stats.invoke.success.unreachable = success.is_none();
        input.stats.invoke.error.unreachable = failure.is_none();
        input.stats.invoke.success.typed = success;
        input.stats.invoke.error.typed = failure;
    } else if let Some(success) = success {
        if input.stats.sparse {
            input.stats.sparse_call = Some(success);
            return Ok(());
        }
        let observations = apply_captured_call_batch(
            super::super::EdgeTransferView {
                operations: input.view,
            },
            success,
            [],
            input.current_order,
            input.state,
        )?;
        input.stats.observations.append(observations);
    } else {
        input.stats.diverges = true;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn capture_typed_user_lane(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    entries: &[call_results::EntryCapture],
    parameters: &[crate::compiler_frontend::datatypes::ids::TypeId],
    facts: &TypedCallAllocationFacts,
    destinations: impl Iterator<Item = HirLocalDestination> + Clone,
    error: bool,
    checkpoints: Vec<super::super::ObservationCheckpoint>,
) -> Result<Option<CapturedCallBatch<MatchedLane>>, BorrowCheckError> {
    let TypedContinuation::Match(lane) = classify_typed_continuation(
        Some(facts),
        false,
        error,
        input.validation.is_some(),
        &input.context.diagnostics,
    )?
    else {
        return Ok(None);
    };
    let types = destinations
        .clone()
        .map(|destination| input.context.local_types[&destination.local()])
        .collect::<InlineVec<_>>();
    let post = match_call_actuals(input, entries, parameters, &types, lane, error)?;
    if post.is_empty() {
        return Ok(None);
    }
    let destinations = destinations
        .map(|dest| {
            let local = call_result_index(input, dest.local())?;
            Ok(match dest {
                HirLocalDestination::Define(_) => CapturedDestination::Define(local),
                HirLocalDestination::Update(_) => CapturedDestination::Update {
                    actor: local,
                    address: capture_call_result_address(input, local)?,
                },
            })
        })
        .collect::<Result<_, BorrowCheckError>>()?;
    Ok(Some(CapturedCallBatch {
        post,
        destinations,
        checkpoints,
        member_reads: Demand::default(),
        detachment: None,
    }))
}

struct CallContinuations<'a, Results> {
    results: Results,
    error: Option<HirLocalDestination>,
    success_lane: CallLane<'a>,
    error_lane: CallLane<'a>,
}

fn publish_call_continuations(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    continuations: CallContinuations<'_, impl Iterator<Item = HirLocalDestination> + Clone>,
    arg_value_roots: &[RootSet],
    storage: CallStorage,
    result_transfer: CallResultTransfer,
) -> Result<(), BorrowCheckError> {
    // Capture both lanes against entry storage before either lane installs stores or bindings.
    if matches!(storage, CallStorage::Remove { .. } | CallStorage::Get) {
        let batch = capture_typed_builtin_element_batch(
            input,
            actuals,
            continuations.results.clone().next(),
            storage,
        )?;
        if let Some(error) = continuations.error {
            let lane = continuations.error_lane;
            let operations = call_result_ops(
                input,
                actuals,
                std::iter::once(error),
                lane,
                InvokeLane::Error,
            )?;
            let result = call_result_local_state(
                input,
                lane.root_sources(0),
                arg_value_roots,
                lane.derive_result(0, actuals, input.view.local_count()),
            )?;
            input
                .stats
                .invoke
                .error
                .results
                .push((error.local(), result));
            input.stats.invoke.error.operations = operations;
            if let Some(batch) = batch {
                input.stats.invoke.success.typed = Some(CapturedCallBatch {
                    post: vec![call_results::MatchedStratum {
                        #[cfg(test)]
                        guard: Vec::new(),
                        components: vec![call_results::MatchedComponent {
                            support: SupportId(0),
                            alternatives: vec![batch.post],
                        }],
                    }],
                    destinations: batch.destinations,
                    member_reads: batch.member_reads,
                    checkpoints: batch.checkpoints,
                    detachment: batch.detachment,
                });
            } else {
                input.stats.invoke.success.unreachable = true;
            }
        } else if let Some(batch) = batch {
            let observations =
                crate::compiler_frontend::analysis::borrow_checker::engine::apply_captured_call_batch(
                    super::super::EdgeTransferView { operations: input.view },
                    batch, [], input.current_order, input.state,
                )?;
            input.stats.observations.append(observations);
        } else {
            input.stats.diverges = true;
        }
        return Ok(());
    }
    let (mut success_ops, error_ops) =
        capture_call_continuations(input, actuals, &continuations, storage)?;
    let CallContinuations {
        results,
        error,
        success_lane,
        error_lane,
    } = continuations;
    let mut writes = Vec::new();
    if matches!(result_transfer, CallResultTransfer::Immediate) {
        // Capture and check every update before installing any result definition.
        for (slot, result) in results.clone().enumerate() {
            let HirLocalDestination::Update(local) = result else {
                continue;
            };
            let value = call_result_local_state(
                input,
                success_lane.root_sources(slot),
                arg_value_roots,
                success_lane.derive_result(slot, actuals, input.view.local_count()),
            )?;
            writes.push(capture_call_result_write(
                input,
                local,
                value,
                &mut success_ops,
            )?);
        }
    }
    for (slot, result) in results.enumerate() {
        if error.is_none() && matches!(result, HirLocalDestination::Update(_)) {
            continue;
        }
        let derived = success_lane.derive_result(slot, actuals, input.view.local_count());
        prepare_call_result_definition(
            input,
            result.local(),
            success_lane.root_sources(slot),
            arg_value_roots,
            derived,
            result_transfer,
            &mut success_ops,
            &mut writes,
        )?;
    }
    if let Some(error) = error {
        let state = call_result_local_state(
            input,
            error_lane.root_sources(0),
            arg_value_roots,
            error_lane.derive_result(0, actuals, input.view.local_count()),
        )?;
        input
            .stats
            .invoke
            .error
            .results
            .push((error.local(), state));
        input.stats.invoke.success.operations = success_ops;
        input.stats.invoke.error.operations = error_ops;
    } else {
        apply_immediate_call_operations(input, success_ops, writes);
        input.state.prune_retired_wrappers();
    }
    Ok(())
}

fn capture_call_continuations(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    continuations: &CallContinuations<'_, impl Iterator<Item = HirLocalDestination> + Clone>,
    storage: CallStorage,
) -> Result<(Vec<InvokeLayerOp>, Vec<InvokeLayerOp>), BorrowCheckError> {
    let CallContinuations {
        results,
        error,
        success_lane,
        error_lane,
    } = continuations;
    let results = results.clone();
    let success_ops = match storage {
        CallStorage::User | CallStorage::Native | CallStorage::Plain | CallStorage::Builtin => {
            call_result_ops(input, actuals, results, *success_lane, InvokeLane::Success)?
        }
        CallStorage::Store(positions) => capture_builtin_store(input, actuals, positions)?,
        CallStorage::ClearMap => call_receiver_allocs(input, actuals)?
            .filter(|receiver| !receiver.ids.is_empty())
            .map_or_else(Vec::new, |receiver| {
                let coverage = receiver.holder_coverage();
                vec![InvokeLayerOp::Store(TypedStore::reset_families(
                    receiver.ids,
                    coverage,
                    EntryFamilies::MapEntries,
                ))]
            }),
        CallStorage::Remove { .. } | CallStorage::Get => {
            unreachable!("member calls are captured first")
        }
    };
    let error_ops = if error.is_some() {
        call_result_ops(
            input,
            actuals,
            error.iter().copied(),
            *error_lane,
            InvokeLane::Error,
        )?
    } else {
        Vec::new()
    };
    Ok((success_ops, error_ops))
}

fn check_call_arguments(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    mut plans: InlineVec<Option<CallAccessPlan>>,
    storage: CallStorage,
    mut retains_argument: impl FnMut(usize) -> bool,
    on_edge: bool,
) -> Result<(), BorrowCheckError> {
    let mut operand_index = None;
    for (arg_index, arg) in actuals.arguments().iter().take(plans.len()).enumerate() {
        let Some(mut plan) = plans[arg_index].take() else {
            continue;
        };
        let argument_location = input
            .context
            .diagnostics
            .value_error_span(arg.argument, input.location);
        let retains_argument = retains_argument(arg_index);
        let optional = matches!(
            arg.effect,
            ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
        );
        if optional {
            increment_frontend_counter(
                FrontendCounter::BorrowOptionalTransferCandidatesBeforeEligibility,
            );
        }
        let layout = input
            .validation
            .as_ref()
            .expect("validation owns move decisions")
            .layout;
        let can_move = optional
            && !retains_argument
            && !plan.roots.is_empty()
            && classify_move_decision(layout, input.block_id, &plan.roots, input.current_order)
                == MoveDecision::Move;
        // Even an ineligible move may have a different published fact when it is forced to
        // borrow: an empty root set changes NotAttempted, and shared access can report fewer
        // roots. Keep checking those observable cases without attempting an exclusive move.
        let needs_overlap = optional
            && !retains_argument
            && (can_move
                || plan.roots.is_empty()
                || arg.effect == ArgEffect::MayConsumeShared && plan.roots != plan.value_roots);
        let conflicts_with_operand = needs_overlap && arg_index + 1 < plans.len() && {
            increment_frontend_counter(
                FrontendCounter::BorrowOptionalTransferCandidatesAfterEligibility,
            );
            let index = operand_index.get_or_insert_with(|| {
                OperandAccessIndex::new(
                    plans.len(),
                    plans.iter().enumerate().filter_map(|(position, plan)| {
                        plan.as_ref().map(|plan| (position, &plan.access))
                    }),
                )
            });
            let usage = plan.access.usage;
            plan.access.usage = usage.for_transfer_probe();
            let conflict = index.any_later(input.state, arg_index, &plan.access, |other| {
                let other = plans[other]
                    .as_ref()
                    .expect("later operand remains planned");
                increment_frontend_counter(FrontendCounter::BorrowOptionalTransferOverlapCalls);
                overlap(input.state, &plan.access, &other.access) != Overlap::No
            });
            plan.access.usage = usage;
            conflict
        };
        let effect = if retains_argument || conflicts_with_operand {
            fallback_call_argument_effect(arg.effect)
        } else {
            arg.effect
        };
        if optional && effect != arg.effect {
            // The retained result only keeps the argument borrowed; it does not make the
            // argument's provenance roots call-argument writes. Merging overlap or value roots
            // here would publish an immutable parameter as `Writes` in the summary.
            input
                .validation
                .as_mut()
                .expect("validation owns argument checks")
                .value_facts
                .record_optional_transfer_status(arg.argument, OptionalTransferStatus::Borrow);
        }
        let resolution = actuals.entry_place(input, arg_index)?;
        transfer_call_argument_access(
            input,
            arg.argument,
            effect,
            can_move && !conflicts_with_operand,
            argument_location,
            plan,
            resolution.as_deref(),
        )?;
    }
    if let Some(validation) = &input.validation
        && input.state.has_dependency_carriers()
    {
        input.stats.invoke.success.checkpoints = record_call_observations(
            &mut input.stats.observations,
            input.state,
            validation.tracker,
            input.current_order,
            input.location,
            storage.reaches_children(),
            on_edge && !storage.reaches_children(),
        );
    }
    // Call loans end before any result destination is written.
    if let Some(validation) = &mut input.validation {
        validation.tracker.clear_reservations();
    }
    Ok(())
}

/// Allocations the receiver argument reaches, when the layer has anything to say about them.
fn call_receiver_allocs(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
) -> Result<Option<ResolvedPlace>, BorrowCheckError> {
    let Some(place) = actuals.entry_place(input, 0)? else {
        return Ok(None);
    };
    input
        .state
        .load_place_content(place, input.view.local_mutable, false);
    Ok(place.content.clone())
}

/// A fallible builtin stores nothing when it fails.
fn capture_builtin_store(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    positions: &[(usize, FieldKey)],
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let Some(holders) = call_receiver_allocs(input, actuals)? else {
        return Ok(Vec::new());
    };
    let mut ops = Vec::new();
    for &(index, key) in positions {
        let Some(argument) = actuals.arguments().get(index) else {
            continue;
        };
        if !input.stats.sparse
            && !needs_layer_store_capture(
                input.context,
                input.state,
                &holders.ids,
                key,
                argument.argument,
            )?
        {
            continue;
        }
        let stored = actuals.root(input, index)?.clone();
        ops.push(InvokeLayerOp::Store(TypedStore::write_projected(
            holders.ids.iter().copied(),
            key,
            holders.holder_coverage(),
            stored,
            true,
        )));
    }
    Ok(ops)
}

#[derive(Default)]
struct BuiltinElementCapture {
    result: Option<CapturedValue>,
    removal: Option<(Vec<AllocId>, bool)>,
}

fn capture_builtin_element(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    result: Option<HirLocalDestination>,
    storage: CallStorage,
    member_reads: &mut Demand,
    receiver: Option<ResolvedPlace>,
) -> BuiltinElementCapture {
    let heap_result = result
        .and_then(|result| input.view.index_of(result.local()))
        .is_some_and(|index| input.view.local_has_in_place_mutation_surface[index]);
    if !heap_result && input.state.layer_is_empty() && !matches!(storage, CallStorage::Get) {
        return BuiltinElementCapture::default();
    }
    let Some(mut elements) = receiver else {
        return BuiltinElementCapture::default();
    };
    if input.validation.is_some() && input.state.has_dependency_carriers() {
        member_reads.extend(selected_members(input.state, &elements));
    }
    let holders = if matches!(storage, CallStorage::Remove { .. })
        && elements.holder_coverage() == Coverage::Complete
    {
        elements.ids.clone()
    } else {
        Vec::new()
    };
    let lookup = matches!(storage, CallStorage::Get);
    let value =
        if result.is_some_and(|_| lookup || heap_result || input.state.has_value_alternatives()) {
            let dependency = lookup.then(|| ReceiverDependency {
                entries: SmallSet::from_iter(elements.ids.iter().copied()),
                coverage: elements.holder_coverage(),
            });
            let mut value = if heap_result {
                input
                    .state
                    .step_resolved(&mut elements, FieldKey::Elem, input.view.local_mutable);
                capture_graph_values(elements)
            } else {
                input
                    .state
                    .capture_scalar_projection(&elements, FieldKey::Elem)
            };
            let readonly = value.readonly() || lookup;
            value.restrict_permission(readonly);
            if let Some(dependency) = dependency {
                // The selected value may inherit dependencies from both its holder and its payload.
                value
                    .alternatives
                    .retain(|alt| !matches!(alt.kind, PayloadKind::Absent));
                if value.alternatives.is_empty() {
                    if heap_result {
                        value.alternatives.extend(
                            value
                                .values
                                .iter()
                                .copied()
                                .map(|value| ValueAlt::plain(PayloadKind::Allocation(value))),
                        );
                    }
                    if value.alternatives.is_empty() {
                        value.alternatives.push(ValueAlt::plain(if heap_result {
                            PayloadKind::Unknown
                        } else {
                            PayloadKind::Scalar
                        }));
                    }
                }
                for alt in &mut value.alternatives {
                    if let Err(index) = alt.deps.binary_search(&dependency) {
                        alt.deps.insert(index, dependency.clone());
                    }
                }
            }
            Some(value)
        } else {
            None
        };
    let removal = match storage {
        CallStorage::Remove { map } => Some((holders, map)),
        _ => None,
    };
    BuiltinElementCapture {
        result: value,
        removal,
    }
}

fn capture_typed_builtin_element_batch(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    result: Option<HirLocalDestination>,
    storage: CallStorage,
) -> Result<Option<CapturedCallBatch>, BorrowCheckError> {
    let mut member_reads = Demand::default();
    let receiver = if actuals.entry_place(input, 0)?.is_none() {
        Some(ResolvedPlace::from_capture(actuals.root(input, 0)?))
    } else {
        call_receiver_allocs(input, actuals)?
    };
    if matches!(storage, CallStorage::Get)
        && receiver.as_ref().is_some_and(|receiver| {
            receiver.holder_coverage() == Coverage::Complete
                && !receiver.ids.is_empty()
                && receiver
                    .ids
                    .iter()
                    .all(|&holder| input.state.alloc_group_is_empty(holder, FieldKey::Elem))
        })
    {
        return Ok(None);
    }
    let capture = capture_builtin_element(input, result, storage, &mut member_reads, receiver);
    let mut post = call_results::CapturedPost::default();
    let mut destinations = InlineVec::Empty;
    let mut result_index = None;
    if let Some(destination) = result {
        let (roots, derivation) = if matches!(storage, CallStorage::Get) {
            let receiver = actuals.expression(0).ok_or_else(|| {
                input
                    .context
                    .diagnostics
                    .internal_error("builtin lookup has no captured receiver")
            })?;
            let location = input
                .context
                .diagnostics
                .value_error_span(receiver, input.location);
            let roots = call_argument_value_roots(
                input.context,
                input.view,
                input.validation.as_ref().map(|sink| sink.layout),
                input.state,
                receiver,
                location,
            )?;
            let derivation = super::alias_derivation_roots_from_expression(
                input.view,
                input.state,
                input.context.expressions,
                receiver,
            );
            (
                Some(roots),
                Some(call_results::CapturedDerivation::Roots(derivation)),
            )
        } else {
            (None, None)
        };
        let actor = call_result_index(input, destination.local())?;
        result_index = Some(0);
        let mut value = capture.result.unwrap_or_default();
        if value.is_empty() {
            value.alternatives.push(ValueAlt::plain(
                if input.view.local_has_in_place_mutation_surface[actor] {
                    PayloadKind::Unknown
                } else {
                    PayloadKind::Scalar
                },
            ));
        }
        post.results.push(call_results::CapturedResult {
            slot: 0,
            value,
            roots,
            derivation,
        });
        destinations.push(match destination {
            HirLocalDestination::Define(_) => CapturedDestination::Define(actor),
            HirLocalDestination::Update(_) => CapturedDestination::Update {
                actor,
                address: capture_call_result_address(input, actor)?,
            },
        });
    }
    Ok(Some(CapturedCallBatch {
        post,
        destinations,
        member_reads,
        checkpoints: Vec::new(),
        detachment: capture.removal.map(|(holders, map)| CapturedDetachment {
            holders,
            map,
            result: result_index,
        }),
    }))
}

struct CallAccessPlan {
    roots: RootSet,
    value_roots: RootSet,
    access: AccessFootprint,
}

fn plan_call_actual(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    index: usize,
    arg: &CallArgumentTransfer,
    location: Option<SourceSpan>,
    storage: CallStorage,
) -> Result<Option<CallAccessPlan>, BorrowCheckError> {
    let expressions = input.context.expressions;
    let expression = expressions.expression(arg.argument);
    let mut roots = RootSet::empty(input.view.local_count());
    let layout = input
        .validation
        .as_ref()
        .expect("argument plans always validate")
        .layout;
    let mut env = input.shared_read_env(location);
    let mutable = arg.may_write();
    let receiver = index == 0
        && !matches!(
            storage,
            CallStorage::User | CallStorage::Native | CallStorage::Plain
        );
    let usage = match (mutable, receiver) {
        (false, false) => PlaceUse::Read,
        (false, true) => PlaceUse::ReadEntries,
        (true, true) => PlaceUse::WriteEntries,
        (true, false) => PlaceUse::Mutate {
            deep: storage.reaches_children(),
        },
    };
    let (place, copy) = match &expression.kind {
        HirExpressionKind::Load(place) => (place, false),
        HirExpressionKind::Copy(place) => (place, true),
        _ => {
            record_shared_reads_in_expression(&mut env, arg.argument, location, &mut roots)?;
            if !layer::is_heap_value(input.context, arg.argument)? {
                return Ok(None);
            }
            let value = actuals.root(input, index)?;
            let mut access =
                AccessFootprint::cells(AddressSet::from_cells([], Coverage::Complete), usage);
            access.content = Some(ResolvedPlace::from_values(
                &value.values,
                value.views.clone(),
            ));
            return Ok(Some(CallAccessPlan {
                value_roots: roots.clone(),
                roots,
                access,
            }));
        }
    };
    record_shared_reads_in_place_indices(&mut env, place, location, &mut roots)?;
    let value_roots = roots_for_place(
        layout,
        input.state,
        place,
        location,
        expression.span,
        &input.context.diagnostics,
    )?;
    let heap = if place.projections.is_empty() {
        input
            .view
            .index_of(place.root)
            .is_some_and(|local| input.view.local_has_in_place_mutation_surface[local])
    } else {
        layer::is_heap_value(input.context, arg.argument)?
    };
    let usage = if copy { PlaceUse::Read } else { usage };
    let resolution = if copy {
        actuals.entry_copy_source(input, index)?
    } else {
        actuals.entry_place(input, index)?
    };
    let mut roots = if mutable && !heap && !copy && place.projections.is_empty() {
        RootSet::empty(input.view.local_count())
    } else {
        value_roots.clone()
    };
    let access = if let Some(resolution) = resolution {
        if heap {
            input
                .state
                .load_place_content(resolution, input.view.local_mutable, false);
        }
        report_place_cells(input.state, &resolution.address, &mut roots);
        let access = footprint(resolution, usage);
        if copy {
            let mut check = call_check_context(
                input,
                expression.span,
                location,
                Some(resolution.route.local),
            );
            check_access(&mut check, &roots, access, Some(resolution), BORROW_ACCESS)?;
            input
                .validation
                .as_mut()
                .expect("validation owns argument checks")
                .value_facts
                .record(
                    arg.argument,
                    ValueAccessClassification::SharedRead,
                    &value_roots,
                );
            return Ok(None);
        }
        access
    } else {
        AccessFootprint::cells(AddressSet::from_cells([], Coverage::Partial), usage)
    };
    Ok(Some(CallAccessPlan {
        roots,
        value_roots,
        access,
    }))
}

/// Roots of the value an argument expression denotes, separate from roots read to evaluate it.
///
/// WHAT: a direct place read reports the place's effective roots, `copy` reports a fresh value and
///       computed shapes keep their conservative child-root union.
/// WHY: return-alias summaries describe the result's value provenance. Evaluation roots also
///      include roots that cannot be aliased by the produced value, so using them as result
///      provenance falsely extends a copied source's borrow.
fn call_argument_value_roots(
    context: &BorrowTransferContext<'_>,
    view: OperationView<'_>,
    validation: Option<&FunctionLayout>,
    state: &BorrowState,
    expression_id: HirValueId,
    location: Option<SourceSpan>,
) -> Result<RootSet, BorrowCheckError> {
    let expression = context.expressions.expression(expression_id);
    let roots = |place: &HirPlace, span| {
        if let Some(layout) = validation {
            roots_for_place(layout, state, place, location, span, &context.diagnostics)
        } else {
            let index =
                require_place_index(view.index_of(place.root), &context.diagnostics, place)?;
            Ok(state.effective_roots(index))
        }
    };

    match &expression.kind {
        HirExpressionKind::Copy(_) => Ok(RootSet::empty(view.local_count())),

        HirExpressionKind::Load(place) => match expression.value_kind {
            ValueKind::Place | ValueKind::RValue => roots(place, expression.span),
            ValueKind::Const => Ok(RootSet::empty(view.local_count())),
        },

        _ => {
            let mut value_roots = RootSet::empty(view.local_count());
            collect_expression_roots(
                context.expressions,
                expression_id,
                &mut value_roots,
                expression.span,
                roots,
            )?;
            Ok(value_roots)
        }
    }
}

fn fallback_call_argument_effect(effect: ArgEffect) -> ArgEffect {
    match effect {
        ArgEffect::MayConsumeShared => ArgEffect::SharedBorrow,
        ArgEffect::MayConsumeMutable => ArgEffect::MutableBorrow,
        _ => effect,
    }
}

fn call_check_context<'a, 'module>(
    input: &'a mut CallTransferContext<'_, 'module, '_, '_, '_, '_>,
    span: Option<SourceSpan>,
    location: Option<SourceSpan>,
    actor_index_hint: Option<usize>,
) -> AccessCheckContext<'a, 'module> {
    let validation = input
        .validation
        .as_mut()
        .expect("validation owns call access checks");
    AccessCheckContext {
        context: input.context,
        layout: validation.layout,
        state: input.state,
        block_id: input.block_id,
        tracker: validation.tracker,
        location,
        span,
        stats: input.stats,
        actor_index_hint,
        current_order: input.current_order,
    }
}

fn transfer_call_argument_access(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    argument: HirValueId,
    effect: ArgEffect,
    can_move: bool,
    location: Option<SourceSpan>,
    plan: CallAccessPlan,
    source: Option<&PlaceResolution>,
) -> Result<(), BorrowCheckError> {
    let expression = input.context.expressions.expression(argument);
    let mutable = plan.access.usage.kind() == AccessKind::Mutable;
    if mutable && let Some(source) = source {
        let actor = source.route.local;
        let check = call_check_context(input, expression.span, location, Some(actor));
        if expression.value_kind == ValueKind::Place && !check.layout.local_mutable[actor] {
            return Err(immutable_cell_error(&check, actor));
        }
        if expression.value_kind == ValueKind::Place {
            check_cell_permission(&check, source)?;
        }
        if let Some(content) = &source.content {
            let readonly_origin = content
                .ids
                .iter()
                .filter_map(|&id| check.state.alloc_origin_local(id))
                .find(|&origin| !check.layout.local_mutable[origin]);
            if content.readonly || readonly_origin.is_some() {
                return Err(immutable_cell_error(
                    &check,
                    content.readonly_source.or(readonly_origin).unwrap_or(actor),
                ));
            }
        }
    }

    let optional = matches!(
        effect,
        ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
    );
    let transferred = {
        let actor = source.map(|source| source.route.local);
        let mut check = call_check_context(input, expression.span, location, actor);
        let fallback = if can_move {
            probe_mutable_access(
                &mut check,
                &plan.roots,
                plan.access,
                source,
                MutableAccessPolicy {
                    strict_move_exclusivity: true,
                    ..BORROW_ACCESS
                },
            )?
        } else {
            Some(plan.access)
        };
        if let Some(access) = fallback {
            check_call_access(&mut check, &plan.roots, access, source)?;
            false
        } else {
            true
        }
    };
    if optional && !plan.roots.is_empty() {
        input
            .validation
            .as_mut()
            .expect("validation owns argument checks")
            .value_facts
            .record_optional_transfer(
                argument,
                if transferred {
                    OptionalTransferStatus::Transfer
                } else {
                    OptionalTransferStatus::Borrow
                },
                &plan.roots,
            );
    }
    input
        .validation
        .as_mut()
        .expect("validation owns argument checks")
        .value_facts
        .record(
            argument,
            if mutable {
                ValueAccessClassification::MutableArgument
            } else {
                ValueAccessClassification::SharedRead
            },
            if mutable || optional {
                &plan.roots
            } else {
                &plan.value_roots
            },
        );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare_call_result_definition(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    result_local: LocalId,
    alias_arguments: Option<&[usize]>,
    arg_value_roots: &[RootSet],
    derived_roots: RootSet,
    result_transfer: CallResultTransfer,
    operations: &mut [InvokeLayerOp],
    writes: &mut [CallResultWrite],
) -> Result<(), BorrowCheckError> {
    let local_index = call_result_index(input, result_local)?;
    let on_edge = matches!(result_transfer, CallResultTransfer::Edge(_));
    if !on_edge
        && (operations.iter().any(
            |op| matches!(op, InvokeLayerOp::Store(store) if store.captures_slot(local_index)),
        ) || writes
            .iter()
            .any(|write| write.store.captures_slot(local_index)))
    {
        input.state.retain_captured_wrapper(local_index);
    }
    let new_local_state =
        call_result_local_state(input, alias_arguments, arg_value_roots, derived_roots)?;
    if on_edge {
        input
            .stats
            .invoke
            .success
            .results
            .push((result_local, new_local_state));
        return Ok(());
    }

    if matches!(result_transfer, CallResultTransfer::FirstDefinition(index) if index == local_index)
        && input.state.can_publish_first_definition(local_index)
        && !arg_value_roots
            .iter()
            .any(|roots| roots.contains(local_index))
        && {
            let names = |id| input.state.is_current_allocation_of(id, local_index);
            !operations
                .iter()
                .any(|op| op.captures_current(local_index, &names))
                && !writes
                    .iter()
                    .any(|write| write.store.captures_current(local_index, &names))
        }
    {
        input.state.update_local_state(local_index, new_local_state);
        return Ok(());
    }
    let before = input.stats.observations.before_rekey(input.state);
    let retirement = if !input.view.local_has_in_place_mutation_surface[local_index]
        && !matches!(result_transfer, CallResultTransfer::FirstDefinition(_))
    {
        Some(
            input
                .state
                .rebind_local(local_index, new_local_state, input.current_order),
        )
    } else {
        // A captured old wrapper takes the normal retirement path, even on a first definition.
        input.state.rebind_local_after_capture(
            local_index,
            new_local_state,
            input.current_order,
            None,
            true,
        )
    };
    if let Some(retirement) = retirement {
        input
            .stats
            .observations
            .after_retirement(before, input.state, retirement);
        for op in operations {
            op.remap(input.state, retirement);
        }
        for write in writes {
            write.derivation.remove(local_index);
            write.remap(input.state, retirement);
        }
    }
    Ok(())
}

struct CallResultWrite {
    actor: usize,
    definite_slot: Option<usize>,
    derivation: RootSet,
    store: TypedStore,
}

impl CallResultWrite {
    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        if let Some(slot) = self.definite_slot {
            let wrapper = WrapperId::current(slot);
            if retirement.remap_wrapper(wrapper) != wrapper {
                self.definite_slot = None;
            }
        }
        self.store.remap(state, retirement);
    }
}

fn capture_call_result_write(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    result: LocalId,
    value: LocalState,
    ops: &mut Vec<InvokeLayerOp>,
) -> Result<CallResultWrite, BorrowCheckError> {
    let actor = call_result_index(input, result)?;
    let addresses = capture_call_result_address(input, actor)?;

    // Bind carries the captured value for either call form. Immediate updates already
    // captured their receiving address, so move the payload out instead of applying Bind.
    let captured = match ops
        .iter()
        .position(|op| matches!(op, InvokeLayerOp::Bind { local, .. } if *local == result))
        .map(|index| ops.remove(index))
    {
        Some(InvokeLayerOp::Bind { value, .. }) => value,
        _ => CapturedValue::default(),
    };
    let readonly = captured.readonly();
    let definite_slot = match addresses.definite() {
        Some(CellId::Slot(wrapper)) => Some(wrapper.local),
        _ => None,
    };
    Ok(CallResultWrite {
        actor,
        definite_slot,
        derivation: value.derived_from_roots,
        store: TypedStore::receive_result(addresses, captured, actor, readonly)
            .with_slot_roots(value.value_roots),
    })
}

fn capture_call_result_address(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actor: usize,
) -> Result<AddressSet, BorrowCheckError> {
    if let Some(validation) = &input.validation {
        require_initialized_update_target(
            validation.layout,
            input.state,
            actor,
            &input.context.diagnostics,
            input.location,
        )?;
    }
    let resolved = input
        .state
        .resolve_place_path(actor, input.view.local_mutable, []);
    if input.validation.is_some() {
        let location = input.location;
        let mut check = call_check_context(input, location, location, Some(actor));
        check_local_update_access(&mut check, &resolved)?;
    }
    Ok(resolved.address)
}

fn apply_immediate_call_operations(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    ops: Vec<InvokeLayerOp>,
    writes: Vec<CallResultWrite>,
) {
    for op in ops {
        input
            .stats
            .observations
            .publish(op.apply_owned(input.view, input.state));
    }
    let mut writes = writes.into_iter();
    while let Some(mut write) = writes.next() {
        if let Some(slot) = write.definite_slot
            && input.view.local_has_in_place_mutation_surface[slot]
        {
            let before = input.stats.observations.before_rekey(input.state);
            let retirement = input.state.retire_slot_value(slot, input.current_order);
            input
                .stats
                .observations
                .after_retirement(before, input.state, retirement);
            write.remap(input.state, retirement);
            for pending in writes.as_mut_slice() {
                pending.remap(input.state, retirement);
            }
        }
        let receipt = input
            .state
            .publish_store(&write.store, input.current_order, None);
        input.stats.observations.publish(receipt);
        input
            .state
            .mark_slot_derivation(write.definite_slot.unwrap_or(write.actor), write.derivation);
    }
}

fn call_result_index(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    local: LocalId,
) -> Result<usize, BorrowCheckError> {
    input.view.index_of(local).ok_or_else(|| {
        input.context.diagnostics.internal_error(format!(
            "Call result local '{}' is not in the active function layout",
            input.context.diagnostics.local_name(local)
        ))
    })
}

fn call_result_local_state(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    alias_arguments: Option<&[usize]>,
    arg_value_roots: &[RootSet],
    derived_roots: RootSet,
) -> Result<LocalState, BorrowCheckError> {
    let local_count = input.view.local_count();
    let mut alias_roots = RootSet::empty(local_count);
    // Coarse value roots are a may-set. The independently computed derivation stays a must-set.
    if let Some(indices) = alias_arguments {
        for &index in indices {
            let Some(roots) = arg_value_roots.get(index) else {
                return Err(input.context.diagnostics.internal_error(format!(
                    "Borrow checker found out-of-range return-alias index {index} at call site"
                )));
            };
            alias_roots.union_with(roots);
        }
    } else {
        for roots in arg_value_roots {
            alias_roots.union_with(roots);
        }
    }
    let mut state = LocalState::slot_from_value_roots(alias_roots);
    state.derived_from_roots = derived_roots;
    Ok(state)
}

fn map_argument_effect(op: HirMapOp, arg_index: usize) -> ArgEffect {
    if matches!(op, HirMapOp::Set) && matches!(arg_index, 0 | 1) {
        ArgEffect::MayConsumeShared
    } else {
        ArgEffect::SharedBorrow
    }
}
