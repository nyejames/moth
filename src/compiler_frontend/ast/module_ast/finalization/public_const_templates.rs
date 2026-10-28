//! Default const-template interface projection at the AST finalization boundary.
//!
//! WHAT: indexes the bound template content of module constants by declaration path and converts it
//! to the owned public interface value consumed by generated materialisation.
//! WHY: the constant store already holds each template's projection. Finalization reads that bound
//! content rather than projecting, validating or folding the same template a second time.

use super::finalizer::AstFinalizer;
use super::normalize_ast::TemplateNormalizationError;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, PublicConstTemplate, owned_folded_string_from_store_string,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;

impl AstFinalizer<'_, '_> {
    /// Project the public template value of every module constant whose bound initializer is a
    /// `Constant` leaf, keyed by declaration path.
    ///
    /// WHAT: reads the template content already bound in the constant store.
    /// WHY: generated materialisation rebuilds these declarations from this path index. Template
    ///      initializers were already projected when constant resolution or body-local parsing
    ///      bound them, so finalization projects nothing else here.
    pub(super) fn project_default_const_templates(
        &self,
        string_table: &StringTable,
    ) -> Result<
        FxHashMap<PathId, (PublicConstTemplate, Option<OwnedFoldedString>)>,
        TemplateNormalizationError,
    > {
        let mut default_values = Vec::new();
        for declaration_id in self.environment.lookups.resolved_module_constants.iter() {
            let declaration = self
                .environment
                .lookups
                .declaration_table
                .get_by_id(declaration_id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "Resolved module-constant ID had no declaration-table row.",
                    )
                })?;
            let Some(initializer) = declaration.value.initializer() else {
                continue;
            };
            if let ExpressionOperationView::Constant(value) = self
                .context
                .expression_builder
                .view(initializer)?
                .operation()
            {
                default_values.push((declaration.id, value));
            }
        }
        let resources = self
            .context
            .file_value_resolution
            .as_ref()
            .map(|services| services.module_resources.borrow());
        let mut by_path = FxHashMap::default();
        for (path, value) in default_values {
            insert_default_template(
                &mut by_path,
                path,
                value,
                &self.context.const_values,
                resources.as_deref(),
                string_table,
            )?;
        }
        Ok(by_path)
    }
}

fn insert_default_template(
    by_path: &mut FxHashMap<PathId, (PublicConstTemplate, Option<OwnedFoldedString>)>,
    path: PathId,
    value: ConstValueId,
    constants: &ConstValueStore,
    resources: Option<&ModuleResourceTable>,
    string_table: &StringTable,
) -> Result<(), TemplateNormalizationError> {
    let payload = constants.payload(value).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "Template default references missing constant content.",
        )
    })?;
    let ConstValuePayload::Template { template, folded } = payload else {
        return Ok(());
    };
    let folded = folded
        .as_ref()
        .map(|value| owned_folded_string_from_store_string(value, resources, string_table))
        .transpose()?;
    let entry = (template.clone(), folded);
    if let Some(existing) = by_path.get(&path)
        && *existing != entry
    {
        return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
            "Public const-template projection produced conflicting values for one declaration path.",
        ).into());
    }
    by_path.insert(path, entry);
    Ok(())
}
