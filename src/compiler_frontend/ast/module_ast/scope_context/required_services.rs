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
    pub(crate) fn trait_environment(&self) -> &TraitEnvironment {
        self.shared.trait_environment.as_ref()
    }

    pub(crate) fn trait_evidence_environment(&self) -> &TraitEvidenceEnvironment {
        self.shared.trait_evidence_environment.as_ref()
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

    /// Return a visible, already-resolved module constant initializer by canonical path.
    ///
    /// WHAT: exposes only declarations confirmed in the scope's resolved-constant set and
    ///       applies the same path visibility gate used by source reference lookup.
    /// WHY: emission-time TIR folding precedes `ConstValueStore` construction, but its
    ///      numeric range operands still need the existing Stage 3-resolved initializer.
    pub(crate) fn resolved_module_constant_expression(
        &self,
        path: &PathId,
    ) -> Option<&crate::compiler_frontend::ast::expressions::expression::Expression> {
        let declaration = self
            .shared
            .top_level_declarations
            .get_visible_resolved_by_path(path, self.visible_declaration_ids.as_deref())?;
        self.is_explicit_compile_time_constant(declaration)
            .then_some(&declaration.value)
    }

    /// Build the narrow TIR fold state for the current AST scope.
    pub fn new_tir_fold_context<'b>(
        &'b self,
        string_table: &'b mut StringTable,
    ) -> TirFoldContext<'b> {
        TirFoldContext {
            string_table,
            template_const_loop_iteration_limit: self.shared.template_const_loop_iteration_limit,
            numeric_profile: self.shared.numeric_profile,
            bindings: Vec::new(),
            source_scope: Some(self),
        }
    }
}
