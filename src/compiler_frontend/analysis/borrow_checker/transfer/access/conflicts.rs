//! Conflict detection helpers for borrow-transfer access checks.
//!
//! WHAT: classifies when a requested access overlaps an active borrow state.
//! WHY: transfer logic needs one place to keep borrow-conflict rules deterministic and testable.

use super::{AccessCheckContext, BorrowTransferContext, MutableAccessPolicy};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, BorrowState, CellId, Coverage, FieldKey, FunctionLayout, FutureUseKind,
    PlaceResolution, RootSet, SmallSet, WrapperAge, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{
    AccessFootprint, AccessReservation, PlaceUse,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{AccessKind, LocalMode};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::BlockId;
use std::cell::OnceCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Overlap {
    No,
    Yes,
    Unknown,
}

fn cell_relation(
    state: &BorrowState,
    a: &[CellId],
    a_coverage: Coverage,
    b: &[CellId],
    b_coverage: Coverage,
) -> Overlap {
    if a.iter().any(|&x| {
        b.iter().any(|&y| {
            x == y
                || matches!((x, y), (CellId::Projected(x, k), CellId::Projected(y, l))
            if k == l && state.alloc_paths_may_share(&[x], &[], &[y], &[]))
        })
    }) {
        Overlap::Yes
    } else if a.is_empty() && a_coverage == Coverage::Complete
        || b.is_empty() && b_coverage == Coverage::Complete
    {
        Overlap::No
    } else if a_coverage == Coverage::Partial || b_coverage == Coverage::Partial {
        Overlap::Unknown
    } else {
        Overlap::No
    }
}

fn writable(footprint: &AccessFootprint) -> bool {
    footprint.usage.kind() == AccessKind::Mutable
}

fn deep(footprint: &AccessFootprint) -> bool {
    matches!(
        footprint.usage,
        PlaceUse::Read | PlaceUse::Mutate { deep: true }
    )
}

fn touches_entries(key: FieldKey) -> bool {
    matches!(key, FieldKey::Elem | FieldKey::MapKey)
}

fn projected_edit_overlaps(
    state: &BorrowState,
    edit: &AccessFootprint,
    observer: &AccessFootprint,
) -> Overlap {
    if !writable(edit) || !edit.usage.touches_cell() {
        return Overlap::No;
    }
    let entries = matches!(
        observer.usage,
        PlaceUse::ReadEntries | PlaceUse::WriteEntries
    );
    let descendants = deep(observer);
    let Some(content) = observer.content.as_ref() else {
        return Overlap::No;
    };
    if !entries && !descendants {
        return Overlap::No;
    }
    let mut projected = false;
    for &cell in edit.address.cells() {
        let CellId::Projected(holder, key) = cell else {
            continue;
        };
        if entries && !touches_entries(key) {
            continue;
        }
        projected = true;
        if state.alloc_reaches_typed(&content.ids, holder, descendants, writable(observer)) {
            return Overlap::Yes;
        }
    }
    if projected && content.coverage == Coverage::Partial
        || edit.address.may_coverage() == Coverage::Partial
            && (!content.ids.is_empty() || content.coverage == Coverage::Partial)
    {
        return Overlap::Unknown;
    }
    Overlap::No
}

fn graph_reaches(state: &BorrowState, from: &AccessFootprint, targets: &[AllocId]) -> bool {
    deep(from)
        && from.content.as_ref().is_some_and(|content| {
            content.ids.iter().any(|&id| {
                targets.iter().any(|&target| {
                    id != target && state.alloc_reaches_typed(&[id], target, true, writable(from))
                })
            })
        })
}

fn graph_relation(state: &BorrowState, a: &AccessFootprint, b: &AccessFootprint) -> Overlap {
    let (Some(x), Some(y)) = (&a.content, &b.content) else {
        return Overlap::No;
    };
    if state.alloc_paths_may_share(&x.ids, &x.views, &y.ids, &y.views)
        || graph_reaches(state, a, &y.ids)
        || graph_reaches(state, b, &x.ids)
    {
        return Overlap::Yes;
    }
    if deep(a) && deep(b) && state.has_alloc_edges() {
        // Distinct roots can share a child reachable from neither root's peer.
        let descendants = if writable(a) {
            state.alloc_writable_closure(&x.ids)
        } else {
            state.alloc_readable_closure(&x.ids)
        };
        if descendants
            .iter()
            .any(|&child| state.alloc_reaches_typed(&y.ids, child, true, writable(b)))
        {
            return Overlap::Yes;
        }
    }
    if x.coverage == Coverage::Partial && (!y.ids.is_empty() || y.coverage == Coverage::Partial)
        || y.coverage == Coverage::Partial && !x.ids.is_empty()
    {
        return Overlap::Unknown;
    }
    Overlap::No
}

pub(super) fn overlap(state: &BorrowState, a: &AccessFootprint, b: &AccessFootprint) -> Overlap {
    overlap_with_detachment(state, a, b, false)
}

fn overlap_with_detachment(
    state: &BorrowState,
    a: &AccessFootprint,
    b: &AccessFootprint,
    detached: bool,
) -> Overlap {
    if !writable(a) && !writable(b) {
        return Overlap::No;
    }
    if let (Some(a), Some(b)) = (a.implicit_slot(), b.implicit_slot()) {
        return if a == b { Overlap::Yes } else { Overlap::No };
    }
    let mut result = Overlap::No;
    let mut consider = |relation| {
        if relation == Overlap::Unknown {
            result = Overlap::Unknown;
        }
        relation == Overlap::Yes
    };
    if ((writable(a) && a.usage.touches_cell()) || (writable(b) && b.usage.touches_cell()))
        && consider(cell_relation(
            state,
            a.address.cells(),
            a.address.may_coverage(),
            b.address.cells(),
            b.address.may_coverage(),
        ))
    {
        return Overlap::Yes;
    }
    for (writer, reader) in [(a, b), (b, a)] {
        if writable(writer)
            && writer.usage.touches_cell()
            && consider(cell_relation(
                state,
                writer.address.cells(),
                writer.address.may_coverage(),
                reader.traversal.as_slice(),
                reader.traversal_coverage,
            ))
        {
            return Overlap::Yes;
        }
        if consider(projected_edit_overlaps(state, writer, reader)) {
            return Overlap::Yes;
        }
    }
    if !detached && consider(graph_relation(state, a, b)) {
        return Overlap::Yes;
    }
    result
}

pub(super) fn footprint(resolved: &PlaceResolution, usage: PlaceUse) -> AccessFootprint {
    AccessFootprint {
        address: resolved.address.clone(),
        traversal: resolved.traversal.clone(),
        traversal_coverage: resolved.traversal_coverage,
        content: resolved.content.clone(),
        usage,
    }
}

fn into_footprint(resolved: PlaceResolution, usage: PlaceUse) -> AccessFootprint {
    AccessFootprint {
        address: resolved.address,
        traversal: resolved.traversal,
        traversal_coverage: resolved.traversal_coverage,
        content: resolved.content,
        usage,
    }
}

pub(super) fn check_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    access: AccessFootprint,
    source: Option<&PlaceResolution>,
    policy: MutableAccessPolicy,
) -> Result<(), BorrowCheckError> {
    validate_access(check, &access, source, policy)?;
    publish_access(check, roots, access);
    Ok(())
}

/// A successful optimisation probe publishes the checked reservation; a failed probe
/// never mutates statement facts or emits its transfer-only diagnostic.
pub(super) fn probe_mutable_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    mut reservation: AccessFootprint,
    source: Option<&PlaceResolution>,
    policy: MutableAccessPolicy,
) -> Result<Option<AccessFootprint>, BorrowCheckError> {
    let usage = reservation.usage;
    if usage.kind() == AccessKind::Shared {
        reservation.usage = PlaceUse::Mutate { deep: true };
    }
    let result = validate_access(check, &reservation, source, policy);
    reservation.usage = usage;
    match result {
        Ok(()) => {
            // The optional transfer is exclusive, but the callee receives only its declared access.
            check.tracker.report(roots, AccessKind::Mutable);
            publish_access(check, roots, reservation);
            Ok(None)
        }
        Err(BorrowCheckError::Diagnostic(_)) => Ok(Some(reservation)),
        Err(error) => Err(error),
    }
}

fn publish_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    footprint: AccessFootprint,
) {
    check.stats.conflicts_checked += 1;
    check.tracker.report(roots, footprint.usage.kind());
    check.tracker.reserve(AccessReservation {
        footprint,
        actor: check.actor_index_hint,
        span: check.span,
    });
}

fn validate_access(
    check: &AccessCheckContext<'_, '_>,
    access: &AccessFootprint,
    source: Option<&PlaceResolution>,
    policy: MutableAccessPolicy,
) -> Result<(), BorrowCheckError> {
    let actor = check.actor_index_hint;
    let kind = access.usage.kind();
    let mutable = kind == AccessKind::Mutable;
    let activity = AliasActivityContext {
        layout: check.layout,
        block_id: check.block_id,
        current_order: check.current_order,
        preserve_cfg_future_use_activity: preserves_cfg_future_use_activity(
            check.context,
            check.layout,
            actor,
        ),
    };
    let diagnostics = &check.context.diagnostics;
    let describe = |index: usize| diagnostics.local_place(check.layout.local_ids[index]);
    let place = || actor.map_or(DiagnosticPlace::Unknown, describe);
    let reject_mutation = |candidate: usize| {
        diagnostics.invalid_mutable_access(
            place(),
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
            Some(describe(candidate)),
            diagnostics.local_source_span(check.layout.local_ids[candidate]),
            check.span,
        )
    };
    // Stability is not discharged by derivation. If a CFG-live source alias also conflicts,
    // preserve its ordinary shared/mutable diagnostic.
    let receiver_loan = if mutable {
        receiver_loan_conflict(check, access)
    } else {
        None
    };
    let old_projection = OnceCell::new();
    let derivation = source
        .filter(|_| !policy.strict_move_exclusivity)
        .map(|place| check.state.binding(place.route.local).derivation());

    let candidate_conflicts = |local, wrapped, mode| {
        if Some(local) == actor
            || mode == LocalMode::UNINIT
            || derivation.is_some_and(|roots| roots.contains(local))
        {
            return false;
        }
        let source_alias = wrapped
            && check.layout.local_mutable[local]
            && !matches!(diagnostics.local_origin_kind(check.layout.local_ids[local]),
                Some(kind) if kind != HirLocalOriginKind::User);
        if !mutable && !source_alias {
            return false;
        }
        let active = is_local_active_for_alias_conflict(
            &activity,
            local,
            wrapped,
            source_alias,
            policy.strict_move_exclusivity,
        ) || source_alias && !mutable && local_alias_never_read(check.layout, local);
        if !active && (!policy.strict_move_exclusivity || !wrapped) {
            return false;
        }
        let mut resolved = check
            .state
            .resolve_place_path(local, &check.layout.local_mutable, []);
        if !active && !resolved.address.cells().iter().any(|cell| {
            matches!(cell, CellId::Slot(WrapperId { age: WrapperAge::Current, local: owner }) if *owner != local)
        })
        {
            return false;
        }
        let slot = WrapperId::current(local);
        let owns_slot = !wrapped && resolved.address.definite() == Some(CellId::Slot(slot));
        let terminal_owner =
            owns_slot && source.is_some_and(|place| place.names_terminal_slot(slot));
        if terminal_owner && access.address.definite() == Some(CellId::Slot(slot)) {
            return false;
        }
        let usage = if source_alias {
            PlaceUse::Mutate { deep: true }
        } else {
            PlaceUse::Read
        };
        if check.layout.local_has_in_place_mutation_surface[local] {
            check
                .state
                .load_place_content(&mut resolved, &check.layout.local_mutable, false);
        }
        if owns_slot
            && source.and_then(PlaceResolution::projection_owner) == Some(slot)
            && let Some(content) = &resolved.content
            && !access.address.cells().is_empty()
            && access.address.cells().iter().all(|cell| {
                matches!(cell, CellId::Projected(holder, _) if
                check.state.alloc_reaches_typed(&content.ids, *holder, true, false))
            })
        {
            // A writer uses this owner's captured cell, not an independent observation.
            return false;
        }
        let holder_route = if !mutable
            && let Some(actor) = actor
            && source.is_some_and(|source| {
                source.route.projected
                    || source
                        .address
                        .cells()
                        .iter()
                        .any(|cell| matches!(cell, CellId::Slot(slot) if slot.local == actor))
            }) {
            resolved
                .projected_holder_route(actor)
                .map(|isolated| (actor, isolated))
        } else {
            None
        };
        let mut candidate = into_footprint(resolved, usage);
        if terminal_owner {
            // Exempt this slot alternative only. The owner's content may still observe a
            // different projected write alternative of a joined destination.
            candidate.address = AddressSet::from_cells([], Coverage::Complete);
        }
        if let Some((actor, isolated)) = holder_route
            && !holder_routes_separate(check.state, access, &candidate, actor, isolated)
        {
            return true;
        }
        let detached = access
            .content
            .as_ref()
            .zip(candidate.content.as_ref())
            .is_some_and(|(a, b)| {
                actor.is_some_and(|actor| check.state.detached_from(actor, &b.ids))
                    || check.state.detached_from(local, &a.ids)
            });
        if matches!(access.usage, PlaceUse::Replace)
            && wrapped
            && access.address.cells().iter().any(|cell| {
                matches!(
                    cell,
                    CellId::Projected(_, FieldKey::Field(_) | FieldKey::Payload { .. })
                )
            })
            && candidate.address.cells().iter().any(|cell| {
                matches!(
                    cell,
                    CellId::Projected(_, FieldKey::Field(_) | FieldKey::Payload { .. })
                )
            })
            && let Some(source) = source
            && !detached
        {
            // A retained field route can observe a shared old child. Replacing the cell
            // does not write that child or conflict with an independently acquired value.
            let old = old_projection.get_or_init(|| {
                let mut old = source.clone();
                check
                    .state
                    .load_place_content(&mut old, &check.layout.local_mutable, false);
                into_footprint(old, PlaceUse::Read)
            });
            if graph_relation(check.state, old, &candidate) != Overlap::No {
                return true;
            }
        }
        overlap_with_detachment(check.state, access, &candidate, detached) != Overlap::No
    };
    let mut conflicting = check.state.wrapper_locals().find(|&local| {
        if !mutable && !check.layout.local_mutable[local] {
            return false;
        }
        let mode = check.state.binding_mode(local);
        let alias = mode.contains(LocalMode::ALIAS);
        (mutable || alias) && candidate_conflicts(local, alias, mode)
    });
    if mutable
        && (access.content.is_some()
            || access
                .address
                .cells()
                .iter()
                .any(|cell| matches!(cell, CellId::Projected(..))))
    {
        let end = conflicting.unwrap_or(check.layout.local_count());
        let graph_edges = check.state.has_alloc_edges();
        let mut origins = SmallSet::Empty;
        let mut unknown_origin = false;
        let mut record_origin = |id| match check.state.alloc_origin_local(id) {
            Some(local) => origins.insert(local),
            None => unknown_origin = true,
        };
        if let Some(content) = &access.content {
            content.ids.iter().copied().for_each(&mut record_origin);
            if deep(access) && graph_edges {
                check
                    .state
                    .alloc_writable_closure(&content.ids)
                    .into_iter()
                    .for_each(&mut record_origin);
            }
        }
        for &cell in access.address.cells() {
            if let CellId::Projected(holder, _) = cell {
                record_origin(holder);
            }
        }
        let plain = |local| {
            if Some(local) == actor {
                return false;
            }
            let roots = check.state.binding(local).value_roots();
            let matching_origin = if roots.is_empty() {
                origins.as_slice().contains(&local)
            } else {
                graph_edges
                    || unknown_origin
                    || origins
                        .as_slice()
                        .iter()
                        .any(|&origin| roots.contains(origin))
            };
            matching_origin
                && !check.state.has_wrapped_binding(local)
                && candidate_conflicts(local, false, check.state.binding_mode(local))
        };
        conflicting = (0..end).find(|&local| plain(local)).or(conflicting);
        if let Some(local) = check
            .state
            .allocation_observer_locals()
            .filter(|&local| {
                local < conflicting.unwrap_or(check.layout.local_count())
                    && !check.state.has_wrapped_binding(local)
                    && candidate_conflicts(local, false, check.state.binding_mode(local))
            })
            .min()
        {
            conflicting = Some(local);
        }
    }
    if let Some(candidate) = conflicting {
        if !mutable
            || (!policy.strict_move_exclusivity
                && activity.preserve_cfg_future_use_activity
                && has_cfg_future_use_after_linear_expiry(
                    check.layout,
                    candidate,
                    check.block_id,
                    check.current_order,
                )
                && is_shared_alias_conflict(check.context, check.layout, candidate))
        {
            return Err(diagnostics.shared_mutable_conflict(
                place(),
                if mutable {
                    BorrowAccessKind::Shared
                } else {
                    BorrowAccessKind::Mutable
                },
                borrow_access_kind(kind),
                Some(describe(candidate)),
                None,
                check.span,
            ));
        }
        return Err(reject_mutation(receiver_loan.unwrap_or(candidate)));
    }
    if let Some(local) = receiver_loan {
        return Err(reject_mutation(local));
    }
    if let Some(existing) = check.tracker.conflict(access, |existing| {
        !(policy.allow_prior_shared && !writable(&existing.footprint))
            && overlap(check.state, access, &existing.footprint) != Overlap::No
    }) {
        let prior = existing.footprint.usage.kind();
        if mutable && prior == AccessKind::Mutable {
            return Err(diagnostics.multiple_mutable_borrows(
                place(),
                existing.actor.map(describe),
                existing.span,
                check.span,
            ));
        }
        return Err(diagnostics.shared_mutable_conflict(
            place(),
            borrow_access_kind(prior),
            borrow_access_kind(kind),
            existing.actor.map(describe),
            existing.span,
            check.span,
        ));
    }
    Ok(())
}

fn receiver_loan_conflict(
    check: &AccessCheckContext<'_, '_>,
    access: &AccessFootprint,
) -> Option<usize> {
    let mut loans = check.state.stability_locals().peekable();
    loans.peek()?;
    // Editing entries invalidates loans of lazy receivers reached through those entries.
    // Only a deep referent write follows independently stored allocations as well.
    let (roots, descendants) = match (&access.content, access.usage) {
        (Some(content), PlaceUse::Mutate { deep: true }) => (
            content.ids.as_slice(),
            check.state.alloc_writable_closure(&content.ids),
        ),
        (Some(content), PlaceUse::WriteEntries) => (content.ids.as_slice(), Vec::new()),
        _ => (&[][..], Vec::new()),
    };
    let edited_cells = match access.usage {
        PlaceUse::Replace | PlaceUse::Mutate { .. } => access.address.cells(),
        _ => &[],
    };
    let projected = edited_cells.iter().filter_map(|cell| match *cell {
        CellId::Projected(id, key) if touches_entries(key) => Some(id),
        _ => None,
    });
    let targets = roots.iter().chain(&descendants).copied().chain(projected);
    targets.clone().next()?;
    loans.find(|&local| {
        Some(local) != check.actor_index_hint
            && check.state.stability_overlaps(local, targets.clone())
            && check
                .layout
                .future_use_kind(check.block_id, local, check.current_order - 1)
                != FutureUseKind::None
    })
}

/// A read through the recorded holder occurrence is conservative across a rebind unless
/// both the old capture and the new holder prove independent slot ownership.
fn holder_routes_separate(
    state: &BorrowState,
    read: &AccessFootprint,
    writer: &AccessFootprint,
    actor: usize,
    isolated: bool,
) -> bool {
    let [CellId::Projected(holder, _)] = read.address.cells() else {
        return false;
    };
    if !state.alloc_singular(*holder)
        || read.address.coverage != Coverage::Complete
        || writer.address.coverage != Coverage::Complete
    {
        return false;
    }
    let siblings = writer
        .address
        .cells()
        .iter()
        .all(|cell| matches!(cell, CellId::Projected(other, _) if holder == other));
    siblings
        || isolated
            && state.alloc_origin_local(*holder) == Some(actor)
            && !state.has_layer_binding(actor)
            && writer
                .address
                .cells()
                .iter()
                .all(|cell| matches!(cell, CellId::Projected(other, _) if holder != other))
}

fn borrow_access_kind(access: AccessKind) -> BorrowAccessKind {
    match access {
        AccessKind::Shared => BorrowAccessKind::Shared,
        AccessKind::Mutable => BorrowAccessKind::Mutable,
    }
}

struct AliasActivityContext<'a> {
    layout: &'a FunctionLayout,
    block_id: BlockId,
    current_order: i32,
    preserve_cfg_future_use_activity: bool,
}

fn is_local_active_for_alias_conflict(
    activity: &AliasActivityContext<'_>,
    local: usize,
    wrapped: bool,
    source_alias: bool,
    strict: bool,
) -> bool {
    if !strict && !wrapped {
        return activity.layout.future_use_kind(
            activity.block_id,
            local,
            activity.current_order - 1,
        ) != FutureUseKind::None;
    }
    let mutable = activity.layout.local_mutable[local];
    if activity.layout.local_last_use_order[local] < 0 {
        return mutable;
    }
    if !activity
        .layout
        .local_is_expired(local, activity.current_order)
        && (!activity.preserve_cfg_future_use_activity
            || activity.layout.future_use_kind(
                activity.block_id,
                local,
                activity.current_order - 1,
            ) != FutureUseKind::None)
    {
        return true;
    }
    if activity.preserve_cfg_future_use_activity
        && ((source_alias
            && activity
                .layout
                .local_has_future_assignment(activity.block_id, local))
            || has_cfg_future_use_after_linear_expiry(
                activity.layout,
                local,
                activity.block_id,
                activity.current_order,
            ))
    {
        return true;
    }
    strict && mutable
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

fn local_alias_never_read(layout: &FunctionLayout, local_index: usize) -> bool {
    let first_write = layout.local_first_write_order[local_index];
    let last_use = layout.local_last_use_order[local_index];
    first_write >= 0 && first_write == last_use
}
