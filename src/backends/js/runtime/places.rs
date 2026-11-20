//! Computed-place helpers for the JS runtime.
//!
//! WHAT: closures capturing the base storage + key for field/index access.
//! WHY: struct field and collection index mutations must route through the same
//! reference layer as slot bindings — returning a composable computed ref achieves
//! this uniformly, and capturing the base storage keeps a projected alias rooted in
//! the allocation the base held when the alias was bound.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits computed-place helpers for field and index access.
    ///
    /// WHAT: `__moth_field` and `__moth_index` each return a computed-place record that captures the
    /// object its base reference held when the record was constructed, plus the field or index key.
    /// The record implements `__moth_get`/`__moth_set` so it composes correctly with
    /// `__moth_read` and `__moth_write`.
    /// WHY: a projected alias must stay rooted in the allocation family its containing object
    /// belonged to at binding time. Re-resolving the base reference on every access would make a
    /// later rebinding of the base slot retarget reads and writes through the alias. Capturing the
    /// storage once keeps write-through aimed at the original field or element and lets other
    /// observers of that allocation see the change.
    pub(crate) fn emit_runtime_computed_place_helpers(&mut self) {
        self.emit_line("function __moth_field(baseRef, field) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const base = __moth_read(baseRef);");
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_ref: true,");
                em.emit_line("__moth_kind: \"computed\",");
                em.emit_line("__moth_get() {");
                em.with_indent(|inner| inner.emit_line("return base[field];"));
                em.emit_line("},");
                em.emit_line("__moth_set(value) {");
                em.with_indent(|inner| inner.emit_line("base[field] = value;"));
                em.emit_line("}");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");

        // Indices are exactly I32, whose JS carrier is the exact Number carrier, so the index
        // helper reads and writes the captured base array directly with no BigInt coercion path.
        self.emit_line("function __moth_index(baseRef, index) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const base = __moth_read(baseRef);");
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_ref: true,");
                em.emit_line("__moth_kind: \"computed\",");
                em.emit_line("__moth_get() {");
                em.with_indent(|inner| inner.emit_line("return base[index];"));
                em.emit_line("},");
                em.emit_line("__moth_set(value) {");
                em.with_indent(|inner| inner.emit_line("base[index] = value;"));
                em.emit_line("}");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
