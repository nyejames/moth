//! Statement/terminator transfer rules.
//!
//! This file contains the forward transfer logic for borrow checking.
//! It classifies shared vs mutable access, checks exclusivity constraints,
//! and emits statement/terminator/value facts.

use super::super::diagnostics::BorrowDiagnostics;
use super::call_semantics::{ArgEffect, resolve_call_semantics};
use super::facts::{StatementAccessTracker, ValueFactBuffer, roots_to_local_ids};
use super::{BlockTransferStats, BorrowTransferContext};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocationRetirement, BorrowState, FunctionLayout, LocalState, OrderKey, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    LocalMode, OptionalTransferStatus, StatementBorrowFact, TerminatorBorrowFact,
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
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
mod conflicts;
mod layer;
mod move_decision;
mod statement;
mod terminator;
#[cfg(test)]
mod tests;

use conflicts::{check_mutable_access, check_shared_access, probe_mutable_access};
use layer::{
    LayerValueContext, capture_layer_alias_write, capture_local_value, check_layer_place_mutation,
    check_layer_place_read, transfer_layer_alias_write,
};
use move_decision::{MoveDecision, classify_move_decision};
pub(in super::super) use statement::bind_call_result_ids;
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
    if state
        .local_state(local_index)
        .mode
        .contains(LocalMode::UNINIT)
    {
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

fn check_local_update_access(
    check: &mut AccessCheckContext<'_, '_>,
    local_index: usize,
    local_state: &LocalState,
    rhs_is_fresh: bool,
) -> Result<RootSet, BorrowCheckError> {
    // Slot writes rebind the binding cell, alias writes target the storage recorded for writes
    // (a whole-binding cell, or a projection's cell and containing allocation), and a joined
    // state must check both possibilities. Replacing a value does not mutate the value it
    // replaces, so old provenance never participates in this check. Both source assignments and
    // dedicated call results use this rule so neither operation can bypass update mutability
    // checks.
    //
    // WHY: `contains(SLOT) && !contains(ALIAS)` differs from exact `mode == SLOT` only on an
    // `UNINIT | SLOT` join, which `require_initialized_update_target` rejects first on both
    // update paths. The question here is whether the fresh value replaces the binding cell
    // itself, so only direct aliases of that cell can conflict, not whether the slot is
    // definitely initialized.
    let replaces_definite_slot = rhs_is_fresh
        && local_state.mode.contains(LocalMode::SLOT)
        && !local_state.mode.contains(LocalMode::ALIAS);
    let storage_roots = check.state.write_storage_cells(local_index);

    if !storage_roots.is_empty() {
        check_mutable_access(
            check,
            &storage_roots,
            MutableAccessPolicy {
                allow_prior_shared: true,
                require_root_mutable: true,
                strict_move_exclusivity: false,
                alias_exclusivity: if replaces_definite_slot {
                    AliasExclusivity::Direct
                } else {
                    AliasExclusivity::All
                },
            },
        )?;
    }

    Ok(storage_roots)
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

/// Which overlapping aliases block an exclusive access to a written root.
///
/// WHAT: a definite-slot rebinding to a fresh value replaces the slot's value provenance, so only
/// aliases of the binding cell itself observe that write. A value alias stored in the slot keeps
/// the older allocation alive without being rebound.
/// WHY: rebinding a result binding to a fresh allocation is legal while an alias of the old
/// allocation is live, but writing a fresh value through a source alias of the binding cell is an
/// overlapping mutation that must still be rejected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AliasExclusivity {
    /// Only direct aliases of the written root conflict.
    Direct,
    /// Every active alias of the written root conflicts.
    All,
}

#[derive(Clone, Copy)]
struct MutableAccessPolicy {
    allow_prior_shared: bool,
    require_root_mutable: bool,
    strict_move_exclusivity: bool,
    alias_exclusivity: AliasExclusivity,
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

/// Identify a first heap definition before its inputs can materialize allocation identities.
fn first_heap_definition_index(
    layout: &FunctionLayout,
    state: &BorrowState,
    block: BlockId,
    order: OrderKey,
    resolve_index: impl FnOnce() -> Option<usize>,
) -> Option<usize> {
    if layout.acyclic_entry_block == Some(block)
        && order >= 0
        && let Some(index) = resolve_index()
        && layout.local_has_in_place_mutation_surface[index]
        && layout.local_first_write_order[index] == order
        && state.has_pristine_alloc_binding(index)
    {
        // Only parameters enter this block initialized. The prefix has never written this
        // local, and a read cannot produce coarse observers of an uninitialized local.
        Some(index)
    } else {
        None
    }
}

/// WHAT: updates borrow state for a single assignment target, checking exclusivity invariants
/// before committing the new state.
///
/// WHY: assignments are the primary write site in the borrow model. Before writing, this function
/// must verify:
/// - no conflicting shared borrow of the target place exists (shared/mutable conflict)
/// - no conflicting mutable borrow exists (multiple-mutable-borrows conflict)
/// - for field/index places: the base object's borrow state is valid for the narrower access
///
/// After verification, it records the assignment fact and transitions the local's state to
/// reflect the new ownership/borrow status of the written value.
fn transfer_assign_target(
    context: &mut AssignTransferContext<'_, '_>,
    target: &HirPlace,
    value: HirValueId,
    is_definition: bool,
    local_index: Option<usize>,
    pristine_definition: Option<usize>,
) -> Result<(), BorrowCheckError> {
    let transfer_context = context.context;
    let layout = context.layout;
    let state = &mut *context.state;
    let block_id = context.block_id;
    let current_order = context.current_order;
    let tracker = &mut *context.tracker;
    let location = context.location;
    let span = context.span;
    let stats = &mut *context.stats;

    let value_expression = transfer_context.expressions.expression(value);
    // Binding role belongs to the incoming value, including native call-result loads.
    let rhs_is_place = value_expression.value_kind == ValueKind::Place;
    let is_direct_self_update = !is_definition
        && HirWriteTarget::AssignPlace(*target).is_direct_self_update_of(value_expression);
    let layer_values = LayerValueContext {
        context: transfer_context,
        layout,
        block_id,
    };

    let Some(local_index) = local_index else {
        return Err(transfer_context.diagnostics.internal_error(format!(
            "Assignment target local '{}' is not in the active function layout",
            transfer_context.diagnostics.local_name(target.root)
        )));
    };
    if !is_definition {
        require_initialized_update_target(
            layout,
            state,
            local_index,
            &transfer_context.diagnostics,
            span.or(location),
        )?;
    }

    match target.projections.is_empty() {
        true => {
            let local_state = state.local_state(local_index).clone();
            // A mutable binding receiving a place becomes a mutable alias of it.
            let mutable_alias = rhs_is_place
                && layout.local_mutable[local_index]
                && !is_compiler_owned_scratch_local(transfer_context, layout, local_index);
            let rhs_roots = direct_value_roots_from_expression(
                layout,
                state,
                transfer_context.expressions,
                value,
                location,
                value_expression.span,
                &transfer_context.diagnostics,
            )?;
            let rhs_direct_alias_roots = rhs_roots.as_ref().map(|_| {
                direct_root_aliases_from_expression(
                    layout,
                    state,
                    transfer_context.expressions,
                    value,
                )
            });
            let rhs_write_target_roots = rhs_roots.as_ref().map(|_| {
                mutable_alias_write_targets_from_expression(
                    layout,
                    state,
                    transfer_context.expressions,
                    value,
                )
            });

            if let Some(rhs_roots) = rhs_roots.as_ref().filter(|roots| !roots.is_empty()) {
                let can_attempt_move = local_state.mode.is_definitely_uninit()
                    && layout.local_mutable[local_index]
                    && rhs_roots
                        .iter_ones()
                        .all(|root_index| layout.local_mutable[root_index]);

                if can_attempt_move {
                    // WHAT: assignments can receive optional transfer responsibility when the target
                    //      takes a fresh slot.
                    // WHY: this records an optional transfer candidate while keeping source-visible
                    //      initialization and alias state available for conservative validation.
                    match classify_move_decision(layout, block_id, rhs_roots, current_order) {
                        MoveDecision::Borrow => context.value_fact_buffer.record_optional_transfer(
                            value,
                            OptionalTransferStatus::Borrow,
                            rhs_roots,
                        ),
                        MoveDecision::Move => context.value_fact_buffer.record_optional_transfer(
                            value,
                            OptionalTransferStatus::Transfer,
                            rhs_roots,
                        ),
                    }
                }
            }

            if is_definition {
                let new_state = match rhs_roots {
                    Some(rhs_roots) => {
                        let target_is_mutable = layout.local_mutable[local_index];
                        let target_is_compiler_owned_scratch =
                            is_compiler_owned_scratch_local(transfer_context, layout, local_index);
                        // WHAT: a fresh compiler-owned scratch/result local carries a shared alias
                        // through compiler-introduced control flow (notably fallible catch joins)
                        // without requesting source-exclusive access. Ordinary user mutable bindings
                        // keep their exclusive-access rules.
                        // WHY: treating scratch-to-scratch alias transfer as a source mutation
                        //      produced a false AliasedValueRequiresExclusiveAccess at the producing
                        //      get and stopped the alias before it could reach the user binding.
                        if target_is_mutable && rhs_is_place && !target_is_compiler_owned_scratch {
                            let mut check = AccessCheckContext {
                                context: transfer_context,
                                layout,
                                state: &*state,
                                block_id,
                                tracker,
                                location,
                                span: value_expression.span,
                                stats,
                                actor_index_hint: Some(local_index),
                                current_order,
                            };
                            let require_root_mutable =
                                check_mutable_alias_source(&check, &value_expression.kind)?;
                            check_mutable_access(
                                &mut check,
                                &rhs_roots,
                                MutableAccessPolicy {
                                    allow_prior_shared: true,
                                    require_root_mutable,
                                    strict_move_exclusivity: false,
                                    alias_exclusivity: AliasExclusivity::All,
                                },
                            )?;
                            if let HirExpressionKind::Load(place) = &value_expression.kind {
                                check_layer_place_mutation(&mut check, place, false)?;
                            }
                        }

                        let direct_roots = rhs_direct_alias_roots
                            .unwrap_or_else(|| RootSet::empty(layout.local_count()));
                        let write_target_roots = if rhs_is_place {
                            if target_is_mutable {
                                rhs_write_target_roots
                                    .unwrap_or_else(|| RootSet::empty(layout.local_count()))
                            } else {
                                // An immutable alias cannot write through, but a projection-free
                                // alias of another alias must keep the same write targets as
                                // overlap identity. A projected load is different: it observes its
                                // containing allocation through value provenance, not the base
                                // binding cell.
                                inherited_alias_targets_from_expression(
                                    layout,
                                    state,
                                    transfer_context.expressions,
                                    value,
                                )
                            }
                        } else {
                            RootSet::empty(layout.local_count())
                        };
                        let mut new_state = if rhs_is_place {
                            LocalState::alias_with_cells(
                                rhs_roots,
                                direct_roots,
                                write_target_roots,
                            )
                        } else {
                            LocalState::slot_with_value_roots(rhs_roots, direct_roots)
                        };
                        new_state.derived_from_roots = alias_derivation_roots_from_expression(
                            layout,
                            state,
                            transfer_context.expressions,
                            value,
                        );
                        new_state
                    }
                    None => LocalState::slot(layout.local_count()),
                };
                let mut captured = capture_local_value(
                    &layer_values,
                    state,
                    value,
                    mutable_alias,
                    value_expression.span.or(location),
                )?;
                if let Some(retirement) = state.rebind_local_after_capture(
                    local_index,
                    new_state,
                    current_order,
                    pristine_definition,
                    false,
                ) {
                    captured.remap(state, retirement);
                }
                captured.apply(state, local_index);
                return Ok(());
            }

            let rhs_is_fresh = rhs_roots.as_ref().is_none_or(RootSet::is_empty);
            let mut check = AccessCheckContext {
                context: transfer_context,
                layout,
                state: &*state,
                block_id,
                tracker,
                location,
                span,
                stats,
                actor_index_hint: Some(local_index),
                current_order,
            };
            let write_roots =
                check_local_update_access(&mut check, local_index, &local_state, rhs_is_fresh)?;
            tracker.record_write(&write_roots);

            if is_direct_self_update {
                // The ordinary write check above covers this place. Rebinding it to itself adds
                // no new alias relationship, so preserve the binding mode and value provenance.
                return Ok(());
            }

            let target_can_acquire_mutable_alias = layout.local_mutable[local_index]
                && !is_compiler_owned_scratch_local(transfer_context, layout, local_index);
            if target_can_acquire_mutable_alias
                && local_state.mode.contains(LocalMode::SLOT)
                && rhs_is_place
                && let Some(rhs_roots) = rhs_roots.as_ref().filter(|roots| !roots.is_empty())
            {
                // A slot path receiving a Place becomes a mutable alias. Validate that incoming
                // relationship before the destination's previous binding state is replaced. Roots
                // already checked as part of this write must not issue a second overlapping access.
                let mut unchecked_roots = rhs_roots.clone();
                unchecked_roots.subtract_with(&write_roots);
                if !unchecked_roots.is_empty() {
                    let mut check = AccessCheckContext {
                        context: transfer_context,
                        layout,
                        state: &*state,
                        block_id,
                        tracker,
                        location,
                        span: value_expression.span,
                        stats,
                        actor_index_hint: Some(local_index),
                        current_order,
                    };
                    check_mutable_access(
                        &mut check,
                        &unchecked_roots,
                        MutableAccessPolicy {
                            allow_prior_shared: true,
                            require_root_mutable: true,
                            strict_move_exclusivity: false,
                            alias_exclusivity: AliasExclusivity::All,
                        },
                    )?;
                }
            }

            let rhs_derivation = alias_derivation_roots_from_expression(
                layout,
                state,
                transfer_context.expressions,
                value,
            );

            match (
                local_state.mode.contains(LocalMode::SLOT),
                local_state.mode.contains(LocalMode::ALIAS),
            ) {
                (has_slot, true) => {
                    // The alias path writes through; a simultaneous slot path makes each
                    // referent only a possible destination.
                    transfer_layer_alias_write(
                        &mut AccessCheckContext {
                            context: transfer_context,
                            layout,
                            state: &*state,
                            block_id,
                            tracker,
                            location,
                            span,
                            stats,
                            actor_index_hint: Some(local_index),
                            current_order,
                        },
                        local_index,
                    )?;
                    let captured = capture_layer_alias_write(
                        &layer_values,
                        state,
                        local_index,
                        &local_state,
                        value,
                        has_slot && mutable_alias,
                        value_expression.span.or(location),
                    )?;
                    let (observed, retirement) = refresh_alias_referents(
                        state,
                        local_index,
                        &local_state,
                        rhs_roots.as_ref(),
                        current_order,
                        captured.has_facts(),
                        captured.publishes_weak_whole_ids(),
                    );
                    captured.apply(state, local_index, &local_state, retirement);
                    if !has_slot {
                        return Ok(());
                    }

                    // Both alternatives now hold the incoming value. Keep their union of
                    // storage cells, since the joined root state has no per-path partition.
                    let mut direct_alias_roots = local_state.direct_alias_roots;
                    let mut write_target_roots = local_state.write_target_roots;
                    if let Some(rhs_direct_roots) = rhs_direct_alias_roots {
                        direct_alias_roots.union_with(&rhs_direct_roots);
                    }
                    if let Some(rhs_target_roots) = rhs_write_target_roots {
                        write_target_roots.union_with(&rhs_target_roots);
                    }

                    let mut derived_from_roots = local_state.derived_from_roots;
                    derived_from_roots.intersect_with(&rhs_derivation);
                    state.update_local_state(
                        local_index,
                        LocalState {
                            mode: LocalMode::SLOT.union(LocalMode::ALIAS),
                            value_roots: observed,
                            direct_alias_roots,
                            write_target_roots,
                            derived_from_roots,
                        },
                    );
                }

                (true, false) => {
                    let mut captured = capture_local_value(
                        &layer_values,
                        state,
                        value,
                        mutable_alias,
                        value_expression.span.or(location),
                    )?;
                    let mut new_state = match rhs_roots {
                        Some(roots) if rhs_is_place => LocalState::alias_with_cells(
                            roots,
                            rhs_direct_alias_roots
                                .unwrap_or_else(|| RootSet::empty(layout.local_count())),
                            rhs_write_target_roots
                                .unwrap_or_else(|| RootSet::empty(layout.local_count())),
                        ),
                        Some(roots) if !roots.is_empty() => LocalState::slot_with_value_roots(
                            roots,
                            rhs_direct_alias_roots
                                .unwrap_or_else(|| RootSet::empty(layout.local_count())),
                        ),
                        _ => LocalState::slot(layout.local_count()),
                    };
                    new_state.derived_from_roots = rhs_derivation;
                    let retirement = if captured.has_facts() && state.layer_is_empty() {
                        // The captured RHS may be the first sparse fact about the old allocation.
                        let retirement =
                            state.retire_binding_allocation(local_index, current_order);
                        state.update_local_state(local_index, new_state);
                        retirement
                    } else {
                        state.rebind_local(local_index, new_state, current_order)
                    };
                    state.release_projected_observers(local_index, current_order);
                    captured.remap(state, retirement);
                    captured.apply(state, local_index);
                }

                (false, false) => {
                    let mut new_state = LocalState::slot(layout.local_count());
                    new_state.derived_from_roots = rhs_derivation;
                    state.rebind_local(local_index, new_state, current_order);
                }
            }
        }

        false => {
            // The projection writes through the base binding. Cell roots decide mutability and
            // cell overlap; value provenance still names whatever allocation the place observes.
            let roots = access_roots_for_place(
                layout,
                state,
                target,
                location,
                span,
                &transfer_context.diagnostics,
            )?;
            tracker.record_write(&roots);
            let mut check = AccessCheckContext {
                context: transfer_context,
                layout,
                state: &*state,
                block_id,
                tracker,
                location,
                span,
                stats,
                actor_index_hint: Some(local_index),
                current_order,
            };
            let field_write =
                layer::check_layer_field_write(&mut check, local_index, target, &roots, value)?;
            if let Some(field_write) = field_write {
                field_write.apply(&layer_values, state, value)?;
            }
        }
    }

    Ok(())
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

/// Record the value a write through an alias stores in its referent cells and return the value
/// the alias now observes.
///
/// WHAT: a non-fresh write stores the incoming provenance. A fresh write stores an allocation
///       owned by the referent storage, so the alias observes the designated cells themselves.
/// WHY: a previous projection observed an older allocation stored in the cell, and an alias-only
///      binding has no own root in `effective_roots`. Without recording the referent storage, a
///      returned value or projected argument of the fresh write observed nothing and missed later
///      exclusive accesses. A write through a joined state can reach its own slot or several
///      referents, so it only unions provenance into the possible cells. Only a pure alias of
///      exactly one whole cell, with no other write target, definitely replaces that cell's value.
///      A projected-origin alias writes one field of the base value, so the base's allocation
///      provenance stays unchanged.
fn refresh_alias_referents(
    state: &mut BorrowState,
    local_index: usize,
    alias_state: &LocalState,
    new_value_roots: Option<&RootSet>,
    order: OrderKey,
    preserve_allocation: bool,
    publishes_weak_whole_ids: bool,
) -> (RootSet, Option<AllocationRetirement>) {
    let provenance = new_value_roots.filter(|roots| !roots.is_empty());
    let storage_identity = state.write_storage_cells(local_index);
    let observed = match provenance {
        Some(roots) => roots.clone(),
        None => storage_identity,
    };
    let retirement = if let Some(cell_index) =
        definite_slot_referent(state, local_index, alias_state)
    {
        let retirement = (preserve_allocation || !state.layer_is_empty())
            .then(|| state.retire_binding_allocation(cell_index, order));
        let local_count = state.local_state(cell_index).value_roots.bit_len();
        state.local_state_mut(cell_index).value_roots = provenance
            .cloned()
            .unwrap_or_else(|| RootSet::empty(local_count));
        retirement
    } else {
        if let Some(roots) = provenance {
            for cell_index in alias_state.direct_alias_roots.iter_ones() {
                if cell_index == local_index {
                    continue;
                }
                if publishes_weak_whole_ids
                    && state.local_state(cell_index).mode.contains(LocalMode::SLOT)
                    && !state.has_layer_binding(cell_index)
                {
                    // Capture only the old allocations. The incoming coarse roots may name
                    // a containing allocation rather than the exact child the sparse write uses.
                    let (old_ids, _) = state.local_alloc_ids(cell_index);
                    state.bind_alloc_ids(cell_index, &old_ids, false);
                }
                let referent = state.local_state_mut(cell_index);
                if referent.mode.contains(LocalMode::SLOT) {
                    // Its old value survives on paths that write elsewhere. Make the
                    // implicit slot root explicit before the incoming roots displace it.
                    if !referent.mode.contains(LocalMode::ALIAS) && referent.value_roots.is_empty()
                    {
                        referent.value_roots.insert(cell_index);
                    }
                    referent.value_roots.union_with(roots);
                }
            }
        }
        None
    };
    state.local_state_mut(local_index).value_roots = observed.clone();
    (observed, retirement)
}

/// The one whole cell a write through `alias_state` definitely replaces.
///
/// WHAT: A definite referent means every path writes one whole cell.
/// WHY: one direct cell is not definite when the joined state also carries its own slot or a
///      write target for another cell; that path leaves the direct cell's value untouched.
fn definite_referent(alias_state: &LocalState) -> Option<usize> {
    let mut direct_cells = alias_state.direct_alias_roots.iter_ones();
    match (direct_cells.next(), direct_cells.next()) {
        (Some(cell_index), None)
            if alias_state.mode.contains(LocalMode::ALIAS)
                && !alias_state.mode.contains(LocalMode::SLOT)
                && alias_state
                    .write_target_roots
                    .iter_ones()
                    .all(|target| target == cell_index) =>
        {
            Some(cell_index)
        }
        _ => None,
    }
}

/// A definite whole-cell update also requires that the target itself is a definite slot.
fn definite_slot_referent(
    state: &BorrowState,
    local_index: usize,
    alias_state: &LocalState,
) -> Option<usize> {
    definite_referent(alias_state).filter(|&cell| {
        cell != local_index
            && state.local_state(cell).mode == LocalMode::SLOT
            && state.alias_cells(local_index).is_empty()
    })
}

fn mutable_argument_roots(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &BorrowState,
    expression_id: HirValueId,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> Result<RootSet, BorrowCheckError> {
    let expression = context.expressions.expression(expression_id);
    if let HirExpressionKind::Load(place) = &expression.kind {
        // A mutable parameter may write the caller's binding cell and, for a heap-allocated
        // value, mutate shared storage. Cell roots decide mutability and cell overlap; value
        // provenance decides allocation overlap.
        //
        // WHAT: designated cells always participate; value provenance participates only when the
        //       type has an in-place mutation surface.
        // WHY: a scalar value copied into a definite slot owns its own cell, so immutable
        //      provenance does not weaken the write check. Mixed `SLOT | ALIAS` states need the
        //      alias branch's cells too, which `effective_roots` does not report.
        if place.projections.is_empty()
            && let Some(local_index) = layout.index_of(place.root)
        {
            let mut roots = state.designated_cells(local_index);
            if layout.local_has_in_place_mutation_surface[local_index] {
                roots.union_with(&state.local_state(local_index).value_roots);
            }
            if !roots.is_empty() {
                return Ok(roots);
            }
        }

        return roots_for_place(
            layout,
            state,
            place,
            location,
            expression.span,
            &context.diagnostics,
        );
    }

    let mut roots = RootSet::empty(layout.local_count());
    collect_expression_roots(
        context,
        layout,
        state,
        expression_id,
        &mut roots,
        location,
        span,
    )?;
    Ok(roots)
}

fn direct_place_from_expression(
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> Option<HirPlace> {
    match &expressions.expression(expression_id).kind {
        HirExpressionKind::Load(place) => Some(*place),
        _ => None,
    }
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

/// A shared binding cannot grant exclusive access. For a whole scalar cell, value roots
/// still govern overlap but can include immutable provenance unrelated to cell mutability.
fn check_mutable_alias_source(
    check: &AccessCheckContext<'_, '_>,
    source: &HirExpressionKind,
) -> Result<bool, BorrowCheckError> {
    if let HirExpressionKind::Load(place) = source
        && let Some(local_index) = place_root_local_index(check.layout, place)
    {
        require_mutable_local(
            check.layout,
            local_index,
            &check.context.diagnostics,
            check.span.or(check.location),
        )?;
        return Ok(!place.projections.is_empty()
            || check.layout.local_has_in_place_mutation_surface[local_index]);
    }
    Ok(true)
}

fn direct_value_roots_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
    location: Option<SourceSpan>,
    _span: Option<SourceSpan>,
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

/// Cell index of a projection-free definite slot, for overlap identity.
///
/// WHAT: reports the slot's own binding cell, never allocation provenance.
/// WHY: exclusivity and same-call overlap ask which cells a place touches, while value
///      provenance asks which immutable origins a value came from. Keeping them separate
///      prevents a cell bit from travelling through call results into fresh scalars.
fn place_cell_root(
    layout: &FunctionLayout,
    state: &BorrowState,
    place: &HirPlace,
) -> Option<usize> {
    if !place.projections.is_empty() {
        return None;
    }
    let local_index = layout.index_of(place.root)?;
    let mode = state.local_state(local_index).mode;
    (mode.contains(LocalMode::SLOT) && !mode.contains(LocalMode::ALIAS)).then_some(local_index)
}

fn direct_root_aliases_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> RootSet {
    let mut direct_roots = RootSet::empty(layout.local_count());
    let Some((source_index, place)) = place_load_source(layout, expressions, expression_id) else {
        return direct_roots;
    };

    // A projected load observes the value stored under a projection: the older allocation, which
    // a later rebinding of the source binding cannot replace. Only the source place's value
    // provenance records that observation, so a projection must not inherit the source cell's
    // direct alias identity. A later whole-binding replacement then stays legal while the
    // projected value is live.
    if !place.projections.is_empty() {
        return direct_roots;
    }

    // A place load binds the source place's own cell when that source is a definite slot. Cell
    // identity is overlap information only; it must not enter the value's provenance roots or a
    // returned scalar would inherit a later slot write.
    if let Some(cell_index) = place_cell_root(layout, state, &place) {
        direct_roots.insert(cell_index);
    }
    let source_state = state.local_state(source_index);
    if source_state.mode.contains(LocalMode::ALIAS) {
        // An alias source carries the referent cells through to the new binding. Chained aliases
        // must keep observing the same cells, while a slot-backed value alias contributes no
        // direct roots and keeps only its allocation provenance.
        direct_roots.union_with(&source_state.direct_alias_roots);
    }

    direct_roots
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

/// Write targets an immutable alias inherits from another alias.
///
/// WHAT: the source alias's write targets for a projection-free load; empty for a projected load
///       or a slot source.
/// WHY: `view = writer` observes the storage `writer` writes through, so a replacement through
///      `writer` must see the live view. `old = holder.first` observes its containing allocation
///      through value provenance rather than the holder binding cell, so it inherits no cell
///      identity.
fn inherited_alias_targets_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> RootSet {
    let Some((source_index, place)) = place_load_source(layout, expressions, expression_id) else {
        return RootSet::empty(layout.local_count());
    };
    if !place.projections.is_empty() {
        return RootSet::empty(layout.local_count());
    }
    let source_state = state.local_state(source_index);
    if source_state.mode.contains(LocalMode::ALIAS) {
        source_state.write_target_roots.clone()
    } else {
        RootSet::empty(layout.local_count())
    }
}

/// Storage a mutable alias acquired from this expression writes through.
///
/// WHAT: a whole-binding load targets the source binding's designated cells; a projected load
///       targets those same cells plus the containing allocation(s) the source value observed at
///       binding time.
/// WHY: the cell is the pre-correction conservative identity. A fresh allocation's identity is
///      encoded as its owning cell or as empty provenance, and that identity cannot be carried
///      accurately through every join, alias chain, call or scope exit, so keeping the cell
///      retains the rejections that guard those paths. The allocation roots add the containing
///      allocation family, which publishes a write through a call-returned holder to the
///      parameter that owns it and overlaps other holders of the same allocation. A whole-binding
///      alias always writes through the source binding cells, so it keeps just the cell identity.
fn mutable_alias_write_targets_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> RootSet {
    let Some((source_index, place)) = place_load_source(layout, expressions, expression_id) else {
        return RootSet::empty(layout.local_count());
    };
    let mut targets = state.designated_cells(source_index);
    if !place.projections.is_empty() {
        targets.union_with(&state.effective_roots(source_index));
    }
    targets
}

/// Roots a shared access to `place` must check for overlap.
///
/// WHAT: the place's value provenance plus the cells it designates for access.
/// WHY: value provenance alone leaves a cell-only alias with no roots after pruning, and a place
///      reached through a projected or whole-binding alias also designates the storage that alias
///      writes through. Value flow keeps using `roots_for_place`; these roots only feed access
///      checks.
fn access_roots_for_place(
    layout: &FunctionLayout,
    state: &BorrowState,
    place: &HirPlace,
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<RootSet, BorrowCheckError> {
    let mut roots = roots_for_place(layout, state, place, location, span, diagnostics)?;
    include_designated_cells(layout, state, place, &mut roots);
    Ok(roots)
}

/// Add the cells a place designates for access to an existing root set.
///
/// WHY: value facts and access checks share one rule for turning a place into cell roots, so a
///      cell-only alias cannot be visible to one caller and invisible to another.
fn include_designated_cells(
    layout: &FunctionLayout,
    state: &BorrowState,
    place: &HirPlace,
    roots: &mut RootSet,
) {
    if let Some(local_index) = layout.index_of(place.root) {
        roots.union_with(&state.designated_cells(local_index));
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

    let local_state = state.local_state(local_index);
    if local_state.mode.is_definitely_uninit() {
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
                    let place_roots = check_shared_place_read(env, place, value_span)?;
                    roots.union_with(&place_roots);
                    record_shared_read_classification(env, expression_id, roots);
                }

                HirExpressionKind::Copy(place) => {
                    let place_roots = check_shared_place_read(env, place, value_span)?;
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
    place: &HirPlace,
    value_span: Option<SourceSpan>,
) -> Result<RootSet, BorrowCheckError> {
    // Value facts keep reporting the allocation the read observed. Access overlap also includes
    // the place's cells. The active-exclusive scan covers both the provenance roots the read
    // observes and the binding cells the place designates, so a live exclusive alias of either
    // conflicts unless this read provably derives through it. Mutating through a derived alias
    // still conflicts at the mutation's overlap check.
    let value_roots = roots_for_place(
        env.layout,
        env.state,
        place,
        value_span,
        value_span,
        &env.context.diagnostics,
    )?;
    let mut access_roots = value_roots.clone();
    include_designated_cells(env.layout, env.state, place, &mut access_roots);
    let actor_index_hint = place_root_local_index(env.layout, place);
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
    check_shared_access(&mut check, &access_roots)?;
    check_layer_place_read(&check, place)?;
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
