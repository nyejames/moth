//! Per-scope local declaration mutation for AST scope contexts.

use super::*;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;

impl ScopeContext {
    // --------------------------
    //  Local mutation
    // --------------------------

    /// Replace the declarations in the current scope frame.
    ///
    /// WHAT: validates every parameter's name collision before atomically replacing the frame's
    ///       declarations and rebuilding its name index for a function or start body.
    pub(crate) fn set_local_declarations(
        &mut self,
        declarations: Vec<Declaration>,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Result<(), DiagnosticDraft> {
        // Check every parameter before publishing the frame so a rejected signature installs none.
        for declaration in &declarations {
            let Some(name) = path_fork.component(declaration.id) else {
                continue;
            };
            if let Some(reference) = self.get_reference(&name) {
                let diagnostic = names_declarations::shadowed_name(name, declaration.binding_span);
                let diagnostic = if let Some(previous_span) = reference.binding_span() {
                    diagnostic.with_label(
                        Some(previous_span),
                        Some(LabelDraftMessage::PreviousDeclaration),
                    )
                } else {
                    diagnostic
                };
                return Err(diagnostic);
            }
            if let Some(diagnostic) =
                self.check_body_binding_visibility(name, declaration.binding_span, string_table)
            {
                return Err(diagnostic);
            }
        }
        self.arena
            .borrow_mut()
            .frame_mut(self.current_frame_id)
            .set_local_declarations(declarations, path_fork);
        Ok(())
    }

    pub(crate) fn with_pending_catch_assignment_targets(
        &self,
        target_names: &[StringId],
    ) -> ScopeContext {
        let mut context = self.new_expression_policy_context();
        context
            .pending_catch_assignment_targets
            .extend(target_names.iter().copied());
        context
    }

    pub(crate) fn activate_pending_catch_assignment_targets(&self) -> ScopeContext {
        let mut context = self.new_expression_policy_context();
        context
            .unavailable_assignment_targets
            .extend(self.pending_catch_assignment_targets.iter().copied());
        context
    }

    pub(crate) fn is_assignment_target_unavailable(&self, name: StringId) -> bool {
        self.unavailable_assignment_targets.contains(&name)
    }
}
