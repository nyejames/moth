//! JavaScript helpers for `@core/text`.
//!
//! WHAT: implements the Core Text surface over the canonical JS String runtime boundary.
//! WHY: Moth `String` values may carry reactive-template metadata, so package helpers must
//!      consume content through the shared runtime helper before applying host operations.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_text_helpers(&mut self) {
        let helpers: &[(&str, &str)] = &[
            (
                "__moth_text_length",
                "function __moth_text_length(text) { return Array.from(__moth_string_value(text)).length; }",
            ),
            (
                "__moth_text_is_empty",
                "function __moth_text_is_empty(text) { return __moth_string_value(text).length === 0; }",
            ),
            (
                "__moth_text_contains",
                "function __moth_text_contains(text, pattern) { return __moth_string_value(text).includes(__moth_string_value(pattern)); }",
            ),
            (
                "__moth_text_starts_with",
                "function __moth_text_starts_with(text, prefix) { return __moth_string_value(text).startsWith(__moth_string_value(prefix)); }",
            ),
            (
                "__moth_text_ends_with",
                "function __moth_text_ends_with(text, suffix) { return __moth_string_value(text).endsWith(__moth_string_value(suffix)); }",
            ),
        ];

        self.emit_referenced_core_helpers(helpers);
    }
}
