//! HTML-Wasm numeric capability predicates.
//!
//! WHAT: the one owner of which numeric domains, arithmetic operations and fast compound
//!       write-backs the experimental Wasm backend can lower.
//! WHY: backend feature validation must reject exactly what lowering refuses. Restating the
//!      supported domain sets in validation and lowering lets one edit accept a program the other
//!      rejects, so it fails with an internal lowering error instead of the precise target
//!      capability diagnostic.
//!
//! `$fast_math` does not widen target support: fast and ordinary arithmetic share the same
//! domains and operators. The predicate therefore never inspects the operation's semantics.

use crate::backends::wasm::lir::instructions::{WasmFastStoreConversion, WasmIntegerOperationKind};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::numeric::{
    FastStoreConversionClass, HirFastStoreConversion, HirNumericOp,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

/// Map one exact integer domain onto the LIR operation kind Wasm implements.
///
/// WHAT: exactly `I32`/`U32`/`I64`/`U64`; narrower fixed integers have no checked or fast path.
/// WHY: the checked lowering emits 32/64-bit range predicates and the fast lowering performs
///      native 32/64-bit modular arithmetic, so neither can carry an `I8`/`I16`/`U8`/`U16`
///      semantic domain. Returning `None` keeps capability rejection out of lowering.
pub(crate) fn integer_operation_kind(domain: NumericScalar) -> Option<WasmIntegerOperationKind> {
    match domain {
        NumericScalar::Fixed(FixedScalar::I32) => Some(WasmIntegerOperationKind::Signed32),
        NumericScalar::Fixed(FixedScalar::U32) => Some(WasmIntegerOperationKind::Unsigned32),
        NumericScalar::Fixed(FixedScalar::I64) => Some(WasmIntegerOperationKind::Signed64),
        NumericScalar::Fixed(FixedScalar::U64) => Some(WasmIntegerOperationKind::Unsigned64),
        _ => None,
    }
}

/// The native computation precision of one binary-float domain.
///
/// WHAT: returns the precision only for `F32`/`F64`, the two precisions with native Wasm
///       carriers.
/// WHY: capability validation and lowering must accept the same set, so the mapping from domain
///      to native precision has one owner. `F16` and non-float domains return `None`.
pub(crate) fn native_binary_float_precision(domain: NumericScalar) -> Option<BinaryFloatPrecision> {
    match domain.binary_float_precision() {
        Some(precision @ (BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64)) => {
            Some(precision)
        }
        _ => None,
    }
}

/// Whether one binary-float domain has a native Wasm computation carrier.
pub(crate) fn supports_binary_float_domain(domain: NumericScalar) -> bool {
    native_binary_float_precision(domain).is_some()
}

/// Whether this exact operator/domain pair has a Wasm lowering in either numeric mode.
///
/// WHAT: accepts the native integer domains and `F32`/`F64` for every binary-float operator.
///       `IntegerDivide` has no binary-float domain.
/// WHY: fast and ordinary arithmetic share one support surface (D5). The check deliberately
///      reads only operator and domain, so a fast operation never buys support that its ordinary
///      counterpart lacks.
pub(crate) fn supports_numeric_operation(op: HirNumericOp) -> bool {
    if op.domain.is_integer() {
        return integer_operation_kind(op.domain).is_some();
    }

    supports_binary_float_domain(op.domain) && op.operator != NumericOperator::IntegerDivide
}

/// Map one fast compound-assignment write-back onto the conversion Wasm performs.
///
/// WHAT: accepts integer wrap between supported `I32`/`U32`/`I64`/`U64` targets, `F32`/`F64`
///       truncation into those targets, integer-to-`F32`/`F64` widening and `F32`/`F64`
///       rounding. Narrow integer and `F16` domains return `None`.
/// WHY: the fast write-back is not an explicit cast, so its capability gate and its lowering must
///      share this one mapping. `None` is a capability rejection, never an approximation.
pub(crate) fn fast_store_conversion(
    conversion: HirFastStoreConversion,
) -> Option<WasmFastStoreConversion> {
    let class = conversion.classify()?;
    let source_is_signed = conversion
        .source
        .integer_range()
        .map(|(minimum, _)| minimum < 0)
        .unwrap_or(false);

    match class {
        FastStoreConversionClass::IntegerWrap => Some(WasmFastStoreConversion::IntegerWrap {
            source_signed: source_is_signed,
            target: integer_operation_kind(conversion.target)?,
        }),
        FastStoreConversionClass::FloatToInteger => {
            if !supports_binary_float_domain(conversion.source) {
                return None;
            }
            Some(WasmFastStoreConversion::FloatToInteger {
                target: integer_operation_kind(conversion.target)?,
            })
        }
        FastStoreConversionClass::IntegerToFloat => {
            if !conversion.source.is_integer() || !supports_binary_float_domain(conversion.target) {
                return None;
            }
            Some(WasmFastStoreConversion::IntegerToFloat {
                source_signed: source_is_signed,
                precision: conversion.target.binary_float_precision()?,
            })
        }
        FastStoreConversionClass::FloatToFloat => {
            if !supports_binary_float_domain(conversion.source)
                || !supports_binary_float_domain(conversion.target)
            {
                return None;
            }
            Some(WasmFastStoreConversion::FloatRound {
                precision: conversion.target.binary_float_precision()?,
            })
        }
    }
}

/// Whether this fast compound-assignment write-back has a native Wasm lowering.
pub(crate) fn supports_fast_store_conversion(conversion: HirFastStoreConversion) -> bool {
    fast_store_conversion(conversion).is_some()
}
