//! String helpers for the JS runtime.
//!
//! WHAT: canonical runtime handling for Moth `String` values and value-to-string conversion.
//! WHY: generic output formatting has a distinct contract from typed String operations and must
//!      preserve the existing nullish and map-display behavior.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_runtime_string_helpers(&mut self, emitted_code_uses_maps: bool) {
        self.emit_line("function __moth_value_to_string(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (value === undefined || value === null) {");
            emitter.with_indent(|em| em.emit_line("return \"\";"));
            emitter.emit_line("}");

            if emitted_code_uses_maps {
                emitter.emit_line("if (__moth_map_is_valid(value)) {");
                emitter.with_indent(|em| {
                    em.emit_line("return \"[map display unavailable]\";");
                });
                emitter.emit_line("}");
            }

            emitter.emit_line("return String(value);");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
