//! Borrow diagnostic prose.
//!
//! WHAT: renders borrow-access facts into user-facing conflict and ownership messages.
//! WHY: borrow payloads are structured facts; this module is the render-boundary owner
//! for turning those facts into stable text.

use super::*;

pub(crate) fn diagnostic_place_name(place: &DiagnosticPlace, string_table: &StringTable) -> String {
    match place {
        DiagnosticPlace::Local(name) | DiagnosticPlace::RenderedText(name) => {
            format!("`{}`", string_table.resolve(*name))
        }
        DiagnosticPlace::Path(path) => format!("`{}`", path.to_portable_string(string_table)),
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
    string_table: &StringTable,
) -> String {
    format!(
        "Cannot access {}: existing {} access conflicts with requested {} access.",
        diagnostic_place_name(place, string_table),
        borrow_access_name(existing_access),
        borrow_access_name(requested_access)
    )
}

pub(crate) fn multiple_mutable_borrows_message(
    place: &DiagnosticPlace,
    conflicting_place: Option<&DiagnosticPlace>,
    string_table: &StringTable,
) -> String {
    let place_name = diagnostic_place_name(place, string_table);

    match conflicting_place {
        Some(conflicting) => {
            let conflicting_name = diagnostic_place_name(conflicting, string_table);
            format!(
                "Cannot create another mutable access to {place_name} while {conflicting_name} is active. Reuse {conflicting_name}, or move the new access after its last use."
            )
        }
        None => format!(
            "Cannot create another mutable access to {place_name} while it is already mutably active. Reuse the existing access, or move the new access after its last use."
        ),
    }
}

pub(crate) fn shared_mutable_conflict_message(
    place: &DiagnosticPlace,
    existing_access: BorrowAccessKind,
    requested_access: BorrowAccessKind,
    conflicting_place: Option<&DiagnosticPlace>,
    string_table: &StringTable,
) -> String {
    let place_name = diagnostic_place_name(place, string_table);
    let conflicting_name = conflicting_place
        .map(|place| diagnostic_place_name(place, string_table))
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

pub(crate) fn use_after_possible_move_message(
    place: &DiagnosticPlace,
    string_table: &StringTable,
) -> String {
    format!(
        "Cannot use {} because it may have been moved or left its valid scope.",
        diagnostic_place_name(place, string_table)
    )
}

pub(crate) fn move_while_borrowed_message(
    place: &DiagnosticPlace,
    existing_access: BorrowAccessKind,
    string_table: &StringTable,
) -> String {
    format!(
        "Cannot transfer ownership of {} while it has an active {} access.",
        diagnostic_place_name(place, string_table),
        borrow_access_name(existing_access)
    )
}

pub(crate) fn whole_object_borrow_conflict_message(
    whole_place: &DiagnosticPlace,
    part_place: &DiagnosticPlace,
    string_table: &StringTable,
) -> String {
    format!(
        "Cannot access whole value {} while part {} is already active.",
        diagnostic_place_name(whole_place, string_table),
        diagnostic_place_name(part_place, string_table)
    )
}

pub(crate) fn invalid_mutable_access_message(
    place: &DiagnosticPlace,
    reason: InvalidMutableAccessReason,
    conflicting_place: Option<&DiagnosticPlace>,
    string_table: &StringTable,
) -> String {
    let place_name = diagnostic_place_name(place, string_table);

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
                .map(|place| diagnostic_place_name(place, string_table))
                .unwrap_or_else(|| "another live alias".to_string());
            format!(
                "Cannot mutably access {place_name} because {conflicting_name} may alias the same value."
            )
        }
    }
}

pub(crate) fn use_of_uninitialized_local_message(
    place: &DiagnosticPlace,
    string_table: &StringTable,
) -> String {
    format!(
        "Use of {} before initialization or after scope end.",
        diagnostic_place_name(place, string_table)
    )
}
