//! Binding helpers for the JS runtime.
//!
//! WHAT: reference record construction, parameter normalisation, slot read/write,
//! and alias-chain resolution.
//! WHY: every local and parameter in emitted JS flows through this layer so
//! higher-level emission code can assume uniform binding semantics.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits the core binding and slot read/write helpers.
    ///
    /// WHAT: `__moth_is_ref` identifies reference records; `__moth_binding` constructs slot bindings;
    /// `__moth_param_binding` normalises call arguments from plain JS values or alias refs;
    /// `__moth_resolve` walks alias chains; `__moth_read`/`__moth_write` perform guarded slot or
    /// computed-place reads and writes.
    /// WHY: every local and parameter in emitted JS flows through this layer so higher-level
    /// emission code can assume uniform binding semantics.
    pub(crate) fn emit_runtime_binding_helpers(&mut self) {
        self.emit_line("function __moth_is_ref(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return value !== null && typeof value === \"object\" && value.__moth_ref === true;",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_binding(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return { __moth_ref: true, __moth_kind: \"binding\", __moth_mode: \"slot\", __moth_slot: { value }, __moth_target: null };",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_param_binding(value) {");
        self.with_indent(|emitter| {
            // Calls from JS hosts can pass plain values; Moth-to-Moth calls pass
            // reference records. Normalise both so function bodies only deal with bindings.
            emitter.emit_line("if (!__moth_is_ref(value)) {");
            emitter.with_indent(|em| em.emit_line("return __moth_binding(value);"));
            emitter.emit_line("}");
            emitter.emit_line("if (value.__moth_kind === \"binding\") {");
            emitter.with_indent(|em| em.emit_line("return value;"));
            emitter.emit_line("}");
            // Computed-place ref: wrap in an alias binding so callers get a uniform handle.
            emitter.emit_line("const binding = __moth_binding(undefined);");
            emitter.emit_line("binding.__moth_mode = \"alias\";");
            emitter.emit_line("binding.__moth_target = value;");
            emitter.emit_line("return binding;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_resolve(ref) {");
        self.with_indent(|emitter| {
            // Walk alias chains until a slot binding or computed-place ref is reached.
            emitter.emit_line(
                "while (ref.__moth_kind === \"binding\" && ref.__moth_mode === \"alias\") {",
            );
            emitter.with_indent(|em| em.emit_line("ref = ref.__moth_target;"));
            emitter.emit_line("}");
            emitter.emit_line("return ref;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_read(ref) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const resolved = __moth_resolve(ref);");
            emitter.emit_line(
                "return resolved.__moth_kind === \"binding\" ? resolved.__moth_slot.value : resolved.__moth_get();",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_write(ref, value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const resolved = __moth_resolve(ref);");
            emitter.emit_line("if (resolved.__moth_kind === \"binding\") {");
            emitter.with_indent(|em| em.emit_line("resolved.__moth_slot.value = value;"));
            emitter.emit_line("} else {");
            emitter.with_indent(|em| em.emit_line("resolved.__moth_set(value);"));
            emitter.emit_line("}");
            emitter.emit_line("return value;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
