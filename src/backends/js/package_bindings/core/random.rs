//! JavaScript helpers for `@core/random`.
//!
//! WHAT: emits demand-driven random helpers selected for the active numeric profile.
//! WHY: Float32 must preserve `random_float`'s exclusive upper bound, while BigInt bounds stay exact.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::JsNumericCarrier;
use crate::compiler_frontend::datatypes::numeric_profile::FloatPrecision;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

const RANDOM_INT_HELPER_NAME: &str = "__moth_random_int";
const RANDOM_FLOAT_HELPER_NAME: &str = "__moth_random_float";
const RANDOM_INT_JS: &str = "function __moth_random_int(min, max) { if (min > max) { var t = min; min = max; max = t; } if (min === max) return min; return Math.floor(Math.random() * (max - min + 1)) + min; }";

const RANDOM_FLOAT_BINARY32_JS: &str =
    "function __moth_random_float() { return Math.min(Math.random(), 0.9999999403953552); }";
const RANDOM_FLOAT_BINARY64_JS: &str = "function __moth_random_float() { return Math.random(); }";

pub(crate) const BIGINT_RANDOM_INT_JS: &str = r#"function __moth_random_int(min, max) {
    if (typeof min !== "bigint" || typeof max !== "bigint") {
        throw new TypeError("random_int bounds must use the Int carrier");
    }
    if (
        min < __MOTH_INT_MIN__ ||
        min > __MOTH_INT_MAX__ ||
        max < __MOTH_INT_MIN__ ||
        max > __MOTH_INT_MAX__
    ) {
        throw new RangeError("random_int bounds exceed the Int range");
    }
    if (min > max) {
        const temporary = min;
        min = max;
        max = temporary;
    }
    if (min === max) return min;

    const range = max - min + 1n;
    const bitCount = (range - 1n).toString(2).length;
    let candidate;
    do {
        candidate = 0n;
        for (let remainingBits = bitCount; remainingBits > 0;) {
            const chunkBits = Math.min(remainingBits, 32);
            const chunk = Math.floor(Math.random() * (2 ** chunkBits));
            candidate = (candidate << BigInt(chunkBits)) | BigInt(chunk);
            remainingBits -= chunkBits;
        }
    } while (candidate >= range);
    return min + candidate;
}"#;

fn bigint_random_int_helper(carrier: JsNumericCarrier) -> String {
    let (minimum, maximum) = carrier
        .integer_bounds_js()
        .expect("Int carrier always has integer bounds");
    BIGINT_RANDOM_INT_JS
        .replace("__MOTH_INT_MIN__", &minimum)
        .replace("__MOTH_INT_MAX__", &maximum)
}

pub(crate) const CORE_RANDOM_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: RANDOM_INT_HELPER_NAME,
        source: RANDOM_INT_JS,
    },
    CoreJsHelper {
        name: RANDOM_FLOAT_HELPER_NAME,
        source: RANDOM_FLOAT_BINARY64_JS,
    },
];

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_random_helpers(&mut self) {
        if self.referenced_external_runtime_function(RANDOM_FLOAT_HELPER_NAME) {
            let source = match self.config.numeric_profile.float_precision {
                FloatPrecision::Bits32 => RANDOM_FLOAT_BINARY32_JS,
                FloatPrecision::Bits64 => RANDOM_FLOAT_BINARY64_JS,
            };
            self.emit_javascript_source(source);
        }

        if !self.referenced_external_runtime_function(RANDOM_INT_HELPER_NAME) {
            return;
        }

        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        match carrier {
            JsNumericCarrier::ExactInteger { .. } => {
                self.emit_javascript_source(RANDOM_INT_JS);
            }
            JsNumericCarrier::BigInteger { .. } => {
                let source = bigint_random_int_helper(carrier);
                self.emit_javascript_source(&source);
            }
            JsNumericCarrier::BinaryFloat { .. } => {
                unreachable!("Int carrier cannot be a binary float")
            }
            JsNumericCarrier::ScaledInteger { .. } => {
                unreachable!("Int carrier cannot be a Number")
            }
        }
    }
}
