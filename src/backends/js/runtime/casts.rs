//! Cast helpers for the JS runtime.
//!
//! WHAT: emits demand-driven implementations of the builtin cast policy table for the JS backend.
//! WHY: each cast must preserve its source grammar, destination range, profile precision and
//!      structured error result instead of inheriting JavaScript's implicit conversions.

use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::JsNumericCarrier;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};
use std::collections::HashSet;

impl<'hir> JsEmitter<'hir> {
    /// Emits the cast helpers selected by reachable HIR policies.
    pub(crate) fn emit_runtime_cast_helpers(&mut self) {
        let mut emitted = HashSet::<&'static str>::new();

        if self
            .used_cast_policies
            .contains(&BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            })
        {
            self.emit_int_to_float32_helper(&mut emitted);
        }

        if self
            .used_cast_policies
            .contains(&BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Float,
                target: NumericScalar::Int,
            })
        {
            self.emit_cast_float_to_int(&mut emitted);
        }

        for policy in [
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float),
            BuiltinCastPolicyId::NumericToString(NumericScalar::Int),
            BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
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
                BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int) => {
                    self.emit_cast_int(&mut emitted);
                }
                BuiltinCastPolicyId::StringToNumeric(NumericScalar::Float) => {
                    self.emit_cast_float(&mut emitted);
                }
                BuiltinCastPolicyId::NumericToString(NumericScalar::Int) => {
                    self.emit_cast_int_to_string(&mut emitted);
                }
                BuiltinCastPolicyId::NumericToString(NumericScalar::Float) => {
                    self.emit_cast_float_to_string(&mut emitted);
                }
                BuiltinCastPolicyId::BoolToString => self.emit_cast_bool_to_string(&mut emitted),
                BuiltinCastPolicyId::CharToString => self.emit_cast_char_to_string(&mut emitted),
                BuiltinCastPolicyId::CharToInt => self.emit_cast_char_to_int(&mut emitted),
                BuiltinCastPolicyId::StringToError => self.emit_cast_string_to_error(&mut emitted),
                BuiltinCastPolicyId::ErrorToString => self.emit_cast_error_to_string(&mut emitted),
                BuiltinCastPolicyId::IntToChar => self.emit_cast_int_to_char(&mut emitted),
                BuiltinCastPolicyId::StringToBool => self.emit_cast_string_to_bool(&mut emitted),
                BuiltinCastPolicyId::StringToChar => self.emit_cast_string_to_char(&mut emitted),
                BuiltinCastPolicyId::NumericConversion { .. }
                | BuiltinCastPolicyId::ByteToU8
                | BuiltinCastPolicyId::U8ToByte
                | BuiltinCastPolicyId::NumericToString(_)
                | BuiltinCastPolicyId::StringToNumeric(_) => {}
            }
        }
    }

    fn emit_cast_int_range_helpers(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int_in_range") {
            return;
        }

        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        let (min, max) = carrier
            .integer_bounds_js()
            .expect("Int carrier always owns integer bounds");

        self.emit_line(&format!("const __BS_INT_CAST_MIN = {min};"));
        self.emit_line(&format!("const __BS_INT_CAST_MAX = {max};"));
        self.emit_line("function __moth_cast_int_in_range(value) {");
        self.with_indent(|emitter| match carrier {
            JsNumericCarrier::ExactInteger { .. } => emitter.emit_line(
                "return typeof value === \"number\" && Number.isInteger(value) && value >= __BS_INT_CAST_MIN && value <= __BS_INT_CAST_MAX;",
            ),
            JsNumericCarrier::BigInteger { .. } => emitter.emit_line(
                "return typeof value === \"bigint\" && value >= __BS_INT_CAST_MIN && value <= __BS_INT_CAST_MAX;",
            ),
            JsNumericCarrier::BinaryFloat { .. } => {
                unreachable!("Int carrier cannot be a binary float")
            }
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int") {
            return;
        }

        let invalid_format = BuiltinErrorCode::IntParseInvalidFormat;
        let invalid_format_code = JsNumericCarrier::int_literal(
            invalid_format.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let invalid_format_message = invalid_format.default_message();
        let out_of_range = BuiltinErrorCode::IntParseOutOfRange;
        let out_of_range_code = JsNumericCarrier::int_literal(
            out_of_range.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let out_of_range_message = out_of_range.default_message();
        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        self.emit_cast_int_range_helpers(emitted);

        self.emit_line("function __moth_cast_int(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value === \"bigint\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (!__moth_cast_int_in_range(value)) {");
                em.with_indent(|inner| inner.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                )));
                em.emit_line("}");
                em.emit_line("return { tag: \"ok\", value };");
            });
            emitter.emit_line("}");

            emitter.emit_line("if (typeof value === \"number\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (!Number.isFinite(value)) {");
                em.with_indent(|inner| inner.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                )));
                em.emit_line("}");
                em.emit_line("if (!Number.isInteger(value)) {");
                em.with_indent(|inner| inner.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};"
                )));
                em.emit_line("}");
                match carrier {
                    JsNumericCarrier::ExactInteger { .. } => {
                        em.emit_line("if (!__moth_cast_int_in_range(value)) {");
                        em.with_indent(|inner| inner.emit_line(&format!(
                            "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                        )));
                        em.emit_line("}");
                        em.emit_line("return { tag: \"ok\", value };");
                    }
                    JsNumericCarrier::BigInteger { .. } => {
                        em.emit_line("const parsed = BigInt(value);");
                        em.emit_line("if (!__moth_cast_int_in_range(parsed)) {");
                        em.with_indent(|inner| inner.emit_line(&format!(
                            "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                        )));
                        em.emit_line("}");
                        em.emit_line("return { tag: \"ok\", value: parsed };");
                    }
                    JsNumericCarrier::BinaryFloat { .. } => {
                        unreachable!("Int carrier cannot be a binary float")
                    }
                }
            });
            emitter.emit_line("}");

            emitter.emit_line("if (typeof value === \"string\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (/^-?(?:\\d+(?:_\\d+)*)$/.test(value)) {");
                em.with_indent(|inner| {
                    match carrier {
                        JsNumericCarrier::ExactInteger { .. } => {
                            inner.emit_line("const parsed = Number.parseInt(value.replace(/_/g, \"\"), 10);");
                            inner.emit_line("if (!__moth_cast_int_in_range(parsed)) {");
                            inner.with_indent(|deep| deep.emit_line(&format!(
                                "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                            )));
                            inner.emit_line("}");
                            inner.emit_line("return { tag: \"ok\", value: parsed };");
                        }
                        JsNumericCarrier::BigInteger { .. } => {
                            inner.emit_line("const parsed = BigInt(value.replace(/_/g, \"\"));");
                            inner.emit_line("if (!__moth_cast_int_in_range(parsed)) {");
                            inner.with_indent(|deep| deep.emit_line(&format!(
                                "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                            )));
                            inner.emit_line("}");
                            inner.emit_line("return { tag: \"ok\", value: parsed };");
                        }
                        JsNumericCarrier::BinaryFloat { .. } => {
                            unreachable!("Int carrier cannot be a binary float")
                        }
                    }
                });
                em.emit_line("}");
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};"
                ));
            });
            emitter.emit_line("}");

            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"Cast to Int only accepts Int, Float, or string values\", {invalid_format_code}, null, null) }};"
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float") {
            return;
        }

        let invalid_format = BuiltinErrorCode::FloatParseInvalidFormat;
        let invalid_format_code = JsNumericCarrier::int_literal(
            invalid_format.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let invalid_format_message = invalid_format.default_message();
        let out_of_range = BuiltinErrorCode::FloatParseOutOfRange;
        let out_of_range_code = JsNumericCarrier::int_literal(
            out_of_range.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let out_of_range_message = out_of_range.default_message();
        let precision =
            JsNumericCarrier::for_scalar(NumericScalar::Float, self.config.numeric_profile)
                .and_then(JsNumericCarrier::float_precision)
                .expect("Float always has a JavaScript binary-float carrier");

        if precision == BinaryFloatPrecision::Binary32 {
            self.emit_decimal_to_float32_helper(emitted);
        }

        self.emit_line("function __moth_cast_float(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value === \"number\") {");
            emitter.with_indent(|em| {
                if precision == BinaryFloatPrecision::Binary32 {
                    em.emit_line("value = Math.fround(value);");
                }
                em.emit_line("if (!Number.isFinite(value)) {");
                em.with_indent(|inner| inner.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                )));
                em.emit_line("}");
                em.emit_line("return { tag: \"ok\", value };");
            });
            emitter.emit_line("}");

            emitter.emit_line("if (typeof value === \"string\") {");
            emitter.with_indent(|em| {
                em.emit_line("if (/^-?\\d+(?:_\\d+)*(?:\\.\\d+(?:_\\d+)*)?(?:e[+-]?\\d+(?:_\\d+)*)?$/.test(value)) {");
                em.with_indent(|inner| {
                    inner.emit_line("const normalized = value.replace(/_/g, \"\");");
                    match precision {
                        BinaryFloatPrecision::Binary32 => {
                            inner.emit_line("const parsed = __moth_decimal_to_float32(normalized);");
                        }
                        BinaryFloatPrecision::Binary64 => {
                            inner.emit_line("const parsed = Number.parseFloat(normalized);");
                        }
                        BinaryFloatPrecision::Binary16 => {
                            unreachable!("the profile Float domain is never binary16")
                        }
                    }
                    inner.emit_line("if (!Number.isFinite(parsed)) {");
                    inner.with_indent(|deep| deep.emit_line(&format!(
                        "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                    )));
                    inner.emit_line("}");
                    inner.emit_line("return { tag: \"ok\", value: parsed };");
                });
                em.emit_line("}");
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};"
                ));
            });
            emitter.emit_line("}");

            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"Cast to Float only accepts Int, Float, or string values\", {invalid_format_code}, null, null) }};"
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Parses a decimal string as a binary32 value without an intermediate binary64 rounding.
    fn emit_decimal_to_float32_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_decimal_to_float32") {
            return;
        }

        self.emit_line("function __moth_decimal_to_float32(source) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const negative = source.startsWith(\"-\");");
            emitter.emit_line("const unsigned = negative ? source.slice(1) : source;");
            emitter.emit_line("const [mantissa, exponentText] = unsigned.split(\"e\");");
            emitter.emit_line("const decimalExponent = exponentText === undefined ? 0 : Number(exponentText);");
            emitter.emit_line("const point = mantissa.indexOf(\".\");");
            emitter.emit_line("const fractionalDigits = point < 0 ? 0 : mantissa.length - point - 1;");
            emitter.emit_line("let digits = mantissa.replace(\".\", \"\");");
            emitter.emit_line("const firstNonzero = digits.search(/[1-9]/);");
            emitter.emit_line("if (firstNonzero < 0) return negative ? -0 : 0;");
            emitter.emit_line("digits = digits.slice(firstNonzero);");
            emitter.emit_line("let powerOfTen = decimalExponent - fractionalDigits;");
            emitter.emit_line("while (digits.endsWith(\"0\")) {");
            emitter.with_indent(|em| {
                em.emit_line("digits = digits.slice(0, -1);");
                em.emit_line("powerOfTen++;");
            });
            emitter.emit_line("}");
            emitter.emit_line("const decimalOrder = digits.length + powerOfTen - 1;");
            emitter.emit_line("if (decimalOrder > 38) return negative ? -Infinity : Infinity;");
            emitter.emit_line("if (decimalOrder < -46) return negative ? -0 : 0;");
            emitter.emit_line("const numerator = BigInt(digits) * (powerOfTen > 0 ? 10n ** BigInt(powerOfTen) : 1n);");
            emitter.emit_line("const denominator = powerOfTen < 0 ? 10n ** BigInt(-powerOfTen) : 1n;");
            emitter.emit_line("let binaryExponent = numerator.toString(2).length - denominator.toString(2).length;");
            emitter.emit_line("if (binaryExponent >= 0 ? numerator < (denominator << BigInt(binaryExponent)) : (numerator << BigInt(-binaryExponent)) < denominator) binaryExponent--;");
            emitter.emit_line("let rounded;");
            emitter.emit_line("if (binaryExponent >= -126) {");
            emitter.with_indent(|em| {
                em.emit_line("const shift = 23 - binaryExponent;");
                em.emit_line("const scaledNumerator = shift >= 0 ? numerator << BigInt(shift) : numerator;");
                em.emit_line("const scaledDenominator = shift < 0 ? denominator << BigInt(-shift) : denominator;");
                em.emit_line("let significand = scaledNumerator / scaledDenominator;");
                em.emit_line("const remainder = scaledNumerator % scaledDenominator;");
                em.emit_line("const halfway = remainder * 2n;");
                em.emit_line("if (halfway > scaledDenominator || (halfway === scaledDenominator && (significand & 1n) !== 0n)) significand++;");
                em.emit_line("if (significand === 16_777_216n) {");
                em.with_indent(|inner| {
                    inner.emit_line("significand >>= 1n;");
                    inner.emit_line("binaryExponent++;");
                });
                em.emit_line("}");
                em.emit_line("rounded = binaryExponent > 127 ? Infinity : Number(significand) * 2 ** (binaryExponent - 23);");
            });
            emitter.emit_line("} else {");
            emitter.with_indent(|em| {
                em.emit_line("const scaledNumerator = numerator << 149n;");
                em.emit_line("let significand = scaledNumerator / denominator;");
                em.emit_line("const remainder = scaledNumerator % denominator;");
                em.emit_line("const halfway = remainder * 2n;");
                em.emit_line("if (halfway > denominator || (halfway === denominator && (significand & 1n) !== 0n)) significand++;");
                em.emit_line("rounded = significand === 8_388_608n ? 2 ** -126 : Number(significand) * 2 ** -149;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return negative ? -rounded : rounded;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float_to_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float_to_int") {
            return;
        }

        let invalid_value = BuiltinErrorCode::FloatCastToIntInvalidValue;
        let invalid_value_code = JsNumericCarrier::int_literal(
            invalid_value.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let invalid_value_message = invalid_value.default_message();
        let out_of_range = BuiltinErrorCode::FloatCastToIntOutOfRange;
        let out_of_range_code = JsNumericCarrier::int_literal(
            out_of_range.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let out_of_range_message = out_of_range.default_message();
        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        self.emit_cast_int_range_helpers(emitted);

        self.emit_line("function __moth_cast_float_to_int(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value !== \"number\" || !Number.isFinite(value)) {");
            emitter.with_indent(|em| em.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_value_message}\", {invalid_value_code}, null, null) }};"
            )));
            emitter.emit_line("}");
            emitter.emit_line("const truncated = Math.trunc(value);");
            match carrier {
                JsNumericCarrier::ExactInteger { .. } => {
                    emitter.emit_line("if (!__moth_cast_int_in_range(truncated)) {");
                    emitter.with_indent(|em| em.emit_line(&format!(
                        "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                    )));
                    emitter.emit_line("}");
                    emitter.emit_line("return { tag: \"ok\", value: truncated };");
                }
                JsNumericCarrier::BigInteger { .. } => {
                    emitter.emit_line("const integer = BigInt(truncated);");
                    emitter.emit_line("if (!__moth_cast_int_in_range(integer)) {");
                    emitter.with_indent(|em| em.emit_line(&format!(
                        "return {{ tag: \"err\", value: __moth_make_error(\"{out_of_range_message}\", {out_of_range_code}, null, null) }};"
                    )));
                    emitter.emit_line("}");
                    emitter.emit_line("return { tag: \"ok\", value: integer };");
                }
                JsNumericCarrier::BinaryFloat { .. } => {
                    unreachable!("Int carrier cannot be a binary float")
                }
            }
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Directly rounds an exact BigInt to binary32; Number(value) first could double-round.
    fn emit_int_to_float32_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        let source = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        let target =
            JsNumericCarrier::for_scalar(NumericScalar::Float, self.config.numeric_profile)
                .expect("Float always has a JavaScript numeric carrier");
        if source.float_precision().is_some()
            || target.float_precision() != Some(BinaryFloatPrecision::Binary32)
            || !matches!(source, JsNumericCarrier::BigInteger { .. })
            || !emitted.insert("__moth_int_to_float32")
        {
            return;
        }

        self.emit_line("function __moth_int_to_float32(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (value === 0n) return 0;");
            emitter.emit_line("const negative = value < 0n;");
            emitter.emit_line("const magnitude = negative ? -value : value;");
            emitter.emit_line("let exponent = magnitude.toString(2).length - 1;");
            emitter.emit_line("const shift = exponent - 23;");
            emitter.emit_line("let significand = shift > 0 ? magnitude >> BigInt(shift) : magnitude << BigInt(-shift);");
            emitter.emit_line("if (shift > 0) {");
            emitter.with_indent(|em| {
                em.emit_line("const remainder = magnitude - (significand << BigInt(shift));");
                em.emit_line("const halfway = 1n << BigInt(shift - 1);");
                em.emit_line("if (remainder > halfway || (remainder === halfway && (significand & 1n) !== 0n)) significand++;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (significand === 16_777_216n) {");
            emitter.with_indent(|em| {
                em.emit_line("significand >>= 1n;");
                em.emit_line("exponent++;");
            });
            emitter.emit_line("}");
            emitter.emit_line("const rounded = Number(significand) * 2 ** (exponent - 23);");
            emitter.emit_line("return negative ? -rounded : rounded;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int_to_string") {
            return;
        }
        self.emit_line("function __moth_cast_int_to_string(value) {");
        self.with_indent(|emitter| emitter.emit_line("return String(value);"));
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
        self.with_indent(|emitter| emitter.emit_line("return value ? \"true\" : \"false\";"));
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_char_to_string(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_char_to_string") {
            return;
        }
        self.emit_line("function __moth_cast_char_to_string(value) {");
        self.with_indent(|emitter| emitter.emit_line("return value;"));
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_char_to_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_char_to_int") {
            return;
        }

        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        let code_point = match carrier {
            JsNumericCarrier::ExactInteger { .. } => "value.codePointAt(0)",
            JsNumericCarrier::BigInteger { .. } => "BigInt(value.codePointAt(0))",
            JsNumericCarrier::BinaryFloat { .. } => {
                unreachable!("Int carrier cannot be a binary float")
            }
        };

        self.emit_line("function __moth_cast_char_to_int(value) {");
        self.with_indent(|emitter| emitter.emit_line(&format!("return {code_point};")));
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_string_to_error(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_string_to_error") {
            return;
        }
        let unknown_code = JsNumericCarrier::int_literal(
            BuiltinErrorCode::UnknownOrUnassigned.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        self.emit_line("function __moth_cast_string_to_error(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "return __moth_make_error(value, {unknown_code}, null, null);"
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
        self.with_indent(|emitter| emitter.emit_line("return __moth_error_message(value);"));
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int_to_char(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int_to_char") {
            return;
        }

        let invalid_codepoint = BuiltinErrorCode::IntCastToCharInvalidCodepoint;
        let invalid_codepoint_code = JsNumericCarrier::int_literal(
            invalid_codepoint.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let invalid_codepoint_message = invalid_codepoint.default_message();
        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");
        let zero = JsNumericCarrier::int_literal(0, self.config.numeric_profile)
            .expect("zero always fits the Int numeric profile");
        let maximum = JsNumericCarrier::int_literal(0x10FFFF, self.config.numeric_profile)
            .expect("maximum Unicode scalar fits the Int numeric profile");
        let surrogate_start = JsNumericCarrier::int_literal(0xD800, self.config.numeric_profile)
            .expect("surrogate boundary fits the Int numeric profile");
        let surrogate_end = JsNumericCarrier::int_literal(0xDFFF, self.config.numeric_profile)
            .expect("surrogate boundary fits the Int numeric profile");

        let (validation, code_point) = match carrier {
            JsNumericCarrier::ExactInteger { .. } => (
                format!(
                    "!Number.isInteger(value) || value < {zero} || value > {maximum} || (value >= {surrogate_start} && value <= {surrogate_end})"
                ),
                "value",
            ),
            JsNumericCarrier::BigInteger { .. } => (
                format!(
                    "typeof value !== \"bigint\" || value < {zero} || value > {maximum} || (value >= {surrogate_start} && value <= {surrogate_end})"
                ),
                "Number(value)",
            ),
            JsNumericCarrier::BinaryFloat { .. } => {
                unreachable!("Int carrier cannot be a binary float")
            }
        };

        self.emit_line("function __moth_cast_int_to_char(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!("if ({validation}) {{"));
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_codepoint_message}\", {invalid_codepoint_code}, null, null) }};"
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line(&format!(
                "return {{ tag: \"ok\", value: String.fromCodePoint({code_point}) }};"
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_string_to_bool(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_string_to_bool") {
            return;
        }
        let invalid_format = BuiltinErrorCode::StringParseBoolInvalidFormat;
        let invalid_format_code = JsNumericCarrier::int_literal(
            invalid_format.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let invalid_format_message = invalid_format.default_message();
        self.emit_line("function __moth_cast_string_to_bool(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const normalized = value.trim();");
            emitter.emit_line("if (normalized === \"true\") {");
            emitter.with_indent(|em| em.emit_line("return { tag: \"ok\", value: true };") );
            emitter.emit_line("}");
            emitter.emit_line("if (normalized === \"false\") {");
            emitter.with_indent(|em| em.emit_line("return { tag: \"ok\", value: false };") );
            emitter.emit_line("}");
            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};"
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
        let invalid_format_code = JsNumericCarrier::int_literal(
            invalid_format.as_i32() as i64,
            self.config.numeric_profile,
        )
        .expect("cast error code always fits the Int numeric profile");
        let invalid_format_message = invalid_format.default_message();
        self.emit_line("function __moth_cast_string_to_char(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const codePoints = Array.from(value);");
            emitter.emit_line("if (codePoints.length === 1) {");
            emitter.with_indent(|em| em.emit_line("return { tag: \"ok\", value: codePoints[0] };") );
            emitter.emit_line("}");
            emitter.emit_line(&format!(
                "return {{ tag: \"err\", value: __moth_make_error(\"{invalid_format_message}\", {invalid_format_code}, null, null) }};"
            ));
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
