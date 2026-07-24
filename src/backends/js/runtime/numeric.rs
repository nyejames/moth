//! Checked numeric helpers for the JavaScript runtime.
//!
//! WHAT: implements Moth Alpha `Int = i32` and `Float = finite f64` arithmetic, Float
//!       boundary validation, and Moth Float-to-String formatting for the HTML-JS backend.
//!       Every helper returns an internal fallible carrier (`{ tag, value }`); trap-mode lowering
//!       wraps that carrier in `__moth_numeric_trap` to extract the scalar success value or throw on
//!       failure.
//! WHY: HIR `NumericOp`, `FormatFloat`, and `ValidateFloat` expose explicit checked numeric and
//!      Float-handling effects; the JS runtime must preserve those semantics without rediscovering
//!      source operator shapes, failure modes, or formatting contexts.

use super::NumericRuntimeHelperUsage;
use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::casts::numeric_limits::{I32_MAX, I32_MIN};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use std::collections::HashSet;

impl<'hir> JsEmitter<'hir> {
    /// Emits the checked numeric helper groups needed by reachable HIR statements.
    ///
    /// WHAT: emits arithmetic helpers for `NumericOp`, Float formatting for `FormatFloat`, and
    ///      finite-Float boundary validation for `ValidateFloat`. The trap helper is shared by all
    ///      three statement families and is emitted only once.
    /// WHY: demand-driven emission keeps arithmetic-only bundles from growing a Float-formatting
    ///      prelude while still giving every checked numeric statement one carrier contract.
    pub(crate) fn emit_runtime_numeric_helpers(&mut self, usage: NumericRuntimeHelperUsage) {
        let mut emitted = HashSet::<&'static str>::new();

        if usage.numeric_ops {
            self.emit_numeric_int_range_constants(&mut emitted);
        }
        self.emit_numeric_trap_helper(&mut emitted);

        if usage.numeric_ops {
            self.emit_int_ok_helper(&mut emitted);
            self.emit_int_check_helper(&mut emitted);

            // Int helpers
            self.emit_int_add_helper(&mut emitted);
            self.emit_int_sub_helper(&mut emitted);
            self.emit_int_mul_helper(&mut emitted);
            self.emit_int_div_helper(&mut emitted);
            self.emit_int_mod_helper(&mut emitted);
            self.emit_int_pow_helper(&mut emitted);
            self.emit_int_neg_helper(&mut emitted);

            // Float arithmetic helpers
            self.emit_float_add_helper(&mut emitted);
            self.emit_float_sub_helper(&mut emitted);
            self.emit_float_mul_helper(&mut emitted);
            self.emit_float_div_helper(&mut emitted);
            self.emit_float_mod_helper(&mut emitted);
            self.emit_float_pow_helper(&mut emitted);
            self.emit_float_neg_helper(&mut emitted);
        }

        if usage.format_float {
            self.emit_format_float_helper(&mut emitted);
        }

        if usage.validate_float {
            self.emit_float_validate_helper(&mut emitted);
        }
    }

    fn emit_numeric_int_range_constants(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_numeric_int_range_constants") {
            return;
        }

        self.emit_line(&format!("const __BS_INT_MIN = {I32_MIN};"));
        self.emit_line(&format!("const __BS_INT_MAX = {I32_MAX};"));
        self.emit_line("");
    }

    fn emit_numeric_trap_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_numeric_trap") {
            return;
        }

        self.emit_line("function __moth_numeric_trap(carrier) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (carrier && carrier.tag === \"ok\") {");
            emitter.with_indent(|em| em.emit_line("return carrier.value;"));
            emitter.emit_line("}");
            emitter.emit_line("if (carrier && carrier.tag === \"err\") {");
            emitter.with_indent(|em| {
                em.emit_line("throw new Error(__moth_error_message(carrier.value));");
            });
            emitter.emit_line("}");
            emitter.emit_line(
                "throw new Error(\"Expected internal numeric result carrier during trap lowering\");",
            );
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_int_add_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_add") {
            return;
        }

        self.emit_int_binary_helper("__moth_int_add", "a + b")
    }

    fn emit_int_sub_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_sub") {
            return;
        }

        self.emit_int_binary_helper("__moth_int_sub", "a - b")
    }

    fn emit_int_mul_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_mul") {
            return;
        }

        self.emit_int_binary_helper("__moth_int_mul", "a * b")
    }

    fn emit_int_div_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_div") {
            return;
        }

        self.emit_line("function __moth_int_div(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::DivideByZero));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (a === __BS_INT_MIN && b === -1) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::IntOverflow));
            });
            emitter.emit_line("}");
            emitter.emit_line("return __moth_int_check(Math.trunc(a / b));");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_int_mod_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_mod") {
            return;
        }

        self.emit_line("function __moth_int_mod(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::DivideByZero));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (a === __BS_INT_MIN && b === -1) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::IntOverflow));
            });
            emitter.emit_line("}");
            emitter.emit_line("return __moth_int_check(a % b);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_int_pow_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_pow") {
            return;
        }

        let invalid_exponent_call = Self::error_result_call(BuiltinErrorCode::InvalidExponent);

        self.emit_line("function __moth_int_pow(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!Number.isInteger(b) || b < 0) {");
            emitter.with_indent(|em| em.emit_line(&invalid_exponent_call));
            emitter.emit_line("}");
            emitter.emit_line("const result = Math.pow(a, b);");
            emitter.emit_line("return __moth_int_check(result);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_int_neg_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_neg") {
            return;
        }

        self.emit_line("function __moth_int_neg(a) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (a === __BS_INT_MIN) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::IntOverflow));
            });
            emitter.emit_line("}");
            emitter.emit_line("return __moth_int_check(-a);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_float_add_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_add") {
            return;
        }

        self.emit_float_binary_helper("__moth_float_add", "a + b")
    }

    fn emit_float_sub_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_sub") {
            return;
        }

        self.emit_float_binary_helper("__moth_float_sub", "a - b")
    }

    fn emit_float_mul_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_mul") {
            return;
        }

        self.emit_float_binary_helper("__moth_float_mul", "a * b")
    }

    fn emit_float_div_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_div") {
            return;
        }

        self.emit_float_divmod_helper("__moth_float_div", "a / b")
    }

    fn emit_float_mod_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_mod") {
            return;
        }

        self.emit_float_divmod_helper("__moth_float_mod", "a % b")
    }

    fn emit_float_pow_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_pow") {
            return;
        }

        self.emit_line("function __moth_float_pow(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const result = Math.pow(a, b);");
            emitter.emit_line("if (!Number.isFinite(result)) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::FloatNonFinite));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: result };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_float_neg_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_neg") {
            return;
        }

        self.emit_line("function __moth_float_neg(a) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const result = -a;");
            emitter.emit_line("if (!Number.isFinite(result)) {");
            emitter.with_indent(|em| {
                em.emit_line(&Self::error_result_call(BuiltinErrorCode::FloatNonFinite));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: result };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    // Shared helper bodies for the simple int binary ops (add/sub/mul).
    fn emit_int_binary_helper(&mut self, name: &str, operation: &str) {
        self.emit_line(&format!("function {name}(a, b) {{"));
        self.with_indent(|emitter| {
            emitter.emit_line(&format!("const result = {operation};"));
            emitter.emit_line("return __moth_int_check(result);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    // Shared helper body for the simple float binary ops (add/sub/mul).
    fn emit_float_binary_helper(&mut self, name: &str, operation: &str) {
        let non_finite_call = Self::error_result_call(BuiltinErrorCode::FloatNonFinite);

        self.emit_line(&format!("function {name}(a, b) {{"));
        self.with_indent(|emitter| {
            emitter.emit_line(&format!("const result = {operation};"));
            emitter.emit_line("if (!Number.isFinite(result)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite_call));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: result };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    // Shared helper body for float division and modulo, which check divide-by-zero first.
    fn emit_float_divmod_helper(&mut self, name: &str, operation: &str) {
        let divide_by_zero_call = Self::error_result_call(BuiltinErrorCode::DivideByZero);
        let non_finite_call = Self::error_result_call(BuiltinErrorCode::FloatNonFinite);

        self.emit_line(&format!("function {name}(a, b) {{"));
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0) {");
            emitter.with_indent(|em| em.emit_line(&divide_by_zero_call));
            emitter.emit_line("}");
            emitter.emit_line(&format!("const result = {operation};"));
            emitter.emit_line("if (!Number.isFinite(result)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite_call));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: result };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn error_result_call(code: BuiltinErrorCode) -> String {
        let message = code.default_message();
        let code_value = code.as_i32();
        format!("return __moth_error_result(\"{message}\", {code_value});")
    }

    fn emit_int_ok_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_ok") {
            return;
        }

        self.emit_line("function __moth_int_ok(value) {");
        self.with_indent(|emitter| {
            // Moth `Int` is i32, which has no negative zero. JS arithmetic can create `-0`
            // through negation, division, and remainder, so normalize it at the helper boundary.
            emitter.emit_line("return { tag: \"ok\", value: Object.is(value, -0) ? 0 : value };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the shared signed i32 validation helper used by integer arithmetic results.
    ///
    /// WHAT: `__moth_int_check` returns a carrier: it reports `IntOverflow` for any value that is
    ///      not an integer or lies outside the signed i32 range, and otherwise returns the value
    ///      through `__moth_int_ok` so JS `-0` is normalized at the success boundary.
    /// WHY: the i32 range check and `-0` normalization were duplicated across every integer helper;
    ///      centralising them keeps the helper bodies small and prevents the range logic from drifting.
    fn emit_int_check_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_int_check") {
            return;
        }

        let overflow_call = Self::error_result_call(BuiltinErrorCode::IntOverflow);

        self.emit_line("function __moth_int_check(value) {");
        self.with_indent(|emitter| {
            // Moth `Int` is signed i32. Any non-integer or out-of-range result from an
            // operation is reported as an overflow so the trap/error path matches the HIR contract.
            emitter.emit_line(
                "if (!Number.isInteger(value) || value < __BS_INT_MIN || value > __BS_INT_MAX) {",
            );
            emitter.with_indent(|em| em.emit_line(&overflow_call));
            emitter.emit_line("}");
            emitter.emit_line("return __moth_int_ok(value);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_float_validate_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_float_validate") {
            return;
        }

        let non_finite_call = Self::error_result_call(BuiltinErrorCode::FloatBoundaryNonFinite);

        self.emit_line("function __moth_float_validate(value) {");
        self.with_indent(|emitter| {
            // Moth `Float` is finite `f64`; values entering from external/backend boundaries
            // must be checked explicitly rather than trusted implicitly.
            emitter.emit_line("if (!Number.isFinite(value)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite_call));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_format_float_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_format_float") {
            return;
        }

        let non_finite_call = Self::error_result_call(BuiltinErrorCode::FloatFormatInvariant);

        self.emit_line("function __moth_format_float(value) {");
        self.with_indent(|emitter| {
            // The formatter is defensive: valid Moth `Float` is finite, but non-finite values
            // can appear from unchecked JS boundaries during formatting if HIR is malformed.
            emitter.emit_line("if (!Number.isFinite(value)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite_call));
            emitter.emit_line("}");

            // Moth formats `-0.0` as the string "0" so negative zero is not observable in
            // text output.
            emitter.emit_line("if (Object.is(value, -0)) {");
            emitter.with_indent(|em| {
                em.emit_line("return { tag: \"ok\", value: \"0\" };");
            });
            emitter.emit_line("}");

            // JS `Number.prototype.toString()` produces a round-trippable decimal and already
            // matches Moth's exponent thresholds, lowercase `e`, and omitted trailing `.0` on
            // compliant engines. We only normalize the exponent sign to guarantee `+` for positive
            // exponents across engines.
            emitter.emit_line("let text = value.toString();");
            emitter
                .emit_line("text = text.replace(/e([+-]?)(\\d+)/i, function (_, sign, digits) {");
            emitter.with_indent(|em| {
                em.emit_line(r#"const explicitSign = sign === "-" ? "-" : "+";"#);
                em.emit_line(r#"return "e" + explicitSign + digits;"#);
            });
            emitter.emit_line("});");
            emitter.emit_line("return { tag: \"ok\", value: text };");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
