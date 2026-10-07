//! JavaScript helpers for `@core/text`.
//!
//! WHAT: implements the Core Text surface over the canonical JS String runtime boundary.
//! WHY: the typed external-call boundary passes concrete JavaScript strings, so package helpers
//!      can apply their operations directly while preserving Moth's scalar-counting contract.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;
use crate::backends::js::numeric_carrier::JsNumericCarrier;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

macro_rules! core_text_length_helper_source {
    ($return_expression:literal) => {
        concat!(
            r#"function __moth_text_length(text) {
    let count = 0;
    let index = 0;
    while (index < text.length) {
        const codeUnit = text.charCodeAt(index);
        const isSurrogatePair =
            codeUnit >= 0xd800 &&
            codeUnit <= 0xdbff &&
            index + 1 < text.length &&
            text.charCodeAt(index + 1) >= 0xdc00 &&
            text.charCodeAt(index + 1) <= 0xdfff;
        index += isSurrogatePair ? 2 : 1;
        count += 1;
    }"#,
            "\n    return ",
            $return_expression,
            ";\n}"
        )
    };
}

pub(crate) const CORE_TEXT_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: "__moth_text_length",
        // A valid UTF-16 surrogate pair is one scalar, so it advances the index twice and the
        // count once. Every other code unit, including a lone surrogate, counts on its own.
        source: core_text_length_helper_source!("count"),
    },
    CoreJsHelper {
        name: "__moth_text_is_empty",
        source: "function __moth_text_is_empty(text) { return text.length === 0; }",
    },
    CoreJsHelper {
        name: "__moth_text_contains",
        source: "function __moth_text_contains(text, pattern) { return text.includes(pattern); }",
    },
    CoreJsHelper {
        name: "__moth_text_starts_with",
        source: "function __moth_text_starts_with(text, prefix) { return text.startsWith(prefix); }",
    },
    CoreJsHelper {
        name: "__moth_text_ends_with",
        source: "function __moth_text_ends_with(text, suffix) { return text.endsWith(suffix); }",
    },
];

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_text_helpers(&mut self) {
        let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, self.config.numeric_profile)
            .expect("Int always has a JavaScript numeric carrier");

        for helper in CORE_TEXT_JS_HELPERS {
            if !self.referenced_external_runtime_function(helper.name) {
                continue;
            }

            if helper.name == "__moth_text_length" {
                match carrier {
                    JsNumericCarrier::ExactInteger { .. } => {
                        self.emit_javascript_source(helper.source);
                    }
                    JsNumericCarrier::BigInteger { .. } => {
                        self.emit_javascript_source(core_text_length_helper_source!(
                            "BigInt(count)"
                        ));
                    }
                    JsNumericCarrier::BinaryFloat { .. } => {
                        unreachable!("Int carrier cannot be a binary float")
                    }
                    JsNumericCarrier::ScaledInteger { .. } => {
                        unreachable!("Int carrier cannot be a Number")
                    }
                }
            } else {
                self.emit_javascript_source(helper.source);
            }
        }
    }
}
