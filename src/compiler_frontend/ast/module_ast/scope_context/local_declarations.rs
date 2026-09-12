//! Per-scope local declaration mutation for AST scope contexts.

use super::*;

impl ScopeContext {
    // --------------------------
    //  Local mutation
    // --------------------------

    /// Replace the declarations in the current scope frame.
    ///
    /// WHAT: rebuilds the frame-local name index and declaration vec. Used when a
    ///       function or start body frame is initialised with parameter declarations.
    pub(crate) fn set_local_declarations(
        &mut self,
        declarations: Vec<Declaration>,
        path_fork: &PathInternerFork,
    ) {
        self.arena
            .borrow_mut()
            .frame_mut(self.current_frame_id)
            .set_local_declarations(declarations, path_fork);
    }

    pub(crate) fn with_pending_catch_assignment_targets(
        &self,
        target_names: &[StringId],
    ) -> ScopeContext {
        let mut context = self.clone();
        context
            .pending_catch_assignment_targets
            .extend(target_names.iter().copied());
        context
    }

    pub(crate) fn activate_pending_catch_assignment_targets(&self) -> ScopeContext {
        let mut context = self.clone();
        context
            .unavailable_assignment_targets
            .extend(self.pending_catch_assignment_targets.iter().copied());
        context
    }

    pub(crate) fn is_assignment_target_unavailable(&self, name: StringId) -> bool {
        self.unavailable_assignment_targets.contains(&name)
    }
}
