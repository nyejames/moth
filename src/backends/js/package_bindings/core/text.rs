//! JavaScript helpers for `@core/text`.
//!
//! WHAT: implements the Core Text surface over the canonical JS String runtime boundary.
//! WHY: the typed external-call boundary passes concrete JavaScript strings, so package helpers
//!      can apply their operations directly while preserving Moth's scalar-counting contract.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;

pub(crate) const CORE_TEXT_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: "__moth_text_length",
        // `length` returns exactly I32, whose JS carrier is the exact Number carrier, so the
        // helper returns the scalar count directly.
        // A valid UTF-16 surrogate pair is one scalar, so it advances the index twice and the
        // count once. Every other code unit, including a lone surrogate, counts on its own.
        source: r#"function __moth_text_length(text) {
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
    }
    if (count > 2147483647) throw new RangeError("Text length exceeds the I32 length carrier");
    return count;
}"#,
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
        for helper in CORE_TEXT_JS_HELPERS {
            if self.referenced_external_runtime_function(helper.name) {
                self.emit_javascript_source(helper.source);
            }
        }
    }
}
