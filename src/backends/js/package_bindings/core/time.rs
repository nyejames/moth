//! JavaScript helpers for `@core/time`.
//!
//! WHAT: emits the non-inline helper used by the typed time package.
//! WHY: most `@core/time` calls lower to pure JS expressions, but ISO parsing needs validation
//! and must return Moth's internal fallible carrier shape.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_time_helpers(&mut self) {
        if !self.referenced_external_runtime_function("__moth_time_timestamp_from_iso_string") {
            return;
        }

        self.emit_line("function __moth_time_timestamp_from_iso_string(text) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const millis = Date.parse(text);");
            emitter.emit_line("if (Number.isNaN(millis)) {");
            emitter.with_indent(|em| {
                em.emit_line(
                    "const err = __moth_make_error(\"Invalid ISO timestamp\", 400, null, null);",
                );
                em.emit_line("return { tag: \"err\", value: err };");
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: millis };");
        });
        self.emit_line("}");
    }
}
