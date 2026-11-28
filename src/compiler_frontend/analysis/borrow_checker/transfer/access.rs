//! Statement/terminator transfer rules.
//!
//! This file contains the forward transfer logic for borrow checking.
//! It classifies shared vs mutable access, checks exclusivity constraints,
//! and emits statement/terminator/value facts.

use super::super::diagnostics::BorrowDiagnostics;
use super::call_semantics::{ArgEffect, resolve_call_semantics};
use super::facts::{
    AccessFootprint, PlaceUse, StatementAccessTracker, ValueFactBuffer, roots_to_local_ids,
};
use super::{BlockTransferStats, BorrowTransferContext};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocationRetirement, BindingSpec, BorrowState, CellId, Coverage, FunctionLayout,
    LocalState, OrderKey, PlaceResolution, Replacement, RootSet, TypedStore, WrapperAge, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    AccessKind, LocalMode, OptionalTransferStatus, StatementBorrowFact, TerminatorBorrowFact,
    ValueAccessClassification,
};
use crate::compiler_frontend::compiler_messages::InvalidMutableAccessReason;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
pub(in super::super) mod conflicts;
mod layer;
mod move_decision;
mod statement;
mod terminator;
#[cfg(test)]
mod tests;

use conflicts::{check_access, footprint, probe_mutable_access};
use layer::{
    CapturedDetachments, LayerValueContext, capture_projected_binding, capture_slot_value,
    check_cell_permission, check_place_replacement, check_shared_place_access, resolve_place,
    stored_values,
};
use move_decision::{MoveDecision, classify_move_decision};
pub(in super::super) use statement::call_member_result;
pub(super) use statement::transfer_statement;
pub(super) use terminator::transfer_terminator;

fn require_initialized_update_target(
    layout: &FunctionLayout,
    state: &BorrowState,
    local_index: usize,
    diagnostics: &BorrowDiagnostics<'_>,
    span: Option<SourceSpan>,
) -> Result<(), BorrowCheckError> {
    // WHY: an update rejects a possible-uninitialized target, unlike a read, which only rejects
    // `is_definitely_uninit`. Replacing a binding that some path never created has no valid
    // interpretation. This guard also keeps an `UNINIT | SLOT` join away from the slot predicates
    // on both update paths.
    if state.binding_mode(local_index).contains(LocalMode::UNINIT) {
        return Err(diagnostics.use_of_uninitialized_local(
            diagnostics.local_place(layout.local_ids[local_index]),
            span,
        ));
    }

    require_mutable_local(layout, local_index, diagnostics, span)
}

fn require_mutable_local(
    layout: &FunctionLayout,
    local_index: usize,
    diagnostics: &BorrowDiagnostics<'_>,
    span: Option<SourceSpan>,
) -> Result<(), BorrowCheckError> {
    if !layout.local_mutable[local_index] {
        return Err(diagnostics.invalid_mutable_access(
            diagnostics.local_place(layout.local_ids[local_index]),
            InvalidMutableAccessReason::ImmutablePlace,
            None,
            None,
            span,
        ));
    }

    Ok(())
}

fn immutable_cell_error(check: &AccessCheckContext<'_, '_>, origin: usize) -> BorrowCheckError {
    check.context.diagnostics.invalid_mutable_access(
        check
            .context
            .diagnostics
            .local_place(check.layout.local_ids[origin]),
        InvalidMutableAccessReason::ImmutablePlace,
        None,
        None,
        check.span.or(check.location),
    )
}

fn check_local_update_access(
    check: &mut AccessCheckContext<'_, '_>,
    resolved: &PlaceResolution,
) -> Result<RootSet, BorrowCheckError> {
    let mut roots = RootSet::empty(check.layout.local_count());
    report_place_cells(check.state, &resolved.address, &mut roots);
    check_place_replacement(check, resolved, &roots)?;
    Ok(roots)
}

// WHAT: These helper contexts split statement transfer into two concerns:
// shared-read collection and access-conflict validation.
// WHY: transfer threads the same diagnostics/layout/state bundle through many helpers, so keeping
// those bundles explicit avoids wide argument lists without cloning the same context structs.
// WHAT: Shared-read traversal context used while scanning one statement/terminator.
// WHY: Keeps helper signatures compact while threading diagnostics and fact sinks.
struct SharedReadEnv<'a, 'module> {
    context: &'a BorrowTransferContext<'module>,
    layout: &'a FunctionLayout,
    state: &'a BorrowState,
    block_id: BlockId,
    tracker: &'a mut StatementAccessTracker,
    location: Option<SourceSpan>,
    current_order: i32,
    stats: &'a mut BlockTransferStats,
    value_fact_buffer: &'a mut ValueFactBuffer,
}

// WHAT: Shared and mutable conflict checks both inspect the same transfer bundle.
// WHY: Keeping one access-check context avoids duplicating the same layout/state/tracker fields.
struct AccessCheckContext<'a, 'module> {
    context: &'a BorrowTransferContext<'module>,
    layout: &'a FunctionLayout,
    state: &'a BorrowState,
    block_id: BlockId,
    tracker: &'a mut StatementAccessTracker,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
    stats: &'a mut BlockTransferStats,
    actor_index_hint: Option<usize>,
    current_order: i32,
}

const BORROW_ACCESS: MutableAccessPolicy = MutableAccessPolicy {
    allow_prior_shared: false,
    strict_move_exclusivity: false,
};

const STORE_ACCESS: MutableAccessPolicy = MutableAccessPolicy {
    allow_prior_shared: true,
    strict_move_exclusivity: false,
};

#[derive(Clone, Copy)]
struct MutableAccessPolicy {
    allow_prior_shared: bool,
    strict_move_exclusivity: bool,
}

/// Shared assignment-transfer environment for one statement.
///
/// WHAT: packages the borrow-transfer diagnostics/layout/state bundle used by assignment writes.
/// WHY: assignment transfer needs many correlated parameters, and bundling them keeps helpers clear.
struct AssignTransferContext<'a, 'module> {
    context: &'a BorrowTransferContext<'module>,
    layout: &'a FunctionLayout,
    state: &'a mut BorrowState,
    block_id: BlockId,
    current_order: i32,
    tracker: &'a mut StatementAccessTracker,
    value_fact_buffer: &'a mut ValueFactBuffer,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
    stats: &'a mut BlockTransferStats,
}

fn record_shared_reads_in_pattern(
    env: &mut SharedReadEnv<'_, '_>,
    arm: &HirMatchArm,
) -> Result<(), BorrowCheckError> {
    if let HirPattern::Literal(expression)
    | HirPattern::OptionValue { value: expression }
    | HirPattern::OptionRelational {
        value: expression, ..
    } = &arm.pattern
    {
        let location = env.location;
        record_shared_reads_in_expression(
            env,
            *expression,
            location,
            &mut RootSet::empty(env.layout.local_count()),
        )?;
    }

    if let Some(guard) = &arm.guard {
        let location = env.location;
        record_shared_reads_in_expression(
            env,
            *guard,
            location,
            &mut RootSet::empty(env.layout.local_count()),
        )?;
    }

    Ok(())
}

/// An acyclic entry prefix cannot contain an earlier legal reference to an unwritten local.
fn first_definition_index(
    layout: &FunctionLayout,
    state: &BorrowState,
    block: BlockId,
    order: OrderKey,
    resolve_index: impl FnOnce() -> Option<usize>,
) -> Option<usize> {
    if layout.acyclic_entry_block == Some(block)
        && order >= 0
        && let Some(index) = resolve_index()
        && layout.local_first_write_order[index] == order
        && state.is_virgin_definition(index)
    {
        // Only parameters enter this block initialized. The prefix has never written this
        // local, and a read cannot produce coarse observers of an uninitialized local.
        Some(index)
    } else {
        None
    }
}

/// Check and capture an assignment in the pre-state, then publish its cell stores and role change.
fn transfer_assign_target(
    context: &mut AssignTransferContext<'_, '_>,
    target: &HirPlace,
    value: HirValueId,
    is_definition: bool,
    local_index: Option<usize>,
    pristine_definition: Option<usize>,
) -> Result<(), BorrowCheckError> {
    let transfer = context.context;
    let layout = context.layout;
    let state = &mut *context.state;
    let Some(local) = local_index else {
        return Err(transfer.diagnostics.internal_error(format!(
            "Assignment target local '{}' is not in the active function layout",
            transfer.diagnostics.local_name(target.root)
        )));
    };
    if !is_definition {
        require_initialized_update_target(
            layout,
            state,
            local,
            &transfer.diagnostics,
            context.span.or(context.location),
        )?;
    }

    let values = LayerValueContext {
        context: transfer,
        layout,
        block_id: context.block_id,
    };
    let mut check = AccessCheckContext {
        context: transfer,
        layout,
        state,
        block_id: context.block_id,
        tracker: context.tracker,
        location: context.location,
        span: context.span,
        stats: &mut *context.stats,
        actor_index_hint: Some(local),
        current_order: context.current_order,
    };

    if !target.projections.is_empty() {
        let mut roots = roots_for_place(
            layout,
            check.state,
            target,
            check.location,
            check.span,
            &transfer.diagnostics,
        )?;
        let resolved = resolve_place(layout, check.state, transfer.expressions, target)
            .expect("projected target's root is in the layout");
        report_place_cells(check.state, &resolved.address, &mut roots);
        check_place_replacement(&mut check, &resolved, &roots)?;
        let addresses = resolved.address;
        check.tracker.record_write(&roots);
        let has_layer = !state.layer_is_empty();
        let stored = stored_values(&values, state, value, has_layer)?;
        if has_layer || !stored.is_empty() {
            let receipt = state.publish_store(
                &TypedStore::replace_cells(addresses, stored),
                context.current_order,
                None,
            );
            context.stats.observations.tail.replace(receipt);
        }
        return Ok(());
    }

    let expression = transfer.expressions.expression(value);
    let source = place_load_source(layout, transfer.expressions, value);
    let mode = check.state.binding_mode(local);
    let own_cell = CellId::Slot(WrapperId::current(local));

    let rhs_roots = direct_value_roots_from_expression(
        layout,
        check.state,
        transfer.expressions,
        value,
        check.location,
        &transfer.diagnostics,
    )?;
    let derivation =
        alias_derivation_roots_from_expression(layout, check.state, transfer.expressions, value);

    if is_definition
        && mode.is_definitely_uninit()
        && layout.local_mutable[local]
        && let Some(roots) = rhs_roots.as_ref().filter(|roots| !roots.is_empty())
        && roots.iter_ones().all(|root| layout.local_mutable[root])
    {
        let status =
            match classify_move_decision(layout, context.block_id, roots, context.current_order) {
                MoveDecision::Borrow => OptionalTransferStatus::Borrow,
                MoveDecision::Move => OptionalTransferStatus::Transfer,
            };
        context
            .value_fact_buffer
            .record_optional_transfer(value, status, roots);
    }
    let rhs_roots = rhs_roots.unwrap_or_else(|| RootSet::empty(layout.local_count()));
    let local_addresses = (!is_definition).then(|| {
        check
            .state
            .resolve_place_path(local, &layout.local_mutable, [])
    });
    if let Some(resolved) = &local_addresses {
        let roots = check_local_update_access(&mut check, resolved)?;
        check.tracker.record_write(&roots);
    }
    // The cell store and the new link are two accesses of one operation.
    check.tracker.clear_reservations();
    // Scratch may carry a shared link, but its synthetic mutability cannot grant exclusivity.
    let writable =
        layout.local_mutable[local] && !is_compiler_owned_scratch_local(transfer, layout, local);
    // Only the slot alternative acquires a link; existing aliases receive the loaded value.
    let acquisition = source.filter(|_| is_definition || mode.contains(LocalMode::SLOT));
    let mut preserve_slot = false;
    let mixed_acquisition = !is_definition && mode.contains(LocalMode::ALIAS);
    let mut place_payload = None;
    let binding = if let Some((source, place)) = acquisition {
        if place.projections.is_empty() && !is_definition && source == local {
            preserve_slot = true;
            None
        } else if place.projections.is_empty() {
            let mut resolved = check
                .state
                .resolve_place_path(source, &layout.local_mutable, []);
            let addresses = &resolved.address;
            preserve_slot = !is_definition
                && (addresses.cells().contains(&own_cell)
                    || addresses.coverage == Coverage::Partial);
            if !is_definition && addresses.definite() == Some(own_cell) {
                None
            } else {
                let heap = layout.local_has_in_place_mutation_surface[source];
                let capture_payload =
                    mixed_acquisition && (heap || check.state.has_value_alternatives());
                if capture_payload || writable && heap {
                    check
                        .state
                        .load_place_content(&mut resolved, &layout.local_mutable, !heap);
                }
                if writable {
                    check.span = expression.span;
                    check_cell_permission(&check, &resolved)?;
                    let mut roots = rhs_roots.clone();
                    report_place_cells(check.state, &resolved.address, &mut roots);
                    check.actor_index_hint = Some(source);
                    check_access(
                        &mut check,
                        &roots,
                        footprint(&resolved, PlaceUse::Acquire),
                        Some(&resolved),
                        STORE_ACCESS,
                    )?;
                }
                if capture_payload && let Some(content) = resolved.content {
                    place_payload = Some(layer::stored_allocations(
                        layout,
                        check.state,
                        content,
                        resolved.permission.wrapper_readonly_source.is_none(),
                    ));
                }
                Some(BindingSpec::Link {
                    source: WrapperId::current(source),
                    writable,
                })
            }
        } else {
            check.span = expression.span;
            check.actor_index_hint = Some(source);
            let heap = layer::is_heap_value(transfer, value)?;
            let capture_payload =
                mixed_acquisition && (heap || check.state.has_value_alternatives());
            let (binding, payload) = capture_projected_binding(
                &mut check,
                &place,
                writable,
                &rhs_roots,
                heap,
                capture_payload,
            )?;
            place_payload = payload;
            Some(binding)
        }
    } else {
        None
    };
    let observations = &mut context.stats.observations;

    if is_definition {
        let defines_slot = binding.is_none();
        let mut slot = if defines_slot {
            capture_slot_value(&values, state, value)?
        } else {
            Default::default()
        };
        let mut detachments = if defines_slot {
            Default::default()
        } else {
            CapturedDetachments::from_load(&values, state, value)
        };
        let mut binding = binding.unwrap_or(BindingSpec::Slot);
        if matches!(&binding, BindingSpec::Link { source, .. } if *source == WrapperId::current(local))
        {
            state.retain_captured_wrapper(local);
        }
        let first = pristine_definition == Some(local);
        let names = |id| state.is_current_allocation_of(id, local);
        let captures_old = first
            && (rhs_roots.contains(local)
                || slot.names_allocation(&names)
                || detachments.names_allocation(&names)
                || match &binding {
                    BindingSpec::Slot => false,
                    BindingSpec::Link { source, .. } => *source == WrapperId::current(local),
                    BindingSpec::Projected {
                        addresses,
                        origin,
                        owner,
                        route_roots,
                        ..
                    } => {
                        *origin == WrapperId::current(local)
                            || *owner == Some(WrapperId::current(local))
                            || route_roots.contains(local)
                            || addresses.captures_current(local, names)
                    }
                    #[cfg(test)]
                    BindingSpec::Alias { .. } => {
                        unreachable!("assignment captures a link or projection")
                    }
                });
        if first && !captures_old && state.can_publish_first_definition(local) {
            state.update_local_state(local, LocalState::slot(layout.local_count()));
        } else {
            let before = observations.before_rekey(state);
            let next = LocalState::slot(layout.local_count());
            let retirement = if first {
                let retirement = state.retire_binding_allocation(local, context.current_order);
                state.update_local_state(local, next);
                retirement
            } else {
                state.rebind_local(local, next, context.current_order)
            };
            observations.after_retirement(before, state, retirement);
            slot.remap(state, retirement);
            detachments.remap(state, retirement);
            remap_binding_capture(&mut binding, state, retirement);
        }
        let mut derivation = derivation;
        derivation.remove(local);
        if defines_slot {
            state.set_binding(
                local,
                binding,
                RootSet::empty(layout.local_count()),
                derivation,
            );
            let receipt = layer::publish_slot_capture(
                state,
                local,
                None,
                slot,
                rhs_roots,
                context.current_order,
            );
            observations.tail.replace(receipt);
        } else {
            state.set_binding(local, binding, rhs_roots, derivation);
            detachments.apply(state, local);
        }
        // Retiring an alias can leave a prior wrapper without any remaining routes.
        if mode.contains(LocalMode::ALIAS) {
            observations.prune_retired_wrappers(state);
        }
        return Ok(());
    }

    let mut addresses = local_addresses
        .expect("an assignment has pre-state destinations")
        .address;
    if binding.is_some() || preserve_slot {
        if mode.contains(LocalMode::ALIAS) {
            addresses = state.resolve_alias_cells(local);
        } else {
            addresses = AddressSet::from_cells([], Coverage::Complete);
        }
    }

    if let Some(mut binding) = binding {
        let stores = mode.contains(LocalMode::ALIAS);
        let mut stored = place_payload.unwrap_or_default();
        let mut detachments = CapturedDetachments::from_load(&values, state, value);
        let captures_old_holder = matches!(&binding, BindingSpec::Projected { addresses, .. }
            if addresses.cells().iter().any(|cell| matches!(*cell, CellId::Projected(holder, _)
                if state.alloc_origin_local(holder) == Some(local))));
        if !preserve_slot && (!state.layer_is_empty() || !stored.is_empty() || captures_old_holder)
        {
            let before = observations.before_rekey(state);
            let retirement = state.retire_slot_value(local, context.current_order);
            observations.after_retirement(before, state, retirement);
            addresses.remap_allocations(state, retirement);
            stored.remap(state, retirement);
            detachments.remap(state, retirement);
            remap_binding_capture(&mut binding, state, retirement);
        }
        if stores {
            let receipt = state.publish_store(
                &TypedStore::write_through(addresses, stored, Replacement::May, local)
                    .with_slot_roots(rhs_roots.clone()),
                context.current_order,
                None,
            );
            observations.tail.replace(receipt);
        }
        let before = observations.before_rekey(state);
        state.transition_slot_to_alias(local, binding, rhs_roots, derivation, preserve_slot);
        observations.after_rekey(before, state, |slot| slot, |id| id);
        detachments.apply(state, local);
    } else if !preserve_slot && let Some(CellId::Slot(wrapper)) = addresses.definite() {
        let mut captured = capture_slot_value(&values, state, value)?;
        if captured.has_facts() || !state.layer_is_empty() {
            let before = observations.before_rekey(state);
            let retirement = state.retire_slot_value(wrapper.local, context.current_order);
            observations.after_retirement(before, state, retirement);
            captured.remap(state, retirement);
        }
        state.mark_slot_derivation(wrapper.local, derivation);
        let receipt = layer::publish_slot_capture(
            state,
            wrapper.local,
            (local != wrapper.local).then_some(local),
            captured,
            rhs_roots,
            context.current_order,
        );
        observations.tail.replace(receipt);
    } else if !preserve_slot || mode.contains(LocalMode::ALIAS) {
        let stored = stored_values(&values, state, value, true)?;
        let mut store = if preserve_slot {
            TypedStore::write_through(addresses, stored, Replacement::May, local)
        } else {
            let complete = addresses.coverage == Coverage::Complete;
            let store = TypedStore::replace_cells(addresses, stored);
            if complete && mode.contains(LocalMode::ALIAS) {
                store.with_actor_observation(local)
            } else {
                store
            }
        };
        store = store.with_slot_roots(rhs_roots);
        if mode.contains(LocalMode::SLOT) && !preserve_slot {
            state.mark_slot_derivation(local, derivation);
        }
        let receipt = state.publish_store(&store, context.current_order, None);
        observations.tail.replace(receipt);
    }
    Ok(())
}

fn remap_binding_capture(
    binding: &mut BindingSpec,
    state: &BorrowState,
    retirement: AllocationRetirement,
) {
    match binding {
        BindingSpec::Link { source, .. } => *source = retirement.remap_wrapper(*source),
        BindingSpec::Projected {
            addresses,
            origin,
            owner,
            ..
        } => {
            addresses.remap_allocations(state, retirement);
            *origin = retirement.remap_wrapper(*origin);
            *owner = owner.map(|owner| retirement.remap_wrapper(owner));
        }
        BindingSpec::Slot => {}
        #[cfg(test)]
        BindingSpec::Alias { .. } => {
            unreachable!("assignments capture immediate links or projections")
        }
    }
}

// WHAT: Reports whether a local is compiler-owned scratch/result storage rather than a
//      user-declared mutable binding.
// WHY: fresh compiler-owned scratch locals carry shared alias state through compiler-introduced
//      control flow without requesting source-exclusive access, so the uninit-assignment
//      exclusive-access check must be skipped for them. User bindings and any origin outside the
//      proven compiler-owned set keep the existing rules.
fn is_compiler_owned_scratch_local(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    local_index: usize,
) -> bool {
    matches!(
        context
            .diagnostics
            .local_origin_kind(layout.local_ids[local_index]),
        Some(HirLocalOriginKind::CompilerTemp)
    )
}

fn place_root_local_index(layout: &FunctionLayout, place: &HirPlace) -> Option<usize> {
    layout.index_of(place.root)
}

/// The source binding index and place a place-valued load names.
///
/// WHY: the identity helpers below must agree on which expressions are places, and they all need
/// the source binding index. One check keeps their preambles from drifting apart.
fn place_load_source(
    layout: &FunctionLayout,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> Option<(usize, HirPlace)> {
    let expression = expressions.expression(expression_id);
    if expression.value_kind != ValueKind::Place {
        return None;
    }
    let HirExpressionKind::Load(place) = &expression.kind else {
        return None;
    };
    Some((layout.index_of(place.root)?, *place))
}

fn direct_value_roots_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
    location: Option<SourceSpan>,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<Option<RootSet>, BorrowCheckError> {
    let expression = expressions.expression(expression_id);

    let HirExpressionKind::Load(place) = &expression.kind else {
        return Ok(None);
    };

    match expression.value_kind {
        ValueKind::Place | ValueKind::RValue => Ok(Some(roots_for_place(
            layout,
            state,
            place,
            location,
            expression.span,
            diagnostics,
        )?)),
        ValueKind::Const => Ok(Some(RootSet::empty(layout.local_count()))),
    }
}

/// Alias bindings a value expression was provably created through.
///
/// WHAT: the source binding's transitive derivation chain, including the source itself when it is
///       a pure alias.
/// WHY: a read through a binding derived from a mutably active alias is not a foreign access to
///      that alias. Recording the full chain as a must fact lets joins drop paths that did not
///      derive.
fn alias_derivation_roots_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> RootSet {
    let expression = expressions.expression(expression_id);
    let HirExpressionKind::Load(place) = &expression.kind else {
        return RootSet::empty(layout.local_count());
    };
    let Some(source_index) = layout.index_of(place.root) else {
        return RootSet::empty(layout.local_count());
    };
    state.derived_roots_from_source(source_index)
}

/// Report authored origins without making attribution part of overlap.
fn report_place_cells(state: &BorrowState, addresses: &AddressSet, roots: &mut RootSet) {
    for &cell in addresses.cells() {
        match cell {
            CellId::Slot(wrapper) if wrapper.age == WrapperAge::Current => {
                roots.insert(wrapper.local)
            }
            CellId::Projected(holder, _) => {
                if let Some(origin) = state.alloc_origin_local(holder) {
                    roots.insert(origin);
                }
            }
            CellId::Slot(_) => {}
        }
    }
}

fn roots_for_place(
    layout: &FunctionLayout,
    state: &BorrowState,
    place: &HirPlace,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<RootSet, BorrowCheckError> {
    increment_frontend_counter(FrontendCounter::BorrowPlaceAccessCount);

    let Some(local_index) = layout.index_of(place.root) else {
        return Err(diagnostics.internal_error(format!(
            "Borrow checker could not resolve place local '{}' in the current function",
            place.root
        )));
    };

    if state.binding_mode(local_index).is_definitely_uninit() {
        return Err(diagnostics.use_of_uninitialized_local(
            diagnostics.local_place(layout.local_ids[local_index]),
            span.or(location),
        ));
    }

    Ok(state.effective_roots(local_index))
}

/// Pending step for the shared-read walk over one expression tree.
///
/// WHAT: `Visit` enters an expression. `Finish` runs the work that follows its children, which is
/// a place access check for `Load`/`Copy` or the shared-read classification for other nodes.
/// WHY: children must be processed before a parent's own check. The shared root accumulator
/// is read after those children finish. An explicit stack keeps that order.
#[derive(Clone, Copy)]
enum SharedReadStep {
    Visit {
        expression_id: HirValueId,
        location: Option<SourceSpan>,
    },
    Finish {
        expression_id: HirValueId,
        value_span: Option<SourceSpan>,
    },
}

fn record_shared_reads_in_place_indices(
    env: &mut SharedReadEnv<'_, '_>,
    place: &HirPlace,
    location: Option<SourceSpan>,
    roots: &mut RootSet,
) -> Result<(), BorrowCheckError> {
    let expressions = env.context.expressions;
    let mut steps = Vec::new();
    steps.extend(
        expressions
            .place_index_values(*place)
            .rev()
            .map(|index| SharedReadStep::Visit {
                expression_id: index,
                location,
            }),
    );
    run_shared_read_steps(env, &mut steps, roots)
}

fn record_assignment_value_reads(
    env: &mut SharedReadEnv<'_, '_>,
    target: &HirPlace,
    is_definition: bool,
    value: HirValueId,
    location: Option<SourceSpan>,
) -> Result<(), BorrowCheckError> {
    let layout = env.layout;
    let expressions = env.context.expressions;
    let mut roots = RootSet::empty(layout.local_count());
    if target.projections.is_empty()
        && let Some(local) = layout.index_of(target.root)
        && layout.local_mutable[local]
        && (is_definition || env.state.binding_mode(local).contains(LocalMode::SLOT))
        && !is_compiler_owned_scratch_local(env.context, layout, local)
        && let Some((source, place)) = place_load_source(layout, expressions, value)
        && (is_definition || source != local || !place.projections.is_empty())
    {
        // Another writer must conflict with the exclusive acquisition, not with
        // a preliminary shared checkout of the same terminal cell.
        record_shared_reads_in_place_indices(env, &place, location, &mut roots)?;
        roots.union_with(&roots_for_place(
            layout,
            env.state,
            &place,
            location,
            expressions.expression(value).span,
            &env.context.diagnostics,
        )?);
        record_shared_read_classification(env, value, &roots);
        return Ok(());
    }
    record_shared_reads_in_expression(env, value, location, &mut roots)
}

fn record_shared_reads_in_expression(
    env: &mut SharedReadEnv<'_, '_>,
    expression_id: HirValueId,
    location: Option<SourceSpan>,
    roots: &mut RootSet,
) -> Result<(), BorrowCheckError> {
    let mut steps = vec![SharedReadStep::Visit {
        expression_id,
        location,
    }];
    run_shared_read_steps(env, &mut steps, roots)
}

/// WHAT: drains the shared-read stack, visiting children in source order.
/// WHY: the single `roots` accumulator is shared across the whole walk, so each post-children
/// classification sees all roots accumulated up to that point.
fn run_shared_read_steps(
    env: &mut SharedReadEnv<'_, '_>,
    steps: &mut Vec<SharedReadStep>,
    roots: &mut RootSet,
) -> Result<(), BorrowCheckError> {
    let expressions = env.context.expressions;

    while let Some(step) = steps.pop() {
        match step {
            SharedReadStep::Visit {
                expression_id,
                location,
            } => {
                let expression = expressions.expression(expression_id);
                match &expression.kind {
                    HirExpressionKind::Number(_)
                    | HirExpressionKind::FixedScalar(_)
                    | HirExpressionKind::Bool(_)
                    | HirExpressionKind::Char(_)
                    | HirExpressionKind::StringLiteral(_)
                    | HirExpressionKind::StructuralString { .. } => {
                        record_shared_read_classification(env, expression_id, roots);
                    }

                    HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => {
                        let value_span = env
                            .context
                            .diagnostics
                            .value_error_span(expression_id, expression.span.or(location));
                        steps.push(SharedReadStep::Finish {
                            expression_id,
                            value_span,
                        });
                        let first = expressions.push_child_values_in_source_order(
                            expression_id,
                            steps,
                            |child| SharedReadStep::Visit {
                                expression_id: child,
                                location: value_span,
                            },
                        );
                        steps.extend(first);
                    }

                    // WHY: the store lists the children of every other compound expression in source
                    // order; this node's classification runs in `Finish` after they are visited.
                    _ => {
                        steps.push(SharedReadStep::Finish {
                            expression_id,
                            value_span: None,
                        });
                        let first = expressions.push_child_values_in_source_order(
                            expression_id,
                            steps,
                            |child| SharedReadStep::Visit {
                                expression_id: child,
                                location,
                            },
                        );
                        steps.extend(first);
                    }
                }
            }

            SharedReadStep::Finish {
                expression_id,
                value_span,
            } => match &expressions.expression(expression_id).kind {
                HirExpressionKind::Load(place) => {
                    let place_roots =
                        check_shared_place_read(env, expression_id, place, value_span)?;
                    roots.union_with(&place_roots);
                    record_shared_read_classification(env, expression_id, roots);
                }

                HirExpressionKind::Copy(place) => {
                    let place_roots =
                        check_shared_place_read(env, expression_id, place, value_span)?;
                    roots.union_with(&place_roots);
                    // WHY: a direct copy records only its own roots and skips classification
                    // from the accumulated roots.
                    env.value_fact_buffer.record(
                        expression_id,
                        ValueAccessClassification::SharedRead,
                        &place_roots,
                    );
                }

                _ => record_shared_read_classification(env, expression_id, roots),
            },
        }
    }

    Ok(())
}

/// WHAT: checks one direct place read against live borrows and returns the roots it touches.
/// WHY: `Load` and `Copy` share this access check, so the walk keeps one copy of it.
fn check_shared_place_read(
    env: &mut SharedReadEnv<'_, '_>,
    expression: HirValueId,
    place: &HirPlace,
    value_span: Option<SourceSpan>,
) -> Result<RootSet, BorrowCheckError> {
    let value_roots = roots_for_place(
        env.layout,
        env.state,
        place,
        value_span,
        value_span,
        &env.context.diagnostics,
    )?;
    let mut access_roots = value_roots.clone();
    let actor_index_hint = place_root_local_index(env.layout, place);
    let heap = if place.projections.is_empty() {
        actor_index_hint.is_some_and(|actor| env.layout.local_has_in_place_mutation_surface[actor])
    } else {
        layer::is_heap_value(env.context, expression)?
    };
    let mut check = AccessCheckContext {
        context: env.context,
        layout: env.layout,
        state: env.state,
        block_id: env.block_id,
        tracker: env.tracker,
        location: value_span,
        span: value_span,
        stats: env.stats,
        actor_index_hint,
        current_order: env.current_order,
    };
    check_shared_place_access(&mut check, place, &mut access_roots, heap)?;
    Ok(value_roots)
}

/// WHAT: records the shared-read classification of an expression from the accumulated roots.
/// WHY: every non-copy node reports the roots gathered so far, so a leaf after a shared read
/// also reports those roots.
fn record_shared_read_classification(
    env: &mut SharedReadEnv<'_, '_>,
    expression_id: HirValueId,
    roots: &RootSet,
) {
    let classification = if roots.is_empty() {
        ValueAccessClassification::None
    } else {
        ValueAccessClassification::SharedRead
    };
    env.value_fact_buffer
        .record(expression_id, classification, roots);
}

/// Pending expression for root collection.
///
/// WHAT: one expression still to visit, with the span fallback for its root collection.
/// WHY: the walk runs on an explicit stack, so deep expression chains do not consume native stack.
#[derive(Clone, Copy)]
struct RootCollectStep {
    expression_id: HirValueId,
    span: Option<SourceSpan>,
}

fn collect_expression_roots(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &BorrowState,
    expression_id: HirValueId,
    out: &mut RootSet,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> Result<(), BorrowCheckError> {
    let expressions = context.expressions;
    let mut pending = vec![RootCollectStep {
        expression_id,
        span,
    }];

    while let Some(step) = pending.pop() {
        let expression = expressions.expression(step.expression_id);
        if let HirExpressionKind::Load(place) = &expression.kind {
            let roots = roots_for_place(
                layout,
                state,
                place,
                location,
                step.span.or(expression.span),
                &context.diagnostics,
            )?;
            out.union_with(&roots);
        }
        let first = expressions.push_child_values_in_source_order(
            step.expression_id,
            &mut pending,
            |child| RootCollectStep {
                expression_id: child,
                span: expressions.expression(child).span,
            },
        );
        pending.extend(first);
    }

    Ok(())
}

/// Shared inputs for aggregate-literal optional transfer analysis.
///
/// WHAT: keeps diagnostic lookup and advisory fact emission together while aggregate traversal
/// visits nested expressions on an explicit work stack.
/// WHY: optional transfer facts must be emitted without threading a wide argument list through
/// every aggregate child.
pub(super) struct AggregateTransferContext<'a, 'module> {
    pub(super) diagnostics: &'a BorrowDiagnostics<'module>,
    pub(super) expressions: &'a HirExpressionStore,
    pub(super) value_fact_buffer: &'a mut ValueFactBuffer,
}

/// Pending step for aggregate optional-transfer analysis.
///
/// WHAT: `Visit` traverses an expression. `AggregateChild` handles only direct children of map
/// and collection literals, recording optional transfer advice for their place reads.
/// WHY: the explicit stack preserves traversal order without adding transfer checks for direct
/// loads in struct and variant fields.
#[derive(Clone, Copy)]
enum AggregateOwnershipStep {
    Visit(HirValueId),
    AggregateChild(HirValueId),
}

/// WHAT: records optional transfer outcomes for children of aggregate literals.
/// WHY: aggregate construction can receive destruction responsibility, but failed proof must
///      leave mandatory source state available for conservative borrow validation.
pub(super) fn transfer_aggregate_expression_ownership(
    layout: &FunctionLayout,
    state: &mut BorrowState,
    expression_id: HirValueId,
    block_id: BlockId,
    current_order: i32,
    location: Option<SourceSpan>,
    aggregate_context: &mut AggregateTransferContext<'_, '_>,
) -> Result<(), BorrowCheckError> {
    let expressions = aggregate_context.expressions;
    let mut pending = vec![AggregateOwnershipStep::Visit(expression_id)];

    while let Some(step) = pending.pop() {
        match step {
            AggregateOwnershipStep::Visit(expression_id) => {
                match &expressions.expression(expression_id).kind {
                    // WHY: aggregate transfer does not walk place index expressions, so a place
                    // read is a leaf here.
                    HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => {}

                    HirExpressionKind::MapLiteral(_) | HirExpressionKind::Collection(_) => {
                        let first = expressions.push_child_values_in_source_order(
                            expression_id,
                            &mut pending,
                            AggregateOwnershipStep::AggregateChild,
                        );
                        pending.extend(first);
                    }

                    // WHY: only map and collection children can receive optional transfer advice;
                    // other children are walked as plain expressions.
                    _ => {
                        let first = expressions.push_child_values_in_source_order(
                            expression_id,
                            &mut pending,
                            AggregateOwnershipStep::Visit,
                        );
                        pending.extend(first);
                    }
                }
            }

            AggregateOwnershipStep::AggregateChild(expression_id) => {
                if matches!(
                    &expressions.expression(expression_id).kind,
                    HirExpressionKind::Load(_)
                ) {
                    record_aggregate_load_transfer(
                        layout,
                        state,
                        expression_id,
                        block_id,
                        current_order,
                        location,
                        aggregate_context,
                    )?;
                } else {
                    pending.push(AggregateOwnershipStep::Visit(expression_id));
                }
            }
        }
    }

    Ok(())
}

/// WHAT: records optional transfer for a direct place child of a map or collection literal.
/// WHY: these children can receive destruction responsibility without changing mandatory source
///      state. Computed children are visited by the walk instead.
fn record_aggregate_load_transfer(
    layout: &FunctionLayout,
    state: &BorrowState,
    expression_id: HirValueId,
    block_id: BlockId,
    current_order: i32,
    location: Option<SourceSpan>,
    aggregate_context: &mut AggregateTransferContext<'_, '_>,
) -> Result<(), BorrowCheckError> {
    let diagnostics = aggregate_context.diagnostics;
    let expression = aggregate_context.expressions.expression(expression_id);
    let HirExpressionKind::Load(place) = &expression.kind else {
        return Ok(());
    };

    let roots = roots_for_place(layout, state, place, location, expression.span, diagnostics)?;
    if roots.is_empty() {
        return Ok(());
    }

    // Scalar builtins are implicitly copied into aggregate literals. Non-scalar values keep
    // shared storage unless this site has a proven optional transfer opportunity. The implicitly
    // copied numeric scalars are `I32`, `U32` and `F64`. Other fixed widths keep their ordinary
    // transfer decision.
    if expression.ty == builtin_type_ids::BOOL
        || expression.ty == builtin_type_ids::fixed_scalar(FixedScalar::I32)
        || expression.ty == builtin_type_ids::fixed_scalar(FixedScalar::U32)
        || expression.ty == builtin_type_ids::fixed_scalar(FixedScalar::F64)
        || expression.ty == builtin_type_ids::CHAR
        || expression.ty == builtin_type_ids::NONE
    {
        return Ok(());
    }

    match classify_move_decision(layout, block_id, &roots, current_order) {
        MoveDecision::Move => {
            aggregate_context
                .value_fact_buffer
                .record_optional_transfer(expression_id, OptionalTransferStatus::Transfer, &roots);
        }
        MoveDecision::Borrow => {
            aggregate_context
                .value_fact_buffer
                .record_optional_transfer(expression_id, OptionalTransferStatus::Borrow, &roots);
        }
    }

    Ok(())
}
