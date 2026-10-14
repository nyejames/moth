//! JavaScript helpers for `@core/random`.
//!
//! WHAT: emits the demand-driven `@core/random` helpers for their exact contracts.
//! WHY: `random_int` bounds are exactly I32, whose JS carrier is the exact Number carrier, and
//!      `random_float` returns F64 directly, so both helpers stay fixed to their exact contracts.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;

const RANDOM_INT_HELPER_NAME: &str = "__moth_random_int";
const RANDOM_FLOAT_HELPER_NAME: &str = "__moth_random_float";
const RANDOM_INT_JS: &str = "function __moth_random_int(min, max) { if (min > max) { var t = min; min = max; max = t; } if (min === max) return min; return Math.floor(Math.random() * (max - min + 1)) + min; }";
const RANDOM_FLOAT_JS: &str = "function __moth_random_float() { return Math.random(); }";

pub(crate) const CORE_RANDOM_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: RANDOM_INT_HELPER_NAME,
        source: RANDOM_INT_JS,
    },
    CoreJsHelper {
        name: RANDOM_FLOAT_HELPER_NAME,
        source: RANDOM_FLOAT_JS,
    },
];

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_random_helpers(&mut self) {
        if self.referenced_external_runtime_function(RANDOM_FLOAT_HELPER_NAME) {
            self.emit_javascript_source(RANDOM_FLOAT_JS);
        }

        if self.referenced_external_runtime_function(RANDOM_INT_HELPER_NAME) {
            self.emit_javascript_source(RANDOM_INT_JS);
        }
    }
}
