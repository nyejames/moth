//! Choice helpers for the JS runtime.
//!
//! WHAT: structural equality for nominal choice carriers.
//! WHY: choice carriers are object literals with a numeric `tag` and named payload fields,
//!      so reference equality (`===`) is incorrect; we must compare tags and then every
//!      payload field recursively.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits the choice structural equality helper.
    ///
    /// WHAT: `__moth_choice_eq` compares two choice carriers by tag and then by every
    /// payload field. Nested tagged choice or option carriers recurse and String payloads use
    /// the canonical content equality helper rather than relying on a backend representation.
    ///
    /// WHY: the frontend only approves choice equality when every payload field supports
    /// structural equality, so the helper can safely assume primitives are `===`-comparable
    /// and only needs recursive handling for tagged choice or option carriers.
    pub(crate) fn emit_runtime_choice_helpers(&mut self) {
        self.emit_line("function __moth_choice_eq(a, b) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (a === b) return true;");
            emitter.emit_line("if (!a || !b) return false;");
            emitter.emit_line("if (a.tag !== b.tag) return false;");
            emitter.emit_line("var keys = Object.keys(a);");
            emitter.emit_line("for (var i = 0; i < keys.length; i++) {");
            emitter.with_indent(|inner| {
                inner.emit_line("var k = keys[i];");
                inner.emit_line("if (k === \"tag\") continue;");
                inner.emit_line("var av = a[k], bv = b[k];");
                inner.emit_line("if (__moth_string_like(av) || __moth_string_like(bv)) {");
                inner.with_indent(|deepest| {
                    deepest.emit_line("if (!__moth_string_equal(av, bv)) return false;");
                });
                inner.emit_line("} else if (av && typeof av === \"object\" && \"tag\" in av) {");
                inner.with_indent(|deepest| {
                    deepest.emit_line("if (!__moth_choice_eq(av, bv)) return false;");
                });
                inner.emit_line("} else if (av !== bv) {");
                inner.with_indent(|deepest| {
                    deepest.emit_line("return false;");
                });
                inner.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("return true;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
