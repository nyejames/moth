//! Stable alias target projection and generated-local alias restoration.

use super::nominal_blueprints::intern_generated_canonical_type;
use super::{
    GenericTemplateArtefact, ModuleMaterialisationContext, ModuleMaterialisationPreparation,
    append_materialised_declaration,
};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::module_ast::environment::AstModuleEnvironment;
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAlias;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
impl ModuleMaterialisationPreparation {
    pub(super) fn stable_alias_target_identity(
        &self,
        path: &InternedPath,
        alias: &ResolvedTypeAlias,
    ) -> Result<CanonicalTypeIdentity, CompilerError> {
        self.stable_type_identity(alias.target_type_id)
            .map_err(|error| {
                CompilerError::new(
                    format!(
                        "Retained local type alias '{}' violated the completed-target invariant: {}",
                        path.to_string(&self.string_table),
                        error.msg,
                    ),
                    alias.declaration_location.clone(),
                    ErrorType::Compiler,
                )
                .with_render_context(self.string_table.clone())
            })
    }
}
pub(super) fn restore_generated_local_alias(
    nominal_source: &GenericTemplateArtefact,
    context: &ModuleMaterialisationContext,
    environment: &mut AstModuleEnvironment,
    local_path: InternedPath,
    local_path_components: &[String],
    external_package_registry: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> Result<bool, CompilerError> {
    let Some(alias) = context
        .semantic_closure
        .aliases
        .iter()
        .find(|alias| alias.local_path.as_ref() == local_path_components)
    else {
        return Ok(false);
    };
    let type_id = intern_generated_canonical_type(
        &alias.target_type_identity,
        &mut environment.type_environment,
        external_package_registry,
        nominal_source,
        string_table,
    )?;
    let declaration = Declaration {
        id: local_path.clone(),
        value: Expression::new(
            ExpressionKind::NoValue,
            Default::default(),
            type_id,
            diagnostic_type_spelling(type_id, &environment.type_environment),
            ValueMode::ImmutableReference,
        ),
        config_qualifier: None,
    };
    let lookups = Rc::make_mut(&mut environment.lookups);
    if lookups.declaration_table.get_by_path(&local_path).is_none() {
        append_materialised_declaration(lookups, declaration)?;
    }
    Rc::make_mut(&mut lookups.resolved_type_aliases_by_path).insert(
        local_path.clone(),
        ResolvedTypeAlias {
            diagnostic_type: diagnostic_type_spelling(type_id, &environment.type_environment),
            target_type_id: type_id,
            declaration_location: alias.declaration_location.materialise(string_table),
        },
    );
    Rc::make_mut(&mut lookups.declaration_semantics).register_materialised_value(local_path);
    Ok(true)
}
