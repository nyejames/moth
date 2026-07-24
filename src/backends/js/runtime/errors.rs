//! Error helpers for the JS runtime.
//!
//! WHAT: canonical runtime `Error` construction plus hidden context helpers.
//! WHY: generated errors must use the same lowered struct fields as source-authored `Error(...)`
//! values while still allowing the backend to preserve source context internally.

use crate::backends::js::{
    JsEmitter, builtin_error_code_js_field_name, builtin_error_message_js_field_name,
};

impl<'hir> JsEmitter<'hir> {
    /// Emits canonical builtin error helpers used by collection and cast lowering.
    ///
    /// WHAT: normalises location paths, constructs canonical error records, and provides hidden
    /// context helpers used by backend-generated propagation paths.
    /// WHY: public Moth code accesses `Error.message` and `Error.code` through the lowered
    /// struct-field symbols. Backend-created errors must therefore construct those same fields.
    pub(crate) fn emit_runtime_error_helpers(&mut self) {
        let release_build = !self.config.pretty;
        let message_field = builtin_error_message_js_field_name(release_build);
        let code_field = builtin_error_code_js_field_name(release_build);
        let message_field_literal = format!("{message_field:?}");
        let code_field_literal = format!("{code_field:?}");

        self.emit_line("function __moth_error_normalize_file(file) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof file !== \"string\") {");
            emitter.with_indent(|em| em.emit_line("return \"\";"));
            emitter.emit_line("}");
            emitter.emit_line("if (file.startsWith(\"/\")) {");
            emitter.with_indent(|em| {
                em.emit_line("const parts = file.split(/[\\\\/]/).filter(Boolean);");
                em.emit_line("return parts.length > 0 ? parts[parts.length - 1] : file;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (/^[A-Za-z]:[\\\\/]/.test(file)) {");
            emitter.with_indent(|em| {
                em.emit_line("const parts = file.split(/[\\\\/]/).filter(Boolean);");
                em.emit_line("return parts.length > 0 ? parts[parts.length - 1] : file;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return file;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_make_error(message, code, location, trace) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line(&format!("{message_field}: message,"));
                em.emit_line(&format!("{code_field}: code,"));
                em.emit_line("__moth_location: location ?? null,");
                em.emit_line("__moth_trace: trace ?? null");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_error_result(message, code) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return { tag: \"err\", value: __moth_make_error(message, code, null, null) };",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_error_message(error) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "const message = error && error[{message_field_literal}] !== undefined ? error[{message_field_literal}] : error && error.message;",
            ));
            emitter.emit_line("return typeof message === \"string\" ? message : String(message ?? \"Unknown error\");");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_error_code(error) {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "const code = error && error[{code_field_literal}] !== undefined ? error[{code_field_literal}] : error && error.code;",
            ));
            emitter.emit_line("return typeof code === \"number\" ? code : 0;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_error_with_location(error, location) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return __moth_make_error(__moth_error_message(error), __moth_error_code(error), location, error.__moth_trace);",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_error_push_trace(error, frame) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const nextTrace = error.__moth_trace ? error.__moth_trace.concat([frame]) : [frame];");
            emitter.emit_line(
                "return __moth_make_error(__moth_error_message(error), __moth_error_code(error), error.__moth_location, nextTrace);",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_error_bubble(error, file, line, column, functionName) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const safeFunction = typeof functionName === \"string\" && functionName.length > 0 ? functionName : \"<unknown>\";");
            emitter.emit_line("const location = {");
            emitter.with_indent(|em| {
                em.emit_line("file: __moth_error_normalize_file(file),");
                em.emit_line("line,");
                em.emit_line("column,");
                em.emit_line("function: safeFunction === \"<unknown>\" ? null : safeFunction");
            });
            emitter.emit_line("};");
            emitter.emit_line("const frame = { function: safeFunction, location };");
            emitter.emit_line("const nextLocation = error.__moth_location ?? location;");
            emitter.emit_line("const located = __moth_error_with_location(error, nextLocation);");
            emitter.emit_line("return __moth_error_push_trace(located, frame);");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
