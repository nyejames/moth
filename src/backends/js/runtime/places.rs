//! Computed-place helpers for the JS runtime.
//!
//! WHAT: closures capturing base reference + key for field/index access.
//! WHY: struct field and collection index mutations must route through the same
//! reference layer as slot bindings — returning a composable computed ref achieves
//! this uniformly.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits computed-place helpers for field and index access.
    ///
    /// WHAT: `__moth_field` and `__moth_index` each return a computed-place record capturing the base
    /// reference and key. The record implements `__moth_get`/`__moth_set` so it composes correctly
    /// with `__moth_read` and `__moth_write`.
    /// WHY: struct field and collection index mutations must route through the same reference
    /// layer as slot bindings — returning a composable computed ref achieves this uniformly.
    pub(crate) fn emit_runtime_computed_place_helpers(&mut self) {
        self.emit_line("function __moth_field(baseRef, field) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_ref: true,");
                em.emit_line("__moth_kind: \"computed\",");
                em.emit_line("__moth_get() {");
                em.with_indent(|inner| inner.emit_line("return __moth_read(baseRef)[field];"));
                em.emit_line("},");
                em.emit_line("__moth_set(value) {");
                em.with_indent(|inner| inner.emit_line("__moth_read(baseRef)[field] = value;"));
                em.emit_line("}");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_index(baseRef, index) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_ref: true,");
                em.emit_line("__moth_kind: \"computed\",");
                em.emit_line("__moth_get() {");
                em.with_indent(|inner| inner.emit_line("return __moth_read(baseRef)[index];"));
                em.emit_line("},");
                em.emit_line("__moth_set(value) {");
                em.with_indent(|inner| inner.emit_line("__moth_read(baseRef)[index] = value;"));
                em.emit_line("}");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
