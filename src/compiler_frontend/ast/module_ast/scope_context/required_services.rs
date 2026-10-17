//! Required service accessors for AST scope contexts.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means a missing compiler setup service or internal
//! infrastructure failure. These are not user-facing diagnostics.

use super::lookup::ScopeDeclarationRef;
use super::*;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
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

    /// Resolve a visible module constant's canonical content, never its executable occurrence.
    pub(crate) fn resolved_module_constant_value(
        &self,
        path: &PathId,
        const_values: &ConstValueStore,
    ) -> Option<ConstValueId> {
        let declaration = self
            .shared
            .top_level_declarations
            .get_visible_resolved_by_path(path, self.visible_declaration_ids.as_deref())?;
        if !self.is_explicit_compile_time_constant(declaration) {
            return None;
        }
        const_values.value_for_path(path)
    }

    /// Apply ordinary visibility and authored `#` provenance before reading folded content.
    pub(crate) fn explicit_constant_value(
        &self,
        path: &PathId,
        path_fork: &PathInternerFork,
        const_values: &ConstValueStore,
    ) -> Option<ConstValueId> {
        if let Some(value) = self.resolved_module_constant_value(path, const_values) {
            return Some(value);
        }

        let name = path_fork.component(*path)?;
        let declaration = self.get_reference(&name)?;
        let is_explicit = match declaration {
            ScopeDeclarationRef::Local { declaration, .. } => {
                declaration.id == *path && self.is_explicit_compile_time_constant(&declaration)
            }
            ScopeDeclarationRef::Shared(declaration) => {
                declaration.id == *path && self.is_explicit_compile_time_constant(declaration)
            }
            ScopeDeclarationRef::PendingNumeric { .. } | ScopeDeclarationRef::RouteParameter(_) => {
                false
            }
        };
        is_explicit
            .then(|| const_values.value_for_path(path))
            .flatten()
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
        expression_builder: &'b mut AstExpressionBuilder,
        const_values: &'b mut ConstValueStore,
    ) -> TirFoldContext<'b> {
        TirFoldContext {
            string_table,
            expression_builder,
            const_values,
            template_const_loop_iteration_limit: self.shared.template_const_loop_iteration_limit,
            bindings: Vec::new(),
            source_scope: Some(self),
            external_package_registry: Some(self.shared.external_package_registry.as_ref()),
        }
    }
}
