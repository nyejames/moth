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
mod move_decision;
mod statement;
mod terminator;

use conflicts::{check_mutable_access, check_shared_access, probe_mutable_access};
use move_decision::{MoveDecision, classify_move_decision};
pub(super) use statement::transfer_statement;
pub(super) use terminator::transfer_terminator;

fn require_initialized_update_target(
    layout: &FunctionLayout,
    state: &BorrowState,
    local_index: usize,
    diagnostics: &BorrowDiagnostics<'_>,
    span: Option<SourceSpan>,
) -> Result<(), BorrowCheckError> {
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
    // Slot writes replace the binding, alias writes target their referents, and a joined state
    // must check both possibilities. Both source assignments and dedicated call results use this
    // rule so neither operation can bypass update mutability checks.
    let replaces_definite_slot = rhs_is_fresh
        && local_state.mode.contains(LocalMode::SLOT)
        && !local_state.mode.contains(LocalMode::ALIAS);
    let mut write_roots = RootSet::empty(check.layout.local_count());
    if local_state.mode.contains(LocalMode::SLOT) {
        write_roots.insert(local_index);
    }
    if local_state.mode.contains(LocalMode::ALIAS) && local_state.has_value_aliases() {
        write_roots.union_with(&local_state.value_roots);
    }

    if !write_roots.is_empty() {
        check_mutable_access(
            check,
            &write_roots,
            MutableAccessPolicy {
                allow_prior_shared: true,
                require_root_mutable: true,
                strict_move_exclusivity: false,
                check_alias_exclusivity: !replaces_definite_slot,
            },
        )?;
    }

    Ok(write_roots)
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

    if !is_definition {
        let Some(local_index) = layout.index_of(target.root) else {
            return Err(transfer_context.diagnostics.internal_error(
                format!(
                    "Assignment target local '{}' is not in the active function layout",
                    transfer_context.diagnostics.local_name(target.root)
                ),
                location,
            ));
        };
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
            let local_id = target.root;
            let Some(local_index) = layout.index_of(local_id) else {
                return Err(transfer_context.diagnostics.internal_error(
                    format!(
                        "Assignment target local '{}' is not in the active function layout",
                        transfer_context.diagnostics.local_name(local_id)
                    ),
                    location,
                ));
            };

            let local_state = state.local_state(local_index).clone();
            let rhs_roots = direct_value_roots_from_expression(
                layout,
                state,
                transfer_context.expressions,
                value,
                location,
                value_expression.span,
                &transfer_context.diagnostics,
            )?;
            let rhs_direct_alias_roots = rhs_roots.as_ref().map(|roots| {
                direct_root_aliases_from_expression(
                    layout,
                    state,
                    transfer_context.expressions,
                    value,
                    roots,
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
                match rhs_roots {
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
                        if target_is_mutable
                            && !rhs_roots.is_empty()
                            && rhs_is_place
                            && !target_is_compiler_owned_scratch
                        {
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
                        let new_state = if rhs_is_place {
                            LocalState::alias_with_direct(rhs_roots, direct_roots)
                        } else {
                            LocalState::slot_with_value_roots(rhs_roots, direct_roots)
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
                            check_alias_exclusivity: true,
                        },
                    )?;
                }
            }

            match (
                local_state.mode.contains(LocalMode::SLOT),
                local_state.mode.contains(LocalMode::ALIAS),
            ) {
                (false, true) => {
                    // Alias-view writes through to referent and does not rebind.
                }

                (true, false) => match rhs_roots {
                    Some(rhs_roots) if rhs_is_place => {
                        let direct_roots = rhs_direct_alias_roots
                            .unwrap_or_else(|| RootSet::empty(layout.local_count()));
                        state.update_local_state(
                            local_index,
                            LocalState::alias_with_direct(rhs_roots, direct_roots),
                        );
                    }
                    rhs_roots => apply_slot_rebinding(
                        state,
                        layout.local_count(),
                        local_index,
                        rhs_roots,
                        rhs_direct_alias_roots,
                    ),
                },

                (true, true) => {
                    // The incoming value replaces the slot on its slot path, but the joined
                    // state has no per-path root partition. Keep the union because it still
                    // contains the referent roots on the alias path, where this write goes
                    // through instead of replacing the binding.
                    let mut value_roots = local_state.value_roots;
                    let mut direct_alias_roots = local_state.direct_alias_roots;
                    if let Some(rhs_roots) = rhs_roots {
                        value_roots.union_with(&rhs_roots);
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

                (false, false) => {
                    state.update_local_state(local_index, LocalState::slot(layout.local_count()));
                }
            }
        }

        false => {
            let roots = roots_for_place(
                layout,
                state,
                target,
                location,
                span,
                &transfer_context.diagnostics,
            )?;
            let mut check = AccessCheckContext {
                context: transfer_context,
                layout,
                state: &*state,
                block_id,
                tracker,
                location,
                span,
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
    rhs_roots: Option<RootSet>,
    rhs_direct_alias_roots: Option<RootSet>,
) {
    match rhs_roots {
        Some(roots) if !roots.is_empty() => {
            let direct_roots =
                rhs_direct_alias_roots.unwrap_or_else(|| RootSet::empty(local_count));
            state.update_local_state(
                local_index,
                LocalState::slot_with_value_roots(roots, direct_roots),
            )
        }
        None => state.update_local_state(local_index, LocalState::slot(local_count)),
        Some(_) => state.update_local_state(local_index, LocalState::slot(local_count)),
    }
}

fn place_root_local_index(layout: &FunctionLayout, place: &HirPlace) -> Option<usize> {
    layout.index_of(place.root)
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

fn direct_root_aliases_from_expression(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
    rhs_roots: &RootSet,
) -> RootSet {
    let mut direct_roots = RootSet::empty(layout.local_count());
    let expression = expressions.expression(expression_id);

    if expression.value_kind != ValueKind::Place {
        return direct_roots;
    }

    let HirExpressionKind::Load(place) = &expression.kind else {
        return direct_roots;
    };

    if !place.projections.is_empty() {
        return direct_roots;
    }

    let Some(source_index) = layout.index_of(place.root) else {
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
    location: Option<SourceSpan>,
    span: Option<SourceSpan>,
    diagnostics: &BorrowDiagnostics<'_>,
) -> Result<RootSet, BorrowCheckError> {
    increment_frontend_counter(FrontendCounter::BorrowPlaceAccessCount);

    let Some(local_index) = layout.index_of(place.root) else {
        return Err(diagnostics.internal_error(
            format!(
                "Borrow checker could not resolve place local '{}' in the current function",
                place.root
            ),
            location,
        ));
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
    let mut steps = Vec::new();
    push_children_in_source_order(
        &mut steps,
        env.context.expressions.place_index_values(*place),
        |index| SharedReadStep::Visit {
            expression_id: index,
            location,
        },
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

/// WHAT: pushes `children` onto a work stack so they pop in source order.
/// WHY: the stack pops the last pushed item first, so the run this call just pushed is reversed in
/// place. Reversing in place keeps source order without collecting the children into a `Vec`.
fn push_children_in_source_order<T>(
    pending: &mut Vec<T>,
    children: impl Iterator<Item = HirValueId>,
    mut to_step: impl FnMut(HirValueId) -> T,
) {
    let start = pending.len();
    pending.extend(children.map(&mut to_step));
    pending[start..].reverse();
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
                        push_children_in_source_order(
                            steps,
                            expressions.child_values(expression_id),
                            |child| SharedReadStep::Visit {
                                expression_id: child,
                                location: value_span,
                            },
                        );
                    }

                    // WHY: the store lists the children of every other compound expression in source
                    // order; this node's classification runs in `Finish` after they are visited.
                    _ => {
                        steps.push(SharedReadStep::Finish {
                            expression_id,
                            value_span: None,
                        });
                        push_children_in_source_order(
                            steps,
                            expressions.child_values(expression_id),
                            |child| SharedReadStep::Visit {
                                expression_id: child,
                                location,
                            },
                        );
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
    let place_roots = roots_for_place(
        env.layout,
        env.state,
        place,
        value_span,
        value_span,
        &env.context.diagnostics,
    )?;
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
    check_shared_access(&mut check, &place_roots)?;
    Ok(place_roots)
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
        push_children_in_source_order(
            &mut pending,
            expressions.child_values(step.expression_id),
            |child| RootCollectStep {
                expression_id: child,
                span: expressions.expression(child).span,
            },
        );
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
                        push_children_in_source_order(
                            &mut pending,
                            expressions.child_values(expression_id),
                            AggregateOwnershipStep::AggregateChild,
                        );
                    }

                    // WHY: only map and collection children can receive optional transfer advice;
                    // other children are walked as plain expressions.
                    _ => {
                        push_children_in_source_order(
                            &mut pending,
                            expressions.child_values(expression_id),
                            AggregateOwnershipStep::Visit,
                        );
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
