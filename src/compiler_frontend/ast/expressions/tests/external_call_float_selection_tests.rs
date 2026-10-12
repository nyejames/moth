//! External-call binary-float validation selection tests.
//!
//! WHAT: pins `external_call_requires_float_validation`, the single parse-time fact that decides
//!       whether HIR must guard a raw external success value.
//! WHY: the fact follows only the resolved success slot: exactly one binary-float result needs
//!      the guard, and no JavaScript glue, package or lowering shape may claim its own float
//!      validation instead.

use super::external_call_requires_float_validation;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn fixed_type(scalar: FixedScalar) -> TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

#[test]
fn every_single_binary_float_success_slot_requires_validation() {
    let environment = TypeEnvironment::new();
    for success_type in [
        fixed_type(FixedScalar::F16),
        fixed_type(FixedScalar::F32),
        fixed_type(FixedScalar::F64),
    ] {
        assert!(
            external_call_requires_float_validation(&[success_type], &environment),
            "one binary-float success slot must request the HIR guard at its resolved precision"
        );
    }
}

#[test]
fn non_float_success_slots_never_require_validation() {
    let environment = TypeEnvironment::new();
    for success_type in [
        fixed_type(FixedScalar::I32),
        fixed_type(FixedScalar::U64),
        builtin_type_ids::STRING,
        builtin_type_ids::BOOL,
    ] {
        assert!(
            !external_call_requires_float_validation(&[success_type], &environment),
            "only binary-float success slots request the HIR guard"
        );
    }
}

#[test]
fn arity_other_than_one_never_requires_validation() {
    let environment = TypeEnvironment::new();
    let float = fixed_type(FixedScalar::F64);
    assert!(
        !external_call_requires_float_validation(&[float, float], &environment),
        "the guard validates one success payload, so multi-slot results stay outside this fact"
    );
    assert!(
        !external_call_requires_float_validation(&[], &environment),
        "an empty resolved success list is never a single float boundary"
    );
}
