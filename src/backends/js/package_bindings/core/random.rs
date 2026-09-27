//! JavaScript helpers for `@core/random`.
//!
//! WHAT: emits a profile-selected `random_int` helper while `random_float` remains inline.
//! WHY: BigInt bounds must stay exact and never pass through Number arithmetic.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::JsNumericCarrier;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

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

pub(crate) const CORE_RANDOM_JS_HELPERS: &[CoreJsHelper] = &[CoreJsHelper {
    name: "__moth_random_int",
    source: "function __moth_random_int(min, max) { if (min > max) { var t = min; min = max; max = t; } if (min === max) return min; return Math.floor(Math.random() * (max - min + 1)) + min; }",
}];

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_random_helpers(&mut self) {
        if !self.referenced_external_runtime_function(CORE_RANDOM_JS_HELPERS[0].name) {
            return;
        }

        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        match carrier {
            JsNumericCarrier::ExactInteger { .. } => {
                self.emit_javascript_source(CORE_RANDOM_JS_HELPERS[0].source);
            }
            JsNumericCarrier::BigInteger { .. } => {
                let source = bigint_random_int_helper(carrier);
                self.emit_javascript_source(&source);
            }
            JsNumericCarrier::BinaryFloat { .. } => {
                unreachable!("Int carrier cannot be a binary float")
            }
        }
    }
}
