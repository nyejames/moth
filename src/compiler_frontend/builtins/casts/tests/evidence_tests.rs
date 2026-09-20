//! Evidence table unit tests for the builtin cast surface.
//!
//! WHAT: covers lookup, fallibility classification, and policy id dispatch
//!      for the initial builtin evidence rows.
//! WHY: the evidence table is the single source of truth for which (source,
//!      target) pairs are valid and how they fold. Tests here pin the table
//!      contents so they cannot drift silently.

use crate::compiler_frontend::builtins::casts::evidence::{
    builtin_evidence_rows, lookup_builtin_evidence,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};

#[test]
fn evidence_table_covers_every_initial_row() {
    let rows = builtin_evidence_rows();
    assert_eq!(rows.len(), 14);
}

#[test]
fn int_to_float_is_infallible_with_dedicated_policy() {
    let row = lookup_builtin_evidence(BuiltinCastTarget::Int, BuiltinCastTarget::Float)
        .expect("Int -> Float evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Infallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::IntToFloat);
}

#[test]
fn float_to_int_is_fallible_with_truncation_policy() {
    let row = lookup_builtin_evidence(BuiltinCastTarget::Float, BuiltinCastTarget::Int)
        .expect("Float -> Int evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Fallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::FloatToInt);
}

#[test]
fn single_lookup_row_carries_policy_and_fallibility() {
    let row = lookup_builtin_evidence(BuiltinCastTarget::String, BuiltinCastTarget::Bool)
        .expect("String -> Bool evidence should exist");
    assert_eq!(row.fallibility, BuiltinCastFallibility::Fallible);
    assert_eq!(row.policy, BuiltinCastPolicyId::StringToBool);
    assert!(lookup_builtin_evidence(BuiltinCastTarget::Bool, BuiltinCastTarget::Int).is_none());
}
