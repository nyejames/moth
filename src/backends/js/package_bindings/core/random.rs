//! JavaScript helpers for `@core/random`.
//!
//! WHAT: implements the initial JS-backed random helper skeleton.
//! WHY: `random_float` now uses `InlineExpression` lowering; only `random_int` retains a helper.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_random_helpers(&mut self) {
        let helpers: &[(&str, &str)] = &[(
            "__moth_random_int",
            "function __moth_random_int(min, max) { if (min > max) { var t = min; min = max; max = t; } if (min === max) return min; return Math.floor(Math.random() * (max - min + 1)) + min; }",
        )];

        self.emit_referenced_core_helpers(helpers);
    }
}
