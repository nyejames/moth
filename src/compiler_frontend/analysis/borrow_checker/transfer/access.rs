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
    BorrowState, FunctionLayout, LocalState, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    LocalMode, OptionalTransferStatus, ReactiveInvalidationFact, ReactiveInvalidationKind,
    ReactivePlaceWriteKind, StatementBorrowFact, TerminatorBorrowFact, ValueAccessClassification,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};

mod conflicts;
mod move_decision;
mod statement;
mod terminator;

use conflicts::{check_mutable_access, check_shared_access, probe_mutable_access};
use move_decision::{MoveDecision, classify_move_decision};
pub(super) use statement::transfer_statement;
pub(super) use terminator::transfer_terminator;

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
    location: SourceLocation,
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
    location: SourceLocation,
    stats: &'a mut BlockTransferStats,
    actor_index_hint: Option<usize>,
    current_order: i32,
}

#[derive(Clone, Copy)]
struct MutableAccessPolicy {
    allow_prior_shared: bool,
    require_root_mutable: bool,
    strict_move_exclusivity: bool,
    /// Whether to check alias exclusivity for the written roots.
    /// WHAT: local-target rebinding to a fresh value releases the old allocation without
    /// mutating it, so aliases of the old allocation should not block the rebinding.
    /// WHY: a binding-versus-allocation modelling fix. Without this flag, the alias count
    /// check rejects rebinding when a returned alias of the old allocation is live.
    check_alias_exclusivity: bool,
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
    location: SourceLocation,
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
        let location = env.location.clone();
        record_shared_reads_in_expression(
            env,
            expression,
            location,
            &mut RootSet::empty(env.layout.local_count()),
        )?;
    }

    if let Some(guard) = &arm.guard {
        let location = env.location.clone();
        record_shared_reads_in_expression(
            env,
            guard,
            location,
            &mut RootSet::empty(env.layout.local_count()),
        )?;
    }

    Ok(())
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
    value: &HirExpression,
) -> Result<(), BorrowCheckError> {
    let transfer_context = context.context;
    let layout = context.layout;
    let state = &mut *context.state;
    let block_id = context.block_id;
    let current_order = context.current_order;
    let tracker = &mut *context.tracker;
    let location = context.location.clone();
    let stats = &mut *context.stats;

    match target {
        HirPlace::Local(local_id) => {
            let Some(local_index) = layout.index_of(*local_id) else {
                return Err(transfer_context.diagnostics.internal_error(
                    format!(
                        "Assignment target local '{}' is not in the active function layout",
                        transfer_context.diagnostics.local_name(*local_id)
                    ),
                    location.clone(),
                ));
            };

            let local_state = state.local_state(local_index).clone();
            let rhs_provenance = direct_value_provenance_from_expression(
                layout,
                state,
                value,
                location.clone(),
                &transfer_context.diagnostics,
            )?;
            let rhs_direct_alias_roots = rhs_provenance.as_ref().map(|provenance| {
                direct_root_aliases_from_expression(layout, state, value, &provenance.roots)
            });

            if let Some(rhs_roots) = rhs_provenance
                .as_ref()
                .map(|provenance| &provenance.roots)
                .filter(|roots| !roots.is_empty())
            {
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
                            value.id,
                            OptionalTransferStatus::Borrow,
                            rhs_roots,
                        ),
                        MoveDecision::Move => context.value_fact_buffer.record_optional_transfer(
                            value.id,
                            OptionalTransferStatus::Transfer,
                            rhs_roots,
                        ),
                    }
                }
            }

            if local_state.mode.is_definitely_uninit() {
                match rhs_provenance {
                    Some(provenance) => {
                        let rhs_roots = provenance.roots;
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
                        if target_is_mutable
                            && !rhs_roots.is_empty()
                            && !provenance.slot_backed
                            && !target_is_compiler_owned_scratch
                        {
                            let mut check = AccessCheckContext {
                                context: transfer_context,
                                layout,
                                state: &*state,
                                block_id,
                                tracker,
                                location: location.clone(),
                                stats,
                                actor_index_hint: Some(local_index),
                                current_order,
                            };
                            check_mutable_access(
                                &mut check,
                                &rhs_roots,
                                MutableAccessPolicy {
                                    allow_prior_shared: true,
                                    require_root_mutable: false,
                                    strict_move_exclusivity: false,
                                    check_alias_exclusivity: true,
                                },
                            )?;
                        }

                        let direct_roots = rhs_direct_alias_roots
                            .unwrap_or_else(|| RootSet::empty(layout.local_count()));
                        let new_state = if provenance.slot_backed {
                            LocalState::slot_with_value_roots(rhs_roots, direct_roots)
                        } else {
                            LocalState::alias_with_direct(rhs_roots, direct_roots)
                        };
                        state.update_local_state(local_index, new_state);
                    }
                    None => {
                        state.update_local_state(
                            local_index,
                            LocalState::slot(layout.local_count()),
                        );
                    }
                }
                return Ok(());
            }

            // A fresh RHS (no alias roots) means the rebinding replaces the binding's target
            // with a new allocation. The old allocation stays alive through any aliases, so
            // alias exclusivity should not block the rebinding.
            let rhs_is_fresh = rhs_provenance
                .as_ref()
                .is_none_or(|provenance| provenance.roots.is_empty());

            let mut write_roots = RootSet::empty(layout.local_count());
            if local_state.mode.contains(LocalMode::SLOT) {
                write_roots.insert(local_index);
            }
            // An alias-only binding writes through to its current allocation. A slot-backed
            // binding instead replaces its cell, so its previous value roots must not block the
            // rebind even when the new RHS aliases a different existing allocation.
            if local_state.mode.contains(LocalMode::ALIAS) && local_state.has_value_aliases() {
                write_roots.union_with(&local_state.value_roots);
            }

            let mut check = AccessCheckContext {
                context: transfer_context,
                layout,
                state: &*state,
                block_id,
                tracker,
                location: location.clone(),
                stats,
                actor_index_hint: Some(local_index),
                current_order,
            };
            check_mutable_access(
                &mut check,
                &write_roots,
                MutableAccessPolicy {
                    allow_prior_shared: true,
                    require_root_mutable: true,
                    strict_move_exclusivity: false,
                    check_alias_exclusivity: !rhs_is_fresh,
                },
            )?;

            match (
                local_state.mode.contains(LocalMode::SLOT),
                local_state.mode.contains(LocalMode::ALIAS),
            ) {
                (false, true) => {
                    // Alias-view writes through to referent and does not rebind.
                }

                (true, false) => {
                    apply_slot_rebinding(
                        state,
                        layout.local_count(),
                        local_index,
                        rhs_provenance,
                        rhs_direct_alias_roots,
                    );
                }

                (true, true) => {
                    if rhs_is_fresh {
                        // Rebinding to a fresh value releases the old allocation. The binding
                        // becomes a pure slot with no alias roots.
                        apply_slot_rebinding(
                            state,
                            layout.local_count(),
                            local_index,
                            rhs_provenance,
                            rhs_direct_alias_roots,
                        );
                    } else {
                        let mut value_roots = local_state.value_roots;
                        let mut direct_alias_roots = local_state.direct_alias_roots;
                        if let Some(provenance) = rhs_provenance {
                            value_roots.union_with(&provenance.roots);
                        }
                        if let Some(rhs_direct_roots) = rhs_direct_alias_roots {
                            direct_alias_roots.union_with(&rhs_direct_roots);
                        }

                        state.update_local_state(
                            local_index,
                            LocalState {
                                mode: LocalMode::SLOT.union(LocalMode::ALIAS),
                                value_roots,
                                direct_alias_roots,
                            },
                        );
                    }
                }

                (false, false) => {
                    state.update_local_state(local_index, LocalState::slot(layout.local_count()));
                }
            }
        }

        _ => {
            let roots = roots_for_place(
                layout,
                state,
                target,
                location.clone(),
                &transfer_context.diagnostics,
            )?;
            let mut check = AccessCheckContext {
                context: transfer_context,
                layout,
                state: &*state,
                block_id,
                tracker,
                location: location.clone(),
                stats,
                actor_index_hint: place_root_local_index(layout, target),
                current_order,
            };
            check_mutable_access(
                &mut check,
                &roots,
                MutableAccessPolicy {
                    allow_prior_shared: true,
                    require_root_mutable: true,
                    strict_move_exclusivity: false,
                    check_alias_exclusivity: true,
                },
            )?;
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

fn apply_slot_rebinding(
    state: &mut BorrowState,
    local_count: usize,
    local_index: usize,
    rhs_provenance: Option<DirectValueProvenance>,
    rhs_direct_alias_roots: Option<RootSet>,
) {
    match rhs_provenance {
        Some(provenance) if !provenance.roots.is_empty() => {
            let direct_roots =
                rhs_direct_alias_roots.unwrap_or_else(|| RootSet::empty(local_count));
            state.update_local_state(
                local_index,
                LocalState::slot_with_value_roots(provenance.roots, direct_roots),
            )
        }
        None => state.update_local_state(local_index, LocalState::slot(local_count)),
        Some(_) => state.update_local_state(local_index, LocalState::slot(local_count)),
    }
}

fn place_root_local_index(layout: &FunctionLayout, place: &HirPlace) -> Option<usize> {
    match place {
        HirPlace::Local(local_id) => layout.index_of(*local_id),
        HirPlace::Field { base, .. } | HirPlace::Index { base, .. } => {
            place_root_local_index(layout, base)
        }
    }
}

fn mutable_argument_roots(
    layout: &FunctionLayout,
    state: &BorrowState,
    expression: &HirExpression,
    location: SourceLocation,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<RootSet, BorrowCheckError> {
    if let HirExpressionKind::Load(place) = &expression.kind {
        return roots_for_place(layout, state, place, location, diagnostics);
    }

    let mut roots = RootSet::empty(layout.local_count());
    collect_expression_roots(layout, state, expression, &mut roots, location, diagnostics)?;
    Ok(roots)
}

// WHAT: identifies a call argument that still denotes one direct HIR place after a transparent
//      result projection.
// WHY: optional transfer must decide move-versus-borrow on the place itself. A fallible success
//      unwrap around a load must not first register an independent shared read of that same place.
//      Computed and aggregate expressions intentionally stay on the recursive read path.
fn transparent_place_from_expression(expression: &HirExpression) -> Option<&HirPlace> {
    match &expression.kind {
        HirExpressionKind::Load(place) => Some(place),
        HirExpressionKind::FallibleUnwrapSuccess { result } => {
            transparent_place_from_expression(result)
        }
        _ => None,
    }
}

#[derive(Debug)]
struct DirectValueProvenance {
    roots: RootSet,
    slot_backed: bool,
}

fn direct_value_provenance_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expression: &HirExpression,
    location: SourceLocation,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<Option<DirectValueProvenance>, BorrowCheckError> {
    // WHAT: a fallible success unwrap carries the success payload's alias roots through the
    //      carrier local, so a map `get` result keeps aliasing the map across the catch join.
    // WHY: without this, the shared alias established at `get` is dropped at the unwrap and
    //      never reaches the user binding, so a later mutation is not blocked.
    match &expression.kind {
        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::TupleGet { tuple: result, .. } => {
            return direct_value_provenance_from_expression(
                layout,
                state,
                result,
                location,
                diagnostics,
            );
        }
        HirExpressionKind::TupleConstruct { elements } => {
            let mut roots = RootSet::empty(layout.local_count());
            for element in elements {
                if let Some(provenance) = direct_value_provenance_from_expression(
                    layout,
                    state,
                    element,
                    location.clone(),
                    diagnostics,
                )? {
                    roots.union_with(&provenance.roots);
                }
            }

            return Ok(Some(DirectValueProvenance {
                roots,
                // The tuple itself always enters a binding slot. Its elements may retain older
                // allocations, which the union above records conservatively.
                slot_backed: true,
            }));
        }
        _ => {}
    }

    let HirExpressionKind::Load(place) = &expression.kind else {
        return Ok(None);
    };

    if expression.value_kind == ValueKind::Place {
        return Ok(Some(DirectValueProvenance {
            roots: roots_for_place(layout, state, place, location, diagnostics)?,
            slot_backed: false,
        }));
    }

    if let HirPlace::Local(local_id) = place
        && let Some(local_index) = layout.index_of(*local_id)
        && state.local_state(local_index).has_value_aliases()
    {
        let source_state = state.local_state(local_index);
        return Ok(Some(DirectValueProvenance {
            roots: roots_for_place(layout, state, place, location, diagnostics)?,
            slot_backed: source_state.mode.contains(LocalMode::SLOT),
        }));
    }

    Ok(None)
}

fn direct_root_aliases_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expression: &HirExpression,
    rhs_roots: &RootSet,
) -> RootSet {
    let mut direct_roots = RootSet::empty(layout.local_count());

    if expression.value_kind != ValueKind::Place {
        return direct_roots;
    }

    let HirExpressionKind::Load(place) = &expression.kind else {
        return direct_roots;
    };

    let HirPlace::Local(source_local_id) = place else {
        return direct_roots;
    };

    let Some(source_index) = layout.index_of(*source_local_id) else {
        return direct_roots;
    };

    let source_state = state.local_state(source_index);
    if source_state.mode.contains(LocalMode::SLOT) && rhs_roots.contains(source_index) {
        direct_roots.insert(source_index);
    }

    direct_roots
}

fn roots_for_place(
    layout: &FunctionLayout,
    state: &BorrowState,
    place: &HirPlace,
    location: SourceLocation,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<RootSet, BorrowCheckError> {
    increment_frontend_counter(FrontendCounter::BorrowPlaceAccessCount);

    match place {
        HirPlace::Local(local_id) => {
            let Some(local_index) = layout.index_of(*local_id) else {
                return Err(diagnostics.internal_error(
                    format!(
                        "Borrow checker could not resolve place local '{local_id}' in the current function"
                    ),
                    location,
                ));
            };

            let local_state = state.local_state(local_index);
            if local_state.mode.is_definitely_uninit() {
                return Err(diagnostics.use_of_uninitialized_local(
                    diagnostics.local_place(layout.local_ids[local_index]),
                    location,
                ));
            }

            Ok(state.effective_roots(local_index))
        }

        HirPlace::Field { base, .. } => roots_for_place(layout, state, base, location, diagnostics),

        HirPlace::Index { base, .. } => roots_for_place(layout, state, base, location, diagnostics),
    }
}

fn record_shared_reads_in_place_indices(
    env: &mut SharedReadEnv<'_, '_>,
    place: &HirPlace,
    location: SourceLocation,
    roots: &mut RootSet,
) -> Result<(), BorrowCheckError> {
    match place {
        HirPlace::Local(_) => Ok(()),

        HirPlace::Field { base, .. } => {
            record_shared_reads_in_place_indices(env, base, location, roots)
        }

        HirPlace::Index { base, index } => {
            record_shared_reads_in_place_indices(env, base, location.clone(), roots)?;
            record_shared_reads_in_expression(env, index, location, roots)
        }
    }
}

fn record_shared_reads_in_expression(
    env: &mut SharedReadEnv<'_, '_>,
    expression: &HirExpression,
    location: SourceLocation,
    roots: &mut RootSet,
) -> Result<(), BorrowCheckError> {
    match &expression.kind {
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => {}

        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                record_shared_reads_in_expression(env, &field.value, location.clone(), roots)?;
            }
        }

        HirExpressionKind::Load(place) => {
            let value_location = env
                .context
                .diagnostics
                .value_error_location(expression.id, location.clone());
            record_shared_reads_in_place_indices(env, place, value_location.clone(), roots)?;

            let place_roots = roots_for_place(
                env.layout,
                env.state,
                place,
                value_location.clone(),
                &env.context.diagnostics,
            )?;
            let actor_index_hint = place_root_local_index(env.layout, place);
            let mut check = AccessCheckContext {
                context: env.context,
                layout: env.layout,
                state: env.state,
                block_id: env.block_id,
                tracker: env.tracker,
                location: value_location,
                stats: env.stats,
                actor_index_hint,
                current_order: env.current_order,
            };
            check_shared_access(&mut check, &place_roots)?;
            roots.union_with(&place_roots);
        }

        HirExpressionKind::Copy(place) => {
            let value_location = env
                .context
                .diagnostics
                .value_error_location(expression.id, location.clone());
            record_shared_reads_in_place_indices(env, place, value_location.clone(), roots)?;

            let place_roots = roots_for_place(
                env.layout,
                env.state,
                place,
                value_location.clone(),
                &env.context.diagnostics,
            )?;
            let actor_index_hint = place_root_local_index(env.layout, place);
            let mut check = AccessCheckContext {
                context: env.context,
                layout: env.layout,
                state: env.state,
                block_id: env.block_id,
                tracker: env.tracker,
                location: value_location.clone(),
                stats: env.stats,
                actor_index_hint,
                current_order: env.current_order,
            };
            check_shared_access(&mut check, &place_roots)?;
            roots.union_with(&place_roots);

            env.value_fact_buffer.record(
                expression.id,
                ValueAccessClassification::SharedRead,
                &place_roots,
            );
            return Ok(());
        }

        HirExpressionKind::BinOp { left, right, .. } => {
            record_shared_reads_in_expression(env, left, location.clone(), roots)?;
            record_shared_reads_in_expression(env, right, location.clone(), roots)?;
        }

        HirExpressionKind::UnaryOp { operand, .. } => {
            record_shared_reads_in_expression(env, operand, location.clone(), roots)?;
        }

        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in fields {
                record_shared_reads_in_expression(env, value, location.clone(), roots)?;
            }
        }

        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in elements {
                record_shared_reads_in_expression(env, element, location.clone(), roots)?;
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                record_shared_reads_in_expression(env, &entry.key, location.clone(), roots)?;
                record_shared_reads_in_expression(env, &entry.value, location.clone(), roots)?;
            }
        }
        HirExpressionKind::TupleGet { tuple, .. } => {
            record_shared_reads_in_expression(env, tuple, location.clone(), roots)?;
        }

        HirExpressionKind::Range { start, end } => {
            record_shared_reads_in_expression(env, start, location.clone(), roots)?;
            record_shared_reads_in_expression(env, end, location.clone(), roots)?;
        }

        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result }
        | HirExpressionKind::Cast { source: result, .. } => {
            record_shared_reads_in_expression(env, result, location.clone(), roots)?;
        }

        HirExpressionKind::VariantPayloadGet { source, .. } => {
            record_shared_reads_in_expression(env, source, location.clone(), roots)?;
        }
    }

    let classification = if roots.is_empty() {
        ValueAccessClassification::None
    } else {
        ValueAccessClassification::SharedRead
    };
    env.value_fact_buffer
        .record(expression.id, classification, roots);

    Ok(())
}

fn collect_expression_roots(
    layout: &FunctionLayout,
    state: &BorrowState,
    expression: &HirExpression,
    out: &mut RootSet,
    location: SourceLocation,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<(), BorrowCheckError> {
    match &expression.kind {
        HirExpressionKind::Load(place) => {
            let roots = roots_for_place(layout, state, place, location.clone(), diagnostics)?;
            out.union_with(&roots);

            if let HirPlace::Index { index, .. } = place {
                collect_expression_roots(layout, state, index, out, location, diagnostics)?;
            }
        }

        HirExpressionKind::Copy(place) => {
            if let HirPlace::Index { index, .. } = place {
                collect_expression_roots(layout, state, index, out, location, diagnostics)?;
            }
        }

        HirExpressionKind::BinOp { left, right, .. } => {
            collect_expression_roots(layout, state, left, out, location.clone(), diagnostics)?;
            collect_expression_roots(layout, state, right, out, location, diagnostics)?;
        }

        HirExpressionKind::UnaryOp { operand, .. } => {
            collect_expression_roots(layout, state, operand, out, location, diagnostics)?;
        }

        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in fields {
                collect_expression_roots(layout, state, value, out, location.clone(), diagnostics)?;
            }
        }

        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            for element in elements {
                collect_expression_roots(
                    layout,
                    state,
                    element,
                    out,
                    location.clone(),
                    diagnostics,
                )?;
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                collect_expression_roots(
                    layout,
                    state,
                    &entry.key,
                    out,
                    location.clone(),
                    diagnostics,
                )?;
                collect_expression_roots(
                    layout,
                    state,
                    &entry.value,
                    out,
                    location.clone(),
                    diagnostics,
                )?;
            }
        }
        HirExpressionKind::TupleGet { tuple, .. } => {
            collect_expression_roots(layout, state, tuple, out, location.clone(), diagnostics)?;
        }

        HirExpressionKind::Range { start, end } => {
            collect_expression_roots(layout, state, start, out, location.clone(), diagnostics)?;
            collect_expression_roots(layout, state, end, out, location, diagnostics)?;
        }

        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result }
        | HirExpressionKind::Cast { source: result, .. } => {
            collect_expression_roots(layout, state, result, out, location, diagnostics)?;
        }

        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => {}

        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                collect_expression_roots(
                    layout,
                    state,
                    &field.value,
                    out,
                    location.clone(),
                    diagnostics,
                )?;
            }
        }

        HirExpressionKind::VariantPayloadGet { source, .. } => {
            collect_expression_roots(layout, state, source, out, location, diagnostics)?;
        }
    }

    Ok(())
}

/// Shared inputs for aggregate-literal optional transfer analysis.
///
/// WHAT: keeps diagnostic lookup and advisory fact emission together while aggregate traversal
/// recurses through nested expressions.
/// WHY: optional transfer facts must be emitted without threading a wide argument list through
/// every aggregate child.
pub(super) struct AggregateTransferContext<'a, 'module> {
    pub(super) diagnostics: &'a BorrowDiagnostics<'module>,
    pub(super) value_fact_buffer: &'a mut ValueFactBuffer,
}

/// WHAT: records optional transfer outcomes for children of aggregate literals.
/// WHY: aggregate construction can receive destruction responsibility, but failed proof must
///      leave mandatory source state available for conservative borrow validation.
pub(super) fn transfer_aggregate_expression_ownership(
    layout: &FunctionLayout,
    state: &mut BorrowState,
    expression: &HirExpression,
    block_id: BlockId,
    current_order: i32,
    location: SourceLocation,
    aggregate_context: &mut AggregateTransferContext<'_, '_>,
) -> Result<(), BorrowCheckError> {
    match &expression.kind {
        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                transfer_aggregate_child(
                    layout,
                    state,
                    &entry.key,
                    block_id,
                    current_order,
                    location.clone(),
                    aggregate_context,
                )?;
                transfer_aggregate_child(
                    layout,
                    state,
                    &entry.value,
                    block_id,
                    current_order,
                    location.clone(),
                    aggregate_context,
                )?;
            }
        }

        HirExpressionKind::Collection(elements) => {
            for element in elements {
                transfer_aggregate_child(
                    layout,
                    state,
                    element,
                    block_id,
                    current_order,
                    location.clone(),
                    aggregate_context,
                )?;
            }
        }

        HirExpressionKind::BinOp { left, right, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                left,
                block_id,
                current_order,
                location.clone(),
                aggregate_context,
            )?;
            transfer_aggregate_expression_ownership(
                layout,
                state,
                right,
                block_id,
                current_order,
                location,
                aggregate_context,
            )?;
        }

        HirExpressionKind::UnaryOp { operand, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                operand,
                block_id,
                current_order,
                location,
                aggregate_context,
            )?;
        }

        HirExpressionKind::TupleGet { tuple, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                tuple,
                block_id,
                current_order,
                location,
                aggregate_context,
            )?;
        }

        HirExpressionKind::Range { start, end } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                start,
                block_id,
                current_order,
                location.clone(),
                aggregate_context,
            )?;
            transfer_aggregate_expression_ownership(
                layout,
                state,
                end,
                block_id,
                current_order,
                location,
                aggregate_context,
            )?;
        }

        HirExpressionKind::FallibleUnwrapSuccess { result }
        | HirExpressionKind::FallibleUnwrapError { result }
        | HirExpressionKind::Cast { source: result, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                result,
                block_id,
                current_order,
                location,
                aggregate_context,
            )?;
        }

        HirExpressionKind::VariantPayloadGet { source, .. } => {
            transfer_aggregate_expression_ownership(
                layout,
                state,
                source,
                block_id,
                current_order,
                location,
                aggregate_context,
            )?;
        }

        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, value) in fields {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    value,
                    block_id,
                    current_order,
                    location.clone(),
                    aggregate_context,
                )?;
            }
        }

        HirExpressionKind::TupleConstruct { elements } => {
            for element in elements {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    element,
                    block_id,
                    current_order,
                    location.clone(),
                    aggregate_context,
                )?;
            }
        }

        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    &field.value,
                    block_id,
                    current_order,
                    location.clone(),
                    aggregate_context,
                )?;
            }
        }

        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. }
        | HirExpressionKind::Copy(_)
        | HirExpressionKind::Load(_) => {}
    }

    Ok(())
}

/// WHAT: classifies one direct child of an aggregate literal for optional transfer.
/// WHY: direct place children can receive destruction responsibility, while computed children
///      still need recursive analysis before the outer aggregate stores the fresh result.
fn transfer_aggregate_child(
    layout: &FunctionLayout,
    state: &mut BorrowState,
    expression: &HirExpression,
    block_id: BlockId,
    current_order: i32,
    location: SourceLocation,
    aggregate_context: &mut AggregateTransferContext<'_, '_>,
) -> Result<(), BorrowCheckError> {
    let diagnostics = aggregate_context.diagnostics;

    // Nested aggregate literals are traversed before the outer aggregate stores the resulting
    // fresh value. This keeps `{ "outer" = { "inner" = value } }` aligned with the direct
    // `{ "inner" = value }` case while preserving shared storage on failed transfer proofs.
    transfer_aggregate_expression_ownership(
        layout,
        state,
        expression,
        block_id,
        current_order,
        location.clone(),
        aggregate_context,
    )?;

    if let HirExpressionKind::Load(place) = &expression.kind {
        let roots = roots_for_place(layout, state, place, location.clone(), diagnostics)?;
        if roots.is_empty() {
            return Ok(());
        }

        // Scalar builtins are implicitly copied into aggregate literals. Non-scalar values keep
        // shared storage unless this site has a proven optional transfer opportunity.
        if expression.ty == builtin_type_ids::BOOL
            || expression.ty == builtin_type_ids::INT
            || expression.ty == builtin_type_ids::FLOAT
            || expression.ty == builtin_type_ids::CHAR
            || expression.ty == builtin_type_ids::NONE
        {
            return Ok(());
        }

        match classify_move_decision(layout, block_id, &roots, current_order) {
            MoveDecision::Move => {
                aggregate_context
                    .value_fact_buffer
                    .record_optional_transfer(
                        expression.id,
                        OptionalTransferStatus::Transfer,
                        &roots,
                    );
            }
            MoveDecision::Borrow => {
                aggregate_context
                    .value_fact_buffer
                    .record_optional_transfer(
                        expression.id,
                        OptionalTransferStatus::Borrow,
                        &roots,
                    );
            }
        }
    }

    Ok(())
}
