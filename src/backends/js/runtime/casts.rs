//! Cast helpers for the JS runtime.
//!
//! WHAT: implements the runtime side of the builtin cast policy table for the JavaScript backend.
//! WHY: explicit cast operations must return structured carriers (`{{ tag: "ok"/"err", value }}`)
//!      for fallible policies and plain values for infallible ones, so Moth's `Error!` and
//!      `cast ... catch:` semantics work uniformly.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::casts::numeric_limits::{I32_MAX, I32_MIN};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use std::collections::HashSet;

impl<'hir> JsEmitter<'hir> {
    /// Emits the cast runtime helpers used by the generated JS.
    ///
    /// WHAT: walks the used cast policies in a fixed, deterministic order and emits exactly the
    ///      helpers that the reachable code needs. `Int -> Float` is a JS identity and never
    ///      emits a helper; numeric parsing helpers are only emitted when at least one parsing
    ///      helper is reached.
    /// WHY: demand-driven emission keeps the prelude minimal. Modules that only use the identity
    ///      `Int -> Float` cast should not pay for numeric parsing helpers they never call, and
    ///      modules that only use a single numeric cast should still see only the helpers they
    ///      reach.
    pub(crate) fn emit_runtime_cast_helpers(&mut self) {
        let mut emitted = HashSet::<&'static str>::new();

        // Walk policies in a fixed dependency-aware order so shared helpers
        // are emitted before their callers and prelude output stays stable.
        for policy in [
            BuiltinCastPolicyId::StringToInt,
            BuiltinCastPolicyId::StringToFloat,
            BuiltinCastPolicyId::FloatToInt,
            BuiltinCastPolicyId::IntToString,
            BuiltinCastPolicyId::FloatToString,
            BuiltinCastPolicyId::BoolToString,
            BuiltinCastPolicyId::CharToString,
            BuiltinCastPolicyId::CharToInt,
            BuiltinCastPolicyId::StringToError,
            BuiltinCastPolicyId::ErrorToString,
            BuiltinCastPolicyId::IntToChar,
            BuiltinCastPolicyId::StringToBool,
            BuiltinCastPolicyId::StringToChar,
        ] {
            if !self.used_cast_policies.contains(&policy) {
                continue;
            }

            match policy {
                BuiltinCastPolicyId::StringToInt => self.emit_cast_int(&mut emitted),
                BuiltinCastPolicyId::StringToFloat => self.emit_cast_float(&mut emitted),
                BuiltinCastPolicyId::FloatToInt => self.emit_cast_float_to_int(&mut emitted),
                BuiltinCastPolicyId::IntToString => self.emit_cast_int_to_string(&mut emitted),
                BuiltinCastPolicyId::FloatToString => self.emit_cast_float_to_string(&mut emitted),
                BuiltinCastPolicyId::BoolToString => self.emit_cast_bool_to_string(&mut emitted),
                BuiltinCastPolicyId::CharToString => self.emit_cast_char_to_string(&mut emitted),
                BuiltinCastPolicyId::CharToInt => self.emit_cast_char_to_int(&mut emitted),
                BuiltinCastPolicyId::StringToError => self.emit_cast_string_to_error(&mut emitted),
                BuiltinCastPolicyId::ErrorToString => self.emit_cast_error_to_string(&mut emitted),
                BuiltinCastPolicyId::IntToChar => self.emit_cast_int_to_char(&mut emitted),
                BuiltinCastPolicyId::StringToBool => self.emit_cast_string_to_bool(&mut emitted),
                BuiltinCastPolicyId::StringToChar => self.emit_cast_string_to_char(&mut emitted),
                BuiltinCastPolicyId::IntToFloat => {}
            }
        }
    }

    /// Emits the shared signed i32 cast range constants and predicate.
    ///
    /// WHAT: `__moth_cast_int_in_range` and the `__BS_INT_CAST_MIN`/`__BS_INT_CAST_MAX`
    ///      constants are derived from the Rust `numeric_limits` owner so the JS
    ///      runtime cannot drift from the Rust-side fold policy.
    /// WHY: keeping one source of truth for the i32 bounds prevents the runtime from
    ///      accepting or rejecting values that the compiler already folded differently.
    fn emit_cast_int_range_helpers(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int_in_range") {
            return;
        }

        self.emit_line(&format!("const __BS_INT_CAST_MIN = {I32_MIN};"));
        self.emit_line(&format!("const __BS_INT_CAST_MAX = {I32_MAX};"));
        self.emit_line("function __moth_cast_int_in_range(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return Number.isInteger(value) && value >= __BS_INT_CAST_MIN && value <= __BS_INT_CAST_MAX;",
            );
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int") {
            return;
        }

        let int_invalid_format = BuiltinErrorCode::IntParseInvalidFormat;
        let int_invalid_format_code = int_invalid_format.as_i32();
        let int_invalid_format_message = int_invalid_format.default_message();

        let int_out_of_range = BuiltinErrorCode::IntParseOutOfRange;
        let int_out_of_range_code = int_out_of_range.as_i32();
        let int_out_of_range_message = int_out_of_range.default_message();

        self.emit_cast_int_range_helpers(emitted);

        self.emit_line("function __moth_cast_int(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value === \"number\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (!Number.isFinite(value) || !__moth_cast_int_in_range(value)) {");
                em.with_indent(|inner| {
                    inner.emit_line(&format!(
                        "return {{ tag: \"err\", value: __moth_make_error(\"{int_out_of_range_message}\", {int_out_of_range_code}, null, null) }};",
                    ));
                });
                em.emit_line("}");
                em.emit_line("if (Number.isInteger(value)) {");
                em.with_indent(|inner| inner.emit_line("return { tag: \"ok\", value };"));
                em.emit_line("}");
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"Float value is not an exact integer\", {int_invalid_format_code}, null, null) }};",
                ));
            });
            emitter.emit_line("}");

            emitter.emit_line("if (typeof value === \"string\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (/^-?(?:\\d+(?:_\\d+)*)$/.test(value)) {");
                em.with_indent(|inner| {
                    inner.emit_line("const parsed = Number.parseInt(value.replace(/_/g, ''), 10);");
                    inner.emit_line("if (!__moth_cast_int_in_range(parsed)) {");
                    inner.with_indent(|deep| {
                        deep.emit_line(&format!(
                            "return {{ tag: \"err\", value: __moth_make_error(\"{int_out_of_range_message}\", {int_out_of_range_code}, null, null) }};",
                        ));
                    });
                    inner.emit_line("}");
                    inner.emit_line("return { tag: \"ok\", value: parsed };");
                });
                em.emit_line("}");
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{int_invalid_format_message}\", {int_invalid_format_code}, null, null) }};",
                ));
            });
            emitter.emit_line("}");

            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"Cast to Int only accepts Int, Float, or string values\", {int_invalid_format_code}, null, null) }};",
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float") {
            return;
        }

        let float_invalid_format = BuiltinErrorCode::FloatParseInvalidFormat;
        let float_invalid_format_code = float_invalid_format.as_i32();
        let float_invalid_format_message = float_invalid_format.default_message();

        let float_out_of_range = BuiltinErrorCode::FloatParseOutOfRange;
        let float_out_of_range_code = float_out_of_range.as_i32();
        let float_out_of_range_message = float_out_of_range.default_message();

        self.emit_line("function __moth_cast_float(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value === \"number\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (!Number.isFinite(value)) {");
                em.with_indent(|inner| {
                    inner.emit_line(&format!(
                        "return {{ tag: \"err\", value: __moth_make_error(\"{float_out_of_range_message}\", {float_out_of_range_code}, null, null) }};",
                    ));
                });
                em.emit_line("}");
                em.emit_line("return { tag: \"ok\", value };");
            });
            emitter.emit_line("}");

            emitter.emit_line("if (typeof value === \"string\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (/^-?\\d+(?:_\\d+)*(?:\\.\\d+(?:_\\d+)*)?(?:e[+-]?\\d+(?:_\\d+)*)?$/.test(value)) {");
                em.with_indent(|inner| {
                    inner.emit_line("const parsed = Number.parseFloat(value.replace(/_/g, \"\"));");
                    inner.emit_line("if (!Number.isFinite(parsed)) {");
                    inner.with_indent(|deep| {
                        deep.emit_line(&format!(
                            "return {{ tag: \"err\", value: __moth_make_error(\"{float_out_of_range_message}\", {float_out_of_range_code}, null, null) }};",
                        ));
                    });
                    inner.emit_line("}");
                    inner.emit_line("return { tag: \"ok\", value: parsed };");
                });
                em.emit_line("}");
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{float_invalid_format_message}\", {float_invalid_format_code}, null, null) }};",
                ));
            });
            emitter.emit_line("}");

            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"Cast to Float only accepts Int, Float, or string values\", {float_invalid_format_code}, null, null) }};",
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float_to_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float_to_int") {
            return;
        }

        let invalid_value = BuiltinErrorCode::FloatCastToIntInvalidValue;
        let invalid_value_code = invalid_value.as_i32();
        let invalid_value_message = invalid_value.default_message();

        let out_of_range = BuiltinErrorCode::FloatCastToIntOutOfRange;
        let out_of_range_code = out_of_range.as_i32();
        let out_of_range_message = out_of_range.default_message();

        self.emit_cast_int_range_helpers(emitted);

        self.emit_line("function __moth_cast_float_to_int(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value !== \"number\" || !Number.isFinite(value)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_value_message}\", {invalid_value_code}, null, null) }};",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const truncated = Math.trunc(value);");
            emitter.emit_line("if (!__moth_cast_int_in_range(truncated)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: truncated };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int_to_string") {
            return;
        }

        self.emit_line("function __moth_cast_int_to_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return String(value);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float_to_string") {
            return;
        }

        self.emit_line("function __moth_cast_float_to_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return __moth_numeric_trap(__moth_format_float(value));");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_bool_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_bool_to_string") {
            return;
        }

        self.emit_line("function __moth_cast_bool_to_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return value ? \"true\" : \"false\";");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_char_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_char_to_string") {
            return;
        }

        self.emit_line("function __moth_cast_char_to_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return value;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_char_to_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_char_to_int") {
            return;
        }

        self.emit_line("function __moth_cast_char_to_int(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return value.codePointAt(0);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_string_to_error(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_string_to_error") {
            return;
        }

        let unknown_code = BuiltinErrorCode::UnknownOrUnassigned.as_i32();

        self.emit_line("function __moth_cast_string_to_error(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "return __moth_make_error(value, {unknown_code}, null, null);",
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_error_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_error_to_string") {
            return;
        }

        self.emit_line("function __moth_cast_error_to_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return __moth_error_message(value);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int_to_char(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int_to_char") {
            return;
        }

        let invalid_codepoint = BuiltinErrorCode::IntCastToCharInvalidCodepoint;
        let invalid_codepoint_code = invalid_codepoint.as_i32();
        let invalid_codepoint_message = invalid_codepoint.default_message();

        self.emit_line("function __moth_cast_int_to_char(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!Number.isInteger(value) || value < 0 || value > 0x10FFFF || (value >= 0xD800 && value <= 0xDFFF)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_codepoint_message}\", {invalid_codepoint_code}, null, null) }};",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: String.fromCodePoint(value) };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_string_to_bool(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_string_to_bool") {
            return;
        }

        let invalid_format = BuiltinErrorCode::StringParseBoolInvalidFormat;
        let invalid_format_code = invalid_format.as_i32();
        let invalid_format_message = invalid_format.default_message();

        self.emit_line("function __moth_cast_string_to_bool(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const normalized = value.trim();");
            emitter.emit_line("if (normalized === \"true\") {");
            emitter.with_indent(|em| em.emit_line("return { tag: \"ok\", value: true };"));
            emitter.emit_line("}");
            emitter.emit_line("if (normalized === \"false\") {");
            emitter.with_indent(|em| em.emit_line("return { tag: \"ok\", value: false };"));
            emitter.emit_line("}");
            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};",
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_string_to_char(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_string_to_char") {
            return;
        }

        let invalid_format = BuiltinErrorCode::StringParseCharInvalidFormat;
        let invalid_format_code = invalid_format.as_i32();
        let invalid_format_message = invalid_format.default_message();

        self.emit_line("function __moth_cast_string_to_char(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const codePoints = Array.from(value);");
            emitter.emit_line("if (codePoints.length === 1) {");
            emitter.with_indent(|em| em.emit_line("return { tag: \"ok\", value: codePoints[0] };"));
            emitter.emit_line("}");
            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};",
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
