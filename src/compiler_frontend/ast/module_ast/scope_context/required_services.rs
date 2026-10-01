//! Required service accessors for AST scope contexts.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means a missing compiler setup service or internal
//! infrastructure failure. These are not user-facing diagnostics.

use super::lookup::ScopeDeclarationRef;
use super::*;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
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
    pub(crate) fn resolved_module_constant_expression(&self, path: &PathId) -> Option<&Expression> {
        let declaration = self
            .shared
            .top_level_declarations
            .get_visible_resolved_by_path(path, self.visible_declaration_ids.as_deref())?;
        self.is_explicit_compile_time_constant(declaration)
            .then_some(&declaration.value)
    }

    /// Visit the visible explicit constant initializer for either a local or module declaration.
    ///
    /// WHAT: resolves the initializer by explicit-constant visibility and hands the borrowed
    ///       value to `visit` once, cloning nothing.
    /// WHY: the module table and the local arena already own every declaration, so consumers
    ///       borrow during lookup instead of materialising an owned initializer (a full
    ///       const-record clone is never needed for a membership check, and an eager clone
    ///       would also copy ordinary runtime initializers that fail the explicit check).
    pub(crate) fn with_explicit_constant_expression<T>(
        &self,
        path: &PathId,
        path_fork: &PathInternerFork,
        visit: impl FnOnce(&Expression) -> T,
    ) -> Option<T> {
        if let Some(expression) = self.resolved_module_constant_expression(path) {
            return Some(visit(expression));
        }

        let name = path_fork.component(*path)?;
        let declaration = self.get_reference(&name)?;
        match declaration {
            // Body-local constants are arena-owned `Rc` handles, so the visit runs while the
            // handle lives; module declarations already borrow from the immutable lookup table.
            ScopeDeclarationRef::Local { declaration, .. } => (declaration.id == *path
                && self.is_explicit_compile_time_constant(declaration.as_ref()))
            .then(|| visit(&declaration.value)),
            ScopeDeclarationRef::Shared(declaration) => (declaration.id == *path
                && self.is_explicit_compile_time_constant(declaration))
            .then(|| visit(&declaration.value)),
        }
    }

    /// Resolve retained source-`#Config` provenance from the complete map or bootstrap records.
    pub(crate) fn source_config_diagnostic_origin(
        &self,
        input_name: &str,
    ) -> Option<(BuildConfigValueOrigin, Option<SourceSpan>)> {
        let resolved_map_origin = self
            .shared
            .source_build_config_values
            .as_ref()
            .and_then(|values| values.diagnostic_origin_and_span(input_name));
        if let Some((origin, Some(span))) = resolved_map_origin.as_ref() {
            return Some((*origin, Some(*span)));
        }

        let bootstrap_origin = self
            .shared
            .config_resolution
            .as_ref()
            .and_then(|services| services.diagnostic_source_config_origin(input_name));

        match (resolved_map_origin, bootstrap_origin) {
            (Some((map_origin, map_span)), Some((bootstrap_origin, bootstrap_span)))
                if map_origin == bootstrap_origin =>
            {
                Some((map_origin, bootstrap_span.or(map_span)))
            }
            (Some(map_origin), None) => Some(map_origin),
            (None, Some(bootstrap_origin)) => Some(bootstrap_origin),
            _ => None,
        }
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
