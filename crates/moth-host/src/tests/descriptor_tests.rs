//! Tests for the descriptor vocabulary the compiler reads across the crate boundary.

use crate::{HostFunction, HostType};

#[test]
fn descriptors_expose_the_declared_signature() {
    let function = HostFunction::new("parse_count")
        .parameter(HostType::String)
        .parameter(HostType::U8)
        .returns(HostType::optional(HostType::U32))
        .fallible();

    assert_eq!(function.name(), "parse_count");
    let signature = function.signature();
    assert_eq!(signature.parameters(), [HostType::String, HostType::U8]);
    assert_eq!(
        signature.success_values(),
        [HostType::optional(HostType::U32)]
    );
    assert!(signature.is_fallible());

    let plain = HostFunction::new("ping");
    assert!(plain.signature().parameters().is_empty());
    assert!(plain.signature().success_values().is_empty());
    assert!(!plain.signature().is_fallible());
}
