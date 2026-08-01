//! Conflict detection helpers for borrow-transfer access checks.
//!
//! WHAT: classifies when a requested access overlaps an active borrow state.
//! WHY: transfer logic needs one place to keep borrow-conflict rules deterministic and testable.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FunctionLayout, FutureUseKind, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::types::AccessKind;
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::BlockId;

use super::{AccessCheckContext, BlockTransferStats, BorrowTransferContext, MutableAccessPolicy};

// WHAT: Validates shared-root reads against statement-local and active mutable conflicts.
// WHY: Shared reads must be blocked when the same root is mutably active.
pub(super) fn check_shared_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
) -> Result<(), BorrowCheckError> {
    let preserve_cfg_future_use_activity =
        preserves_cfg_future_use_activity(check.context, check.layout, check.actor_index_hint);
    let alias_activity = AliasActivityContext {
        layout: check.layout,
        state: check.state,
        block_id: check.block_id,
        current_order: check.current_order,
        preserve_cfg_future_use_activity,
    };

    for root_index in roots.iter_ones() {
        check.stats.conflicts_checked += 1;

        if let Some(existing) = check.tracker.conflict(root_index, AccessKind::Shared) {
            let place = check
                .context
                .diagnostics
                .local_place(check.layout.local_ids[root_index]);
            let existing_location = check.tracker.access_location(root_index).cloned();
            return Err(check.context.diagnostics.shared_mutable_conflict(
                place,
                borrow_access_kind(existing),
                BorrowAccessKind::Shared,
                None,
                existing_location,
                check.location.clone(),
            ));
        }

        if let Some(conflicting_index) = active_mutable_alias_for_root(
            check.context,
            &alias_activity,
            root_index,
            check.actor_index_hint,
        ) {
            let place = check
                .context
                .diagnostics
                .local_place(check.layout.local_ids[root_index]);
            let conflicting_place = check
                .context
                .diagnostics
                .local_place(check.layout.local_ids[conflicting_index]);
            return Err(check.context.diagnostics.shared_mutable_conflict(
                place,
                BorrowAccessKind::Mutable,
                BorrowAccessKind::Shared,
                Some(conflicting_place),
                None,
                check.location.clone(),
            ));
        }

        check
            .tracker
            .record(root_index, AccessKind::Shared, check.location.clone());
    }

    Ok(())
}

// WHAT: Validates mutable accesses against overlap, mutability, and alias exclusivity.
// WHY: Mutable access is only valid when the acting root has exclusive active ownership view.
pub(super) fn check_mutable_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    policy: MutableAccessPolicy,
) -> Result<(), BorrowCheckError> {
    let preserve_cfg_future_use_activity =
        preserves_cfg_future_use_activity(check.context, check.layout, check.actor_index_hint);
    let alias_activity = AliasActivityContext {
        layout: check.layout,
        state: check.state,
        block_id: check.block_id,
        current_order: check.current_order,
        preserve_cfg_future_use_activity,
    };

    for root_index in roots.iter_ones() {
        check.stats.conflicts_checked += 1;

        if let Some(existing) = check.tracker.conflict(root_index, AccessKind::Mutable)
            && !(policy.allow_prior_shared && existing == AccessKind::Shared)
        {
            let place = check
                .context
                .diagnostics
                .local_place(check.layout.local_ids[root_index]);
            let existing_location = check.tracker.access_location(root_index).cloned();
            if existing == AccessKind::Mutable {
                return Err(check.context.diagnostics.multiple_mutable_borrows(
                    place,
                    None,
                    existing_location,
                    check.location.clone(),
                ));
            }

            return Err(check.context.diagnostics.shared_mutable_conflict(
                place,
                borrow_access_kind(existing),
                BorrowAccessKind::Mutable,
                None,
                existing_location,
                check.location.clone(),
            ));
        }

        if policy.require_root_mutable && !check.layout.local_mutable[root_index] {
            let place = check
                .context
                .diagnostics
                .local_place(check.layout.local_ids[root_index]);
            return Err(check.context.diagnostics.invalid_mutable_access(
                place,
                InvalidMutableAccessReason::ImmutablePlace,
                None,
                None,
                check.location.clone(),
            ));
        }

        if policy.check_alias_exclusivity {
            let actor_index = check.actor_index_hint.unwrap_or(root_index);
            let alias_count = active_alias_count_for_root(
                &alias_activity,
                root_index,
                actor_index,
                policy.strict_move_exclusivity,
            );
            if alias_count > 1 {
                let conflicting_local_index = conflicting_active_local_for_root(
                    &alias_activity,
                    root_index,
                    actor_index,
                    policy.strict_move_exclusivity,
                );
                let place = check
                    .context
                    .diagnostics
                    .local_place(check.layout.local_ids[actor_index]);

                if !policy.strict_move_exclusivity
                    && conflicting_local_index.is_some_and(|index| {
                        // CFG lowering may use mutable storage slots for iterable aliases, but the
                        // alias remains shared access from the source program's perspective.
                        preserve_cfg_future_use_activity
                            && has_cfg_future_use_after_linear_expiry(
                                check.layout,
                                index,
                                check.block_id,
                                check.current_order,
                            )
                            && is_shared_alias_conflict(check.context, check.layout, index)
                    })
                {
                    let conflicting_place = conflicting_local_index.map(|index| {
                        check
                            .context
                            .diagnostics
                            .local_place(check.layout.local_ids[index])
                    });
                    return Err(check.context.diagnostics.shared_mutable_conflict(
                        place,
                        BorrowAccessKind::Shared,
                        BorrowAccessKind::Mutable,
                        conflicting_place,
                        None,
                        check.location.clone(),
                    ));
                }

                let conflicting_place = conflicting_local_index
                    .map(|index| {
                        check
                            .context
                            .diagnostics
                            .local_place(check.layout.local_ids[index])
                    })
                    .or(Some(DiagnosticPlace::Unknown));
                let conflicting_location = conflicting_local_index.and_then(|index| {
                    check
                        .context
                        .diagnostics
                        .local_source_location(check.layout.local_ids[index])
                });
                return Err(check.context.diagnostics.invalid_mutable_access(
                    place,
                    InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
                    conflicting_place,
                    conflicting_location,
                    check.location.clone(),
                ));
            }
        }

        check
            .tracker
            .record(root_index, AccessKind::Mutable, check.location.clone());
    }

    Ok(())
}

// WHAT: Probes whether optional transfer has the same exclusivity proof as mutable access.
// WHY: A failed optimisation proof must fall back to ordinary borrowing without publishing a
//      transfer-only source diagnostic or mutating the statement access tracker.
pub(super) fn probe_mutable_access(
    check: &AccessCheckContext<'_, '_>,
    roots: &RootSet,
    policy: MutableAccessPolicy,
) -> Result<bool, BorrowCheckError> {
    let mut tracker = check.tracker.clone();
    let mut stats = BlockTransferStats::default();
    let mut probe = AccessCheckContext {
        context: check.context,
        layout: check.layout,
        state: check.state,
        block_id: check.block_id,
        tracker: &mut tracker,
        location: check.location.clone(),
        stats: &mut stats,
        actor_index_hint: check.actor_index_hint,
        current_order: check.current_order,
    };

    match check_mutable_access(&mut probe, roots, policy) {
        Ok(()) => Ok(true),
        Err(BorrowCheckError::Diagnostic(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

fn borrow_access_kind(access: AccessKind) -> BorrowAccessKind {
    match access {
        AccessKind::Shared => BorrowAccessKind::Shared,
        AccessKind::Mutable => BorrowAccessKind::Mutable,
    }
}

struct AliasActivityContext<'a> {
    layout: &'a FunctionLayout,
    state: &'a BorrowState,
    block_id: BlockId,
    current_order: i32,
    preserve_cfg_future_use_activity: bool,
}

fn active_alias_count_for_root(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    actor_index: usize,
    strict_move_exclusivity: bool,
) -> u32 {
    let mut count = 0u32;
    let actor_state = activity.state.local_state(actor_index);
    let actor_is_alias_for_root = actor_index != root_index
        && actor_state.is_alias_only()
        && actor_state.value_roots.contains(root_index);

    for candidate_index in 0..activity.layout.local_count() {
        if actor_is_alias_for_root && candidate_index == root_index {
            continue;
        }

        let roots = activity.state.effective_roots(candidate_index);
        if !roots.contains(root_index) {
            continue;
        }

        if candidate_index == actor_index {
            count += 1;
            continue;
        }

        if !is_local_active_for_alias_conflict(
            activity,
            root_index,
            candidate_index,
            strict_move_exclusivity,
        ) {
            continue;
        }

        count += 1;
    }

    count
}

fn active_mutable_alias_for_root(
    context: &BorrowTransferContext<'_>,
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    actor_index_hint: Option<usize>,
) -> Option<usize> {
    for candidate_index in 0..activity.layout.local_count() {
        if Some(candidate_index) == actor_index_hint {
            continue;
        }

        if !activity.layout.local_mutable[candidate_index] {
            continue;
        }

        if matches!(
            context
                .diagnostics
                .local_origin_kind(activity.layout.local_ids[candidate_index]),
            Some(kind) if kind != HirLocalOriginKind::User
        ) {
            continue;
        }

        let candidate_state = activity.state.local_state(candidate_index);
        if !candidate_state.has_value_aliases() {
            continue;
        }

        if !is_local_active_for_alias_conflict(activity, root_index, candidate_index, false)
            && !local_alias_never_read(activity.layout, activity.state, candidate_index)
        {
            continue;
        }

        let roots = activity.state.effective_roots(candidate_index);
        if roots.contains(root_index) {
            return Some(candidate_index);
        }
    }

    None
}

fn conflicting_active_local_for_root(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    actor_index: usize,
    strict_move_exclusivity: bool,
) -> Option<usize> {
    let actor_state = activity.state.local_state(actor_index);
    let actor_is_alias_for_root = actor_index != root_index
        && actor_state.is_alias_only()
        && actor_state.value_roots.contains(root_index);

    for candidate_index in 0..activity.layout.local_count() {
        if actor_is_alias_for_root && candidate_index == root_index {
            continue;
        }

        if candidate_index == actor_index {
            continue;
        }

        if !is_local_active_for_alias_conflict(
            activity,
            root_index,
            candidate_index,
            strict_move_exclusivity,
        ) {
            continue;
        }

        let roots = activity.state.effective_roots(candidate_index);
        if roots.contains(root_index) {
            return Some(candidate_index);
        }
    }

    None
}

// WHAT: Determines whether a candidate alias can still participate in exclusivity conflicts.
// WHY: Expired alias views should stop blocking mutable access unless strict move rules apply.
fn is_local_active_for_alias_conflict(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    local_index: usize,
    strict_move_exclusivity: bool,
) -> bool {
    let last_use = activity.layout.local_last_use_order[local_index];
    if last_use >= 0 {
        if !activity
            .layout
            .local_is_expired(local_index, activity.current_order)
        {
            return true;
        }

        let local_state = activity.state.local_state(local_index);
        if !local_state.has_value_aliases() {
            return false;
        }

        if activity.preserve_cfg_future_use_activity
            && has_cfg_future_use_after_linear_expiry(
                activity.layout,
                local_index,
                activity.block_id,
                activity.current_order,
            )
        {
            return true;
        }

        if strict_move_exclusivity {
            return local_state.direct_alias_roots.contains(root_index)
                || activity.layout.local_mutable[local_index];
        }

        return false;
    }

    let local_state = activity.state.local_state(local_index);
    if !local_state.has_value_aliases() {
        return false;
    }

    if strict_move_exclusivity {
        return local_state.direct_alias_roots.contains(root_index)
            || activity.layout.local_mutable[local_index];
    }

    // Unused mutable aliases remain active until the end of the scope.
    activity.layout.local_mutable[local_index]
}

// WHAT: Limits CFG future-use retention to source-semantic access actors.
// WHY: compiler-temporary rebinding in template lowering is not a source mutation and must keep
//      its existing linear-expiry behaviour.
fn preserves_cfg_future_use_activity(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    actor_index_hint: Option<usize>,
) -> bool {
    actor_index_hint.is_none_or(|actor_index| {
        !matches!(
            context
                .diagnostics
                .local_origin_kind(layout.local_ids[actor_index]),
            Some(HirLocalOriginKind::CompilerTemp)
        )
    })
}

fn has_cfg_future_use_after_linear_expiry(
    layout: &FunctionLayout,
    local_index: usize,
    block_id: BlockId,
    current_order: i32,
) -> bool {
    layout.local_is_expired(local_index, current_order)
        && matches!(
            layout.future_use_kind(block_id, local_index, current_order),
            FutureUseKind::May | FutureUseKind::Must
        )
}

fn is_shared_alias_conflict(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    local_index: usize,
) -> bool {
    !layout.local_mutable[local_index]
        || matches!(
            context
                .diagnostics
                .local_origin_kind(layout.local_ids[local_index]),
            Some(HirLocalOriginKind::CompilerTemp)
        )
}

fn local_alias_never_read(
    layout: &FunctionLayout,
    state: &BorrowState,
    local_index: usize,
) -> bool {
    let local_state = state.local_state(local_index);
    if !local_state.has_value_aliases() {
        return false;
    }

    let first_write = layout.local_first_write_order[local_index];
    let last_use = layout.local_last_use_order[local_index];
    first_write >= 0 && first_write == last_use
}
