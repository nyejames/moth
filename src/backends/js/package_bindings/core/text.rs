//! JavaScript helpers for `@core/text`.
//!
//! WHAT: implements the Core Text surface over the canonical JS String runtime boundary.
//! WHY: Moth `String` values may carry reactive-template metadata, so package helpers must
//!      consume content through the shared runtime helper before applying host operations.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;

pub(crate) const CORE_TEXT_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: "__moth_text_length",
        source: "function __moth_text_length(text) { const value = __moth_string_value(text); let count = 0; let index = 0; while (index < value.length) { const codeUnit = value.charCodeAt(index); const isSurrogatePair = codeUnit >= 0xd800 && codeUnit <= 0xdbff && index + 1 < value.length && value.charCodeAt(index + 1) >= 0xdc00 && value.charCodeAt(index + 1) <= 0xdfff; index += isSurrogatePair ? 2 : 1; count += 1; } return count; }",
    },
    CoreJsHelper {
        name: "__moth_text_is_empty",
        source: "function __moth_text_is_empty(text) { return __moth_string_value(text).length === 0; }",
    },
    CoreJsHelper {
        name: "__moth_text_contains",
        source: "function __moth_text_contains(text, pattern) { return __moth_string_value(text).includes(__moth_string_value(pattern)); }",
    },
    CoreJsHelper {
        name: "__moth_text_starts_with",
        source: "function __moth_text_starts_with(text, prefix) { return __moth_string_value(text).startsWith(__moth_string_value(prefix)); }",
    },
    CoreJsHelper {
        name: "__moth_text_ends_with",
        source: "function __moth_text_ends_with(text, suffix) { return __moth_string_value(text).endsWith(__moth_string_value(suffix)); }",
    },
];

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_text_helpers(&mut self) {
        self.emit_referenced_core_helpers(CORE_TEXT_JS_HELPERS);
    }
}
