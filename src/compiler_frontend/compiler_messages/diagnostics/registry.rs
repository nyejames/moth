//! The table of family rows, joined from every group, and the lookup by internal code.
//!
//! WHAT: `FAMILY_GROUPS` lists each group's rows. `family_for_code` finds the row for a code.
//! WHY: one table gives a single place to check, at compile time, that no two families share a code or a stable code.

use super::families::{
    calls_data, choices_control_flow, generics_traits_api, imports, names_declarations,
    project_config, templates, token_lexical, types_borrows_targets,
};
use super::record::DiagnosticCode;
use super::schema::FamilyRow;

/// Every group table. A group with no family rows holds an empty slice.
pub(super) const FAMILY_GROUPS: &[&[FamilyRow]] = &[
    token_lexical::FAMILIES,
    imports::FAMILIES,
    names_declarations::FAMILIES,
    calls_data::FAMILIES,
    choices_control_flow::FAMILIES,
    types_borrows_targets::FAMILIES,
    templates::FAMILIES,
    project_config::FAMILIES,
    generics_traits_api::FAMILIES,
];

/// The family row that declares `code`, if one does.
pub(super) fn family_for_code(code: DiagnosticCode) -> Option<&'static FamilyRow> {
    FAMILY_GROUPS
        .iter()
        .flat_map(|group| group.iter())
        .find(|family| family.code == code)
}

/// Fails the build when two families share an internal code or a stable code. Each pair is compared once.
const fn check_registry() {
    let mut group = 0;
    while group < FAMILY_GROUPS.len() {
        let rows = FAMILY_GROUPS[group];
        let mut row = 0;
        while row < rows.len() {
            let mut later_group = group;
            while later_group < FAMILY_GROUPS.len() {
                let later_rows = FAMILY_GROUPS[later_group];
                let mut later_row = if later_group == group { row + 1 } else { 0 };
                while later_row < later_rows.len() {
                    assert!(
                        later_rows[later_row].code.get() != rows[row].code.get(),
                        "two families share an internal code"
                    );
                    assert!(
                        !str_eq(
                            later_rows[later_row].descriptor.code,
                            rows[row].descriptor.code
                        ),
                        "two families share a stable code"
                    );
                    later_row += 1;
                }
                later_group += 1;
            }
            row += 1;
        }
        group += 1;
    }
}

/// Byte-wise string equality, usable in a `const` context where `str ==` is not.
const fn str_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

const _: () = check_registry();
