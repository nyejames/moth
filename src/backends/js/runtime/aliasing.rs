//! Alias helpers for the JS runtime.
//!
//! WHAT: binding-mode transitions for borrow and value assignment.
//! WHY: Moth has distinct borrow-assign and value-assign semantics that must
//! map to distinct JS operations — conflating them would silently break aliasing.

use crate::backends::js::JsEmitter;

impl<'hir> JsEmitter<'hir> {
    /// Emits binding-mode transition helpers for borrow and value assignment.
    ///
    /// WHAT: `__moth_assign_borrow` points a slot binding at another reference (alias mode);
    /// `__moth_assign_value` writes a plain value into a slot binding. Both write through when
    /// the binding is already an alias.
    /// WHY: assignment through a mutable alias writes to its referent. These helpers serve
    /// writes whose binding mode depends on the control-flow path. Writes that create a binding
    /// or target a definite alias are resolved at emission time instead.
    pub(crate) fn emit_runtime_alias_helpers(&mut self) {
        self.emit_line("function __moth_assign_borrow(binding, ref) {");
        self.with_indent(|emitter| {
            // An existing alias writes through rather than rebinding: assignment must not detach it.
            emitter.emit_line("if (binding.__moth_mode === \"alias\") {");
            emitter
                .with_indent(|em| em.emit_line("return __moth_write(binding, __moth_read(ref));"));
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
            emitter.emit_line("binding.__moth_mode = \"slot\";");
            emitter.emit_line("binding.__moth_target = null;");
            emitter.emit_line("binding.__moth_slot.value = value;");
            emitter.emit_line("return binding;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
