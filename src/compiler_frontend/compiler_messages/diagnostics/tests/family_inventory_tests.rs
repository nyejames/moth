//! Family inventory: the registry holds one row per non-infrastructure diagnostic kind.
//!
//! WHAT: compares `FAMILY_GROUPS` with `DiagnosticKind::all()` on stable code, category, descriptor,
//! registry lookup, and count.
//! WHY: a kind without a schema row or a row without a diagnostic kind breaks the shared inventory.

use crate::compiler_frontend::compiler_messages::DiagnosticKind;
use crate::compiler_frontend::compiler_messages::diagnostics::registry::{
    FAMILY_GROUPS, family_for_code,
};

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
            .filter(|row| row.descriptor.code == kind.code())
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
            kind_codes.contains(&row.descriptor.code),
            "{} has no diagnostic kind",
            row.descriptor.code
        );
    }
}

#[test]
fn family_row_matches_its_kind_on_category_descriptor_and_registry_code() {
    for kind in DiagnosticKind::all()
        .into_iter()
        .filter(|kind| !matches!(kind, DiagnosticKind::Infrastructure(_)))
    {
        let row = FAMILY_GROUPS
            .iter()
            .copied()
            .flat_map(|group| group.iter())
            .find(|row| row.descriptor.code == kind.code())
            .unwrap_or_else(|| panic!("{kind:?} has no family row"));
        assert_eq!(row.category, kind.category(), "{kind:?} category");
        assert_eq!(row.descriptor, kind.descriptor(), "{kind:?} descriptor");
        assert_eq!(
            family_for_code(row.code).map(|found| found.descriptor.code),
            Some(row.descriptor.code),
            "{kind:?} registry lookup"
        );
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
