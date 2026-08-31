//! Local mutation plus warning sinks for AST scope contexts.

use super::*;

impl ScopeContext {
    // --------------------------
    //  Local mutation
    // --------------------------

    /// Add a body-local declaration to the current scope frame.
    ///
    /// WHAT: records the declaration in the current frame, updates the visibility
    ///       gate if one is installed, and updates the frame-local name index.
    /// WHY: child scopes inherit ancestor frames by parent link, so additions must
    ///      stay in the current frame and never leak into the parent.
    pub fn add_var(&mut self, declaration: Declaration, binding_location: SourceLocation) {
        if let Some(visible_declarations) = self.visible_declaration_ids.as_mut() {
            Arc::make_mut(visible_declarations).insert(declaration.id.clone());
        }
        self.arena
            .borrow_mut()
            .frame_mut(self.current_frame_id)
            .add_var(declaration, binding_location);
        increment_ast_counter(AstCounter::ScopeLocalDeclarationsInserted);
    }

    /// Register a body-local declaration authored with `#`.
    ///
    /// WHAT: keeps normal local lookup behavior while recording the syntax-origin fact.
    /// WHY: foldability alone is broader than the fixed-capacity rule, which requires
    ///      a bare explicit compile-time constant name.
    pub(crate) fn add_compile_time_var(
        &mut self,
        declaration: Declaration,
        binding_location: SourceLocation,
    ) {
        if let Some(visible_declarations) = self.visible_declaration_ids.as_mut() {
            Arc::make_mut(visible_declarations).insert(declaration.id.clone());
        }
        self.arena
            .borrow_mut()
            .frame_mut(self.current_frame_id)
            .add_compile_time_var(declaration, binding_location);
        increment_ast_counter(AstCounter::ScopeLocalDeclarationsInserted);
    }

    pub fn is_inside_loop(&self) -> bool {
        self.loop_depth > 0
    }

    pub fn emit_warning(&self, warning: CompilerDiagnostic) {
        self.shared.emitted_warnings.borrow_mut().push(warning);
    }

    pub fn take_emitted_warnings(&self) -> Vec<CompilerDiagnostic> {
        std::mem::take(&mut *self.shared.emitted_warnings.borrow_mut())
    }
}
