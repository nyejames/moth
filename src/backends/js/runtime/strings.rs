//! String helpers for the JS runtime.
//!
//! WHAT: canonical runtime handling for Moth `String` values and value-to-string conversion.
//! WHY: quoted slices, runtime templates and reactive template values share one language type,
//!      but a reactive template may still carry backend metadata until a value boundary consumes
//!      it. These helpers keep equality, text operations and map keys content-based.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_runtime_string_helpers(&mut self, emitted_code_uses_maps: bool) {
        self.emit_line("function __moth_string_like(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (__moth_is_ref(value)) {");
            emitter.with_indent(|em| {
                em.emit_line("return __moth_string_like(__moth_read(value));");
            });
            emitter.emit_line("}");
            emitter.emit_line(
                "return typeof value === \"string\" || (value !== null && typeof value === \"object\" && value.__moth_template === true && typeof value.snapshot === \"function\");",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_string_value(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (value === undefined || value === null) {");
            emitter.with_indent(|em| em.emit_line("return \"\";"));
            emitter.emit_line("}");
            emitter.emit_line("if (__moth_is_ref(value)) {");
            emitter.with_indent(|em| {
                em.emit_line("return __moth_string_value(__moth_read(value));");
            });
            emitter.emit_line("}");
            emitter.emit_line(
                "if (value.__moth_template === true && typeof value.snapshot === \"function\") {",
            );
            emitter.with_indent(|em| em.emit_line("return value.snapshot();"));
            emitter.emit_line("}");
            emitter.emit_line("return value;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_string_equal(left, right) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return __moth_string_value(left) === __moth_string_value(right);");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_map_key(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return __moth_string_like(value) ? __moth_string_value(value) : value;",
            );
        });
        self.emit_line("}");
        self.emit_line("");

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
