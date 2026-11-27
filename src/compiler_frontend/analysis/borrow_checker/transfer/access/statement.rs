//! Statement-level borrow transfer rules.
//!
//! WHAT: applies borrow effects for HIR statements and records statement/value facts.
//! WHY: keeping statement transfer separate from traversal helpers makes the block transfer
//! entrypoint easier to inspect without changing the borrow-analysis model.

#[path = "call_results.rs"]
mod call_results;

use super::conflicts::{check_layer_conflict, check_mutable_access_with_targets};
use super::layer::{
    LayerValueContext, layer_mutation_targets, layer_store_values, resolve_place_allocs,
    transfer_layer_alias_write,
};
use super::*;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, AllocationRetirement, FieldKey, ResolvedPlace, RootSet, StoredValue,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::CallReturns;
use crate::compiler_frontend::analysis::borrow_checker::transfer::{InvokeLane, InvokeLayerOp};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::compiler_messages::BorrowAccessKind;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirMapOp;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::hir::terminators::HirFallibleOperation;
use crate::compiler_frontend::source::SourceSpan;
use call_results::{CallActuals, CallLane, call_result_ops};

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
                first_heap_definition_index(layout, state, block_id, statement_order, || {
                    local_index
                })
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
                record_shared_reads_in_expression(
                    &mut read_env,
                    *value,
                    location,
                    &mut RootSet::empty(layout.local_count()),
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
                map_result_alias(*op),
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
                CallStorage::Builtin,
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
                CallStorage::Builtin,
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
                CallStorage::Builtin,
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
            transfer_call_result_alias(
                &mut result_transfer,
                Some(*candidate_result),
                Some(&[]),
                &[],
                RootSet::empty(layout.local_count()),
                CallResultTransfer::Immediate,
                &[],
            )?;
            transfer_call_result_alias(
                &mut result_transfer,
                Some(*in_range_result),
                Some(&[]),
                &[],
                RootSet::empty(layout.local_count()),
                CallResultTransfer::Immediate,
                &[],
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
                map_result_alias(*op),
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
                    CallStorage::Builtin,
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
                    CallStorage::Builtin,
                )
            } else {
                transfer_call_arguments_and_result(
                    input,
                    &[first],
                    results,
                    CallReturns::Fresh,
                    CallResultTransfer::Edge(*error_result),
                    CallStorage::Builtin,
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

/// How a call moves values between its arguments' allocations.
#[derive(Clone, Copy)]
enum CallStorage {
    /// A user function may write through any field a mutable argument reaches.
    User,
    Native,
    /// A builtin edits only its receiver's own contents.
    Builtin,
    /// A builtin stores these argument positions into its receiver, argument 0.
    Store(&'static [(usize, FieldKey)]),
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
        first_heap_definition_index(layout, state, block, order, || layout.index_of(*local))
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
    let success_lane = CallLane::new(returns, InvokeLane::Success);
    let error_returns = if matches!(storage, CallStorage::User | CallStorage::Native) {
        returns
    } else {
        CallReturns::Fresh
    };
    let error_lane = CallLane::new(error_returns, InvokeLane::Error);
    let retains_success = results.clone().next().is_some() && success_lane.retains_result();
    let retains_error = error.is_some() && error_lane.retains_result();
    let mut arg_roots = vec![RootSet::empty(input.layout.local_count()); args.len()];

    for (arg_index, arg) in args.iter().enumerate() {
        let argument = arg.argument;
        let argument_location = input
            .context
            .diagnostics
            .value_error_span(argument, input.location);

        record_call_argument_reads(
            input,
            argument,
            arg.effect,
            &argument_location,
            &mut arg_roots[arg_index],
        )?;
    }

    // Result provenance comes from the argument values, not from every root their evaluation
    // reads. `copy place` reads the source place but produces an independent graph, so a
    // retargeted alias into the copied value must not keep the source root alive.
    //
    // WHAT: only a retained result that may alias an argument reads these roots.
    // WHY: `Fresh` results and calls without a result never inspect them, so building the sets
    //      would walk every argument expression for no consumer.
    let mut arg_value_roots = Vec::new();
    let mut arg_derived_roots = Vec::new();
    if retains_success || retains_error {
        arg_value_roots.reserve(args.len());
        arg_derived_roots.reserve(args.len());
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
            // The derivation ledger travels with the value, so a result derived through an alias
            // keeps the same exemption from that alias's exclusivity.
            arg_derived_roots.push(alias_derivation_roots_from_expression(
                input.layout,
                input.state,
                input.context.expressions,
                arg.argument,
            ));
        }
    }

    for (arg_index, arg) in args.iter().enumerate() {
        let argument_location = input
            .context
            .diagnostics
            .value_error_span(arg.argument, input.location);
        let effect = effective_call_argument_effect(
            arg.effect,
            arg_index,
            &arg_roots,
            (retains_success && success_lane.may_retain(arg_index, &arg_value_roots))
                || (retains_error && error_lane.may_retain(arg_index, &arg_value_roots))
                || success_lane.may_store(arg_index)
                || (error.is_some() && error_lane.may_store(arg_index)),
        );
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
        if matches!(arg.effect, ArgEffect::MayConsumeShared)
            && matches!(effect, ArgEffect::SharedBorrow)
        {
            let mut read_env = SharedReadEnv {
                context: input.context,
                layout: input.layout,
                state: input.state,
                block_id: input.block_id,
                tracker: input.tracker,
                location: argument_location,
                current_order: input.current_order,
                stats: input.stats,
                value_fact_buffer: input.value_fact_buffer,
            };
            record_shared_reads_in_expression(
                &mut read_env,
                arg.argument,
                argument_location,
                &mut RootSet::empty(input.layout.local_count()),
            )?;
        }
        transfer_call_argument_access(
            input,
            arg.argument,
            effect,
            argument_location,
            storage.reaches_children(),
        )?;
    }
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
        let mut actuals = CallActuals::new(args);
        let mut success_ops = match storage {
            CallStorage::User | CallStorage::Native | CallStorage::Builtin => call_result_ops(
                input,
                &mut actuals,
                results.clone(),
                success_lane,
                InvokeLane::Success,
                error.is_none(),
            )?,
            CallStorage::Store(positions) => capture_builtin_store(input, args, positions)?,
            CallStorage::Remove { .. } | CallStorage::Get => {
                capture_builtin_element_result(input, args, results.clone().next(), storage)
            }
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
        if !success_lane.unreachable() {
            for (slot, result) in results.enumerate() {
                let derived = success_lane.derive_result(
                    slot,
                    &arg_derived_roots,
                    input.layout.local_count(),
                );
                if let Some(retirement) = transfer_call_result_alias(
                    input,
                    Some(result),
                    success_lane.root_sources(slot, &mut root_sources),
                    &arg_value_roots,
                    derived,
                    result_transfer,
                    &success_ops,
                )? {
                    for op in &mut success_ops {
                        op.remap(input.state, retirement);
                    }
                }
            }
        }
        if let Some(error) = error {
            if !error_lane.unreachable() {
                let state = call_result_local_state(
                    input,
                    error_lane.root_sources(0, &mut root_sources),
                    &arg_value_roots,
                    error_lane.derive_result(0, &arg_derived_roots, input.layout.local_count()),
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
            input.stats.invoke.error.operations = error_ops;
        } else {
            for op in success_ops {
                op.apply(input.layout, input.state, None);
            }
        }
    }
    // A mutable argument and a shared argument naming the same binding cell overlap inside one
    // call. Check after ordinary argument and result transfer so existing alias exclusivity keeps
    // its diagnostic, then reject the same-cell pair the tracker cannot see.
    let mut shared_arguments = Vec::new();
    let mut shared_roots = RootSet::empty(input.layout.local_count());
    for (index, argument) in args.iter().enumerate() {
        if !matches!(
            argument.effect,
            ArgEffect::SharedBorrow | ArgEffect::MayConsumeShared
        ) {
            continue;
        }
        let Some(actor) = call_argument_actor_index(input, argument.argument) else {
            continue;
        };
        shared_roots.union_with(&arg_roots[index]);
        shared_arguments.push((
            index,
            actor,
            input
                .context
                .diagnostics
                .value_error_span(argument.argument, input.location),
        ));
    }
    if shared_arguments.is_empty() {
        return Ok(());
    }
    for mutable_arg in args {
        if !matches!(
            mutable_arg.effect,
            ArgEffect::MutableBorrow | ArgEffect::MayConsumeMutable
        ) {
            continue;
        }
        let Some(mutable_actor) = call_argument_actor_index(input, mutable_arg.argument) else {
            continue;
        };
        let mutable_location = input
            .context
            .diagnostics
            .value_error_span(mutable_arg.argument, input.location);
        let mutable_roots = mutable_argument_roots(
            input.context,
            input.layout,
            input.state,
            mutable_arg.argument,
            mutable_location,
            input
                .context
                .expressions
                .expression(mutable_arg.argument)
                .span,
        )?;
        if !mutable_roots.intersects(&shared_roots) {
            continue;
        }
        for &(shared_index, shared_actor, shared_location) in &shared_arguments {
            if !mutable_roots.intersects(&arg_roots[shared_index]) {
                continue;
            }
            let diagnostics = &input.context.diagnostics;
            return Err(diagnostics.shared_mutable_conflict(
                diagnostics.local_place(input.layout.local_ids[mutable_actor]),
                BorrowAccessKind::Shared,
                BorrowAccessKind::Mutable,
                Some(diagnostics.local_place(input.layout.local_ids[shared_actor])),
                shared_location,
                mutable_location,
            ));
        }
    }

    Ok(())
}

/// Record what a call result holds in its destination.
///
/// WHY: a destination that is a mutable alias of a projected place stores the result into that
///      field, so the field must hold it too.
pub(in crate::compiler_frontend::analysis::borrow_checker) fn bind_call_result_ids(
    state: &mut BorrowState,
    local_index: usize,
    ids: &[AllocId],
    views: &[AllocId],
    readonly: bool,
) {
    let alias = state.local_state(local_index);
    if !alias.mode.contains(LocalMode::ALIAS) {
        state.bind_alloc_ids(local_index, ids, readonly);
        if !views.is_empty() {
            state.bind_alloc_views(local_index, views);
        }
        return;
    }
    let cells = state.alias_cells(local_index);
    let exact_projected = state.exact_projected_cell(local_index, &cells);
    let exact_whole = definite_slot_referent(state, local_index, alias);
    let possible_whole = if exact_whole.is_none() {
        alias.direct_alias_roots.iter_ones().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if !cells.is_empty() {
        let stored = ids
            .iter()
            .map(|&id| StoredValue {
                id,
                writable: !readonly,
            })
            .collect::<Vec<_>>();
        for (holder, key) in cells {
            state.store_alloc_field_with_views(
                &[holder],
                key,
                &stored,
                views,
                exact_projected,
                true,
            );
        }
    }
    if let Some(whole) = exact_whole {
        state.bind_alloc_ids(whole, ids, readonly);
        if !views.is_empty() {
            state.bind_alloc_views(whole, views);
        }
    } else {
        for whole in possible_whole {
            if whole != local_index && state.local_state(whole).mode.contains(LocalMode::SLOT) {
                state.extend_alloc_ids(whole, ids, views, readonly);
            }
        }
    }
    state.bind_alloc_ids(local_index, ids, readonly);
    if !views.is_empty() {
        state.bind_alloc_views(local_index, views);
    }
}

/// Allocations the receiver argument reaches, when the layer has anything to say about them.
fn call_receiver_allocs(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
) -> Option<ResolvedPlace> {
    let receiver = args.first()?;
    let place = direct_place_from_expression(input.context.expressions, receiver.argument)?;
    resolve_place_allocs(input.layout, input.state, input.context.expressions, &place)
}

/// A fallible builtin stores nothing when it fails.
fn capture_builtin_store(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
    positions: &[(usize, FieldKey)],
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let Some(holders) = call_receiver_allocs(input, args) else {
        return Ok(Vec::new());
    };
    let values = LayerValueContext {
        context: input.context,
        layout: input.layout,
        block_id: input.block_id,
    };
    let mut ops = Vec::new();
    for &(index, key) in positions {
        let Some(argument) = args.get(index) else {
            continue;
        };
        let Some(stored) =
            layer_store_values(&values, input.state, &holders.ids, key, argument.argument)?
        else {
            continue;
        };
        ops.push(InvokeLayerOp::Store {
            holders: holders.ids.clone(),
            key,
            values: stored.values,
            views: stored.views,
            distinct_occurrence: true,
        });
    }
    Ok(ops)
}

/// Capture the element before removal and before a result can rebind the receiver.
fn capture_builtin_element_result(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
    result: Option<HirLocalDestination>,
    storage: CallStorage,
) -> Vec<InvokeLayerOp> {
    let heap_result = result
        .and_then(|result| input.layout.index_of(result.local()))
        .is_some_and(|index| input.layout.local_has_in_place_mutation_surface[index]);
    if !heap_result && input.state.layer_is_empty() {
        return Vec::new();
    }
    let Some(mut elements) = call_receiver_allocs(input, args) else {
        return Vec::new();
    };
    let holders = if matches!(storage, CallStorage::Remove { .. }) {
        elements.ids.clone()
    } else {
        Vec::new()
    };
    let mut ops = Vec::new();
    if let Some(result) = result.filter(|_| heap_result) {
        input
            .state
            .step_resolved(&mut elements, FieldKey::Elem, &input.layout.local_mutable);
        if !elements.ids.is_empty() {
            ops.push(InvokeLayerOp::Bind {
                local: result.local(),
                ids: elements.ids,
                views: elements.views,
                readonly: elements.readonly || matches!(storage, CallStorage::Get),
            });
        }
    }
    if let CallStorage::Remove { map } = storage {
        ops.push(InvokeLayerOp::Remove {
            holders,
            map,
            result: result.map(HirLocalDestination::local),
        });
    }
    ops
}

fn record_call_argument_reads(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    argument: HirValueId,
    effect: ArgEffect,
    argument_location: &Option<SourceSpan>,
    arg_roots: &mut RootSet,
) -> Result<(), BorrowCheckError> {
    // WHAT: a direct place argument also overlaps the cells its root local designates.
    // WHY: the mutable-argument check asks about those cells, while the shared argument's value
    //      provenance names only its allocation. Recording the cells here makes same-call overlap
    //      and the shared/mutable conflict see both spellings of the same binding, including an
    //      alias whose only surviving identity is a referent cell.
    let direct_place = direct_place_from_expression(input.context.expressions, argument);
    let direct_root_index = direct_place.and_then(|place| input.layout.index_of(place.root));
    let cell_roots = direct_root_index.map(|root_index| input.state.designated_cells(root_index));
    if let Some(cell_roots) = &cell_roots
        && matches!(effect, ArgEffect::SharedBorrow)
    {
        let mut check = AccessCheckContext {
            context: input.context,
            layout: input.layout,
            state: input.state,
            block_id: input.block_id,
            tracker: input.tracker,
            location: *argument_location,
            span: input.context.expressions.expression(argument).span,
            stats: input.stats,
            actor_index_hint: direct_root_index,
            current_order: input.current_order,
        };
        check_shared_access(&mut check, cell_roots)?;
    }

    if matches!(
        effect,
        ArgEffect::MutableBorrow | ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
    ) && let Some(place) = direct_place
    {
        let mut read_env = SharedReadEnv {
            context: input.context,
            layout: input.layout,
            state: input.state,
            block_id: input.block_id,
            tracker: input.tracker,
            location: *argument_location,
            current_order: input.current_order,
            stats: input.stats,
            value_fact_buffer: input.value_fact_buffer,
        };
        let mut place_read_roots = RootSet::empty(input.layout.local_count());
        record_shared_reads_in_place_indices(
            &mut read_env,
            &place,
            *argument_location,
            &mut place_read_roots,
        )?;
        let place_roots = access_roots_for_place(
            input.layout,
            input.state,
            &place,
            *argument_location,
            input.context.expressions.expression(argument).span,
            &input.context.diagnostics,
        )?;
        place_read_roots.union_with(&place_roots);
        arg_roots.union_with(&place_read_roots);
    } else {
        let mut read_env = SharedReadEnv {
            context: input.context,
            layout: input.layout,
            state: input.state,
            block_id: input.block_id,
            tracker: input.tracker,
            location: *argument_location,
            current_order: input.current_order,
            stats: input.stats,
            value_fact_buffer: input.value_fact_buffer,
        };
        record_shared_reads_in_expression(&mut read_env, argument, *argument_location, arg_roots)?;
    }

    if let Some(cell_roots) = &cell_roots {
        arg_roots.union_with(cell_roots);
    }

    Ok(())
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

#[derive(Clone, Copy)]
struct CallArgumentAccess {
    argument: HirValueId,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
    reaches_children: bool,
}

fn transfer_call_argument_access(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    argument: HirValueId,
    effect: ArgEffect,
    argument_location: Option<SourceSpan>,
    reaches_children: bool,
) -> Result<(), BorrowCheckError> {
    let argument_span = input.context.expressions.expression(argument).span;
    let access = CallArgumentAccess {
        argument,
        location: argument_location,
        span: argument_span,
        reaches_children,
    };
    match effect {
        ArgEffect::SharedBorrow => Ok(()),
        ArgEffect::MutableBorrow => {
            let mutable_roots = mutable_argument_roots(
                input.context,
                input.layout,
                input.state,
                argument,
                argument_location,
                argument_span,
            )?;
            let actor_index_hint = call_argument_actor_index(input, argument);
            check_call_mutable_borrow(input, access, &mutable_roots, actor_index_hint)?;
            input.value_fact_buffer.record(
                argument,
                ValueAccessClassification::MutableArgument,
                &mutable_roots,
            );
            Ok(())
        }
        ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable => {
            let fallback_effect = fallback_call_argument_effect(effect);
            let mutable_roots = mutable_argument_roots(
                input.context,
                input.layout,
                input.state,
                argument,
                argument_location,
                argument_span,
            )?;
            let actor_index_hint = call_argument_actor_index(input, argument);
            check_call_may_consume(
                input,
                access,
                &mutable_roots,
                actor_index_hint,
                fallback_effect,
            )?;
            input.value_fact_buffer.record(
                argument,
                if matches!(fallback_effect, ArgEffect::SharedBorrow) {
                    ValueAccessClassification::SharedRead
                } else {
                    ValueAccessClassification::MutableArgument
                },
                &mutable_roots,
            );
            Ok(())
        }
    }
}

/// Check a mutable argument's place against bindings that reach it through the layer.
fn check_call_layer_mutation(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    argument: HirValueId,
    argument_location: Option<SourceSpan>,
    reaches_children: bool,
) -> Result<(), BorrowCheckError> {
    let Some(place) = direct_place_from_expression(input.context.expressions, argument) else {
        return Ok(());
    };
    let mut check = AccessCheckContext {
        context: input.context,
        layout: input.layout,
        state: input.state,
        block_id: input.block_id,
        tracker: input.tracker,
        location: argument_location,
        span: input.context.expressions.expression(argument).span,
        stats: input.stats,
        actor_index_hint: input.layout.index_of(place.root),
        current_order: input.current_order,
    };
    check_layer_place_mutation(&mut check, &place, reaches_children)
}

fn effective_call_argument_effect(
    effect: ArgEffect,
    argument_index: usize,
    argument_roots: &[RootSet],
    result_may_alias_argument: bool,
) -> ArgEffect {
    let optional_transfer = matches!(
        effect,
        ArgEffect::MayConsumeShared | ArgEffect::MayConsumeMutable
    );
    if !optional_transfer {
        return effect;
    }

    let has_same_statement_use =
        argument_roots
            .iter()
            .enumerate()
            .any(|(other_index, other_roots)| {
                other_index != argument_index
                    && argument_roots[argument_index].intersects(other_roots)
            });
    if !result_may_alias_argument && !has_same_statement_use {
        return effect;
    }

    fallback_call_argument_effect(effect)
}

fn fallback_call_argument_effect(effect: ArgEffect) -> ArgEffect {
    match effect {
        ArgEffect::MayConsumeShared => ArgEffect::SharedBorrow,
        ArgEffect::MayConsumeMutable => ArgEffect::MutableBorrow,
        _ => effect,
    }
}

fn check_call_mutable_borrow(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    access: CallArgumentAccess,
    mutable_roots: &RootSet,
    actor_index_hint: Option<usize>,
) -> Result<(), BorrowCheckError> {
    let CallArgumentAccess {
        argument,
        location,
        span,
        reaches_children,
    } = access;
    let mut check = AccessCheckContext {
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
    };
    let place = direct_place_from_expression(input.context.expressions, argument);
    let targets = match &place {
        Some(place) => layer_mutation_targets(&check, place, reaches_children)?,
        None => Vec::new(),
    };
    if !mutable_roots.is_empty() {
        check_mutable_access_with_targets(
            &mut check,
            mutable_roots,
            MutableAccessPolicy {
                allow_prior_shared: false,
                require_root_mutable: true,
                strict_move_exclusivity: false,
                alias_exclusivity: AliasExclusivity::All,
            },
            if place.is_some_and(|place| !place.projections.is_empty()) {
                &targets
            } else {
                &[]
            },
        )?;
    }
    if let Some(actor) = actor_index_hint {
        check_layer_conflict(&mut check, actor, &targets, true)?;
    }
    Ok(())
}

fn check_call_may_consume(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    access: CallArgumentAccess,
    mutable_roots: &RootSet,
    actor_index_hint: Option<usize>,
    fallback_effect: ArgEffect,
) -> Result<(), BorrowCheckError> {
    let CallArgumentAccess {
        argument,
        location,
        span,
        reaches_children,
    } = access;
    if mutable_roots.is_empty() {
        if matches!(fallback_effect, ArgEffect::MutableBorrow) {
            return check_call_layer_mutation(input, argument, location, reaches_children);
        }
        return Ok(());
    }

    let move_decision = classify_move_decision(
        input.layout,
        input.block_id,
        mutable_roots,
        input.current_order,
    );
    match move_decision {
        MoveDecision::Borrow => {
            input.value_fact_buffer.record_optional_transfer(
                argument,
                OptionalTransferStatus::Borrow,
                mutable_roots,
            );
            check_call_borrow_fallback(
                input,
                access,
                mutable_roots,
                actor_index_hint,
                fallback_effect,
            )
        }
        MoveDecision::Move => {
            let mut check = AccessCheckContext {
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
            };
            // Inferred transfer moves cleanup responsibility, never mutation authority. The
            // callee's declared access mode still decides whether the argument's root must be
            // mutable, exactly as the non-transfer mutable borrow check requires.
            let transfer_policy = MutableAccessPolicy {
                allow_prior_shared: false,
                require_root_mutable: matches!(fallback_effect, ArgEffect::MutableBorrow),
                strict_move_exclusivity: true,
                alias_exclusivity: AliasExclusivity::All,
            };
            if !probe_mutable_access(&check, mutable_roots, transfer_policy)? {
                input.value_fact_buffer.record_optional_transfer(
                    argument,
                    OptionalTransferStatus::Borrow,
                    mutable_roots,
                );
                return check_call_borrow_fallback(
                    input,
                    access,
                    mutable_roots,
                    actor_index_hint,
                    fallback_effect,
                );
            }

            input.value_fact_buffer.record_optional_transfer(
                argument,
                OptionalTransferStatus::Transfer,
                mutable_roots,
            );
            check_mutable_access(&mut check, mutable_roots, transfer_policy)?;
            if matches!(fallback_effect, ArgEffect::MutableBorrow) {
                check_call_layer_mutation(input, argument, location, reaches_children)?;
            }

            Ok(())
        }
    }
}

fn check_call_borrow_fallback(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    access: CallArgumentAccess,
    roots: &RootSet,
    actor_index_hint: Option<usize>,
    fallback_effect: ArgEffect,
) -> Result<(), BorrowCheckError> {
    if matches!(fallback_effect, ArgEffect::SharedBorrow) {
        check_call_shared_borrow(input, roots, actor_index_hint, access.location, access.span)
    } else {
        check_call_mutable_borrow(input, access, roots, actor_index_hint)
    }
}

fn call_argument_actor_index(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    argument: HirValueId,
) -> Option<usize> {
    // Allocation roots identify the reached value; the actor is the binding the argument reads.
    direct_place_from_expression(input.context.expressions, argument)
        .and_then(|place| input.layout.index_of(place.root))
}

fn check_call_shared_borrow(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    roots: &RootSet,
    actor_index_hint: Option<usize>,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> Result<(), BorrowCheckError> {
    if roots.is_empty() {
        return Ok(());
    }

    let mut check = AccessCheckContext {
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
    };
    check_shared_access(&mut check, roots)
}

fn transfer_call_result_alias(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    result: Option<HirLocalDestination>,
    alias_arguments: Option<&[usize]>,
    arg_value_roots: &[RootSet],
    derived_roots: RootSet,
    result_transfer: CallResultTransfer,
    layer_ops: &[InvokeLayerOp],
) -> Result<Option<AllocationRetirement>, BorrowCheckError> {
    let Some(destination) = result else {
        return Ok(None);
    };
    let result_local = destination.local();

    let Some(local_index) = input.layout.index_of(result_local) else {
        return Err(input.context.diagnostics.internal_error(format!(
            "Call result local '{}' is not in the active function layout",
            input.context.diagnostics.local_name(result_local)
        )));
    };

    let new_local_state =
        call_result_local_state(input, alias_arguments, arg_value_roots, derived_roots)?;
    let derived_roots = &new_local_state.derived_from_roots;
    if matches!(result_transfer, CallResultTransfer::Edge(_)) {
        input
            .stats
            .invoke
            .success
            .results
            .push((result_local, new_local_state));
        return Ok(None);
    }

    if matches!(destination, HirLocalDestination::Update(_)) {
        let previous_state = input.state.local_state(local_index).clone();
        require_initialized_update_target(
            input.layout,
            input.state,
            local_index,
            &input.context.diagnostics,
            input.location,
        )?;

        let mut check = AccessCheckContext {
            context: input.context,
            layout: input.layout,
            state: input.state,
            block_id: input.block_id,
            tracker: input.tracker,
            location: input.location,
            span: input.location,
            stats: input.stats,
            actor_index_hint: Some(local_index),
            current_order: input.current_order,
        };
        let _ = check_local_update_access(
            &mut check,
            local_index,
            &previous_state,
            new_local_state.value_roots.is_empty(),
        )?;

        if previous_state.mode.contains(LocalMode::ALIAS) {
            transfer_layer_alias_write(&mut check, local_index)?;
            let publishes_weak_whole_ids = layer_ops.iter().any(|op| {
                matches!(op, InvokeLayerOp::Bind { local, ids, .. } if *local == result_local && !ids.is_empty())
            });
            let (observed, retirement) = refresh_alias_referents(
                input.state,
                local_index,
                &previous_state,
                Some(&new_local_state.value_roots),
                input.current_order,
                input.layout.local_has_in_place_mutation_surface[local_index],
                publishes_weak_whole_ids,
            );

            if previous_state.is_alias_only() {
                // The binding itself is not rebound, so its derivation ledger stays unchanged.
                return Ok(retirement);
            }

            // A joined state holds the result on both alternatives. Keep the union of cells but
            // record the observed value, and update the cells the alias path writes through.
            let mut derived_from_roots = previous_state.derived_from_roots;
            derived_from_roots.intersect_with(derived_roots);
            input.state.update_local_state(
                local_index,
                LocalState {
                    mode: previous_state.mode,
                    value_roots: observed,
                    direct_alias_roots: previous_state.direct_alias_roots,
                    write_target_roots: previous_state.write_target_roots,
                    derived_from_roots,
                },
            );
            return Ok(retirement);
        }
    }

    if input.layout.local_has_in_place_mutation_surface[local_index] {
        let pristine_definition = match result_transfer {
            CallResultTransfer::FirstDefinition(index) => Some(index),
            _ => None,
        };
        Ok(input.state.rebind_local_after_capture(
            local_index,
            new_local_state,
            input.current_order,
            pristine_definition,
            true,
        ))
    } else {
        input
            .state
            .rebind_local(local_index, new_local_state, input.current_order);
        Ok(None)
    }
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
    let mut state = LocalState::slot_with_value_roots(alias_roots, RootSet::empty(local_count));
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

fn map_result_alias(op: HirMapOp) -> CallReturns<'static> {
    if matches!(op, HirMapOp::Get) {
        CallReturns::Get
    } else {
        CallReturns::Fresh
    }
}
