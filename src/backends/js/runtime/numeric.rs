//! Checked numeric helpers for the JavaScript runtime.
//!
//! WHAT: emits carrier-parameterised bounded integer and binary-float operation families, exact
//!       scale-generic Number coefficient operations, profile-aware Float boundary checks and
//!       canonical Float-to-String formatting for HTML-JS.
//! WHY: HIR numeric statements already own operator domains and failure modes; this runtime only
//!      enforces each selected domain's exact semantics at its operation boundary.
//!
//! Every checked helper returns `{ tag, value }`. Trap-mode lowering extracts the success value or
//! throws, while builtin `Error!` lowering keeps the carrier for normal HIR recovery.

use super::NumericRuntimeHelperUsage;
use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::{JsNumericCarrier, binary_float_precision_bits};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};

impl<'hir> JsEmitter<'hir> {
    /// Emits only numeric helper families used by reachable HIR and cast statements.
    pub(crate) fn emit_runtime_numeric_helpers(&mut self, usage: NumericRuntimeHelperUsage) {
        let float_precision =
            JsNumericCarrier::for_scalar(NumericScalar::Float, self.config.numeric_profile)
                .and_then(JsNumericCarrier::float_precision)
                .expect("Float always has a JavaScript binary-float carrier");
        self.emit_numeric_trap_helper();
        if usage.binary_float_power {
            self.emit_javascript_source(include_str!("float_power.js"));
            self.emit_line("");
        }

        if usage.number_integer_ops {
            self.emit_integer_helpers("int", false);
        }

        if usage.big_integer_ops {
            self.emit_integer_helpers("bigint", true);
        }

        if usage.number_decimal_ops {
            self.emit_number_helpers();
        }

        if usage.binary32_ops {
            self.emit_float_helpers("float32", BinaryFloatPrecision::Binary32);
        }

        if usage.binary64_ops {
            self.emit_float_helpers("float", BinaryFloatPrecision::Binary64);
        }

        if usage.uses_float_formatter() {
            self.emit_format_float_helper(usage);
        }

        if usage.validate_float {
            self.emit_float_validate_helper(float_precision);
        }
    }

    fn emit_numeric_trap_helper(&mut self) {
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

    /// Emits exact Number coefficient arithmetic with one half-even result rounding boundary.
    ///
    /// Scale factors arrive as compile-time BigInt literals from validated HIR, so ordinary
    /// arithmetic never rebuilds a factor or routes a coefficient/exponent through JS Number.
    fn emit_number_helpers(&mut self) {
        let divide_by_zero = self.error_result_call(BuiltinErrorCode::DivideByZero);
        let invalid_exponent = self.error_result_call(BuiltinErrorCode::InvalidExponent);

        self.emit_line("function __moth_number_round_half_even(numerator, denominator) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (denominator === 1n) return numerator;");
            emitter.emit_line("if (denominator === -1n) return -numerator;");
            emitter.emit_line("let quotient = numerator / denominator;");
            emitter.emit_line("const remainder = numerator % denominator;");
            emitter.emit_line("const absoluteRemainder = remainder < 0n ? -remainder : remainder;");
            emitter.emit_line(
                "const absoluteDenominator = denominator < 0n ? -denominator : denominator;",
            );
            emitter.emit_line("const twiceRemainder = absoluteRemainder * 2n;");
            emitter.emit_line("if (twiceRemainder > absoluteDenominator || (");
            emitter.with_indent(|em| {
                em.emit_line("twiceRemainder === absoluteDenominator && quotient % 2n !== 0n");
            });
            emitter.emit_line(")) {");
            emitter.with_indent(|em| {
                em.emit_line("const negative = (numerator < 0n) !== (denominator < 0n);");
                em.emit_line("quotient += negative ? -1n : 1n;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return quotient;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_add(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return { tag: \"ok\", value: a + b };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_sub(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return { tag: \"ok\", value: a - b };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_mul(a, b, scaleFactor) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return { tag: \"ok\", value: __moth_number_round_half_even(a * b, scaleFactor) };",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_div(a, b, scaleFactor) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0n) {");
            emitter.with_indent(|em| em.emit_line(&divide_by_zero));
            emitter.emit_line("}");
            emitter.emit_line(
                "return { tag: \"ok\", value: __moth_number_round_half_even(a * scaleFactor, b) };",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_idiv(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0n) {");
            emitter.with_indent(|em| em.emit_line(&divide_by_zero));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: a / b };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_mod(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0n) {");
            emitter.with_indent(|em| em.emit_line(&divide_by_zero));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: a % b };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_pow(a, b, scaleFactor) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const exponent = BigInt(b);");
            emitter.emit_line("if (exponent < 0n) {");
            emitter.with_indent(|em| em.emit_line(&invalid_exponent));
            emitter.emit_line("}");
            emitter.emit_line("if (exponent === 0n) return { tag: \"ok\", value: scaleFactor };");
            emitter.emit_line("if (exponent === 1n) return { tag: \"ok\", value: a };");
            emitter.emit_line("// For exponent >=2, a +/-1 coefficient at positive scale rounds to zero.");
            emitter.emit_line(
                "if (scaleFactor > 1n && (a === 1n || a === -1n)) return { tag: \"ok\", value: 0n };",
            );
            emitter.emit_line("if (a === 0n) return { tag: \"ok\", value: 0n };");
            emitter.emit_line("if (a === scaleFactor) return { tag: \"ok\", value: scaleFactor };");
            emitter.emit_line(
                "if (a === -scaleFactor) return { tag: \"ok\", value: (exponent & 1n) === 0n ? scaleFactor : -scaleFactor };",
            );
            emitter.emit_line("const numerator = a ** exponent;");
            emitter.emit_line("const denominator = scaleFactor ** (exponent - 1n);");
            emitter.emit_line(
                "return { tag: \"ok\", value: __moth_number_round_half_even(numerator, denominator) };",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_number_neg(a) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return { tag: \"ok\", value: -a };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits one integer family shared by every domain with the same JS carrier.
    ///
    /// WHY: domain bounds are call arguments rather than duplicated helper bodies, so I8 through
    ///      U32 share Number arithmetic and I64/U64/profile-Int64 share exact BigInt arithmetic.
    fn emit_integer_helpers(&mut self, family: &str, uses_bigint: bool) {
        let zero = if uses_bigint { "0n" } else { "0" };
        let one = if uses_bigint { "1n" } else { "1" };
        let minus_one = if uses_bigint { "-1n" } else { "-1" };
        let check = format!("__moth_{family}_check");
        let ok = format!("__moth_{family}_ok");
        let overflow = self.error_result_call(BuiltinErrorCode::IntOverflow);
        let divide_by_zero = self.error_result_call(BuiltinErrorCode::DivideByZero);
        let invalid_exponent = self.error_result_call(BuiltinErrorCode::InvalidExponent);

        self.emit_line(&format!("function {ok}(value) {{"));
        self.with_indent(|emitter| {
            if uses_bigint {
                emitter.emit_line("return { tag: \"ok\", value }; ");
            } else {
                emitter
                    .emit_line("return { tag: \"ok\", value: Object.is(value, -0) ? 0 : value }; ");
            }
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line(&format!("function {check}(value, min, max) {{"));
        self.with_indent(|emitter| {
            if uses_bigint {
                emitter.emit_line("if (value < min || value > max) {");
            } else {
                emitter.emit_line("if (!Number.isInteger(value) || value < min || value > max) {");
            }
            emitter.with_indent(|em| em.emit_line(&overflow));
            emitter.emit_line("}");
            emitter.emit_line(&format!("return {ok}(value);"));
        });
        self.emit_line("}");
        self.emit_line("");

        for (operation, expression) in [("add", "a + b"), ("sub", "a - b"), ("mul", "a * b")] {
            self.emit_line(&format!(
                "function __moth_{family}_{operation}(a, b, min, max) {{"
            ));
            self.with_indent(|emitter| {
                emitter.emit_line(&format!("return {check}({expression}, min, max);"));
            });
            self.emit_line("}");
            self.emit_line("");
        }

        self.emit_line(&format!("function __moth_{family}_div(a, b, min, max) {{"));
        self.with_indent(|emitter| {
            emitter.emit_line(&format!("if (b === {zero}) {{"));
            emitter.with_indent(|em| em.emit_line(&divide_by_zero));
            emitter.emit_line("}");
            emitter.emit_line(&format!("if (a === min && b === {minus_one}) {{"));
            emitter.with_indent(|em| em.emit_line(&overflow));
            emitter.emit_line("}");
            if uses_bigint {
                emitter.emit_line(&format!("return {check}(a / b, min, max);"));
            } else {
                emitter.emit_line(&format!("return {check}(Math.trunc(a / b), min, max);"));
            }
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line(&format!("function __moth_{family}_mod(a, b, min, max) {{"));
        self.with_indent(|emitter| {
            emitter.emit_line(&format!("if (b === {zero}) {{"));
            emitter.with_indent(|em| em.emit_line(&divide_by_zero));
            emitter.emit_line("}");
            emitter.emit_line(&format!("if (b === {minus_one}) {{"));
            emitter.with_indent(|em| em.emit_line(&format!("return {ok}({zero});")));
            emitter.emit_line("}");
            emitter.emit_line(&format!("return {check}(a % b, min, max);"));
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line(&format!("function __moth_{family}_pow(a, b, min, max) {{"));
        self.with_indent(|emitter| {
            if uses_bigint {
                emitter.emit_line("if (b < 0n) {");
                emitter.with_indent(|em| em.emit_line(&invalid_exponent));
                emitter.emit_line("}");
                emitter.emit_line(&format!("if (b === {zero}) return {ok}({one});"));
                emitter.emit_line(&format!("if (a === {zero}) return {ok}({zero});"));
                emitter.emit_line(&format!("if (a === {one}) return {ok}({one});"));
                emitter.emit_line(&format!(
                    "if (a === {minus_one}) return {ok}((b & 1n) === 0n ? 1n : -1n);"
                ));
                emitter.emit_line(&format!("let result = {one};"));
                emitter.emit_line("let factor = a;");
                emitter.emit_line("let exponent = b;");
                emitter.emit_line("while (exponent > 0n) {");
                emitter.with_indent(|em| {
                    em.emit_line("if ((exponent & 1n) !== 0n) {");
                    em.with_indent(|inner| {
                        inner.emit_line("result *= factor;");
                        inner.emit_line("if (result < min || result > max) {");
                        inner.with_indent(|deep| deep.emit_line(&overflow));
                        inner.emit_line("}");
                    });
                    em.emit_line("}");
                    em.emit_line("exponent >>= 1n;");
                    em.emit_line("if (exponent > 0n) {");
                    em.with_indent(|inner| {
                        inner.emit_line("factor *= factor;");
                        inner.emit_line("if (factor < min || factor > max) {");
                        inner.with_indent(|deep| deep.emit_line(&overflow));
                        inner.emit_line("}");
                    });
                    em.emit_line("}");
                });
                emitter.emit_line("}");
                emitter.emit_line(&format!("return {ok}(result);"));
            } else {
                emitter.emit_line("if (!Number.isInteger(b) || b < 0) {");
                emitter.with_indent(|em| em.emit_line(&invalid_exponent));
                emitter.emit_line("}");
                emitter.emit_line(&format!("if (b === {zero}) return {ok}({one});"));
                emitter.emit_line(&format!("if (a === {zero}) return {ok}({zero});"));
                emitter.emit_line(&format!("if (a === {one}) return {ok}({one});"));
                emitter.emit_line(&format!(
                    "if (a === {minus_one}) return {ok}(b % 2 === 0 ? 1 : -1);"
                ));
                emitter.emit_line(&format!("let result = {one};"));
                emitter.emit_line("let factor = a;");
                emitter.emit_line("let exponent = b;");
                emitter.emit_line("while (exponent > 0) {");
                emitter.with_indent(|em| {
                    em.emit_line("if (exponent % 2 === 1) {");
                    em.with_indent(|inner| {
                        inner.emit_line("result *= factor;");
                        inner.emit_line(
                            "if (!Number.isInteger(result) || result < min || result > max) {",
                        );
                        inner.with_indent(|deep| deep.emit_line(&overflow));
                        inner.emit_line("}");
                    });
                    em.emit_line("}");
                    em.emit_line("exponent = Math.floor(exponent / 2);");
                    em.emit_line("if (exponent > 0) {");
                    em.with_indent(|inner| {
                        inner.emit_line("factor *= factor;");
                        inner.emit_line(
                            "if (!Number.isInteger(factor) || factor < min || factor > max) {",
                        );
                        inner.with_indent(|deep| deep.emit_line(&overflow));
                        inner.emit_line("}");
                    });
                    em.emit_line("}");
                });
                emitter.emit_line("}");
                emitter.emit_line(&format!("return {ok}(result);"));
            }
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line(&format!("function __moth_{family}_neg(a, min, max) {{"));
        self.with_indent(|emitter| {
            emitter.emit_line("if (a === min) {");
            emitter.with_indent(|em| em.emit_line(&overflow));
            emitter.emit_line("}");
            emitter.emit_line(&format!("return {check}(-a, min, max);"));
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits checked operations for one binary-float precision.
    fn emit_float_helpers(&mut self, family: &str, precision: BinaryFloatPrecision) {
        let round = match precision {
            BinaryFloatPrecision::Binary32 => Some("Math.fround"),
            BinaryFloatPrecision::Binary64 => None,
            BinaryFloatPrecision::Binary16 => {
                unreachable!("binary16 is promoted to binary32 before arithmetic")
            }
        };
        let non_finite = self.error_result_call(BuiltinErrorCode::FloatNonFinite);
        let divide_by_zero = self.error_result_call(BuiltinErrorCode::DivideByZero);

        for (operation, expression, checks_zero) in [
            ("add", "a + b", false),
            ("sub", "a - b", false),
            ("mul", "a * b", false),
            ("div", "a / b", true),
            ("mod", "a % b", true),
            ("pow", "__moth_float_power(a, b)", false),
        ] {
            self.emit_line(&format!("function __moth_{family}_{operation}(a, b) {{"));
            self.with_indent(|emitter| {
                if checks_zero {
                    emitter.emit_line("if (b === 0) {");
                    emitter.with_indent(|em| em.emit_line(&divide_by_zero));
                    emitter.emit_line("}");
                }
                if let Some(round) = round {
                    emitter.emit_line(&format!("const result = {round}({expression});"));
                } else {
                    emitter.emit_line(&format!("const result = {expression};"));
                }
                emitter.emit_line("if (!Number.isFinite(result)) {");
                emitter.with_indent(|em| em.emit_line(&non_finite));
                emitter.emit_line("}");
                emitter.emit_line("return { tag: \"ok\", value: result };");
            });
            self.emit_line("}");
            self.emit_line("");
        }

        self.emit_line(&format!("function __moth_{family}_neg(a) {{"));
        self.with_indent(|emitter| {
            if let Some(round) = round {
                emitter.emit_line(&format!("const result = {round}(-a);"));
            } else {
                emitter.emit_line("const result = -a;");
            }
            emitter.emit_line("if (!Number.isFinite(result)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: result };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn emit_float_validate_helper(&mut self, precision: BinaryFloatPrecision) {
        let non_finite = self.error_result_call(BuiltinErrorCode::FloatBoundaryNonFinite);
        self.emit_line("function __moth_float_validate(value) {");
        self.with_indent(|emitter| {
            if precision == BinaryFloatPrecision::Binary32 {
                emitter.emit_line("value = Math.fround(value);");
            }
            emitter.emit_line("if (!Number.isFinite(value)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits one precision-parameterised shortest formatter for casts and templates.
    ///
    /// WHAT: binary16/binary32 search shortest round-tripping decimals with exact-distance
    ///       ties-to-even; binary64 uses the engine's shortest binary64 decimal. All precisions
    ///       share Moth's exponent thresholds, sign handling and result carrier.
    /// WHY: fixed floats and profile Float must format according to their semantic precision, not
    ///      merely the wider Number carrier that stores them.
    fn emit_format_float_helper(&mut self, usage: NumericRuntimeHelperUsage) {
        let error_code = BuiltinErrorCode::FloatFormatInvariant.as_u32().to_string();
        self.emit_line("function __moth_format_float(value, precision, targetName) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!Number.isFinite(value)) {");
            emitter.with_indent(|em| {
                em.emit_line("const message = targetName + \" -> String formatting failed: Float value is not finite\";");
                em.emit_line(&format!("return __moth_error_result(message, {error_code});"));
            });
            emitter.emit_line("}");
            // Both zero signs normalise before the sign bit is reflected in formatted text.
            emitter.emit_line("if (value === 0) return { tag: \"ok\", value: \"0\" };");
            emitter.emit_line("const magnitude = Math.abs(value);");
            emitter.emit_line("let text;");

            let mut first_precision = true;
            for (precision, used) in [
                (BinaryFloatPrecision::Binary16, usage.format_binary16),
                (BinaryFloatPrecision::Binary32, usage.format_binary32),
                (BinaryFloatPrecision::Binary64, usage.format_binary64),
            ] {
                if !used {
                    continue;
                }
                let bits = binary_float_precision_bits(precision);
                let prefix = if first_precision { "if" } else { "else if" };
                match precision {
                    BinaryFloatPrecision::Binary16 | BinaryFloatPrecision::Binary32 => {
                        emitter.emit_line(&format!(
                            "{prefix} (precision === {bits}) text = __moth_binary_float_shortest(magnitude, {bits});"
                        ));
                    }
                    BinaryFloatPrecision::Binary64 => {
                        emitter.emit_line(&format!(
                            "{prefix} (precision === {bits}) text = magnitude.toExponential();"
                        ));
                    }
                }
                first_precision = false;
            }
            emitter.emit_line("else throw new Error(\"JS formatter received an unselected binary precision\");");
            emitter.emit_line("const separator = text.indexOf(\"e\");");
            emitter.emit_line("const mantissa = separator < 0 ? text : text.slice(0, separator);");
            emitter.emit_line("const exponent = separator < 0 ? 0 : Number(text.slice(separator + 1));");
            emitter.emit_line("const negative = value < 0;");
            emitter.emit_line("const point = mantissa.indexOf(\".\");");
            emitter.emit_line("const fractionalDigits = point < 0 ? 0 : mantissa.length - point - 1;");
            emitter.emit_line("let digits = mantissa.replace(\".\", \"\");");
            emitter.emit_line("let decimalPosition = digits.length + exponent - fractionalDigits;");
            emitter.emit_line("while (digits.length > 1 && digits.endsWith(\"0\")) digits = digits.slice(0, -1);");
            emitter.emit_line("let rendered;");
            emitter.emit_line("if (magnitude >= 1e21 || magnitude < 1e-6) {");
            emitter.with_indent(|em| {
                em.emit_line("const scientificExponent = decimalPosition - 1;");
                em.emit_line("const coefficient = digits.length === 1 ? digits : digits[0] + \".\" + digits.slice(1);");
                em.emit_line("const sign = scientificExponent >= 0 ? \"+\" : \"\";");
                em.emit_line("rendered = coefficient + \"e\" + sign + scientificExponent;");
            });
            emitter.emit_line("} else if (decimalPosition <= 0) {");
            emitter.with_indent(|em| {
                em.emit_line("rendered = \"0.\" + \"0\".repeat(-decimalPosition) + digits;");
            });
            emitter.emit_line("} else if (decimalPosition >= digits.length) {");
            emitter.with_indent(|em| {
                em.emit_line("rendered = digits + \"0\".repeat(decimalPosition - digits.length);");
            });
            emitter.emit_line("} else {");
            emitter.with_indent(|em| {
                em.emit_line("rendered = digits.slice(0, decimalPosition) + \".\" + digits.slice(decimalPosition);");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (negative) rendered = \"-\" + rendered;");
            emitter.emit_line("return { tag: \"ok\", value: rendered };");
        });
        self.emit_line("}");
        self.emit_line("");

        if usage.format_binary16 || usage.format_binary32 {
            self.emit_binary_float_shortest_helper();
        }
    }

    /// Emits exact decimal-candidate comparisons shared by the binary16 and binary32 formatters.
    fn emit_binary_float_shortest_helper(&mut self) {
        self.emit_line("const __moth_binary_float_view = new DataView(new ArrayBuffer(4));");
        self.emit_line("");
        self.emit_line("function __moth_binary_float_shortest(value, precision) {");
        self.with_indent(|emitter| {
            emitter.emit_line("__moth_binary_float_view.setFloat32(0, value, false);");
            emitter.emit_line("const bits = __moth_binary_float_view.getUint32(0, false);");
            emitter.emit_line("const exponentBits = (bits >>> 23) & 0xff;");
            emitter.emit_line("const fraction = bits & 0x7fffff;");
            emitter.emit_line("const binarySignificand = BigInt(exponentBits === 0 ? fraction : (0x800000 | fraction));");
            emitter.emit_line("const binaryExponent = exponentBits === 0 ? -149 : exponentBits - 150;");
            emitter.emit_line("let exactNumerator = binarySignificand;");
            emitter.emit_line("let exactDenominator = 1n;");
            emitter.emit_line("if (binaryExponent >= 0) {");
            emitter.with_indent(|em| em.emit_line("exactNumerator <<= BigInt(binaryExponent);"));
            emitter.emit_line("} else {");
            emitter.with_indent(|em| em.emit_line("exactDenominator <<= BigInt(-binaryExponent);"));
            emitter.emit_line("}");
            emitter.emit_line("const maximumDigits = precision === 16 ? 5 : 9;");
            emitter.emit_line("for (let digits = 1; digits <= maximumDigits; digits++) {");
            emitter.with_indent(|em| {
                em.emit_line("const nearest = value.toExponential(digits - 1);");
                em.emit_line("const [mantissa, exponentText] = nearest.split(\"e\");");
                em.emit_line("const significand = Number(mantissa.replace(\".\", \"\"));");
                em.emit_line("const exponent = Number(exponentText) - (digits - 1);");
                em.emit_line("let best = null;");
                em.emit_line("let bestDistance = 0n;");
                em.emit_line("let bestDenominator = 1n;");
                em.emit_line("let bestSignificand = 0n;");
                em.emit_line("for (const candidate of [significand, significand + 1, significand - 1]) {");
                em.with_indent(|inner| {
                    inner.emit_line("const candidateSignificand = BigInt(candidate);");
                    inner.emit_line("let candidateDigits = String(candidate);");
                    inner.emit_line("let candidateExponent = exponent;");
                    inner.emit_line("while (candidateDigits.endsWith(\"0\")) {");
                    inner.with_indent(|deep| {
                        deep.emit_line("candidateDigits = candidateDigits.slice(0, -1);");
                        deep.emit_line("candidateExponent++;");
                    });
                    inner.emit_line("}");
                    inner.emit_line("const text = candidateDigits + \"e\" + candidateExponent;");
                    inner.emit_line("const rounded = precision === 16 ? Math.f16round(Number(text)) : Math.fround(Number(text));");
                    inner.emit_line("if (rounded !== value) continue;");
                    inner.emit_line("let candidateNumerator = BigInt(candidateDigits);");
                    inner.emit_line("let candidateDenominator = 1n;");
                    inner.emit_line("if (candidateExponent >= 0) {");
                    inner.with_indent(|deep| {
                        deep.emit_line("candidateNumerator *= 10n ** BigInt(candidateExponent);");
                    });
                    inner.emit_line("} else {");
                    inner.with_indent(|deep| {
                        deep.emit_line("candidateDenominator = 10n ** BigInt(-candidateExponent);");
                    });
                    inner.emit_line("}");
                    inner.emit_line("const difference = candidateNumerator * exactDenominator - exactNumerator * candidateDenominator;");
                    inner.emit_line("const distance = difference < 0n ? -difference : difference;");
                    inner.emit_line("let choose = best === null;");
                    inner.emit_line("if (!choose) {");
                    inner.with_indent(|deep| {
                        deep.emit_line("const candidateDistance = distance * bestDenominator;");
                        deep.emit_line("const currentDistance = bestDistance * candidateDenominator;");
                        deep.emit_line("choose = candidateDistance < currentDistance || (");
                        deep.with_indent(|deeper| {
                            deeper.emit_line("candidateDistance === currentDistance &&");
                            deeper.emit_line("candidateSignificand % 2n === 0n &&");
                            deeper.emit_line("bestSignificand % 2n !== 0n");
                        });
                        deep.emit_line(");");
                    });
                    inner.emit_line("}");
                    inner.emit_line("if (choose) {");
                    inner.with_indent(|deep| {
                        deep.emit_line("best = text;");
                        deep.emit_line("bestDistance = distance;");
                        deep.emit_line("bestDenominator = candidateDenominator;");
                        deep.emit_line("bestSignificand = candidateSignificand;");
                    });
                    inner.emit_line("}");
                });
                em.emit_line("}");
                em.emit_line("if (best !== null) return best;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return value.toExponential();");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    fn error_result_call(&self, code: BuiltinErrorCode) -> String {
        let message = code.default_message();
        format!(
            "return __moth_error_result(\"{message}\", {});",
            code.as_u32()
        )
    }
}
