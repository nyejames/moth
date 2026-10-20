//! Schema rows: declared layouts, reserved flags, registry lookup and multi-bind code keys.
//!
//! WHAT: checks the pilot rows' word counts, the identity-row macro, the registry lookup and the
//! reason keys that the multi-bind codes share with the legacy descriptors.
//! WHY: a layout or key drift would otherwise surface only at render time, far from the producer
//! that caused it.

use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostic_payload::reason_keys::StableReasonKey;
use crate::compiler_frontend::compiler_messages::diagnostic_payload::stable_reason_keys_for_tests;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::UnitCode;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    INVALID_MULTI_BIND, InvalidMultiBindCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    NON_EXHAUSTIVE_MATCH, UNREACHABLE_MATCH_ARM,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    MISSING_IMPORT_TARGET, UNUSED_IMPORT,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    COMPILER_CAPACITY_EXCEEDED, EXPECTED_TOKEN,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    TYPE_MISMATCH, USE_OF_UNINITIALIZED_LOCAL,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::registry::family_for_code;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::diagnostic_family;
use crate::compiler_frontend::compiler_messages::is_well_formed_reason_key;
use std::collections::HashSet;

// An identity row sits in the TYPE band (2000-2999). It is reserved, so nothing produces it.
diagnostic_family! {
    identity TEST_IDENTITY_ROW {
        internal: 2999,
        stable: "MOTH-TYPE-0999",
        category: DiagnosticCategory::Type,
        reserved
    }
}

#[test]
fn pilot_layouts_declare_their_word_counts() {
    assert_eq!(EXPECTED_TOKEN.word_count(), 4);
    assert_eq!(EXPECTED_TOKEN.pending_mask(), 0);
    assert_eq!(TYPE_MISMATCH.word_count(), 3);
    assert_eq!(TYPE_MISMATCH.pending_mask(), 0b11);
    assert_eq!(INVALID_MULTI_BIND.word_count(), 4);
    assert_eq!(USE_OF_UNINITIALIZED_LOCAL.word_count(), 1);
    assert_eq!(COMPILER_CAPACITY_EXCEEDED.word_count(), 1);
    assert_eq!(UNREACHABLE_MATCH_ARM.word_count(), 0);
}

#[test]
fn label_carrying_pilots_declare_their_extra_data_kind() {
    assert!(matches!(
        NON_EXHAUSTIVE_MATCH.extra,
        ExtraDataKind::StringIds
    ));
    assert!(matches!(UNREACHABLE_MATCH_ARM.extra, ExtraDataKind::None));
}

#[test]
fn reserved_rows_are_flagged_and_pilots_are_not() {
    assert!(UNUSED_IMPORT.is_reserved());
    assert!(!MISSING_IMPORT_TARGET.is_reserved());
    assert!(TEST_IDENTITY_ROW.is_reserved());
}

#[test]
fn identity_rows_declare_code_and_category_only() {
    assert_eq!(TEST_IDENTITY_ROW.code.get(), 2999);
    assert_eq!(TEST_IDENTITY_ROW.stable_code, "MOTH-TYPE-0999");
    assert!(matches!(
        TEST_IDENTITY_ROW.category,
        DiagnosticCategory::Type
    ));
    assert!(TEST_IDENTITY_ROW.descriptor.is_none());
    assert_eq!(TEST_IDENTITY_ROW.word_count(), 0);
    assert!(TEST_IDENTITY_ROW.labels.is_empty());
}

#[test]
fn registry_finds_every_pilot_code_and_no_other() {
    for code in [
        1001, 1037, 2001, 3002, 3022, 3034, 3035, 3045, 3050, 4002, 4005, 5009,
    ] {
        assert!(
            family_for_code(DiagnosticCode::new(code)).is_some(),
            "pilot code {code} is registered"
        );
    }
    assert!(family_for_code(DiagnosticCode::new(9999)).is_none());
}

#[test]
fn multi_bind_codes_are_dense_unique_and_well_formed() {
    let mut keys = HashSet::new();
    for code in 1..=15 {
        let key = InvalidMultiBindCode::from_code(code)
            .expect("codes 1 to 15 are assigned")
            .stable_reason_key();
        assert!(is_well_formed_reason_key(key), "{key} is well formed");
        assert!(
            stable_reason_keys_for_tests().contains(&key),
            "{key} is in the shared key table"
        );
        keys.insert(key);
    }
    assert_eq!(keys.len(), 15);
    assert!(InvalidMultiBindCode::from_code(0).is_none());
    assert!(InvalidMultiBindCode::from_code(16).is_none());
}
