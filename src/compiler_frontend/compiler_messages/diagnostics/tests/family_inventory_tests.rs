//! Family inventory: compact schema invariants for registered rows.
//!
//! WHAT: checks unique stable codes, registry lookup consistency, and category code bands.
//! WHY: each compact family row is the source of its stable identity and storage category.

use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::registry::{
    FAMILY_GROUPS, family_for_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    DiagnosticCategory, band_of,
};
use std::collections::HashSet;

#[test]
fn family_rows_have_unique_stable_codes() {
    let mut stable_codes = HashSet::new();

    for row in FAMILY_GROUPS.iter().copied().flat_map(|group| group.iter()) {
        assert!(
            stable_codes.insert(row.descriptor.code),
            "duplicate stable diagnostic code {}",
            row.descriptor.code
        );
        let expected_prefix = match row.category {
            DiagnosticCategory::Syntax => "MOTH-SYNTAX-",
            DiagnosticCategory::Type => "MOTH-TYPE-",
            DiagnosticCategory::Rule => "MOTH-RULE-",
            DiagnosticCategory::Import => "MOTH-IMPORT-",
            DiagnosticCategory::Borrow => "MOTH-BORROW-",
            DiagnosticCategory::Config => "MOTH-CONFIG-",
            DiagnosticCategory::DeferredFeature => "MOTH-DEFERRED-",
        };
        assert!(
            !row.descriptor.code.is_empty(),
            "schema row must have a stable external code"
        );
        assert!(
            !row.descriptor.title.is_empty(),
            "{} must have a user-facing title",
            row.descriptor.code
        );
        assert!(
            row.descriptor.code.starts_with(expected_prefix),
            "{} does not match its {:?} category",
            row.descriptor.code,
            row.category
        );
    }
}

#[test]
fn family_rows_keep_stable_descriptor_codes_and_default_severities() {
    let cases = [
        ("MOTH-SYNTAX-0001", DiagnosticSeverity::Error),
        ("MOTH-SYNTAX-0002", DiagnosticSeverity::Error),
        ("MOTH-SYNTAX-0003", DiagnosticSeverity::Error),
        ("MOTH-SYNTAX-0019", DiagnosticSeverity::Error),
        ("MOTH-SYNTAX-0037", DiagnosticSeverity::Error),
        ("MOTH-RULE-0001", DiagnosticSeverity::Error),
        ("MOTH-RULE-0021", DiagnosticSeverity::Warning),
        ("MOTH-TYPE-0001", DiagnosticSeverity::Error),
        ("MOTH-IMPORT-0005", DiagnosticSeverity::Error),
        ("MOTH-BORROW-0001", DiagnosticSeverity::Error),
        ("MOTH-CONFIG-0001", DiagnosticSeverity::Error),
        ("MOTH-CONFIG-0002", DiagnosticSeverity::Error),
        ("MOTH-CONFIG-0003", DiagnosticSeverity::Error),
        ("MOTH-DEFERRED-0001", DiagnosticSeverity::Error),
    ];

    let mut schema_rows_checked = 0;
    for (expected_code, expected_severity) in cases {
        let row = FAMILY_GROUPS
            .iter()
            .copied()
            .flat_map(|group| group.iter())
            .find(|row| row.descriptor.code == expected_code);
        let row = row.unwrap_or_else(|| panic!("{expected_code} has no schema row"));
        assert_eq!(row.descriptor.code, expected_code);
        assert_eq!(row.descriptor.default_severity, expected_severity);
        schema_rows_checked += 1;
    }
    assert_eq!(schema_rows_checked, 14);
}

#[test]
fn family_rows_match_their_registry_entries() {
    for row in FAMILY_GROUPS.iter().copied().flat_map(|group| group.iter()) {
        let registered = family_for_code(row.code)
            .unwrap_or_else(|| panic!("{} has no registry entry", row.descriptor.code));
        assert!(
            std::ptr::eq(registered, row),
            "{} lookup returned a different family row",
            row.descriptor.code
        );
        assert_eq!(
            registered.descriptor.code, row.descriptor.code,
            "{} registry entry has a different descriptor code",
            row.descriptor.code
        );
    }
}

#[test]
fn family_codes_belong_to_their_category_bands() {
    for row in FAMILY_GROUPS.iter().copied().flat_map(|group| group.iter()) {
        let code = row.code.get();
        let band = band_of(row.category);
        assert!(
            (band..=band + 999).contains(&code),
            "{} code {code} is outside its category band {band}..={}",
            row.descriptor.code,
            band + 999
        );
    }
}
