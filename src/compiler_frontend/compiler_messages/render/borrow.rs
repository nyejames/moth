//! Borrow diagnostic prose.
//!
//! WHAT: renders borrow-access facts into user-facing conflict and ownership messages.
//! WHY: borrow payloads are structured facts; this module is the render-boundary owner
//! for turning those facts into stable text.

use super::*;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    InvalidMutableAccessView, MultipleMutableBorrowsView, SharedMutableConflictView,
    UseOfUninitializedLocalView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;

pub(crate) fn diagnostic_place_name(
    place: &DiagnosticPlace,
    context: DiagnosticRenderContext<'_>,
) -> String {
    match place {
        DiagnosticPlace::Local(name) | DiagnosticPlace::RenderedText(name) => {
            format!("`{}`", context.string_table.resolve(*name))
        }
        DiagnosticPlace::Path(path) => format!("`{}`", context.render_path(*path)),
        DiagnosticPlace::Unknown => "this value".to_string(),
    }
}

pub(crate) fn borrow_access_name(access: BorrowAccessKind) -> &'static str {
    match access {
        BorrowAccessKind::Shared => "shared",
        BorrowAccessKind::Mutable => "mutable",
        BorrowAccessKind::Move => "move",
    }
}

pub(crate) fn borrow_conflict_message(
    place: &DiagnosticPlace,
    existing_access: BorrowAccessKind,
    requested_access: BorrowAccessKind,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Cannot access {}: existing {} access conflicts with requested {} access.",
        diagnostic_place_name(place, context),
        borrow_access_name(existing_access),
        borrow_access_name(requested_access)
    )
}

pub(crate) fn multiple_mutable_borrows_message(
    place: &DiagnosticPlace,
    conflicting_place: Option<&DiagnosticPlace>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let place_name = diagnostic_place_name(place, context);

    match conflicting_place {
        Some(conflicting) => {
            let conflicting_name = diagnostic_place_name(conflicting, context);
            format!(
                "Cannot create another mutable access to {place_name} while {conflicting_name} is active. Reuse {conflicting_name}, or move the new access after its last use."
            )
        }
        None => format!(
            "Cannot create another mutable access to {place_name} while it is already mutably active. Reuse the existing access, or move the new access after its last use."
        ),
    }
}

/// Renders a mutable-borrow conflict from its stored places.
pub(crate) fn render_multiple_mutable_borrows(
    _diagnostic: &DiagnosticView<'_>,
    facts: &MultipleMutableBorrowsView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let place = facts.place.diagnostic_place();
    let conflicting_place = facts
        .conflicting_place
        .map(|place| place.diagnostic_place());
    RenderedPayload {
        message: multiple_mutable_borrows_message(&place, conflicting_place.as_ref(), context),
        guidance: Vec::new(),
    }
}

pub(crate) fn shared_mutable_conflict_message(
    place: &DiagnosticPlace,
    existing_access: BorrowAccessKind,
    requested_access: BorrowAccessKind,
    conflicting_place: Option<&DiagnosticPlace>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let place_name = diagnostic_place_name(place, context);
    let conflicting_name = conflicting_place
        .map(|place| diagnostic_place_name(place, context))
        .unwrap_or_else(|| place_name.clone());

    match (existing_access, requested_access) {
        (BorrowAccessKind::Mutable, BorrowAccessKind::Shared) => format!(
            "Cannot read {place_name} while mutable alias {conflicting_name} is still needed later. Read through {conflicting_name}, or move the later use of {conflicting_name} before this access."
        ),
        (BorrowAccessKind::Shared, BorrowAccessKind::Mutable) => format!(
            "Cannot mutably access {place_name} while shared alias {conflicting_name} is still needed later. Move the mutation after the last use of {conflicting_name}, or create an explicit copy when independent data is required."
        ),
        (BorrowAccessKind::Move, _) | (_, BorrowAccessKind::Move) => format!(
            "Cannot access {place_name} because it conflicts with a possible ownership transfer of {conflicting_name}."
        ),
        _ => format!(
            "Cannot access {place_name}: existing {} access conflicts with requested {} access.",
            borrow_access_name(existing_access),
            borrow_access_name(requested_access)
        ),
    }
}

/// Renders a shared/mutable access conflict from its typed access and place facts.
pub(crate) fn render_shared_mutable_conflict(
    _diagnostic: &DiagnosticView<'_>,
    facts: &SharedMutableConflictView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let place = facts.place.diagnostic_place();
    let conflicting_place = facts
        .conflicting_place
        .map(|place| place.diagnostic_place());
    RenderedPayload {
        message: shared_mutable_conflict_message(
            &place,
            facts.existing_access,
            facts.requested_access,
            conflicting_place.as_ref(),
            context,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_mutable_access_message(
    place: &DiagnosticPlace,
    reason: InvalidMutableAccessReason,
    conflicting_place: Option<&DiagnosticPlace>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let place_name = diagnostic_place_name(place, context);

    match reason {
        InvalidMutableAccessReason::ImmutablePlace => {
            format!("Cannot mutably access immutable local {place_name}.")
        }
        InvalidMutableAccessReason::OverlappingAccess => {
            format!(
                "Cannot mutably access {place_name} due to overlapping access in the same evaluation sequence."
            )
        }
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess => {
            let conflicting_name = conflicting_place
                .map(|place| diagnostic_place_name(place, context))
                .unwrap_or_else(|| "another live alias".to_string());
            format!(
                "Cannot mutably access {place_name} because {conflicting_name} may alias the same value."
            )
        }
    }
}

/// Renders an invalid mutable access from its stored reason and places.
pub(crate) fn render_invalid_mutable_access(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidMutableAccessView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let place = facts.place.diagnostic_place();
    let conflicting_place = facts
        .conflicting_place
        .map(|place| place.diagnostic_place());
    RenderedPayload {
        message: invalid_mutable_access_message(
            &place,
            facts.reason,
            conflicting_place.as_ref(),
            context,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn use_of_uninitialized_local_message(
    place: &DiagnosticPlace,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Use of {} before initialization or after scope end.",
        diagnostic_place_name(place, context)
    )
}

/// WHAT: renders a use of an uninitialized local from its compact place.
/// WHY: the stored place decodes to the same diagnostic place the legacy payload carries.
pub(crate) fn render_use_of_uninitialized_local(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UseOfUninitializedLocalView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: use_of_uninitialized_local_message(&facts.place.diagnostic_place(), context),
        guidance: Vec::new(),
    }
}
