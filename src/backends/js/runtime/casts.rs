//! Cast helpers for the JS runtime.
//!
//! WHAT: emits demand-driven implementations of the builtin cast policy table for the JS backend.
//! WHY: each cast must preserve its source grammar, destination range, profile precision and
//!      structured error result instead of inheriting JavaScript's implicit conversions.

use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::{
    JsNumericCarrier, JsNumericConversion, JsNumericRuntimeHelper, binary_float_precision_bits,
};
use crate::compiler_frontend::builtins::casts::evidence::numeric_scalars;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};
use std::collections::HashSet;

impl<'hir> JsEmitter<'hir> {
    /// Emits the cast helpers selected by reachable HIR policies.
    pub(crate) fn emit_runtime_cast_helpers(&mut self) {
        let mut emitted = HashSet::<&'static str>::new();
        let profile = self.config.numeric_profile;
        let scalars = numeric_scalars().collect::<Vec<_>>();
        let mut float_parse_precisions = [false; 3];

        for scalar in &scalars {
            if self
                .used_cast_policies
                .contains(&BuiltinCastPolicyId::StringToNumeric(*scalar))
            {
                if scalar.is_integer() {
                    self.emit_cast_int(&mut emitted);
                } else if let Some(precision) = scalar.binary_float_precision(profile) {
                    float_parse_precisions[float_precision_index(precision)] = true;
                }
            }

            if self
                .used_cast_policies
                .contains(&BuiltinCastPolicyId::NumericToString(*scalar))
            {
                if scalar.is_integer() {
                    self.emit_cast_int_to_string(&mut emitted);
                } else if scalar.is_binary_float() {
                    self.emit_cast_float_to_string(&mut emitted);
                }
            }

            for target in &scalars {
                let policy = BuiltinCastPolicyId::NumericConversion {
                    source: *scalar,
                    target: *target,
                };
                if !self.used_cast_policies.contains(&policy) {
                    continue;
                }

                let conversion = JsNumericConversion::classify(*scalar, *target, profile)
                    .expect("numeric cast policies contain supported numeric scalars");
                for helper in conversion.required_helpers() {
                    match helper {
                        JsNumericRuntimeHelper::CastIntegerInRange => {
                            self.emit_cast_integer_in_range_helper(&mut emitted);
                        }
                        JsNumericRuntimeHelper::CastIntegerToInteger => {
                            self.emit_cast_integer_to_integer(&mut emitted);
                        }
                        JsNumericRuntimeHelper::BigIntToBinaryFloat => {
                            self.emit_bigint_to_binary_float_helper(&mut emitted);
                        }
                        JsNumericRuntimeHelper::CastIntegerToFloat => {
                            self.emit_cast_integer_to_float(&mut emitted);
                        }
                        JsNumericRuntimeHelper::NumericValueDisplay => {
                            self.emit_cast_numeric_value_display_helper(&mut emitted);
                        }
                        JsNumericRuntimeHelper::CastFloatToInteger => {
                            self.emit_cast_float_to_int(&mut emitted);
                        }
                        JsNumericRuntimeHelper::CastFloatToFloat => {
                            self.emit_cast_float_to_float(&mut emitted);
                        }
                    }
                }
            }
        }

        if float_parse_precisions.iter().any(|used| *used) {
            self.emit_cast_float(&mut emitted, float_parse_precisions);
        }

        for policy in [
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

    fn emit_cast_integer_in_range_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_integer_in_range") {
            return;
        }

        self.emit_line("function __moth_cast_integer_in_range(value, min, max) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value === \"bigint\") {");
            emitter.with_indent(|em| {
                em.emit_line("const minimum = typeof min === \"bigint\" ? min : BigInt(min);");
                em.emit_line("const maximum = typeof max === \"bigint\" ? max : BigInt(max);");
                em.emit_line("return value >= minimum && value <= maximum;");
            });
            emitter.emit_line("}");
            emitter.emit_line(
                "if (typeof value !== \"number\" || !Number.isInteger(value)) return false;",
            );
            emitter.emit_line("const integer = BigInt(value);");
            emitter.emit_line("const minimum = typeof min === \"bigint\" ? min : BigInt(min);");
            emitter.emit_line("const maximum = typeof max === \"bigint\" ? max : BigInt(max);");
            emitter.emit_line("return integer >= minimum && integer <= maximum;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
    fn emit_cast_debug_string_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_debug_string") {
            return;
        }

        self.emit_line("function __moth_debug_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const text = String(value);");
            emitter.emit_line(r#"let escaped = "\"";"#);
            emitter.emit_line("for (const character of text) {");
            emitter.with_indent(|em| {
                em.emit_line(r#"if (character === "\\") escaped += "\\\\";"#);
                em.emit_line(r#"else if (character === "\"") escaped += "\\\"";"#);
                em.emit_line(r#"else if (character === "\0") escaped += "\\0";"#);
                em.emit_line(r#"else if (character === "\n") escaped += "\\n";"#);
                em.emit_line(r#"else if (character === "\r") escaped += "\\r";"#);
                em.emit_line(r#"else if (character === "\t") escaped += "\\t";"#);
                em.emit_line(
                    r#"else if (character !== " " && /[\p{C}\p{Z}\p{Grapheme_Extend}\u115F\u1160\u3164\uFFA0]/u.test(character)) escaped += "\\u{" + character.codePointAt(0).toString(16) + "}";"#,
                );
                em.emit_line("else escaped += character;");
            });
            emitter.emit_line("}");
            emitter.emit_line(r#"return escaped + "\"";"#);
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_numeric_value_display_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_numeric_value_display") {
            return;
        }

        self.emit_line("function __moth_numeric_value_display(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(r#"if (typeof value === "bigint") return value.toString();"#);
            emitter.emit_line(r#"if (typeof value !== "number") return String(value);"#);
            emitter.emit_line("if (Number.isNaN(value)) return \"NaN\";");
            emitter.emit_line("if (value === Infinity) return \"inf\";");
            emitter.emit_line("if (value === -Infinity) return \"-inf\";");
            emitter.emit_line("if (Object.is(value, -0)) return \"-0\";");
            emitter.emit_line("const negative = value < 0;");
            emitter.emit_line("const text = String(Math.abs(value));");
            emitter.emit_line("const separator = text.indexOf(\"e\");");
            emitter.emit_line("if (separator < 0) return (negative ? \"-\" : \"\") + text;");
            emitter.emit_line("const coefficient = text.slice(0, separator);");
            emitter.emit_line("const exponent = Number(text.slice(separator + 1));");
            emitter.emit_line("const point = coefficient.indexOf(\".\");");
            emitter.emit_line("const digits = coefficient.replace(\".\", \"\");");
            emitter.emit_line("const decimalPosition = (point < 0 ? coefficient.length : point) + exponent;");
            emitter.emit_line("let expanded;");
            emitter.emit_line("if (decimalPosition <= 0) {");
            emitter.with_indent(|em| {
                em.emit_line("expanded = \"0.\" + \"0\".repeat(-decimalPosition) + digits;");
            });
            emitter.emit_line("} else if (decimalPosition >= digits.length) {");
            emitter.with_indent(|em| {
                em.emit_line("expanded = digits + \"0\".repeat(decimalPosition - digits.length);");
            });
            emitter.emit_line("} else {");
            emitter.with_indent(|em| {
                em.emit_line("expanded = digits.slice(0, decimalPosition) + \".\" + digits.slice(decimalPosition);");
            });
            emitter.emit_line("}");
            emitter.emit_line("return (negative ? \"-\" : \"\") + expanded;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_int") {
            return;
        }

        self.emit_cast_integer_in_range_helper(emitted);
        self.emit_cast_debug_string_helper(emitted);
        let invalid_format_code = error_code_js(
            BuiltinErrorCode::IntParseInvalidFormat,
            self.config.numeric_profile,
        );
        let out_of_range_code = error_code_js(
            BuiltinErrorCode::IntParseOutOfRange,
            self.config.numeric_profile,
        );

        self.emit_line("function __moth_cast_int(value, min, max, targetName) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "const invalidFormat = () => __moth_error_result(\"Cannot parse \" + targetName + \" from \" + __moth_debug_string(value), {invalid_format_code});"
            ));
            emitter.emit_line(&format!(
                "const outOfRange = () => __moth_error_result(\"Cannot parse \" + targetName + \" from \" + __moth_debug_string(value), {out_of_range_code});"
            ));
            emitter.emit_line("if (!/^-?(?:\\d+(?:_\\d+)*)$/.test(value)) return invalidFormat();");
            emitter.emit_line("if (min == 0 && value.startsWith(\"-\")) return outOfRange();");
            emitter.emit_line("const normalized = value.replace(/_/g, \"\");");
            emitter.emit_line("const parsedValue = typeof min === \"bigint\" ? BigInt(normalized) : Number.parseInt(normalized, 10);");
            emitter.emit_line("const parsed = Object.is(parsedValue, -0) ? 0 : parsedValue;");
            emitter.emit_line("if (!__moth_cast_integer_in_range(parsed, min, max)) return outOfRange();");
            emitter.emit_line("return { tag: \"ok\", value: parsed };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float(&mut self, emitted: &mut HashSet<&'static str>, used_precisions: [bool; 3]) {
        if !emitted.insert("__moth_cast_float") {
            return;
        }

        self.emit_cast_debug_string_helper(emitted);
        if used_precisions[0] || used_precisions[1] {
            self.emit_decimal_to_binary_float_helper(emitted);
        }

        let invalid_format_code = error_code_js(
            BuiltinErrorCode::FloatParseInvalidFormat,
            self.config.numeric_profile,
        );
        let out_of_range_code = error_code_js(
            BuiltinErrorCode::FloatParseOutOfRange,
            self.config.numeric_profile,
        );

        self.emit_line("function __moth_cast_float(value, precision, targetName) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "const invalidFormat = () => __moth_error_result(\"Cannot parse \" + targetName + \" from \" + __moth_debug_string(value), {invalid_format_code});"
            ));
            emitter.emit_line(&format!(
                "const outOfRange = () => __moth_error_result(\"Cannot parse \" + targetName + \" from \" + __moth_debug_string(value), {out_of_range_code});"
            ));
            emitter.emit_line("if (!/^-?\\d+(?:_\\d+)*(?:\\.\\d+(?:_\\d+)*)?(?:e[+-]?\\d+(?:_\\d+)*)?$/.test(value)) return invalidFormat();");
            emitter.emit_line("const normalized = value.replace(/_/g, \"\");");
            emitter.emit_line("let parsed;");
            let mut first_precision = true;
            for (precision, used) in [
                (BinaryFloatPrecision::Binary16, used_precisions[0]),
                (BinaryFloatPrecision::Binary32, used_precisions[1]),
                (BinaryFloatPrecision::Binary64, used_precisions[2]),
            ] {
                if !used {
                    continue;
                }
                let bits = binary_float_precision_bits(precision);
                let prefix = if first_precision { "if" } else { "else if" };
                match precision {
                    BinaryFloatPrecision::Binary16 | BinaryFloatPrecision::Binary32 => emitter
                        .emit_line(&format!(
                            "{prefix} (precision === {bits}) parsed = __moth_decimal_to_binary_float(normalized, {bits});"
                        )),
                    BinaryFloatPrecision::Binary64 => emitter.emit_line(&format!(
                        "{prefix} (precision === {bits}) parsed = Number.parseFloat(normalized);"
                    )),
                }
                first_precision = false;
            }
            if first_precision {
                emitter.emit_line("return invalidFormat();");
            } else {
                emitter.emit_line("else return invalidFormat();");
                emitter.emit_line("return Number.isFinite(parsed) ? { tag: \"ok\", value: parsed } : outOfRange();");
            }
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Parses decimal text directly into binary16 or binary32 without an intermediate binary64.
    fn emit_decimal_to_binary_float_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_decimal_to_binary_float") {
            return;
        }

        self.emit_line("function __moth_decimal_to_binary_float(source, precision) {");
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
            emitter.emit_line("if (decimalOrder > (precision === 16 ? 4 : 38)) return negative ? -Infinity : Infinity;");
            emitter.emit_line("if (decimalOrder < (precision === 16 ? -9 : -46)) return negative ? -0 : 0;");
            emitter.emit_line("const numerator = BigInt(digits) * (powerOfTen > 0 ? 10n ** BigInt(powerOfTen) : 1n);");
            emitter.emit_line("const denominator = powerOfTen < 0 ? 10n ** BigInt(-powerOfTen) : 1n;");
            emitter.emit_line("const fractionBits = precision === 16 ? 10 : 23;");
            emitter.emit_line("const minimumNormalExponent = precision === 16 ? -14 : -126;");
            emitter.emit_line("const maximumExponent = precision === 16 ? 15 : 127;");
            emitter.emit_line("let binaryExponent = numerator.toString(2).length - denominator.toString(2).length;");
            emitter.emit_line("if (binaryExponent >= 0 ? numerator < (denominator << BigInt(binaryExponent)) : (numerator << BigInt(-binaryExponent)) < denominator) binaryExponent--;");
            emitter.emit_line("let rounded;");
            emitter.emit_line("if (binaryExponent >= minimumNormalExponent) {");
            emitter.with_indent(|em| {
                em.emit_line("const shift = fractionBits - binaryExponent;");
                em.emit_line("const scaledNumerator = shift >= 0 ? numerator << BigInt(shift) : numerator;");
                em.emit_line("const scaledDenominator = shift < 0 ? denominator << BigInt(-shift) : denominator;");
                em.emit_line("let significand = scaledNumerator / scaledDenominator;");
                em.emit_line("const remainder = scaledNumerator % scaledDenominator;");
                em.emit_line("const halfway = remainder * 2n;");
                em.emit_line("if (halfway > scaledDenominator || (halfway === scaledDenominator && (significand & 1n) !== 0n)) significand++;");
                em.emit_line("if (significand === (1n << BigInt(fractionBits + 1))) {");
                em.with_indent(|inner| {
                    inner.emit_line("significand >>= 1n;");
                    inner.emit_line("binaryExponent++;");
                });
                em.emit_line("}");
                em.emit_line("rounded = binaryExponent > maximumExponent ? Infinity : Number(significand) * 2 ** (binaryExponent - fractionBits);");
            });
            emitter.emit_line("} else {");
            emitter.with_indent(|em| {
                em.emit_line("const unitExponent = minimumNormalExponent - fractionBits;");
                em.emit_line("const scaledNumerator = numerator << BigInt(-unitExponent);");
                em.emit_line("let significand = scaledNumerator / denominator;");
                em.emit_line("const remainder = scaledNumerator % denominator;");
                em.emit_line("const halfway = remainder * 2n;");
                em.emit_line("if (halfway > denominator || (halfway === denominator && (significand & 1n) !== 0n)) significand++;");
                em.emit_line("rounded = significand === (1n << BigInt(fractionBits)) ? 2 ** minimumNormalExponent : Number(significand) * 2 ** unitExponent;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return negative ? -rounded : rounded;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_integer_to_integer(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_integer_to_integer") {
            return;
        }

        let out_of_range_code = error_code_js(
            BuiltinErrorCode::IntCastOutOfRange,
            self.config.numeric_profile,
        );
        self.emit_line(
            "function __moth_cast_integer_to_integer(value, min, max, sourceName, targetName) {",
        );
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_cast_integer_in_range(value, min, max)) {");
            emitter.with_indent(|em| {
                em.emit_line("const message = sourceName + \" -> \" + targetName + \" source \" + String(value) + \" is out of \" + targetName + \" range\";");
                em.emit_line(&format!("return __moth_error_result(message, {out_of_range_code});"));
            });
            emitter.emit_line("}");
            emitter.emit_line("const converted = typeof min === \"bigint\" ? BigInt(value) : Number(value);");
            emitter.emit_line("return { tag: \"ok\", value: converted };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float_to_int(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float_to_int") {
            return;
        }

        self.emit_cast_numeric_value_display_helper(emitted);
        let invalid_value_code = error_code_js(
            BuiltinErrorCode::FloatCastToIntInvalidValue,
            self.config.numeric_profile,
        );
        let out_of_range_code = error_code_js(
            BuiltinErrorCode::FloatCastToIntOutOfRange,
            self.config.numeric_profile,
        );
        self.emit_line(
            "function __moth_cast_float_to_int(value, min, max, sourceName, targetName) {",
        );
        self.with_indent(|emitter| {
            emitter.emit_line("const valueText = __moth_numeric_value_display(value);");
            emitter.emit_line("if (typeof value !== \"number\" || !Number.isFinite(value)) {");
            emitter.with_indent(|em| {
                em.emit_line("const message = sourceName + \" -> \" + targetName + \" source \" + valueText + \" is not finite\";");
                em.emit_line(&format!("return __moth_error_result(message, {invalid_value_code});"));
            });
            emitter.emit_line("}");
            emitter.emit_line("const truncatedValue = Math.trunc(value);");
            emitter.emit_line("const truncated = Object.is(truncatedValue, -0) ? 0 : truncatedValue;");
            emitter.emit_line("if (!__moth_cast_integer_in_range(truncated, min, max)) {");
            emitter.with_indent(|em| {
                em.emit_line("const message = sourceName + \" -> \" + targetName + \" source \" + valueText + \" is out of \" + targetName + \" range\";");
                em.emit_line(&format!("return __moth_error_result(message, {out_of_range_code});"));
            });
            emitter.emit_line("}");
            emitter.emit_line("const converted = typeof min === \"bigint\" ? BigInt(truncated) : truncated;");
            emitter.emit_line("return { tag: \"ok\", value: converted };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_integer_to_float(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_integer_to_float") {
            return;
        }

        let non_finite_code = error_code_js(
            BuiltinErrorCode::FloatCastNonFinite,
            self.config.numeric_profile,
        );
        self.emit_line(
            "function __moth_cast_integer_to_float(value, precision, sourceName, targetName) {",
        );
        self.with_indent(|emitter| {
            emitter.emit_line("const rounded = typeof value === \"bigint\" ? __moth_bigint_to_binary_float(value, precision) : precision === 16 ? Math.f16round(value) : Math.fround(value);");
            emitter.emit_line("if (!Number.isFinite(rounded)) {");
            emitter.with_indent(|em| {
                em.emit_line("const message = sourceName + \" -> \" + targetName + \" source \" + String(value) + \" produced a non-finite value\";");
                em.emit_line(&format!("return __moth_error_result(message, {non_finite_code});"));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: rounded };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_cast_float_to_float(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_cast_float_to_float") {
            return;
        }

        self.emit_cast_numeric_value_display_helper(emitted);
        let non_finite_code = error_code_js(
            BuiltinErrorCode::FloatCastNonFinite,
            self.config.numeric_profile,
        );
        self.emit_line(
            "function __moth_cast_float_to_float(value, precision, sourceName, targetName) {",
        );
        self.with_indent(|emitter| {
            emitter.emit_line("const rounded = precision === 16 ? Math.f16round(value) : Math.fround(value);");
            emitter.emit_line("if (!Number.isFinite(rounded)) {");
            emitter.with_indent(|em| {
                em.emit_line("const valueText = __moth_numeric_value_display(value);");
                em.emit_line("const message = sourceName + \" -> \" + targetName + \" source \" + valueText + \" produced a non-finite value\";");
                em.emit_line(&format!("return __moth_error_result(message, {non_finite_code});"));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: rounded };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Rounds an exact BigInt directly to binary16 or binary32, avoiding Number double-rounding.
    fn emit_bigint_to_binary_float_helper(&mut self, emitted: &mut HashSet<&'static str>) {
        if !emitted.insert("__moth_bigint_to_binary_float") {
            return;
        }

        self.emit_line("function __moth_bigint_to_binary_float(value, precision) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (value === 0n) return 0;");
            emitter.emit_line("const negative = value < 0n;");
            emitter.emit_line("const magnitude = negative ? -value : value;");
            emitter.emit_line("const fractionBits = precision === 16 ? 10 : 23;");
            emitter.emit_line("let exponent = magnitude.toString(2).length - 1;");
            emitter.emit_line("const shift = exponent - fractionBits;");
            emitter.emit_line("let significand = shift > 0 ? magnitude >> BigInt(shift) : magnitude << BigInt(-shift);");
            emitter.emit_line("if (shift > 0) {");
            emitter.with_indent(|em| {
                em.emit_line("const remainder = magnitude - (significand << BigInt(shift));");
                em.emit_line("const halfway = 1n << BigInt(shift - 1);");
                em.emit_line("if (remainder > halfway || (remainder === halfway && (significand & 1n) !== 0n)) significand++;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (significand === (1n << BigInt(fractionBits + 1))) {");
            emitter.with_indent(|em| {
                em.emit_line("significand >>= 1n;");
                em.emit_line("exponent++;");
            });
            emitter.emit_line("}");
            emitter.emit_line("const rounded = Number(significand) * 2 ** (exponent - fractionBits);");
            emitter.emit_line("const signed = negative ? -rounded : rounded;");
            emitter.emit_line("return precision === 16 ? Math.f16round(signed) : Math.fround(signed);");
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
        self.emit_line("function __moth_cast_float_to_string(value, precision, targetName) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return __moth_numeric_trap(__moth_format_float(value, precision, targetName));",
            );
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

fn float_precision_index(precision: BinaryFloatPrecision) -> usize {
    match precision {
        BinaryFloatPrecision::Binary16 => 0,
        BinaryFloatPrecision::Binary32 => 1,
        BinaryFloatPrecision::Binary64 => 2,
    }
}

fn error_code_js(
    code: BuiltinErrorCode,
    profile: crate::compiler_frontend::datatypes::numeric_profile::NumericProfile,
) -> String {
    JsNumericCarrier::int_literal(code.as_i32() as i64, profile)
        .expect("builtin error code always fits the Int numeric profile")
}
