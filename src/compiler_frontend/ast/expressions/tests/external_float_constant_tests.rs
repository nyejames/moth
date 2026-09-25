//! Boundary-precision projection tests for host package `Float` constants.
//!
//! WHAT: covers the rounding and overflow rules `Expression::float_from_external_constant` applies
//!       when a host `f64` becomes a Moth `Float`.
//! WHY: `ExpressionKind::Float` is finite by contract and must hold a value exactly representable
//!      at the selected boundary precision, so a projection that skipped the rounding or admitted a
//!      non-finite result would leak an unrepresentable value into every later stage.

use super::{Expression, ExpressionKind};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticPayload,
};
use crate::compiler_frontend::datatypes::numeric_profile::FloatPrecision;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn external_float_constant_rounds_to_boundary_precision() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("HOST_PI");

    let narrow = Expression::float_from_external_constant(
        std::f64::consts::PI,
        FloatPrecision::Bits32,
        constant_name,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect("an in-range host float should project at either precision");
    let ExpressionKind::Float(narrow_value) = narrow.kind else {
        panic!("a projected host float should stay a float literal");
    };
    assert_eq!(
        narrow_value,
        f64::from(std::f32::consts::PI),
        "a 32-bit boundary should store the host value rounded to its precision"
    );

    let wide = Expression::float_from_external_constant(
        std::f64::consts::PI,
        FloatPrecision::Bits64,
        constant_name,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect("an in-range host float should project at either precision");
    let ExpressionKind::Float(wide_value) = wide.kind else {
        panic!("a projected host float should stay a float literal");
    };
    assert_eq!(
        wide_value,
        std::f64::consts::PI,
        "a 64-bit boundary should keep the host value unrounded"
    );
}

#[test]
fn external_float_constant_rejects_projection_overflowing_precision() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("HOST_HUGE");

    let error = Expression::float_from_external_constant(
        f64::MAX,
        FloatPrecision::Bits32,
        constant_name,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect_err("a host value beyond the boundary precision is not representable");

    let DiagnosticPayload::CompileTimeEvaluationError { reason, operation } = error.payload else {
        panic!("a precision overflow should report the compile-time evaluation lane");
    };
    assert_eq!(reason, CompileTimeEvaluationErrorReason::FloatOverflow);
    assert_eq!(
        operation,
        Some(constant_name),
        "the diagnostic should name the host constant that overflowed"
    );
}
