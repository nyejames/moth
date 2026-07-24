//! Alias helpers for the JS runtime.
//!
//! WHAT: binding-mode transitions for borrow and value assignment.
//! WHY: Moth has distinct borrow-assign and value-assign semantics that must
//! map to distinct JS operations — conflating them would silently break aliasing.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits binding-mode transition helpers for borrow and value assignment.
    ///
    /// WHAT: `__moth_assign_borrow` makes a fresh slot binding point at another reference (alias
    /// mode), or write-through if already an alias; `__moth_assign_value` collapses an alias and
    /// writes a plain value into the binding's slot.
    /// WHY: Moth has distinct borrow-assign and value-assign semantics that must map to
    /// distinct JS operations — conflating them would silently break aliasing.
    pub(crate) fn emit_runtime_alias_helpers(&mut self) {
        self.emit_line("function __moth_assign_borrow(binding, ref) {");
        self.with_indent(|emitter| {
            // If the binding is already an alias, write through to the existing target rather
            // than rebinding — this preserves the observable aliasing contract.
            emitter.emit_line("if (binding.__moth_mode === \"alias\") {");
            emitter.with_indent(|em| em.emit_line("return __moth_write(binding, __moth_read(ref));"));
            emitter.emit_line("}");
            emitter.emit_line("binding.__moth_mode = \"alias\";");
            emitter.emit_line("binding.__moth_target = ref;");
            emitter.emit_line("return binding;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_assign_value(binding, value) {");
        self.with_indent(|emitter| {
            // If the binding is an alias, write through so the aliased location gets the value.
            emitter.emit_line("if (binding.__moth_mode === \"alias\") {");
            emitter.with_indent(|em| em.emit_line("return __moth_write(binding, value);"));
            emitter.emit_line("}");
            // Slot mode: clear any stale alias target and write directly.
            emitter.emit_line("binding.__moth_mode = \"slot\";");
            emitter.emit_line("binding.__moth_target = null;");
            emitter.emit_line("binding.__moth_slot.value = value;");
            emitter.emit_line("return binding;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
