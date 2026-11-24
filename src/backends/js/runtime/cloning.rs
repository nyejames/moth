//! Clone helpers for the JS runtime.
//!
//! Explicit `copy` duplicates each allocation once, preserving the copied graph's internal edges.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits the deep-copy helper for explicit `copy` semantics.
    ///
    /// One memo belongs to the whole copy, including retained map entries and repeated fields.
    pub(crate) fn emit_runtime_clone_helpers(&mut self, emitted_code_uses_maps: bool) {
        self.emit_line("function __moth_clone_value(value, copies) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (value === null || typeof value !== \"object\") return value;");
            emitter.emit_line("copies ??= new WeakMap();");
            emitter.emit_line("const existing = copies.get(value);");
            emitter.emit_line("if (existing !== undefined) return existing;");
            emitter.emit_line("if (Array.isArray(value)) {");
            emitter.with_indent(|em| {
                em.emit_line("const result = new Array(value.length);");
                em.emit_line("copies.set(value, result);");
                em.emit_line("for (let index = 0; index < value.length; index++) {");
                em.with_indent(|inner| {
                    inner.emit_line("if (index in value) result[index] = __moth_clone_value(value[index], copies);");
                });
                em.emit_line("}");
                em.emit_line("return result;");
            });
            emitter.emit_line("}");

            if emitted_code_uses_maps {
                emitter.emit_line("if (__moth_map_is_valid(value)) {");
                emitter.with_indent(|em| {
                    em.emit_line("const result = __moth_map_new();");
                    em.emit_line("copies.set(value, result);");
                    em.emit_line("value.map.forEach((item, key) => {");
                    em.with_indent(|inner| {
                        inner.emit_line("result.map.set(__moth_clone_value(key, copies), __moth_clone_value(item, copies));");
                    });
                    em.emit_line("});");
                    em.emit_line("return result;");
                });
                emitter.emit_line("}");
            }

            emitter.emit_line("const result = {};");
            emitter.emit_line("copies.set(value, result);");
            emitter.emit_line("for (const key of Object.keys(value)) {");
            emitter.with_indent(|inner| {
                inner.emit_line("result[key] = __moth_clone_value(value[key], copies);");
            });
            emitter.emit_line("}");
            emitter.emit_line("return result;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
