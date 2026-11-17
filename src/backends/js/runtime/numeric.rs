//! Numeric helpers for the JavaScript runtime.
//!
//! WHAT: emits carrier-parameterised bounded integer and binary-float operation families, exact
//!       scale-generic Number coefficient operations, exact-precision binary-float boundary
//!       validation, canonical binary-float-to-String formatting for HTML-JS, fast scalar
//!       arithmetic with fatal traps, and the total or trapping conversions a fast compound
//!       write-back performs.
//! WHY: HIR numeric statements already own operator domains and failure modes; this runtime only
//!      enforces each selected domain's exact semantics at its operation boundary.
//!
//! Every checked helper returns `{ tag, value }`. Trap-mode lowering extracts the success value or
//! throws, while builtin `Error!` lowering keeps the carrier for normal HIR recovery. Fast
//! arithmetic never builds a carrier: total operations return the width-normalised scalar, and
//! trapping helpers throw the canonical fatal message directly. The fast write-back helpers
//! follow the same rule, so a fast trap can never be consumed as a Moth `Error`.

use std::collections::HashSet;

use super::NumericRuntimeHelperUsage;
use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::{
    binary_float_precision_bits, checked_float_helper_family, fast_float_helper_family,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

/// How one generated numeric helper reports a failure.
///
/// WHAT: checked helpers return the recoverable `{ tag, value }` carrier, while fast helpers
///       throw the canonical fatal cause message directly.
/// WHY: both modes must round, trap zero divisors and reject non-finite results identically, so
///      one generator owns the operation bodies and only the failure delivery differs. Reusing
///      the checked carrier for fast arithmetic would make a fast trap catchable as a Moth Error.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NumericHelperDelivery {
    CheckedCarrier,
    FastFatal,
}

impl<'hir> JsEmitter<'hir> {
    /// Emits only numeric helper families used by reachable HIR and cast statements.
    ///
    /// `emitted` carries the helper names the cast group already emitted, so a fast write-back
    /// that shares the exact BigInt rounding helper never duplicates it in the prelude.
    pub(crate) fn emit_runtime_numeric_helpers(
        &mut self,
        usage: NumericRuntimeHelperUsage,
        emitted: &mut HashSet<&'static str>,
    ) {
        if usage.numeric_trap {
            self.emit_numeric_trap_helper();
        }
        if usage.uses_float_power() {
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

        if usage.binary16_ops {
            self.emit_float_helpers(
                checked_float_helper_family(BinaryFloatPrecision::Binary16),
                BinaryFloatPrecision::Binary16,
                NumericHelperDelivery::CheckedCarrier,
                usage.binary16_power,
            );
        }

        if usage.binary32_ops {
            self.emit_float_helpers(
                checked_float_helper_family(BinaryFloatPrecision::Binary32),
                BinaryFloatPrecision::Binary32,
                NumericHelperDelivery::CheckedCarrier,
                usage.binary32_power,
            );
        }

        if usage.binary64_ops {
            self.emit_float_helpers(
                checked_float_helper_family(BinaryFloatPrecision::Binary64),
                BinaryFloatPrecision::Binary64,
                NumericHelperDelivery::CheckedCarrier,
                usage.binary64_power,
            );
        }

        // Fast scalar arithmetic owns its own helpers: fixed integers wrap or trap by the result
        // domain's width, while binary floats keep per-operation rounding and throw on the same
        // causes the checked helpers report. Each helper is demanded by the operation that calls
        // it, so a wrapping add carries no unused trapping helper.
        if usage.fast_number_integer_wrap {
            self.emit_fast_number_integer_wrap_helper();
        }

        if usage.fast_number_integer_division {
            self.emit_fast_number_integer_division_helper();
        }

        if usage.fast_number_integer_remainder {
            self.emit_fast_number_integer_remainder_helper();
        }

        if usage.fast_number_integer_power {
            self.emit_fast_number_integer_power_helper();
        }

        if usage.fast_big_integer_division {
            self.emit_fast_big_integer_division_helper();
        }

        if usage.fast_big_integer_remainder {
            self.emit_fast_big_integer_remainder_helper();
        }

        if usage.fast_big_integer_power {
            self.emit_fast_big_integer_power_helper();
        }

        if usage.fast_binary16_ops {
            self.emit_float_helpers(
                fast_float_helper_family(BinaryFloatPrecision::Binary16),
                BinaryFloatPrecision::Binary16,
                NumericHelperDelivery::FastFatal,
                usage.fast_binary16_power,
            );
        }

        if usage.fast_binary32_ops {
            self.emit_float_helpers(
                fast_float_helper_family(BinaryFloatPrecision::Binary32),
                BinaryFloatPrecision::Binary32,
                NumericHelperDelivery::FastFatal,
                usage.fast_binary32_power,
            );
        }

        if usage.fast_binary64_ops {
            self.emit_float_helpers(
                fast_float_helper_family(BinaryFloatPrecision::Binary64),
                BinaryFloatPrecision::Binary64,
                NumericHelperDelivery::FastFatal,
                usage.fast_binary64_power,
            );
        }

        if usage.uses_float_formatter() {
            self.emit_format_float_helper(usage);
        }

        if usage.validate_float {
            self.emit_float_validate_helper();
        }

        // Fast compound write-backs own their runtime helpers. They are demanded only by a
        // reachable fast conversion, so ordinary modules emit no new code.
        if usage.fast_bigint_to_float {
            self.emit_bigint_to_binary_float_helper(emitted);
        }
        if usage.fast_integer_wrap {
            self.emit_fast_integer_wrap_helper();
        }
        if usage.fast_float_to_integer {
            self.emit_fast_float_to_integer_helper();
        }
        if usage.fast_float_conversion {
            self.emit_fast_float_conversion_helper();
        }
    }

    /// Emits the exact low-bits/signedness normalisation for a fast integer write-back.
    ///
    /// WHAT: reduces the source integer to the destination width with two's-complement BigInt
    ///       arithmetic, then reinterprets the result in the destination signedness.
    /// WHY: JavaScript Number bitwise operators are 32-bit only, and the source or destination can
    ///      be 64-bit. BigInt arithmetic is exact for every fixed width, and the Number carrier
    ///      converts the bounded result back without loss.
    fn emit_fast_integer_wrap_helper(&mut self) {
        self.emit_line("function __moth_fast_wrap_integer(value, bits, signed) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const modulus = 1n << BigInt(bits);");
            emitter.emit_line(
                "const normalized = (typeof value === \"bigint\" ? value : BigInt(value)) % modulus;",
            );
            emitter.emit_line("const low = normalized < 0n ? normalized + modulus : normalized;");
            emitter.emit_line("if (!signed) return low;");
            emitter.emit_line("const signBit = modulus >> 1n;");
            emitter.emit_line("return low >= signBit ? low - modulus : low;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the truncating, range-checked float-to-integer conversion for a fast write-back.
    ///
    /// WHAT: rejects non-finite input, truncates toward zero, normalises `-0`, checks the exact
    ///       target bounds, then returns the target carrier.
    /// WHY: the fast write-back never saturates and never produces a recoverable `Error`; a value
    ///      the target cannot hold is a fatal trap, and the store must not observe the value.
    fn emit_fast_float_to_integer_helper(&mut self) {
        self.emit_line("function __moth_fast_float_to_integer(value, min, max) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value !== \"number\" || !Number.isFinite(value)) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(
                    BuiltinErrorCode::FloatCastToIntInvalidValue,
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const truncatedValue = Math.trunc(value);");
            emitter
                .emit_line("const truncated = Object.is(truncatedValue, -0) ? 0 : truncatedValue;");
            emitter.emit_line("const minimum = typeof min === \"bigint\" ? min : BigInt(min);");
            emitter.emit_line("const maximum = typeof max === \"bigint\" ? max : BigInt(max);");
            emitter.emit_line("const integer = BigInt(truncated);");
            emitter.emit_line("if (integer < minimum || integer > maximum) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(
                    BuiltinErrorCode::FloatCastToIntOutOfRange,
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("return typeof min === \"bigint\" ? integer : truncated;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the rounding, finite-checked binary-float conversion for a fast write-back.
    ///
    /// WHAT: rounds at the destination precision and throws when the rounded value is not finite.
    /// WHY: a fast integer-to-float or float-to-float write-back must use destination-precision
    ///      rounding and never store a non-finite result. `F16` and `F32` round through the engine
    ///      helpers; `F64` input is already at its destination precision.
    fn emit_fast_float_conversion_helper(&mut self) {
        self.emit_line("function __moth_fast_float_conversion(value, precision) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "const rounded = precision === 16 ? Math.f16round(value) : precision === 32 ? Math.fround(value) : value;",
            );
            emitter.emit_line("if (!Number.isFinite(rounded)) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::FloatCastNonFinite));
            });
            emitter.emit_line("}");
            emitter.emit_line("return rounded;");
        });
        self.emit_line("}");
        self.emit_line("");
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
            emitter.emit_line("// Integral bases divide out the scale factor: (a / F) ** e * F is exact.");
            emitter.emit_line("if (scaleFactor > 1n && a % scaleFactor === 0n) {");
            emitter.with_indent(|em| {
                em.emit_line(
                    "return { tag: \"ok\", value: (a / scaleFactor) ** exponent * scaleFactor };",
                );
            });
            emitter.emit_line("}");
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
    ///      U32 share Number arithmetic and I64/U64 share exact BigInt arithmetic.
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

    /// Emits the exact low-bits normalisation for one fast Number-carried integer result.
    ///
    /// WHAT: reduces a value to the semantic result width with 32-bit bitwise arithmetic.
    /// WHY: a Number multiply loses product low bits above 2^53, so the multiply helper must
    ///      narrow an exact 32-bit product. Bitwise narrowing is exact because every
    ///      Number-carried domain is at most 32 bits wide.
    fn emit_fast_number_integer_wrap_helper(&mut self) {
        self.emit_line("function __moth_fast_int_wrap(value, bits, signed) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (bits === 8) return signed ? (value << 24) >> 24 : value & 255;");
            emitter
                .emit_line("if (bits === 16) return signed ? (value << 16) >> 16 : value & 65535;");
            emitter.emit_line("return signed ? value | 0 : value >>> 0;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits fast truncating division for the Number-carried integer widths.
    ///
    /// WHY: `MIN // -1` is the only quotient that must trap beyond a zero divisor, so the
    ///      caller supplies the result domain's minimum instead of duplicating width knowledge.
    fn emit_fast_number_integer_division_helper(&mut self) {
        self.emit_line("function __moth_fast_int_div(a, b, bits, signed, min) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::DivideByZero));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (signed && a === min && b === -1) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::IntOverflow));
            });
            emitter.emit_line("}");
            emitter.emit_line("return __moth_fast_int_wrap(Math.trunc(a / b), bits, signed);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits fast dividend-sign remainder for the Number-carried integer widths.
    fn emit_fast_number_integer_remainder_helper(&mut self) {
        self.emit_line("function __moth_fast_int_mod(a, b, bits, signed) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::DivideByZero));
            });
            emitter.emit_line("}");
            // The signed minimum divided by -1 has exact dividend-sign remainder zero; returning
            // it before `%` keeps the result canonical and independent of the host operator.
            emitter.emit_line("if (b === -1) return 0;");
            emitter.emit_line("return __moth_fast_int_wrap(a % b, bits, signed);");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits fast square-and-multiply power for the Number-carried integer widths.
    ///
    /// WHY: `Math.imul` keeps the exact low product bits at every accumulator step, so the final
    ///      result is the language's modular power rather than a rounded Number power.
    fn emit_fast_number_integer_power_helper(&mut self) {
        self.emit_line("function __moth_fast_int_pow(a, b, bits, signed) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!Number.isInteger(b) || (signed && b < 0)) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::InvalidExponent));
            });
            emitter.emit_line("}");
            emitter.emit_line("let result = 1;");
            emitter.emit_line("let factor = __moth_fast_int_wrap(a, bits, signed);");
            emitter.emit_line("let exponent = b;");
            emitter.emit_line("while (exponent > 0) {");
            emitter.with_indent(|em| {
                em.emit_line("if (exponent % 2 === 1) {");
                em.with_indent(|inner| {
                    inner.emit_line(
                        "result = __moth_fast_int_wrap(Math.imul(result, factor), bits, signed);",
                    );
                });
                em.emit_line("}");
                em.emit_line("exponent = Math.floor(exponent / 2);");
                em.emit_line("if (exponent > 0) {");
                em.with_indent(|inner| {
                    inner.emit_line(
                        "factor = __moth_fast_int_wrap(Math.imul(factor, factor), bits, signed);",
                    );
                });
                em.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("return result;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the fast BigInt truncating quotient.
    ///
    /// WHY: 64-bit add/subtract/multiply/negate need only the inline `BigInt.asIntN`/`asUintN`
    ///      normalisation, so only the trapping quotient represents this family at runtime.
    fn emit_fast_big_integer_division_helper(&mut self) {
        self.emit_line("function __moth_fast_bigint_div(a, b, min) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0n) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::DivideByZero));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (a === min && b === -1n) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::IntOverflow));
            });
            emitter.emit_line("}");
            emitter.emit_line("return a / b;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the fast BigInt dividend-sign remainder.
    fn emit_fast_big_integer_remainder_helper(&mut self) {
        self.emit_line("function __moth_fast_bigint_mod(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (b === 0n) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::DivideByZero));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (b === -1n) return 0n;");
            emitter.emit_line("return a % b;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits the fast BigInt square-and-multiply power.
    ///
    /// WHY: raising the value before normalisation would build a huge host power, so each
    ///      accumulator step wraps to the 64-bit signed or unsigned width first.
    fn emit_fast_big_integer_power_helper(&mut self) {
        self.emit_line("function __moth_fast_bigint_pow(a, b, signed) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (signed && b < 0n) {");
            emitter.with_indent(|em| {
                em.emit_line(&fatal_numeric_trap_throw(BuiltinErrorCode::InvalidExponent));
            });
            emitter.emit_line("}");
            emitter.emit_line("let result = 1n;");
            emitter.emit_line("let factor = a;");
            emitter.emit_line("let exponent = b;");
            emitter.emit_line("while (exponent > 0n) {");
            emitter.with_indent(|em| {
                em.emit_line("if ((exponent & 1n) !== 0n) {");
                em.with_indent(|inner| {
                    inner.emit_line(
                        "result = signed ? BigInt.asIntN(64, result * factor) : BigInt.asUintN(64, result * factor);",
                    );
                });
                em.emit_line("}");
                em.emit_line("exponent >>= 1n;");
                em.emit_line("if (exponent > 0n) {");
                em.with_indent(|inner| {
                    inner.emit_line(
                        "factor = signed ? BigInt.asIntN(64, factor * factor) : BigInt.asUintN(64, factor * factor);",
                    );
                });
                em.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("return result;");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits checked operations for one binary-float precision.
    ///
    /// WHY: every binary-float operation completes at its selected precision, so each family
    ///      rounds once after the operation (binary16/binary32) and then checks finiteness on the
    ///      rounded result; binary64 needs no rounding step. The `pow` member is emitted only
    ///      when `emit_power` is set, because it is the only member that calls the shared power
    ///      helper.
    fn emit_float_helpers(
        &mut self,
        family: &str,
        precision: BinaryFloatPrecision,
        delivery: NumericHelperDelivery,
        emit_power: bool,
    ) {
        let round = match precision {
            BinaryFloatPrecision::Binary16 => Some("Math.f16round"),
            BinaryFloatPrecision::Binary32 => Some("Math.fround"),
            BinaryFloatPrecision::Binary64 => None,
        };
        let emit_failure = |emitter: &mut Self, code: BuiltinErrorCode| match delivery {
            NumericHelperDelivery::CheckedCarrier => {
                let line = emitter.error_result_call(code);
                emitter.emit_line(&line);
            }
            NumericHelperDelivery::FastFatal => {
                emitter.emit_line(&fatal_numeric_trap_throw(code));
            }
        };
        let success_line = match delivery {
            NumericHelperDelivery::CheckedCarrier => "return { tag: \"ok\", value: result };",
            NumericHelperDelivery::FastFatal => "return result;",
        };

        for (operation, expression, checks_zero) in [
            ("add", "a + b", false),
            ("sub", "a - b", false),
            ("mul", "a * b", false),
            ("div", "a / b", true),
            ("mod", "a % b", true),
            ("pow", "__moth_float_power(a, b)", false),
        ] {
            // The `pow` member and the shared helper are demanded together, so a family without
            // a selected Power op emits neither and references no undefined function.
            if operation == "pow" && !emit_power {
                continue;
            }
            self.emit_line(&format!("function __moth_{family}_{operation}(a, b) {{"));
            self.with_indent(|emitter| {
                if checks_zero {
                    emitter.emit_line("if (b === 0) {");
                    emitter.with_indent(|em| emit_failure(em, BuiltinErrorCode::DivideByZero));
                    emitter.emit_line("}");
                }
                if let Some(round) = round {
                    emitter.emit_line(&format!("const result = {round}({expression});"));
                } else {
                    emitter.emit_line(&format!("const result = {expression};"));
                }
                emitter.emit_line("if (!Number.isFinite(result)) {");
                emitter.with_indent(|em| emit_failure(em, BuiltinErrorCode::FloatNonFinite));
                emitter.emit_line("}");
                emitter.emit_line(success_line);
            });
            self.emit_line("}");
            self.emit_line("");
        }

        // Fast binary-float negation is total and lowers inline, so only the checked family
        // owns a negation helper that reports a non-finite carrier.
        if delivery == NumericHelperDelivery::CheckedCarrier {
            self.emit_line(&format!("function __moth_{family}_neg(a) {{"));
            self.with_indent(|emitter| {
                if let Some(round) = round {
                    emitter.emit_line(&format!("const result = {round}(-a);"));
                } else {
                    emitter.emit_line("const result = -a;");
                }
                emitter.emit_line("if (!Number.isFinite(result)) {");
                emitter.with_indent(|em| emit_failure(em, BuiltinErrorCode::FloatNonFinite));
                emitter.emit_line("}");
                emitter.emit_line(success_line);
            });
            self.emit_line("}");
            self.emit_line("");
        }
    }

    /// Emits the exact-precision finite-boundary validator for raw external float carriers.
    ///
    /// WHAT: rejects non-Number carriers, rounds at the caller-supplied binary precision, and
    ///       reports the shared binary-float boundary failure (code 304) for non-finite results.
    /// WHY: the caller owns the source's exact precision, so one helper serves every validated
    ///      boundary. The `typeof` gate must precede rounding because JS rounding coerces
    ///      numeric strings, booleans and objects into values that were never Moth binary floats.
    fn emit_float_validate_helper(&mut self) {
        let non_finite = self.error_result_call(BuiltinErrorCode::FloatBoundaryNonFinite);
        self.emit_line("function __moth_float_validate(value, precision) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof value !== \"number\") {");
            emitter.with_indent(|em| em.emit_line(&non_finite));
            emitter.emit_line("}");
            emitter.emit_line(
                "const rounded = precision === 16 ? Math.f16round(value) : precision === 32 ? Math.fround(value) : value;",
            );
            emitter.emit_line("if (!Number.isFinite(rounded)) {");
            emitter.with_indent(|em| em.emit_line(&non_finite));
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: rounded };");
        });
        self.emit_line("}");
        self.emit_line("");
    }

    /// Emits one precision-parameterised shortest formatter for casts and templates.
    ///
    /// WHAT: binary16/binary32 search shortest round-tripping decimals with exact-distance
    ///       ties-to-even; binary64 uses the engine's shortest binary64 decimal. All precisions
    ///       share Moth's exponent thresholds, sign handling and result carrier.
    /// WHY: every binary float must format according to its exact semantic precision, not
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

/// Renders one fatal numeric trap as a plain JavaScript `Error`.
///
/// WHY: fast arithmetic and fast range guards share the fatal trap transport. A fast trap is not
///      a recoverable Moth `Error`, so it throws the same canonical cause message the ordinary
///      trap path reports instead of constructing and re-throwing a `{ tag, value }` carrier.
pub(crate) fn fatal_numeric_trap_throw(code: BuiltinErrorCode) -> String {
    format!("throw new Error({:?});", code.default_message())
}
