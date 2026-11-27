//! Conflict detection helpers for borrow-transfer access checks.
//!
//! WHAT: classifies when a requested access overlaps an active borrow state.
//! WHY: transfer logic needs one place to keep borrow-conflict rules deterministic and testable.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, BorrowState, CellId, Coverage, FieldKey, FunctionLayout, FutureUseKind,
    MutationTarget, RootSet, WrapperAge, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{AccessKind, LocalMode};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::BlockId;

use super::{
    AccessCheckContext, AliasExclusivity, BlockTransferStats, BorrowTransferContext,
    MutableAccessPolicy,
};

/// A projected location and the values the operation can observe or change through it.
/// Callers may use this to refine coarse root overlap only with complete target coverage.
#[derive(Clone, Copy, Default)]
pub(super) struct PlaceAccess<'a> {
    pub cells: &'a [(AllocId, FieldKey)],
    pub values: &'a [MutationTarget],
}

impl PlaceAccess<'_> {
    fn is_disjoint_from(
        self,
        check: &AccessCheckContext<'_, '_>,
        local: usize,
        root: usize,
    ) -> bool {
        let binding = check.state.binding(local);
        let projected =
            !self.cells.is_empty() && binding.projected_coverage() == Coverage::Complete;
        if !projected && binding.aliases_root(root) {
            return false;
        }
        if projected && binding.projected_cells_touch(self.cells, self.values) {
            return false;
        }
        if projected && !check.layout.local_has_in_place_mutation_surface[local] {
            return true;
        }
        let (ids, layer_only) = check.state.local_alloc_ids(local);
        !self.values.is_empty()
            && (projected || layer_only)
            && !ids.is_empty()
            && !check.state.alloc_reaches(&ids, true, self.values)
    }

    fn overlaps(
        self,
        check: &AccessCheckContext<'_, '_>,
        local: usize,
        held: &[AllocId],
        layer_only: bool,
    ) -> bool {
        let binding = check.state.binding(local);
        ((!self.cells.is_empty() || self.values.iter().any(|value| value.deep))
            && binding.projected_cells_touch(self.cells, self.values))
            || ((check.layout.local_has_in_place_mutation_surface[local]
                || binding.projected_coverage() != Coverage::Complete)
                && check.state.alloc_reaches(held, layer_only, self.values))
    }
}

pub(super) fn check_shared_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
) -> Result<(), BorrowCheckError> {
    check_shared_access_with_place(check, roots, None)
}

// WHAT: Validates shared-root reads against statement-local and active mutable conflicts.
// WHY: Shared reads must be blocked when a root they access is mutably active. A read through a
//      binding that provably derives from the exclusive alias is not a foreign access, so
//      `active_mutable_alias_for_root` exempts those candidates.
pub(super) fn check_shared_access_with_place(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    place: Option<PlaceAccess<'_>>,
) -> Result<(), BorrowCheckError> {
    let preserve_cfg_future_use_activity =
        preserves_cfg_future_use_activity(check.context, check.layout, check.actor_index_hint);
    let alias_activity = AliasActivityContext {
        context: check.context,
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
            let existing_span = check.tracker.access_span(root_index);
            return Err(check.context.diagnostics.shared_mutable_conflict(
                place,
                borrow_access_kind(existing),
                BorrowAccessKind::Shared,
                None,
                existing_span,
                check.span,
            ));
        }

        if let Some(conflicting_index) = active_mutable_alias_for_root(
            &alias_activity,
            root_index,
            check.actor_index_hint,
            &|candidate| {
                let binding = check.state.binding(candidate);
                binding.projected_coverage() == Coverage::Complete
                    && place.is_some_and(|place| {
                        // Different allocations after a rebind do not discharge the
                        // conservative holder-cell rule. Refine it only within one holder.
                        let uses_holder = binding.route_roots().contains(root_index);
                        (!uses_holder
                            || matches!(place.cells, [(holder, _)]
                                if binding.projected_cells_share_singular_holder(*holder)))
                            && place.is_disjoint_from(check, candidate, root_index)
                    })
            },
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
                check.span,
            ));
        }

        check
            .tracker
            .record(root_index, AccessKind::Shared, check.span);
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
    check_mutable_access_with_targets(check, roots, policy, None)
}

pub(super) fn check_mutable_access_with_targets(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    policy: MutableAccessPolicy,
    projected: Option<PlaceAccess<'_>>,
) -> Result<(), BorrowCheckError> {
    let preserve_cfg_future_use_activity =
        preserves_cfg_future_use_activity(check.context, check.layout, check.actor_index_hint);
    let alias_activity = AliasActivityContext {
        context: check.context,
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
            let existing_span = check.tracker.access_span(root_index);
            if existing == AccessKind::Mutable {
                return Err(check.context.diagnostics.multiple_mutable_borrows(
                    place,
                    None,
                    existing_span,
                    check.span,
                ));
            }

            return Err(check.context.diagnostics.shared_mutable_conflict(
                place,
                borrow_access_kind(existing),
                BorrowAccessKind::Mutable,
                None,
                existing_span,
                check.span,
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
                check.span,
            ));
        }

        let direct_alias_only = policy.alias_exclusivity == AliasExclusivity::Direct;
        let actor_index = check.actor_index_hint.unwrap_or(root_index);
        let disjoint = |candidate| {
            candidate != root_index
                && projected
                    .is_some_and(|place| place.is_disjoint_from(check, candidate, root_index))
        };
        let alias_count = active_alias_count_for_root(
            &alias_activity,
            root_index,
            actor_index,
            policy.strict_move_exclusivity,
            direct_alias_only,
            &disjoint,
        );
        if alias_count > 1 {
            let conflicting_local_index = conflicting_active_local_for_root(
                &alias_activity,
                root_index,
                actor_index,
                policy.strict_move_exclusivity,
                direct_alias_only,
                &disjoint,
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
                    check.span,
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
            let conflicting_span = conflicting_local_index.and_then(|index| {
                check
                    .context
                    .diagnostics
                    .local_source_span(check.layout.local_ids[index])
            });
            return Err(check.context.diagnostics.invalid_mutable_access(
                place,
                InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
                conflicting_place,
                conflicting_span,
                check.span,
            ));
        }

        check
            .tracker
            .record(root_index, AccessKind::Mutable, check.span);
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
        location: check.location,
        span: check.span,
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

#[derive(Clone, Copy)]
pub(super) enum LayerConflictAccess<'a> {
    ObservedValue,
    CapturedBinding(&'a AddressSet),
    CapturedProjection(&'a AddressSet),
}

/// Reject a mutation while another active binding observes the captured cell or reaches a
/// mutated allocation through the layer. Coarse allocation overlap retains the route and
/// derivation exemptions; an exact captured-cell overlap only exempts use through a parent.
pub(super) fn check_layer_conflict(
    check: &mut AccessCheckContext<'_, '_>,
    actor_index: usize,
    place: PlaceAccess<'_>,
    access: LayerConflictAccess<'_>,
) -> Result<(), BorrowCheckError> {
    let captured_write = match access {
        LayerConflictAccess::ObservedValue => None,
        LayerConflictAccess::CapturedBinding(cells)
        | LayerConflictAccess::CapturedProjection(cells) => Some(cells),
    };
    let has_layer_targets = !place.values.is_empty() || !place.cells.is_empty();
    if !has_layer_targets && captured_write.is_none() {
        return Ok(());
    }
    let activity = AliasActivityContext {
        context: check.context,
        layout: check.layout,
        state: check.state,
        block_id: check.block_id,
        current_order: check.current_order,
        preserve_cfg_future_use_activity: preserves_cfg_future_use_activity(
            check.context,
            check.layout,
            Some(actor_index),
        ),
    };
    let actor = check.state.binding(actor_index);
    let mut actor_roots = None;
    for candidate_index in 0..check.layout.local_count() {
        if candidate_index == actor_index {
            continue;
        }
        let candidate = check.state.binding(candidate_index);
        let mode = candidate.mode();
        if !mode.contains(LocalMode::SLOT) && !mode.contains(LocalMode::ALIAS) {
            continue;
        }
        let keeps_receiver_stable = check
            .state
            .stability_overlaps(candidate_index, place.values)
            && check.layout.future_use_kind(
                check.block_id,
                candidate_index,
                check.current_order - 1,
            ) != FutureUseKind::None;
        if !keeps_receiver_stable {
            if actor.derivation().contains(candidate_index)
                || !is_local_active_for_alias_conflict(
                    &activity,
                    candidate_index,
                    candidate_index,
                    false,
                )
            {
                continue;
            }
            let observes_captured_cell = mode.contains(LocalMode::ALIAS)
                && captured_write.is_some_and(|written| {
                    check
                        .state
                        .resolve_cells(candidate_index)
                        .cells()
                        .iter()
                        .any(|cell| {
                            // Current slots already participate in the root check. Compare Prior
                            // slots as distinct occurrences, not by the name of the current slot.
                            !matches!(
                                cell,
                                CellId::Slot(WrapperId {
                                    age: WrapperAge::Current,
                                    ..
                                })
                            ) && written.cells().binary_search(cell).is_ok()
                        })
                });
            if !observes_captured_cell {
                if !has_layer_targets
                    || actor_roots
                        .get_or_insert_with(|| {
                            let mut roots = actor.route_roots();
                            if let LayerConflictAccess::CapturedBinding(written) = access {
                                // This local store already resolved the actor's terminal cells.
                                for &cell in written.cells() {
                                    if let CellId::Slot(wrapper) = cell
                                        && wrapper.age == WrapperAge::Current
                                    {
                                        roots.insert(wrapper.local);
                                    }
                                }
                            } else {
                                roots.union_with(&actor.direct_slots());
                            }
                            roots
                        })
                        .contains(candidate_index)
                    || candidate.derivation().contains(actor_index)
                {
                    continue;
                }
                let (held, layer_only) = check.state.local_alloc_ids(candidate_index);
                if (!matches!(access, LayerConflictAccess::CapturedBinding(_))
                    && check.state.detached_from(actor_index, &held))
                    || !place.overlaps(check, candidate_index, &held, layer_only)
                {
                    continue;
                }
            }
        }
        let diagnostics = &check.context.diagnostics;
        let candidate_id = check.layout.local_ids[candidate_index];
        return Err(diagnostics.invalid_mutable_access(
            diagnostics.local_place(check.layout.local_ids[actor_index]),
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
            Some(diagnostics.local_place(candidate_id)),
            diagnostics.local_source_span(candidate_id),
            check.span,
        ));
    }
    Ok(())
}

/// Check a read against exclusive aliases that can reach the same allocation.
pub(super) fn check_layer_shared_access(
    check: &AccessCheckContext<'_, '_>,
    actor_index: usize,
    place: PlaceAccess<'_>,
) -> Result<(), BorrowCheckError> {
    let activity = AliasActivityContext {
        context: check.context,
        layout: check.layout,
        state: check.state,
        block_id: check.block_id,
        current_order: check.current_order,
        preserve_cfg_future_use_activity: preserves_cfg_future_use_activity(
            check.context,
            check.layout,
            Some(actor_index),
        ),
    };
    let actor = check.state.binding(actor_index);
    for candidate in 0..check.layout.local_count() {
        if candidate == actor_index
            || !check.layout.local_mutable[candidate]
            || !check
                .state
                .binding_mode(candidate)
                .contains(LocalMode::ALIAS)
            || actor.derivation().contains(candidate)
            || matches!(
                check.context.diagnostics.local_origin_kind(check.layout.local_ids[candidate]),
                Some(kind) if kind != HirLocalOriginKind::User
            )
            || (!is_local_active_for_alias_conflict(&activity, candidate, candidate, false)
                && !local_alias_never_read(check.layout, check.state, candidate))
        {
            continue;
        }
        let (held, layer_only) = check.state.local_alloc_ids(candidate);
        if place.overlaps(check, candidate, &held, layer_only) {
            let diagnostics = &check.context.diagnostics;
            return Err(diagnostics.shared_mutable_conflict(
                diagnostics.local_place(check.layout.local_ids[actor_index]),
                BorrowAccessKind::Mutable,
                BorrowAccessKind::Shared,
                Some(diagnostics.local_place(check.layout.local_ids[candidate])),
                None,
                check.span,
            ));
        }
    }
    Ok(())
}

fn borrow_access_kind(access: AccessKind) -> BorrowAccessKind {
    match access {
        AccessKind::Shared => BorrowAccessKind::Shared,
        AccessKind::Mutable => BorrowAccessKind::Mutable,
    }
}

struct AliasActivityContext<'a> {
    context: &'a BorrowTransferContext<'a>,
    layout: &'a FunctionLayout,
    state: &'a BorrowState,
    block_id: BlockId,
    current_order: i32,
    preserve_cfg_future_use_activity: bool,
}

/// Whether the acting binding writes through `root_index` as an alias.
///
/// WHY: the actor's referent cells, not its value provenance, say which cell it mutates. A
/// provenance root is an allocation the actor observes, not the cell it writes through.
fn actor_aliases_root(
    activity: &AliasActivityContext<'_>,
    actor_index: usize,
    root_index: usize,
) -> bool {
    let actor = activity.state.binding(actor_index);
    actor_index != root_index && actor.is_alias_only() && actor.aliases_root(root_index)
}

/// A reborrow can exercise the exclusive access it derives from. The reverse is not true:
/// acting through a parent cannot bypass a live child. The derivation ledger is a must fact.
fn reborrow_uses_parent(
    activity: &AliasActivityContext<'_>,
    actor_index: usize,
    candidate_index: usize,
) -> bool {
    let actor = activity.state.binding(actor_index);
    actor.derivation().contains(candidate_index)
        || (actor.projected_coverage() == Coverage::Complete
            && actor.route_roots().contains(candidate_index)
            && activity.state.binding_mode(candidate_index) == LocalMode::SLOT)
}

fn active_alias_count_for_root(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    actor_index: usize,
    strict_move_exclusivity: bool,
    direct_alias_only: bool,
    disjoint: &impl Fn(usize) -> bool,
) -> u32 {
    let mut count = 0u32;
    let actor_is_alias_for_root = actor_aliases_root(activity, actor_index, root_index);

    for candidate_index in 0..activity.layout.local_count() {
        if actor_is_alias_for_root
            && (candidate_index == root_index
                || (!strict_move_exclusivity
                    && reborrow_uses_parent(activity, actor_index, candidate_index)))
        {
            continue;
        }

        // A definite slot whose value carries provenance omits its own cell from
        // `effective_roots`. The acting slot still holds the written cell, so count it
        // explicitly; otherwise one real alias would look like the only participant.
        if candidate_index == actor_index && candidate_index == root_index {
            count += 1;
            continue;
        }

        if !candidate_contains_root(activity, candidate_index, root_index) {
            continue;
        }

        if candidate_index == actor_index {
            count += 1;
            continue;
        }

        if !alias_observes_cell_write(activity, root_index, candidate_index, direct_alias_only) {
            continue;
        }

        if !is_local_active_for_alias_conflict(
            activity,
            root_index,
            candidate_index,
            strict_move_exclusivity,
        ) || disjoint(candidate_index)
        {
            continue;
        }

        count += 1;
    }

    count
}

fn active_mutable_alias_for_root(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    actor_index_hint: Option<usize>,
    disjoint: &impl Fn(usize) -> bool,
) -> Option<usize> {
    for candidate_index in 0..activity.layout.local_count() {
        if Some(candidate_index) == actor_index_hint {
            continue;
        }

        if !activity.layout.local_mutable[candidate_index] {
            continue;
        }

        // Pure filters, so their order does not change which candidate is found. The bit tests
        // run first: the diagnostics lookup and the liveness check cost more, and this scan
        // visits every local on every mutable access.
        let candidate = activity.state.binding(candidate_index);
        if !candidate.mode().contains(LocalMode::ALIAS) || !candidate.has_observed_roots() {
            continue;
        }

        if !candidate_contains_root(activity, candidate_index, root_index) {
            continue;
        }

        // A read through a binding that provably derives from this exclusive alias is the same
        // access path, not a foreign one. The derivation set is a must fact, so a may-join does
        // not exempt a candidate that only some incoming paths derived from.
        if actor_index_hint.is_some_and(|reader| {
            activity
                .state
                .binding(reader)
                .derivation()
                .contains(candidate_index)
        }) {
            continue;
        }

        if matches!(
            activity.context
                .diagnostics
                .local_origin_kind(activity.layout.local_ids[candidate_index]),
            Some(kind) if kind != HirLocalOriginKind::User
        ) {
            continue;
        }

        if !is_local_active_for_alias_conflict(activity, root_index, candidate_index, false)
            && !local_alias_never_read(activity.layout, activity.state, candidate_index)
        {
            continue;
        }
        if disjoint(candidate_index) {
            continue;
        }

        return Some(candidate_index);
    }

    None
}

fn conflicting_active_local_for_root(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    actor_index: usize,
    strict_move_exclusivity: bool,
    direct_alias_only: bool,
    disjoint: &impl Fn(usize) -> bool,
) -> Option<usize> {
    let actor_is_alias_for_root = actor_aliases_root(activity, actor_index, root_index);

    for candidate_index in 0..activity.layout.local_count() {
        if actor_is_alias_for_root
            && (candidate_index == root_index
                || (!strict_move_exclusivity
                    && reborrow_uses_parent(activity, actor_index, candidate_index)))
        {
            continue;
        }

        if candidate_index == actor_index {
            continue;
        }

        // Membership is a bit test; the liveness check does several layout and diagnostics
        // lookups per candidate. Both are pure filters, so testing membership first saves work
        // without changing which candidate is found.
        if !candidate_contains_root(activity, candidate_index, root_index) {
            continue;
        }

        if !alias_observes_cell_write(activity, root_index, candidate_index, direct_alias_only) {
            continue;
        }

        if !is_local_active_for_alias_conflict(
            activity,
            root_index,
            candidate_index,
            strict_move_exclusivity,
        ) || disjoint(candidate_index)
        {
            continue;
        }

        return Some(candidate_index);
    }

    None
}

// WHAT: Whether a candidate observes or writes the root's cell or allocation, for alias scans.
// WHY: a whole-binding alias can observe a value-backed slot's cell even when its value
//      provenance names only older allocations. A projected alias can also overlap by its route.
fn candidate_contains_root(
    activity: &AliasActivityContext<'_>,
    candidate_index: usize,
    root_index: usize,
) -> bool {
    activity
        .state
        .effective_root_contains(candidate_index, root_index)
        || activity
            .state
            .binding(candidate_index)
            .aliases_root(root_index)
}

// WHAT: Decides whether a candidate alias observes a write to the root of this binding cell.
// WHY: a definite-slot rebinding to a fresh value replaces the slot's value provenance. Only an
// alias of the cell itself (a direct source alias) sees that replacement, while a slot-backed
// value alias keeps pointing at the older allocation. An alias of a projection writes the base
// cell but observes the field value, so it is not a direct observer either.
fn alias_observes_cell_write(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    candidate_index: usize,
    direct_alias_only: bool,
) -> bool {
    !direct_alias_only
        || activity
            .state
            .binding(candidate_index)
            .direct_slots()
            .contains(root_index)
}

// WHAT: Determines whether a candidate alias can still participate in exclusivity conflicts.
// WHY: Expired alias views should stop blocking mutable access unless strict move rules apply.
fn is_local_active_for_alias_conflict(
    activity: &AliasActivityContext<'_>,
    root_index: usize,
    local_index: usize,
    strict_move_exclusivity: bool,
) -> bool {
    let binding = activity.state.binding(local_index);

    // Slot-backed holders, including a root-only transport slot with no value_roots, do not
    // write through their own binding cell. Their linear last-use can include a mutually
    // exclusive handler, so decide activity from this CFG point even when a compiler-temp actor
    // uses linear expiry for source-binding aliases.
    if !strict_move_exclusivity && !binding.mode().contains(LocalMode::ALIAS) {
        return activity.layout.future_use_kind(
            activity.block_id,
            local_index,
            activity.current_order - 1,
        ) != FutureUseKind::None;
    }

    let last_use = activity.layout.local_last_use_order[local_index];
    if last_use >= 0 {
        // Traversal ordinals are ordered within a block, not across CFG paths. A handler
        // visited after its merge cannot prolong a dead alias. Include the current statement:
        // a holder read there still excludes a simultaneous shared read through its source.
        if !activity
            .layout
            .local_is_expired(local_index, activity.current_order)
        {
            if !binding.has_observed_roots() || !activity.preserve_cfg_future_use_activity {
                return true;
            }

            if activity.layout.future_use_kind(
                activity.block_id,
                local_index,
                activity.current_order - 1,
            ) != FutureUseKind::None
            {
                return true;
            }
        }

        if !binding.has_observed_roots() {
            return false;
        }

        // A source alias-view assignment writes through its existing referent. Read
        // liveness treats a local assignment as a slot kill, so consult reachable
        // assignments separately without weakening slot rebinding or optional transfers.
        let source_mutable_alias = activity.layout.local_mutable[local_index]
            && binding.mode().contains(LocalMode::ALIAS)
            && !matches!(
                activity
                    .context
                    .diagnostics
                    .local_origin_kind(activity.layout.local_ids[local_index]),
                Some(kind) if kind != HirLocalOriginKind::User
            );
        if activity.preserve_cfg_future_use_activity
            && source_mutable_alias
            && activity
                .layout
                .local_has_future_assignment(activity.block_id, local_index)
        {
            return true;
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
            return binding.direct_slots().contains(root_index)
                || activity.layout.local_mutable[local_index];
        }

        return false;
    }

    if !binding.has_observed_roots() {
        return false;
    }

    if strict_move_exclusivity {
        return binding.direct_slots().contains(root_index)
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
    if !state.binding(local_index).has_observed_roots() {
        return false;
    }

    let first_write = layout.local_first_write_order[local_index];
    let last_use = layout.local_last_use_order[local_index];
    first_write >= 0 && first_write == last_use
}
