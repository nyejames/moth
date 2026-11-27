//! Statement-level borrow transfer rules.
//!
//! WHAT: applies borrow effects for HIR statements and records statement/value facts.
//! WHY: keeping statement transfer separate from traversal helpers makes the block transfer
//! entrypoint easier to inspect without changing the borrow-analysis model.

#[path = "call_results.rs"]
mod call_results;

use super::conflicts::{Overlap, check_call_access, overlap, record_call_observations};
use super::layer::needs_layer_store_capture;
use super::*;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocationRetirement, CapturedValue, Coverage, EntryFamilies, FieldKey, ReceiverDependency,
    ResolvedPlace, RootSet, SmallSet, TypedStore, ValueAlt, ValueKind as PayloadKind, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::CallReturns;
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{
    Demand, selected_members,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{InvokeLane, InvokeLayerOp};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirMapOp;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::hir::terminators::HirFallibleOperation;
use crate::compiler_frontend::source::SourceSpan;
use call_results::{CallActuals, CallLane, bind_result, call_result_ops, capture_graph_values};

pub(crate) fn transfer_statement(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &mut BorrowState,
    block_id: BlockId,
    statement: &HirStatement,
    stats: &mut BlockTransferStats,
    value_fact_buffer: &mut ValueFactBuffer,
) -> Result<(), BorrowCheckError> {
    // WHAT: transfer one statement at the block frontier.
    // WHY: the fixed-point driver merges only block states, so statement effects must be exact.
    let mut tracker = StatementAccessTracker::new(layout.local_count());
    let conflicts_before = stats.conflicts_checked;
    let statement_order = layout.statement_order_or_unknown(statement.id);

    match &statement.kind {
        HirStatementKind::Write { target, value } => {
            let location = context.diagnostics.statement_error_span(statement);
            let span = context.diagnostics.statement_error_span(statement);
            let (target_place, is_definition) = match target {
                HirWriteTarget::DefineLocal(local) => (HirPlace::local(*local), true),
                HirWriteTarget::AssignPlace(place) => (*place, false),
            };
            let local_index = layout.index_of(target_place.root);
            let pristine_definition = if is_definition {
                first_definition_index(layout, state, block_id, statement_order, || local_index)
            } else {
                None
            };
            {
                let mut read_env = SharedReadEnv {
                    context,
                    layout,
                    state,
                    block_id,
                    tracker: &mut tracker,
                    location,
                    current_order: statement_order,
                    stats,
                    value_fact_buffer,
                };
                record_shared_reads_in_place_indices(
                    &mut read_env,
                    &target_place,
                    location,
                    &mut RootSet::empty(layout.local_count()),
                )?;
            }
            {
                let mut read_env = SharedReadEnv {
                    context,
                    layout,
                    state,
                    block_id,
                    tracker: &mut tracker,
                    location,
                    current_order: statement_order,
                    stats,
                    value_fact_buffer,
                };
                record_assignment_value_reads(
                    &mut read_env,
                    &target_place,
                    is_definition,
                    *value,
                    location,
                )?;
            }

            transfer_assign_target(
                &mut AssignTransferContext {
                    context,
                    layout,
                    state,
                    block_id,
                    current_order: statement_order,
                    tracker: &mut tracker,
                    value_fact_buffer,
                    location,
                    span,
                    stats,
                },
                &target_place,
                *value,
                is_definition,
                local_index,
                pristine_definition,
            )?;
        }

        HirStatementKind::Call {
            target,
            args,
            results,
        } => {
            let location = context.diagnostics.statement_error_span(statement);
            let semantics = resolve_call_semantics(context, target, args.len())?;
            let call_args = context
                .expressions
                .values(*args)
                .iter()
                .zip(semantics.arg_effects)
                .map(|(argument, effect)| CallArgumentTransfer {
                    argument: *argument,
                    effect,
                })
                .collect::<Vec<_>>();
            let result_transfer = CallResultTransfer::for_call_definition(
                layout,
                state,
                block_id,
                statement_order,
                results,
            );
            transfer_call_arguments_and_result(
                &mut CallTransferContext {
                    context,
                    layout,
                    state,
                    block_id,
                    current_order: statement_order,
                    tracker: &mut tracker,
                    location,
                    stats,
                    value_fact_buffer,
                },
                &call_args,
                results.iter().copied(),
                semantics.returns,
                result_transfer,
                CallStorage::for_target(target),
            )?;
        }

        HirStatementKind::MapOp {
            op,
            receiver,
            args,
            result,
        } => {
            let location = context.diagnostics.statement_error_span(statement);
            let mut call_args = Vec::with_capacity(args.len() + 1);
            call_args.push(CallArgumentTransfer {
                argument: *receiver,
                effect: if op.requires_mutable_receiver() {
                    ArgEffect::MutableBorrow
                } else {
                    ArgEffect::SharedBorrow
                },
            });
            for (arg_index, arg) in context.expressions.values(*args).iter().enumerate() {
                call_args.push(CallArgumentTransfer {
                    argument: *arg,
                    effect: map_argument_effect(*op, arg_index),
                });
            }
            transfer_call_arguments_and_result(
                &mut CallTransferContext {
                    context,
                    layout,
                    state,
                    block_id,
                    current_order: statement_order,
                    tracker: &mut tracker,
                    location,
                    stats,
                    value_fact_buffer,
                },
                &call_args,
                result.iter().copied(),
                CallReturns::Fresh,
                CallResultTransfer::Immediate,
                CallStorage::for_map_op(*op),
            )?;
        }

        HirStatementKind::FormatFloat { source, result, .. }
        | HirStatementKind::ValidateFloat { source, result, .. }
        | HirStatementKind::FastStoreConversion { source, result, .. } => {
            let location = context.diagnostics.statement_error_span(statement);
            transfer_call_arguments_and_result(
                &mut CallTransferContext {
                    context,
                    layout,
                    state,
                    block_id,
                    current_order: statement_order,
                    tracker: &mut tracker,
                    location,
                    stats,
                    value_fact_buffer,
                },
                &[CallArgumentTransfer {
                    argument: *source,
                    effect: ArgEffect::SharedBorrow,
                }],
                std::iter::once(*result),
                CallReturns::Fresh,
                CallResultTransfer::Immediate,
                CallStorage::Plain,
            )?;
        }

        HirStatementKind::NumericOp {
            operands, result, ..
        } => {
            let location = context.diagnostics.statement_error_span(statement);
            let arguments = numeric_op_arguments(operands);
            let call_args = arguments
                .into_iter()
                .map(|argument| CallArgumentTransfer {
                    effect: ArgEffect::SharedBorrow,
                    argument,
                })
                .collect::<Vec<_>>();

            transfer_call_arguments_and_result(
                &mut CallTransferContext {
                    context,
                    layout,
                    state,
                    block_id,
                    current_order: statement_order,
                    tracker: &mut tracker,
                    location,
                    stats,
                    value_fact_buffer,
                },
                &call_args,
                std::iter::once(*result),
                CallReturns::Fresh,
                CallResultTransfer::Immediate,
                CallStorage::Plain,
            )?;
        }

        HirStatementKind::LoopStepFailure { result, .. } => {
            let location = context.diagnostics.statement_error_span(statement);
            transfer_call_arguments_and_result(
                &mut CallTransferContext {
                    context,
                    layout,
                    state,
                    block_id,
                    current_order: statement_order,
                    tracker: &mut tracker,
                    location,
                    stats,
                    value_fact_buffer,
                },
                &[],
                std::iter::once(*result),
                CallReturns::Fresh,
                CallResultTransfer::Immediate,
                CallStorage::Plain,
            )?;
        }

        HirStatementKind::RangeCandidate {
            current,
            step,
            end,
            candidate_result,
            in_range_result,
            ..
        } => {
            let location = context.diagnostics.statement_error_span(statement);
            {
                let mut read_env = SharedReadEnv {
                    context,
                    layout,
                    state,
                    block_id,
                    tracker: &mut tracker,
                    location,
                    current_order: statement_order,
                    stats,
                    value_fact_buffer,
                };
                for operand in [current, step, end] {
                    record_shared_reads_in_expression(
                        &mut read_env,
                        *operand,
                        location,
                        &mut RootSet::empty(layout.local_count()),
                    )?;
                }
            }

            let mut result_transfer = CallTransferContext {
                context,
                layout,
                state,
                block_id,
                current_order: statement_order,
                tracker: &mut tracker,
                location,
                stats,
                value_fact_buffer,
            };
            transfer_call_arguments_and_result(
                &mut result_transfer,
                &[],
                [*candidate_result, *in_range_result].into_iter(),
                CallReturns::Fresh,
                CallResultTransfer::Immediate,
                CallStorage::Plain,
            )?;
        }

        HirStatementKind::Expr(expression) => {
            let location = context.diagnostics.statement_error_span(statement);
            let mut read_env = SharedReadEnv {
                context,
                layout,
                state,
                block_id,
                tracker: &mut tracker,
                location,
                current_order: statement_order,
                stats,
                value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                *expression,
                location,
                &mut RootSet::empty(layout.local_count()),
            )?;
        }

        HirStatementKind::PushRuntimeFragment { value, .. } => {
            let location = context.diagnostics.statement_error_span(statement);
            let mut read_env = SharedReadEnv {
                context,
                layout,
                state,
                block_id,
                tracker: &mut tracker,
                location,
                current_order: statement_order,
                stats,
                value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                *value,
                location,
                &mut RootSet::empty(layout.local_count()),
            )?;
        }
    }

    // Aggregate literal children receive optional transfer analysis.
    let mut aggregate_context = AggregateTransferContext {
        diagnostics: &context.diagnostics,
        expressions: context.expressions,
        value_fact_buffer,
    };

    match &statement.kind {
        HirStatementKind::Write { value, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                *value,
                block_id,
                statement_order,
                context.diagnostics.statement_error_span(statement),
                &mut aggregate_context,
            )?;
        }
        HirStatementKind::Expr(expression) => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                *expression,
                block_id,
                statement_order,
                context.diagnostics.value_error_span(
                    *expression,
                    context.diagnostics.statement_error_span(statement),
                ),
                &mut aggregate_context,
            )?;
        }
        HirStatementKind::PushRuntimeFragment { value, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                *value,
                block_id,
                statement_order,
                context
                    .diagnostics
                    .value_error_span(*value, context.diagnostics.statement_error_span(statement)),
                &mut aggregate_context,
            )?;
        }
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. }
        | HirStatementKind::FastStoreConversion { source, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                *source,
                block_id,
                statement_order,
                context
                    .diagnostics
                    .value_error_span(*source, context.diagnostics.statement_error_span(statement)),
                &mut aggregate_context,
            )?;
        }
        _ => {}
    }

    let statement_fact = StatementBorrowFact {
        shared_roots: roots_to_local_ids(layout, &tracker.shared_roots),
        mutable_roots: roots_to_local_ids(layout, &tracker.mutable_roots),
        written_roots: roots_to_local_ids(layout, &tracker.written_roots),
        conflicts_checked: stats.conflicts_checked - conflicts_before,
    };
    stats.statement_facts.push((statement.id, statement_fact));
    Ok(())
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
        HirFallibleOperation::Call { target, args } => {
            let semantics = resolve_call_semantics(context, target, args.len())?;
            let arguments = context
                .expressions
                .values(*args)
                .iter()
                .zip(semantics.arg_effects)
                .map(|(argument, effect)| CallArgumentTransfer {
                    argument: *argument,
                    effect,
                })
                .collect::<Vec<_>>();
            transfer_call_arguments_and_result(
                input,
                &arguments,
                results,
                semantics.returns,
                CallResultTransfer::Edge(*error_result),
                CallStorage::for_target(target),
            )
        }
        HirFallibleOperation::MapOp { op, receiver, args } => {
            let mut arguments = Vec::with_capacity(args.len() + 1);
            arguments.push(CallArgumentTransfer {
                argument: *receiver,
                effect: if op.requires_mutable_receiver() {
                    ArgEffect::MutableBorrow
                } else {
                    ArgEffect::SharedBorrow
                },
            });
            for (index, argument) in context.expressions.values(*args).iter().enumerate() {
                arguments.push(CallArgumentTransfer {
                    argument: *argument,
                    effect: map_argument_effect(*op, index),
                });
            }
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
            // Builtin operations have at most two operands; keep their transfer inputs inline.
            let mut operands = operation.operands(context.expressions);
            let Some(first) = operands.next() else {
                return transfer_call_arguments_and_result(
                    input,
                    &[],
                    results,
                    CallReturns::Fresh,
                    CallResultTransfer::Edge(*error_result),
                    CallStorage::Plain,
                );
            };
            let first = CallArgumentTransfer {
                argument: first,
                effect: ArgEffect::SharedBorrow,
            };
            if let Some(second) = operands.next() {
                transfer_call_arguments_and_result(
                    input,
                    &[
                        first,
                        CallArgumentTransfer {
                            argument: second,
                            effect: ArgEffect::SharedBorrow,
                        },
                    ],
                    results,
                    CallReturns::Fresh,
                    CallResultTransfer::Edge(*error_result),
                    CallStorage::Plain,
                )
            } else {
                transfer_call_arguments_and_result(
                    input,
                    &[first],
                    results,
                    CallReturns::Fresh,
                    CallResultTransfer::Edge(*error_result),
                    CallStorage::Plain,
                )
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
    let semantics = resolve_call_semantics(input.context, target, args.len())?;
    let arguments = args
        .iter()
        .copied()
        .zip(semantics.arg_effects)
        .map(|(argument, effect)| CallArgumentTransfer { argument, effect })
        .collect::<Vec<_>>();
    transfer_call_arguments_and_result(
        input,
        &arguments,
        std::iter::empty(),
        CallReturns::Fresh,
        CallResultTransfer::Diverges,
        CallStorage::for_target(target),
    )
}

pub(super) struct CallArgumentTransfer {
    pub(super) argument: HirValueId,
    effect: ArgEffect,
}

impl CallArgumentTransfer {
    pub(super) fn may_write(&self) -> bool {
        matches!(
            self.effect,
            ArgEffect::MutableBorrow | ArgEffect::MayConsumeMutable
        )
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

pub(super) struct CallTransferContext<'a, 'module, 'state, 'tracker, 'stats, 'facts> {
    pub(super) context: &'a BorrowTransferContext<'module>,
    pub(super) layout: &'a FunctionLayout,
    pub(super) state: &'state mut BorrowState,
    pub(super) block_id: BlockId,
    pub(super) current_order: i32,
    pub(super) tracker: &'tracker mut StatementAccessTracker,
    pub(super) location: Option<SourceSpan>,
    pub(super) stats: &'stats mut BlockTransferStats,
    pub(super) value_fact_buffer: &'facts mut ValueFactBuffer,
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
        layout: &FunctionLayout,
        state: &BorrowState,
        block: BlockId,
        order: i32,
        results: &[HirLocalDestination],
    ) -> Self {
        let [HirLocalDestination::Define(local)] = results else {
            return Self::Immediate;
        };
        first_definition_index(layout, state, block, order, || layout.index_of(*local))
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
    if args.iter().any(|arg| {
        matches!(
            arg.effect,
            ArgEffect::MutableBorrow | ArgEffect::MayConsumeMutable
        )
    }) {
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
    let mut plans = Vec::with_capacity(args.len());

    for (arg_index, arg) in args.iter().enumerate() {
        let argument = arg.argument;
        let argument_location = input
            .context
            .diagnostics
            .value_error_span(argument, input.location);

        plans.push(plan_call_actual(
            input,
            &mut actuals,
            arg_index,
            arg,
            argument_location,
            storage,
        )?);
    }
    // Index and copy-source reads evaluated the operands. They are not call loans.
    input.tracker.clear_reservations();

    // Result provenance comes from the argument values, not from every root their evaluation
    // reads. `copy place` reads the source place but produces an independent graph, so a
    // retargeted alias into the copied value must not keep the source root alive.
    //
    // WHAT: only a retained result that may alias an argument reads these roots.
    // WHY: `Fresh` results and calls without a result never inspect them, so building the sets
    //      would walk every argument expression for no consumer.
    let mut arg_value_roots = Vec::new();
    if retains_success || retains_error {
        arg_value_roots.reserve(args.len());
        actuals.capture_derivations(input);
        for arg in args {
            let argument_location = input
                .context
                .diagnostics
                .value_error_span(arg.argument, input.location);
            arg_value_roots.push(call_argument_value_roots(
                input.context,
                input.layout,
                input.state,
                arg.argument,
                argument_location,
            )?);
        }
    }

    for (arg_index, arg) in args.iter().enumerate() {
        let Some(mut plan) = plans[arg_index].take() else {
            continue;
        };
        let argument_location = input
            .context
            .diagnostics
            .value_error_span(arg.argument, input.location);
        let retains_argument = (retains_success
            && success_lane.may_retain(arg_index, &arg_value_roots))
            || (retains_error && error_lane.may_retain(arg_index, &arg_value_roots))
            || success_lane.may_store(arg_index)
            || (error.is_some() && error_lane.may_store(arg_index));
        let optional = matches!(
            arg.effect,
            ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
        );
        let conflicts_with_operand =
            optional && !retains_argument && arg_index + 1 < plans.len() && {
                let usage = plan.access.usage;
                if usage.kind() == AccessKind::Shared {
                    plan.access.usage = PlaceUse::Mutate { deep: true };
                }
                let conflict = plans[arg_index + 1..]
                    .iter()
                    .flatten()
                    .any(|other| overlap(input.state, &plan.access, &other.access) != Overlap::No);
                plan.access.usage = usage;
                conflict
            };
        let effect = if retains_argument || conflicts_with_operand {
            fallback_call_argument_effect(arg.effect)
        } else {
            arg.effect
        };
        if matches!(
            arg.effect,
            ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
        ) && !matches!(
            effect,
            ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
        ) {
            // The retained result only keeps the argument borrowed; it does not make the
            // argument's provenance roots call-argument writes. Merging overlap or value roots
            // here would publish an immutable parameter as `Writes` during summary finalization.
            input.value_fact_buffer.record_optional_transfer(
                arg.argument,
                OptionalTransferStatus::Borrow,
                &RootSet::empty(input.layout.local_count()),
            );
        }
        let resolution = actuals.entry_place(input, arg_index)?;
        transfer_call_argument_access(
            input,
            arg.argument,
            effect,
            argument_location,
            plan,
            resolution.as_deref(),
        )?;
    }
    if input.stats.observations.enabled && input.state.has_value_alternatives() {
        input.stats.invoke.success.checkpoints = record_call_observations(
            &mut input.stats.observations,
            input.state,
            input.tracker,
            input.current_order,
            input.location,
            storage.reaches_children(),
            error.is_some() && !storage.reaches_children(),
        );
    }
    // Call loans end before any result destination is written.
    input.tracker.clear_reservations();
    if storage.reaches_children()
        && args.iter().any(|arg| {
            matches!(
                arg.effect,
                ArgEffect::MutableBorrow | ArgEffect::MayConsumeMutable
            )
        })
    {
        input.state.invalidate_detachments();
    }

    if !matches!(
        result_transfer,
        CallResultTransfer::Edge(_) | CallResultTransfer::Diverges
    ) && success_lane.unreachable()
    {
        input.stats.diverges = true;
    }
    if !matches!(result_transfer, CallResultTransfer::Diverges) {
        // Capture both lanes against entry storage. Neither lane may see the other's stores
        // or a new binding for either outcome.
        let mut member_reads = Demand::default();
        let mut success_ops = match storage {
            CallStorage::User | CallStorage::Native | CallStorage::Plain | CallStorage::Builtin => {
                call_result_ops(
                    input,
                    &mut actuals,
                    results.clone(),
                    success_lane,
                    InvokeLane::Success,
                    error.is_none(),
                )?
            }
            CallStorage::Store(positions) => {
                capture_builtin_store(input, &mut actuals, args, positions)?
            }
            CallStorage::ClearMap => call_receiver_allocs(input, &mut actuals)?
                .filter(|receiver| !receiver.ids.is_empty())
                .map_or_else(Vec::new, |receiver| {
                    vec![InvokeLayerOp::Store(TypedStore::EraseFamily {
                        coverage: receiver.holder_coverage(),
                        receivers: receiver.ids,
                        keys: EntryFamilies::MapEntries,
                    })]
                }),
            CallStorage::Remove { .. } | CallStorage::Get => capture_builtin_element_result(
                input,
                &mut actuals,
                results.clone().next(),
                storage,
                error.is_some().then_some(&mut member_reads),
            )?,
        };
        let error_ops = if error.is_some() {
            call_result_ops(
                input,
                &mut actuals,
                error.into_iter(),
                error_lane,
                InvokeLane::Error,
                false,
            )?
        } else {
            Vec::new()
        };
        let mut root_sources = Vec::new();
        let mut writes = Vec::new();
        if matches!(result_transfer, CallResultTransfer::Immediate) && !success_lane.unreachable() {
            // Capture and check every update before installing any result definition.
            for (slot, result) in results.clone().enumerate() {
                let HirLocalDestination::Update(local) = result else {
                    continue;
                };
                let value = call_result_local_state(
                    input,
                    success_lane.root_sources(slot, &mut root_sources),
                    &arg_value_roots,
                    success_lane.derive_result(slot, &actuals, input.layout.local_count()),
                )?;
                writes.push(capture_call_result_write(
                    input,
                    local,
                    value,
                    &mut success_ops,
                )?);
            }
        }
        if !success_lane.unreachable() {
            for (slot, result) in results.enumerate() {
                if error.is_none() && matches!(result, HirLocalDestination::Update(_)) {
                    continue;
                }
                let derived =
                    success_lane.derive_result(slot, &actuals, input.layout.local_count());
                prepare_call_result_definition(
                    input,
                    result.local(),
                    success_lane.root_sources(slot, &mut root_sources),
                    &arg_value_roots,
                    derived,
                    result_transfer,
                    &mut success_ops,
                    &mut writes,
                )?;
            }
        }
        if let Some(error) = error {
            if !error_lane.unreachable() {
                let state = call_result_local_state(
                    input,
                    error_lane.root_sources(0, &mut root_sources),
                    &arg_value_roots,
                    error_lane.derive_result(0, &actuals, input.layout.local_count()),
                )?;
                input
                    .stats
                    .invoke
                    .error
                    .results
                    .push((error.local(), state));
            }
            input.stats.invoke.success.unreachable = success_lane.unreachable();
            input.stats.invoke.error.unreachable = error_lane.unreachable();
            input.stats.invoke.success.operations = success_ops;
            input.stats.invoke.success.member_reads = member_reads;
            input.stats.invoke.error.operations = error_ops;
        } else {
            apply_immediate_call_operations(input, success_ops, writes);
            input.state.prune_retired_wrappers();
        }
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
        .load_place_content(place, &input.layout.local_mutable, false);
    Ok(place.content.clone())
}

/// A fallible builtin stores nothing when it fails.
fn capture_builtin_store(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    args: &[CallArgumentTransfer],
    positions: &[(usize, FieldKey)],
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let Some(holders) = call_receiver_allocs(input, actuals)? else {
        return Ok(Vec::new());
    };
    let mut ops = Vec::new();
    for &(index, key) in positions {
        let Some(argument) = args.get(index) else {
            continue;
        };
        if !needs_layer_store_capture(
            input.context,
            input.state,
            &holders.ids,
            key,
            argument.argument,
        )? {
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

/// Capture the element before removal and before a result can rebind the receiver.
fn capture_builtin_element_result(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    result: Option<HirLocalDestination>,
    storage: CallStorage,
    deferred_reads: Option<&mut Demand>,
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let heap_result = result
        .and_then(|result| input.layout.index_of(result.local()))
        .is_some_and(|index| input.layout.local_has_in_place_mutation_surface[index]);
    if !heap_result && input.state.layer_is_empty() && !matches!(storage, CallStorage::Get) {
        return Ok(Vec::new());
    }
    let Some(mut elements) = call_receiver_allocs(input, actuals)? else {
        return Ok(Vec::new());
    };
    if input.stats.observations.enabled {
        let reads = selected_members(input.state, &elements);
        if let Some(deferred) = deferred_reads {
            deferred.extend(reads);
        } else {
            input.stats.observations.tail.read(reads);
        }
    }
    let holders = if matches!(storage, CallStorage::Remove { .. })
        && elements.holder_coverage() == Coverage::Complete
    {
        elements.ids.clone()
    } else {
        Vec::new()
    };
    let mut ops = Vec::new();
    let lookup = matches!(storage, CallStorage::Get);
    if let Some(result) =
        result.filter(|_| lookup || heap_result || input.state.has_value_alternatives())
    {
        let dependency = lookup.then(|| ReceiverDependency {
            entries: SmallSet::from_iter(elements.ids.iter().copied()),
            coverage: elements.holder_coverage(),
        });
        let mut value = if heap_result {
            input
                .state
                .step_resolved(&mut elements, FieldKey::Elem, &input.layout.local_mutable);
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
        bind_result(&mut ops, result, value);
    }
    if let CallStorage::Remove { map } = storage {
        ops.push(InvokeLayerOp::Remove {
            holders,
            map,
            result: result.map(HirLocalDestination::local),
        });
    }
    Ok(ops)
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
    let mut roots = RootSet::empty(input.layout.local_count());
    let mut env = SharedReadEnv {
        context: input.context,
        layout: input.layout,
        state: input.state,
        block_id: input.block_id,
        tracker: input.tracker,
        location,
        current_order: input.current_order,
        stats: input.stats,
        value_fact_buffer: input.value_fact_buffer,
    };
    let mutable = matches!(
        arg.effect,
        ArgEffect::MutableBorrow | ArgEffect::MayConsumeMutable
    );
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
        input.layout,
        input.state,
        place,
        location,
        expression.span,
        &input.context.diagnostics,
    )?;
    let heap = if place.projections.is_empty() {
        input
            .layout
            .index_of(place.root)
            .is_some_and(|local| input.layout.local_has_in_place_mutation_surface[local])
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
        RootSet::empty(input.layout.local_count())
    } else {
        value_roots.clone()
    };
    let access = if let Some(resolution) = resolution {
        if heap {
            input
                .state
                .load_place_content(resolution, &input.layout.local_mutable, false);
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
            input.value_fact_buffer.record(
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
    layout: &FunctionLayout,
    state: &BorrowState,
    expression_id: HirValueId,
    location: Option<SourceSpan>,
) -> Result<RootSet, BorrowCheckError> {
    let expression = context.expressions.expression(expression_id);

    match &expression.kind {
        HirExpressionKind::Copy(_) => Ok(RootSet::empty(layout.local_count())),

        HirExpressionKind::Load(place) => match expression.value_kind {
            ValueKind::Place | ValueKind::RValue => roots_for_place(
                layout,
                state,
                place,
                location,
                expression.span,
                &context.diagnostics,
            ),
            ValueKind::Const => Ok(RootSet::empty(layout.local_count())),
        },

        _ => {
            let mut roots = RootSet::empty(layout.local_count());
            collect_expression_roots(
                context,
                layout,
                state,
                expression_id,
                &mut roots,
                location,
                expression.span,
            )?;
            Ok(roots)
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
    AccessCheckContext {
        context: input.context,
        layout: input.layout,
        state: input.state,
        block_id: input.block_id,
        tracker: input.tracker,
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
    let can_move = optional
        && !plan.roots.is_empty()
        && matches!(
            classify_move_decision(
                input.layout,
                input.block_id,
                &plan.roots,
                input.current_order
            ),
            MoveDecision::Move,
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
        input.value_fact_buffer.record_optional_transfer(
            argument,
            if transferred {
                OptionalTransferStatus::Transfer
            } else {
                OptionalTransferStatus::Borrow
            },
            &plan.roots,
        );
    }
    input.value_fact_buffer.record(
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
    let retirement = if !input.layout.local_has_in_place_mutation_surface[local_index]
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
    require_initialized_update_target(
        input.layout,
        input.state,
        actor,
        &input.context.diagnostics,
        input.location,
    )?;
    let resolved = input
        .state
        .resolve_place_path(actor, &input.layout.local_mutable, []);
    let mut check = AccessCheckContext {
        context: input.context,
        layout: input.layout,
        state: input.state,
        block_id: input.block_id,
        tracker: input.tracker,
        location: input.location,
        span: input.location,
        stats: input.stats,
        actor_index_hint: Some(actor),
        current_order: input.current_order,
    };
    check_local_update_access(&mut check, &resolved)?;
    let addresses = resolved.address;

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

fn apply_immediate_call_operations(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    mut ops: Vec<InvokeLayerOp>,
    writes: Vec<CallResultWrite>,
) {
    // Removal must observe the installed result to record its detachment.
    let mut post_result =
        if !writes.is_empty() && matches!(ops.last(), Some(InvokeLayerOp::Remove { .. })) {
            ops.pop()
        } else {
            None
        };
    for op in ops {
        input
            .stats
            .observations
            .tail
            .replace(op.apply_owned(input.layout, input.state));
    }
    let mut writes = writes.into_iter();
    while let Some(mut write) = writes.next() {
        if let Some(slot) = write.definite_slot
            && input.layout.local_has_in_place_mutation_surface[slot]
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
            if let Some(op) = &mut post_result {
                op.remap(input.state, retirement);
            }
        }
        let receipt = input
            .state
            .publish_store(&write.store, input.current_order, None);
        input.stats.observations.tail.replace(receipt);
        input
            .state
            .mark_slot_derivation(write.definite_slot.unwrap_or(write.actor), write.derivation);
    }
    if let Some(op) = post_result {
        input
            .stats
            .observations
            .tail
            .replace(op.apply_owned(input.layout, input.state));
    }
}

fn call_result_index(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    local: LocalId,
) -> Result<usize, BorrowCheckError> {
    input.layout.index_of(local).ok_or_else(|| {
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
    let local_count = input.layout.local_count();
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

fn numeric_op_arguments(operands: &HirNumericOperands) -> Vec<HirValueId> {
    match operands {
        HirNumericOperands::Unary { operand } => vec![*operand],
        HirNumericOperands::Binary { left, right } => {
            vec![*left, *right]
        }
    }
}

fn map_argument_effect(op: HirMapOp, arg_index: usize) -> ArgEffect {
    if matches!(op, HirMapOp::Set) && matches!(arg_index, 0 | 1) {
        ArgEffect::MayConsumeShared
    } else {
        ArgEffect::SharedBorrow
    }
}
