//! Required service accessors for AST scope contexts.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means a missing compiler setup service or internal
//! infrastructure failure. These are not user-facing diagnostics.

use super::*;
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::ids::TraitId;

impl ScopeContext {
    // --------------------------
    //  Required services
    // --------------------------

    pub(crate) fn required_project_path_resolver(
        &self,
        operation: &str,
    ) -> Result<&ProjectPathResolver, CompilerError> {
        let Some(resolver) = self.shared.project_path_resolver.as_ref() else {
            return_compiler_error!(
                "Missing project path resolver during '{}'. Context scope: '{:?}'. This is a compiler setup bug.",
                operation,
                self.scope
            );
        };
        Ok(resolver)
    }

    pub(crate) fn required_source_file_scope(
        &self,
        operation: &str,
    ) -> Result<&InternedPath, CompilerError> {
        let Some(source_scope) = self.shared.source_file_scope.as_ref() else {
            return_compiler_error!(
                "Missing source file scope during '{}'. Context scope: '{:?}'. This is a compiler setup bug.",
                operation,
                self.scope
            );
        };
        Ok(source_scope)
    }

    pub(crate) fn trait_environment(&self) -> &TraitEnvironment {
        if let Some(trait_environment) = &self.shared.trait_environment_override {
            return trait_environment.as_ref();
        }

        self.shared.lookups.trait_environment.as_ref()
    }

    pub(crate) fn trait_evidence_environment(&self) -> &TraitEvidenceEnvironment {
        self.shared.lookups.trait_evidence_environment.as_ref()
    }

    pub(crate) fn trait_id_is_visible(&self, trait_id: TraitId) -> bool {
        let Some(trait_definition) = self.trait_environment().get(trait_id) else {
            return false;
        };

        if matches!(trait_definition.visibility, TraitVisibility::Core) {
            return true;
        }

        let Some(file_visibility) = &self.shared.file_visibility else {
            // Synthetic test contexts may omit file visibility. Keep those contexts permissive;
            // production scopes are built from header visibility and take the branch below.
            return true;
        };

        file_visibility.visible_trait_names.values().any(|target| {
            self.trait_environment()
                .has_path(trait_id, target.local_path())
        })
    }

    /// Build the narrow TIR fold state for the current AST scope.
    pub fn new_tir_fold_context<'b>(
        &'b self,
        string_table: &'b mut StringTable,
    ) -> TirFoldContext<'b> {
        TirFoldContext {
            string_table,
            template_const_loop_iteration_limit: self.shared.template_const_loop_iteration_limit,
            bindings: Vec::new(),
        }
    }
}
