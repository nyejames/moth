//! Family inventory: the registry holds one row per non-infrastructure diagnostic kind.
//!
//! WHAT: compares `FAMILY_GROUPS` with `DiagnosticKind::all()` on stable code, category, and count.
//! WHY: a kind without a row has no schema identity, and a stray row owns a code no kind emits.

use crate::compiler_frontend::compiler_messages::DiagnosticKind;
use crate::compiler_frontend::compiler_messages::diagnostics::registry::FAMILY_GROUPS;

#[test]
fn every_non_infrastructure_kind_has_exactly_one_family_row() {
    for kind in DiagnosticKind::all()
        .into_iter()
        .filter(|kind| !matches!(kind, DiagnosticKind::Infrastructure(_)))
    {
        let rows = FAMILY_GROUPS
            .iter()
            .copied()
            .flat_map(|group| group.iter())
            .filter(|row| row.stable_code == kind.code())
            .count();
        assert_eq!(rows, 1, "{kind:?} must have exactly one family row");
    }
}

#[test]
fn every_family_row_belongs_to_a_non_infrastructure_kind() {
    let kind_codes: Vec<&'static str> = DiagnosticKind::all()
        .into_iter()
        .filter(|kind| !matches!(kind, DiagnosticKind::Infrastructure(_)))
        .map(|kind| kind.code())
        .collect();

    for row in FAMILY_GROUPS.iter().copied().flat_map(|group| group.iter()) {
        assert!(
            kind_codes.contains(&row.stable_code),
            "{} has no diagnostic kind",
            row.stable_code
        );
    }
}

#[test]
fn family_row_category_matches_its_kind() {
    for kind in DiagnosticKind::all()
        .into_iter()
        .filter(|kind| !matches!(kind, DiagnosticKind::Infrastructure(_)))
    {
        let row = FAMILY_GROUPS
            .iter()
            .copied()
            .flat_map(|group| group.iter())
            .find(|row| row.stable_code == kind.code())
            .unwrap_or_else(|| panic!("{kind:?} has no family row"));
        assert_eq!(row.category, kind.category(), "{kind:?} category");
    }
}

#[test]
fn family_row_count_matches_non_infrastructure_kind_count() {
    let kind_count = DiagnosticKind::all()
        .into_iter()
        .filter(|kind| !matches!(kind, DiagnosticKind::Infrastructure(_)))
        .count();
    let row_count: usize = FAMILY_GROUPS.iter().map(|group| group.len()).sum();

    assert_eq!(row_count, kind_count);
}
