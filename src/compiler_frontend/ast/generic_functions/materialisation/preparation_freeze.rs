//! Declaring-module preparation capture and publication freeze.
use super::super::{GenericFunctionBody, GenericFunctionTemplate};
use super::artefact_emit::ModuleMaterialisationContext;
use super::frozen_syntax::StableBodySyntax;
use super::nominal_blueprints::NominalMaterialisationBlueprint;
use super::semantic_closure::{StableSemanticClosure, stable_body_symbol_names};
use super::sidecar_build::bootstrap_call_summary_from_signature;
use super::sidecar_build::fallible_carrier_for_signature;
use super::stable_types::{
    GenericTemplateArtefact, MaterialisationNominalOriginResolver, MaterialisationNominalSource,
    StableFunctionParameter, StableFunctionReturn, StableFunctionSignature, StableGenericParameter,
    StableReceiverKey,
};
use super::visibility::{
    StableExternalSymbol, StableFileVisibility, StableNamespaceBinding, StableReceiverMethod,
    StableVisibleDeclaration,
};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleLookups, DeclarationSemanticTable, ResolvedPublicTraitRoot, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodCatalog;
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedFunctionSignature, ResolvedTypeAlias,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTraitIdentity, CanonicalTypeIdentity, CanonicalTypeProjectionContext,
    ExportedGenericParameterIdentity, GenericDeclarationOrigin, ModulePrivateNominalIdentity,
    ModulePrivateTraitIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    GenericParameterId, GenericParameterListId, TypeId,
};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, FoldedValueProjectionContext, PublicConstTemplate,
    PublicFoldedValue, convert_const_value_to_folded_value, convert_expression_to_folded_value,
};
use crate::compiler_frontend::headers::binding_environment::{
    HeaderBindingEnvironment, SourceDeclarationTarget, SourceFunctionTarget,
};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;

use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

use crate::compiler_frontend::symbols::string_interning::{
    StringId, StringIdRemap, StringTable, StringTableForkSource,
};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
/// Self-contained immutable semantic context owned by one successful declaring module.
#[derive(Clone)]
pub(crate) struct ModuleMaterialisationPreparation {
    pub(crate) string_table: StringTable,
    pub(super) string_table_fork_source: OnceCell<StringTableForkSource>,
    pub(crate) entry_dir: PathId,
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
    pub(crate) stage0_resolution_facts: Option<Arc<Stage0ResolutionFacts>>,
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    /// body before its own materialisation context is published.
    pub(super) module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) declaration_table: Rc<TopLevelDeclarationTable>,
    pub(crate) binding_environment: HeaderBindingEnvironment,
    pub(crate) imported_functions_by_local_path:
        FxHashMap<PathId, AstImportedFunctionContract>,
    pub(crate) imported_struct_definitions:
        Vec<crate::compiler_frontend::ast::AstImportedStructDefinition>,
    pub(crate) imported_choice_definitions: Vec<crate::compiler_frontend::ast::AstChoiceDefinition>,
    pub(crate) const_values: ConstValueStore,
    pub(super) default_const_templates_by_path: FxHashMap<PathId, PublicConstTemplate>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<PathId, Vec<Declaration>>,
    pub(crate) resolved_function_signatures_by_path:
        FxHashMap<PathId, ResolvedFunctionSignature>,
    pub(crate) generic_function_templates_by_path: FxHashMap<PathId, GenericFunctionTemplate>,
    pub(super) generic_template_paths_by_identity:
        FxHashMap<GeneratedDeclarationIdentity, PathId>,
    pub(crate) resolved_type_aliases_by_path: FxHashMap<PathId, ResolvedTypeAlias>,
    pub(crate) choice_variant_shells_by_path: FxHashMap<PathId, Vec<ChoiceVariant>>,
    pub(crate) declaration_semantics: DeclarationSemanticTable,
    pub(crate) generic_declarations_by_path: FxHashMap<PathId, GenericDeclarationKind>,
    pub(crate) nominal_type_ids_by_path: FxHashMap<PathId, TypeId>,
    pub(super) source_nominal_paths: FxHashSet<PathId>,
    pub(super) public_trait_paths: Vec<PathId>,
    pub(super) nominal_blueprints:
        FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
    pub(crate) receiver_methods: ReceiverMethodCatalog,
    pub(crate) trait_environment: TraitEnvironment,
    pub(crate) trait_evidence_environment: TraitEvidenceEnvironment,
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) template_const_loop_iteration_limit: usize,
    pub(crate) capacity_estimate: FrontendArenaCapacityEstimate,
}

/// Construction-only owner for the declaring-module context.
///
/// WHAT: accumulates stable executable identities, validated template identities and exact local
/// call summaries while the declaring module is still compiling, then freezes the completed
/// context before publication.
/// WHY: phase-local mutation must not leak into provider metadata. The builder can either hand
/// its broad state to another in-flight generated compilation or freeze the successful module's
/// retained bodies into [`ModuleMaterialisationContext`].
pub(crate) struct ModuleMaterialisationPreparationBuilder {
    pub(super) context: ModuleMaterialisationPreparation,
}

/// Declaring-module facts captured together before AST finalisation releases its local owners.
pub(crate) struct ModuleMaterialisationEnvironmentInput<'a> {
    pub(crate) lookups: &'a AstModuleLookups,
    pub(crate) const_values: &'a ConstValueStore,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) public_trait_roots: &'a [ResolvedPublicTraitRoot],
    pub(crate) default_const_templates_by_path: FxHashMap<PathId, PublicConstTemplate>,
    pub(crate) entry_dir: PathId,
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
    pub(crate) stage0_resolution_facts: Option<Arc<Stage0ResolutionFacts>>,
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    pub(crate) module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    pub(crate) string_table: &'a StringTable,
    pub(crate) template_const_loop_iteration_limit: usize,
    pub(crate) capacity_estimate: FrontendArenaCapacityEstimate,
}

fn declaration_table_without_module_values(
    declaration_table: &Rc<TopLevelDeclarationTable>,
    const_values: &ConstValueStore,
) -> Result<Rc<TopLevelDeclarationTable>, CompilerError> {
    let mut generated = TopLevelDeclarationTable::fork_for_generated(Rc::clone(declaration_table));
    for path in const_values.module_constant_paths() {
        let value_id = const_values.value_for_path(path).ok_or_else(|| {
            CompilerError::compiler_error(
                "Module materialisation constant path has no store value for its declaration placeholder.",
            )
        })?;
        let metadata = const_values.metadata(value_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "Module materialisation constant has no store metadata for its declaration placeholder.",
            )
        })?;
        let declaration_id = generated.declaration_id_by_path(path).ok_or_else(|| {
            CompilerError::compiler_error(
                "Module materialisation constant has no declaration-table entry.",
            )
        })?;
        if !generated.replace_by_id(
            declaration_id,
            Declaration {
                id: path.clone(),
                value: Expression::no_value_with_type_id(
                    metadata.span,
                    metadata.diagnostic_type.clone(),
                    metadata.type_id,
                    metadata.value_mode.clone(),
                ),
                binding_span: None,
                config_qualifier: None,
            },
        ) {
            return Err(CompilerError::compiler_error(
                "Module materialisation constant could not replace its declaration-table row.",
            ));
        }
    }
    Ok(Rc::new(generated))
}

impl ModuleMaterialisationPreparationBuilder {
    pub(crate) fn from_environment(
        input: ModuleMaterialisationEnvironmentInput<'_>,
    ) -> Result<Self, CompilerError> {
        Ok(Self {
            context: ModuleMaterialisationPreparation::from_environment(input)?,
        })
    }

    pub(crate) fn context(&self) -> &ModuleMaterialisationPreparation {
        &self.context
    }

    pub(crate) fn finish_preparation(
        mut self,
    ) -> Result<ModuleMaterialisationPreparation, CompilerError> {
        self.context
            .rebuild_generic_template_identity_index()
            .map(|()| self.context)
    }

    pub(crate) fn finalize_generic_template_identity_index(&mut self) -> Result<(), CompilerError> {
        self.context.rebuild_generic_template_identity_index()
    }

    pub(crate) fn freeze(
        self,
        public_interface: &PublicSemanticInterface,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<Option<ModuleMaterialisationContext>, CompilerError> {
        self.context.freeze(public_interface, resources, path_fork)
    }

    pub(crate) fn install_concrete_executable_contracts(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<PathId, OriginFunctionId>,
        public_nominal_origins_by_path: &FxHashMap<PathId, OriginTypeId>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<Vec<(PathId, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.context.install_concrete_executable_contracts(
            module_origin,
            public_origins_by_path,
            public_nominal_origins_by_path,
            resources,
            path_fork,
        )
    }

    pub(crate) fn generic_function_templates_mut(
        &mut self,
    ) -> &mut FxHashMap<PathId, GenericFunctionTemplate> {
        &mut self.context.generic_function_templates_by_path
    }

    pub(crate) fn imported_functions_mut(
        &mut self,
    ) -> &mut FxHashMap<PathId, AstImportedFunctionContract> {
        &mut self.context.imported_functions_by_local_path
    }

    pub(super) fn inherit_nominal_blueprints<N: MaterialisationNominalSource>(
        &mut self,
        source: &N,
    ) -> Result<(), CompilerError> {
        for (identity, blueprint) in source.nominal_blueprints() {
            if let Some(existing) = self.context.nominal_blueprints.get_mut(identity) {
                if existing != blueprint {
                    return Err(CompilerError::compiler_error(format!(
                        "Generated materialisation {} disagrees on nominal blueprint {identity:?}",
                        N::SOURCE_LABEL,
                    )));
                }
                existing.merge_provenance_from(blueprint);
            } else {
                self.context
                    .nominal_blueprints
                    .insert(identity.clone(), blueprint.clone());
            }
        }
        Ok(())
    }
}

impl ModuleMaterialisationPreparation {
    pub(super) fn string_table_fork_source(&self) -> &StringTableForkSource {
        self.string_table_fork_source
            .get_or_init(|| self.string_table.fork_source())
    }

    /// Fork one generated-local table from the requester's immutable module prefix.
    ///
    /// The preparation table may acquire a local suffix in future construction phases, so merge
    /// that delta explicitly instead of assuming the fork source still covers the whole table.
    pub(super) fn fork_materialisation_string_table(&self) -> (StringTable, StringIdRemap) {
        let (mut string_table, base_len) = self
            .string_table_fork_source()
            .fork_for_module()
            .into_parts();
        let requester_string_remap = string_table.merge_delta_from(&self.string_table, base_len);
        (string_table, requester_string_remap)
    }

    /// Merge one generated-local table back into the compiler that owns this requester.
    ///
    /// Both tables inherit the preparation's immutable prefix. They may have independent suffixes,
    /// so only those suffixes need interning and remapping when the sidecar rejoins its batch.
    pub(crate) fn merge_materialisation_string_table_into(
        &self,
        target: &mut StringTable,
        materialised: &StringTable,
    ) -> StringIdRemap {
        target.merge_delta_from(materialised, self.string_table_fork_source().base_len())
    }

    pub(super) fn freeze(
        mut self,
        public_interface: &PublicSemanticInterface,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<Option<ModuleMaterialisationContext>, CompilerError> {
        self.rebuild_generic_template_identity_index()?;
        let mut templates = self
            .generic_function_templates_by_path
            .values()
            .filter(|template| template.body_tokens.is_some())
            .collect::<Vec<_>>();
        templates.sort_by(|left, right| left.declaration_identity.cmp(&right.declaration_identity));
        if templates.is_empty() {
            return Ok(None);
        }

        let mut declaration_closure = self
            .binding_environment
            .imported_declarations_by_origin
            .values()
            .cloned()
            .collect::<Vec<_>>();
        declaration_closure.extend(public_interface.declarations.iter().cloned());
        declaration_closure.sort_by(|left, right| left.origin.cmp(&right.origin));
        declaration_closure.dedup_by(|left, right| left.origin == right.origin);

        let mut evidence = self
            .binding_environment
            .imported_evidence_by_identity
            .values()
            .cloned()
            .collect::<Vec<_>>();
        evidence.extend(public_interface.reusable_evidence.iter().cloned());
        evidence.sort_by(|left, right| left.identity.cmp(&right.identity));
        evidence.dedup_by(|left, right| left.identity == right.identity);
        let semantic_closure = self.stable_semantic_closure(resources, path_fork)?;

        let artefacts = templates
            .into_iter()
            .map(|template| {
                self.freeze_template(template, public_interface, &semantic_closure, resources, path_fork)
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        Ok(Some(ModuleMaterialisationContext {
            declaration_closure: declaration_closure.into_boxed_slice(),
            evidence: evidence.into_boxed_slice(),
            semantic_closure,
            artefacts,
            module_origin: self.module_origin.clone(),
            frozen_identity_handle: self.frozen_identity_handle.clone(),
        }))
    }
    fn freeze_template(
        &self,
        template: &GenericFunctionTemplate,
        public_interface: &PublicSemanticInterface,
        semantic_closure: &StableSemanticClosure,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<GenericTemplateArtefact, CompilerError> {
        let declaration_identity = template.declaration_identity.clone().ok_or_else(|| {
            CompilerError::compiler_error(
                "Retained generic template has no stable declaration identity",
            )
        })?;
        let body = template.body_tokens.as_ref().ok_or_else(|| {
            CompilerError::compiler_error("Retained generic template has no body syntax")
        })?;
        let generic_parameters = self.stable_generic_parameters(template)?;
        let generic_parameter_owner = template.generic_parameter_owner.clone();
        let receiver = self
            .resolved_function_signatures_by_path
            .get(&template.function_path)
            .and_then(|resolved| resolved.receiver.as_ref())
            .map(|receiver| StableReceiverKey::capture(receiver, &self.string_table))
            .transpose()?;
        let receiver_nominal_identity = self.receiver_nominal_identity(&template.function_path)?;
        let parameter_slots = self.generic_parameter_slots(template)?;
        let signature = self.stable_function_signature(
            &template.signature,
            &parameter_slots,
            resources,
            path_fork,
        )?;
        let mut referenced_names = stable_body_symbol_names(body.tokens(), &self.string_table);
        self.retain_generic_bound_trait_names(
            &template.source_file,
            &generic_parameters,
            &mut referenced_names,
        )?;
        let selected_paths =
            self.selected_visible_paths(&template.source_file, &referenced_names)?;
        let visibility = self
            .stable_file_visibility(&template.source_file, &referenced_names, resources, path_fork)?;
        let declarations = self.stable_declaration_bindings(&selected_paths, public_interface)?;
        let local_declarations = self.stable_local_declaration_bindings(&selected_paths);
        let callables = self.stable_callable_bindings(&selected_paths, resources, path_fork)?;
        let nominals = self.stable_nominal_bindings(&selected_paths);
        let nominal_blueprints = self.stable_nominal_blueprints(
            &selected_paths,
            &signature,
            semantic_closure,
            resources,
            path_fork,
        )?;
        let content_value_at_path = |logical_path: &PathId| {
            let content_path = self.content_constant_path_for_capture(logical_path)?;
            self.stable_folded_value_at_path(&content_path, resources, path_fork)
        };
        let stage0_resolution_facts = match body {
            GenericFunctionBody::Source(_) => self.stage0_resolution_facts.as_deref(),
            GenericFunctionBody::Materialised {
                resolution_facts, ..
            } => Some(resolution_facts.as_ref()),
        };
        let frozen_identity_handle = body
            .frozen_identity_handle()
            .cloned()
            .unwrap_or_else(|| self.frozen_identity_handle.clone());

        Ok(GenericTemplateArtefact {
            declaration_identity,
            generic_parameter_owner,
            receiver,
            receiver_nominal_identity,
            function_path: template.function_path,
            source_file: template.source_file,
            declaration_span: template.declaration_span,
            body: StableBodySyntax::capture(
                body.tokens(),
                template.source_file,
                path_fork,
                &self.string_table,
                stage0_resolution_facts,
                frozen_identity_handle,
                &content_value_at_path,
            )?,
            signature,
            generic_parameters,
            visibility,
            declarations,
            local_declarations,
            callables,
            nominals,
            nominal_blueprints,
        })
    }

    fn stable_generic_parameters(
        &self,
        template: &GenericFunctionTemplate,
    ) -> Result<Box<[StableGenericParameter]>, CompilerError> {
        let parameters = self
            .type_environment
            .generic_parameters(template.generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained generic template references a missing parameter list",
                )
            })?;
        parameters
            .parameters
            .iter()
            .enumerate()
            .map(|(slot, parameter)| {
                let name = self.string_table.resolve(parameter.name).to_owned();
                let exported_identity = match template.declaration_identity.as_ref() {
                    Some(GeneratedDeclarationIdentity::Public(_)) => Some(
                        ExportedGenericParameterIdentity::new(
                            template
                                .generic_parameter_owner
                                .clone()
                                .ok_or_else(|| {
                                    CompilerError::compiler_error(
                                        "Public retained generic template has no explicit generic-parameter owner",
                                    )
                                })?,
                            slot as u32,
                            name.clone(),
                        ),
                    ),
                    Some(GeneratedDeclarationIdentity::ModulePrivate(_)) => None,
                    None => {
                        return Err(CompilerError::compiler_error(
                            "Retained generic template has no declaration identity",
                        ));
                    }
                };
                let bounds = parameter
                    .trait_bounds
                    .iter()
                    .map(|trait_id| {
                        self.trait_environment
                            .canonical_identity_for_id(*trait_id)
                            .cloned()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Retained generic parameter bound has no stable trait identity",
                                )
                            })
                    })
                    .collect::<Result<Box<[_]>, CompilerError>>()?;
                Ok(StableGenericParameter {
                    name,
                    exported_identity,
                    bounds,
                })
            })
            .collect()
    }

    fn stable_folded_value(
        &self,
        expression: &Expression,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<PublicFoldedValue, CompilerError> {
        let nominal_origins = MaterialisationNominalOriginResolver {
            type_environment: &self.type_environment,
        };
        let generic_parameter_origins = FoldedValueGenericParameterResolver;
        let projection_context = CanonicalTypeProjectionContext::new(
            &nominal_origins,
            &generic_parameter_origins,
            &self.external_package_registry,
        );
        let folded_value_context = FoldedValueProjectionContext {
            type_environment: &self.type_environment,
            string_table: &self.string_table,
            projection_context: &projection_context,
            resources: Some(resources),
            path_fork,
        };
        convert_expression_to_folded_value(expression, &folded_value_context)
    }
    pub(super) fn content_constant_path_for_capture(
        &self,
        logical_path: &PathId,
    ) -> Result<PathId, CompilerError> {
        let identity = self.frozen_identity_handle.get().ok_or_else(|| {
            CompilerError::compiler_error(
                "materialisation preparation has no frozen identity context",
            )
        })?;
        let paths = identity.paths();
        self.const_values
            .module_constant_paths()
            .find(|path| {
                let candidate = **path;
                paths.try_parent(candidate) == Some(*logical_path)
                    && paths
                        .component(candidate)
                        .map(|name| self.string_table.resolve(name) == "content")
                        .unwrap_or(false)
            })
            .copied()
            .ok_or_else(|| {
                let mut scratch = Vec::new();
                let rendered = paths.render_portable(
                    *logical_path,
                    &self.string_table,
                    &mut scratch,
                );
                CompilerError::compiler_error(format!(
                    "synthetic content constant for {} was not present before generic capture",
                    rendered
                ))
            })
    }

    pub(super) fn stable_folded_value_at_expression_path(
        &self,
        path: &PathId,
        expression: &Expression,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<PublicFoldedValue, CompilerError> {
        if matches!(expression.kind, ExpressionKind::Template(_)) {
            return self
                .default_const_templates_by_path
                .get(path)
                .cloned()
                .map(PublicFoldedValue::ConstTemplate)
                .ok_or_else(|| {
                    let rendered = if let Some(identity) = self.frozen_identity_handle.get() {
                        let mut scratch = Vec::new();
                        identity.paths().render_portable(
                            *path,
                            &self.string_table,
                            &mut scratch,
                        )
                    } else {
                        "<unknown>".to_owned()
                    };
                    CompilerError::compiler_error(format!(
                        "Retained const template at {} has no stable folded template value",
                        rendered
                    ))
                });
        }
        self.stable_folded_value(expression, resources, path_fork)
    }

    pub(super) fn stable_folded_value_at_path(
        &self,
        path: &PathId,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<PublicFoldedValue, CompilerError> {
        let value_id = self.const_values.value_for_path(path).ok_or_else(|| {
            let rendered = if let Some(identity) = self.frozen_identity_handle.get() {
                let mut scratch = Vec::new();
                identity.paths().render_portable(
                    *path,
                    &self.string_table,
                    &mut scratch,
                )
            } else {
                "<unknown>".to_owned()
            };
            CompilerError::compiler_error(format!(
                "Retained module constant at {} has no stable folded store value",
                rendered
            ))
        })?;
        self.const_values.metadata(value_id).ok_or_else(|| {
            CompilerError::compiler_error("Retained module constant has no store metadata")
        })?;
        let nominal_origins = MaterialisationNominalOriginResolver {
            type_environment: &self.type_environment,
        };
        let generic_parameter_origins = FoldedValueGenericParameterResolver;
        let projection_context = CanonicalTypeProjectionContext::new(
            &nominal_origins,
            &generic_parameter_origins,
            &self.external_package_registry,
        );
        let folded_value_context = FoldedValueProjectionContext {
            type_environment: &self.type_environment,
            string_table: &self.string_table,
            projection_context: &projection_context,
            resources: Some(resources),
            path_fork,
        };
        convert_const_value_to_folded_value(&self.const_values, value_id, &folded_value_context)
    }

    /// Retains the declaration-file spellings that make non-core generic bounds visible.
    ///
    /// Bound-provided receiver dispatch checks ordinary file visibility even after the bound has
    /// been resolved to a canonical trait identity. Bound names live in the declaration header,
    /// outside the retained body token slice, so body-name filtering alone would silently hide
    /// them in the fresh generated environment.
    fn retain_generic_bound_trait_names(
        &self,
        source_file: &PathId,
        parameters: &[StableGenericParameter],
        referenced_names: &mut FxHashSet<String>,
    ) -> Result<(), CompilerError> {
        let visibility = self.binding_environment.visibility_for(source_file)?;

        for parameter in parameters {
            for bound in &parameter.bounds {
                if matches!(bound, CanonicalTraitIdentity::Core(_)) {
                    continue;
                }

                let trait_id = self
                    .trait_environment
                    .id_for_canonical_identity(bound)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Retained generic bound has no declaring-module trait definition",
                        )
                    })?;
                let mut retained_visible_name = false;

                for (visible_name, target) in &visibility.visible_trait_names {
                    if self
                        .trait_environment
                        .has_path(trait_id, target.local_path())
                    {
                        referenced_names
                            .insert(self.string_table.resolve(*visible_name).to_owned());
                        retained_visible_name = true;
                    }
                }

                if !retained_visible_name {
                    return Err(CompilerError::compiler_error(
                        "Retained generic bound is not visible in its declaration file",
                    ));
                }
            }
        }

        Ok(())
    }

    fn generic_parameter_slots(
        &self,
        template: &GenericFunctionTemplate,
    ) -> Result<FxHashMap<GenericParameterId, usize>, CompilerError> {
        let parameters = self
            .type_environment
            .generic_parameters(template.generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained generic template references a missing parameter list",
                )
            })?;
        Ok(parameters
            .parameters
            .iter()
            .enumerate()
            .map(|(slot, parameter)| (parameter.id, slot))
            .collect())
    }

    pub(super) fn stable_function_signature(
        &self,
        signature: &FunctionSignature,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<StableFunctionSignature, CompilerError> {
        let identity = self.frozen_identity_handle.get().ok_or_else(|| {
            CompilerError::compiler_error(
                "materialisation preparation has no frozen identity context",
            )
        })?;
        let paths = identity.paths();
        let parameters = signature
            .parameters
            .iter()
            .map(|parameter| {
                let name = paths.component(parameter.id).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation function parameter has no defining name",
                    )
                })?;
                Ok(StableFunctionParameter {
                    name: self.string_table.resolve(name).to_owned(),
                    value_mode: parameter.value.value_mode.clone(),
                    reactive: parameter.value.reactive_source.is_some(),
                    folded_default: (!matches!(parameter.value.kind, ExpressionKind::NoValue))
                        .then(|| {
                            self.stable_folded_value_at_expression_path(
                                &parameter.id,
                                &parameter.value,
                                resources,
                                path_fork,
                            )
                        })
                        .transpose()?,
                    parameter_type: self
                        .materialisation_type_blueprint(parameter.value.type_id, parameter_slots)?,
                    span: parameter.value.span,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        let returns = signature
            .returns
            .iter()
            .map(|returned| {
                let type_id = returned.type_id.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation function return has no resolved type",
                    )
                })?;
                Ok(StableFunctionReturn {
                    return_type: self.materialisation_type_blueprint(type_id, parameter_slots)?,
                    channel: returned.channel,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        Ok(StableFunctionSignature {
            parameters,
            returns,
        })
    }

    fn stable_file_visibility(
        &self,
        source_file: &PathId,
        referenced_names: &FxHashSet<String>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<StableFileVisibility, CompilerError> {
        let visibility = self.binding_environment.visibility_for(source_file)?;
        let capture_declarations = |bindings: &FxHashMap<StringId, SourceDeclarationTarget>| {
            bindings
                .iter()
                .filter_map(|(name, target)| {
                    let visible_name = self.string_table.resolve(*name);
                    referenced_names
                        .contains(visible_name)
                        .then(|| StableVisibleDeclaration {
                            visible_name: visible_name.to_owned(),
                            local_path: target.local_path().clone(),
                            origin: match target {
                                SourceDeclarationTarget::Local(_) => None,
                                SourceDeclarationTarget::Imported { origin, .. } => {
                                    Some(origin.clone())
                                }
                            },
                        })
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        let mut external_symbols = Vec::new();
        for (name, symbol_id) in &visibility.visible_external_symbols {
            let visible_name = self.string_table.resolve(*name);
            if !referenced_names.contains(visible_name) {
                continue;
            }
            let identity = self
                .external_package_registry
                .canonical_symbol_identity(*symbol_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialisation external binding has no canonical identity",
                    )
                })?;
            external_symbols.push(StableExternalSymbol {
                visible_name: visible_name.to_owned(),
                identity,
            });
        }
        let mut receiver_methods = Vec::new();
        for (name, methods) in &visibility.visible_receiver_methods {
            if !referenced_names.contains(self.string_table.resolve(*name)) {
                continue;
            }
            for method in methods {
                let local_path = method.target.local_path();
                let Some(target) = self.stable_target_for_path(&method.target) else {
                    continue;
                };
                let resolved = self
                    .resolved_function_signatures_by_path
                    .get(local_path)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Visible receiver method has no resolved function signature",
                        )
                    })?;
                let receiver = resolved
                    .receiver
                    .as_ref()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Visible receiver method has no receiver signature",
                        )
                    })
                    .and_then(|receiver| {
                        StableReceiverKey::capture(receiver, &self.string_table)
                    })?;
                let generic_template = self.generic_function_templates_by_path.get(local_path);
                let parameter_slots = generic_template
                    .map(|template| self.generic_parameter_slots(template))
                    .transpose()?
                    .unwrap_or_default();
                let generic_parameters = generic_template
                    .map(|template| self.stable_generic_parameters(template))
                    .transpose()?
                    .unwrap_or_default();
                receiver_methods.push(StableReceiverMethod {
                    visible_name: self.string_table.resolve(*name).to_owned(),
                    local_path: *local_path,
                    target,
                    receiver,
                    signature: self.stable_function_signature(
                        &resolved.signature,
                        &parameter_slots,
                        resources,
                        path_fork,
                    )?,
                    summary: self
                        .imported_functions_by_local_path
                        .get(local_path)
                        .map(|contract| contract.summary.clone()),
                    generic_parameters,
                    span: method.span,
                });
            }
        }
        let mut namespace_records = visibility
            .visible_namespace_records
            .iter()
            .filter(|(name, _)| referenced_names.contains(self.string_table.resolve(**name)))
            .map(|(name, record)| {
                Ok(StableNamespaceBinding {
                    visible_name: self.string_table.resolve(*name).to_owned(),
                    record: StableFileVisibility::capture_namespace_record(
                        record,
                        &self.external_package_registry,
                        &self.string_table,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        namespace_records.sort_by(|left, right| left.visible_name.cmp(&right.visible_name));
        Ok(StableFileVisibility {
            source_names: capture_declarations(&visibility.visible_source_names),
            type_alias_names: capture_declarations(&visibility.visible_type_alias_names),
            trait_names: capture_declarations(&visibility.visible_trait_names),
            external_symbols: external_symbols.into_boxed_slice(),
            namespace_records: namespace_records.into_boxed_slice(),
            receiver_methods: receiver_methods.into_boxed_slice(),
        })
    }

    fn receiver_nominal_identity(
        &self,
        function_path: &PathId,
    ) -> Result<Option<CanonicalTypeIdentity>, CompilerError> {
        let Some(receiver) = self
            .resolved_function_signatures_by_path
            .get(function_path)
            .and_then(|resolved| resolved.receiver.as_ref())
        else {
            return Ok(None);
        };
        let receiver_path = match receiver {
            ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => path,
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                return Err(CompilerError::compiler_error(
                    "Retained receiver method has no source nominal identity",
                ));
            }
        };
        let type_id = self
            .nominal_type_ids_by_path
            .get(receiver_path)
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained receiver method has no enclosing nominal type handle",
                )
            })?;
        self.type_environment
            .canonical_identity_for_type_id(type_id)
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Retained receiver method has no enclosing nominal identity",
                )
            })
    }

    /// Freeze stable targets for every concrete executable visible to generated bodies.
    ///
    /// Public functions retain their interface identity. Concrete local helpers receive a
    /// distinct artefact-scoped identity and are projected as imported contracts when a generic
    /// body is materialised in an independent sidecar.
    fn install_concrete_executable_contracts(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<PathId, OriginFunctionId>,
        public_nominal_origins_by_path: &FxHashMap<PathId, OriginTypeId>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<Vec<(PathId, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.install_private_semantic_identities(module_origin, public_nominal_origins_by_path)?;
        self.install_nominal_blueprints(resources, path_fork)?;
        let generic_paths = self
            .generic_function_templates_by_path
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        for path in generic_paths {
            let expected_identity = if let Some(origin) = public_origins_by_path.get(&path) {
                GeneratedDeclarationIdentity::Public(origin.clone())
            } else if let Some(existing_identity) = self
                .generic_function_templates_by_path
                .get(&path)
                .and_then(|template| template.declaration_identity.clone())
            {
                existing_identity
            } else {
                let resolved = self
                    .resolved_function_signatures_by_path
                    .get(&path)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Local generic template has no resolved function signature",
                        )
                    })?;
                GeneratedDeclarationIdentity::ModulePrivate(self.private_executable_identity(
                    module_origin,
                    &path,
                    resolved,
                    ModulePrivateExecutableCategory::GenericFunction,
                )?)
            };

            let resolved = self
                .resolved_function_signatures_by_path
                .get(&path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generic template has no resolved function signature",
                    )
                })?;
            let existing_owner = self
                .generic_function_templates_by_path
                .get(&path)
                .and_then(|template| template.generic_parameter_owner.clone());
            let generic_parameter_owner = match resolved.receiver.as_ref() {
                Some(ReceiverKey::Struct(receiver_path) | ReceiverKey::Choice(receiver_path)) => {
                    public_nominal_origins_by_path
                        .get(receiver_path)
                        .cloned()
                        .map(GenericDeclarationOrigin::nominal_type)
                        .transpose()?
                        .or(existing_owner)
                }
                Some(ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_)) => {
                    if public_origins_by_path.contains_key(&path) {
                        return Err(CompilerError::compiler_error(
                            "Public generic receiver template has a non-source receiver owner",
                        ));
                    }
                    existing_owner
                }
                None => public_origins_by_path
                    .get(&path)
                    .map(|origin| GenericDeclarationOrigin::free_function(origin.clone()))
                    .transpose()?
                    .or(existing_owner),
            };

            let template = self
                .generic_function_templates_by_path
                .get_mut(&path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generic template disappeared while installing declaration identity",
                    )
                })?;
            if let Some(existing_identity) = template.declaration_identity.as_ref()
                && existing_identity != &expected_identity
            {
                return Err(CompilerError::compiler_error(
                    "Generic template declaration identity disagrees with its callable origin",
                ));
            }
            if matches!(&expected_identity, GeneratedDeclarationIdentity::Public(_))
                && generic_parameter_owner.is_none()
            {
                return Err(CompilerError::compiler_error(
                    "Public generic template has no explicit generic-parameter owner",
                ));
            }
            if let (Some(existing), Some(expected)) = (
                template.generic_parameter_owner.as_ref(),
                generic_parameter_owner.as_ref(),
            ) && existing != expected
            {
                return Err(CompilerError::compiler_error(
                    "Generic template generic-parameter owner disagrees with its callable origin",
                ));
            }
            template.declaration_identity = Some(expected_identity);
            template.generic_parameter_owner = generic_parameter_owner;
            let owner_for_identity = template.generic_parameter_owner.clone();
            let generic_parameter_list_id = template.generic_parameter_list_id;
            let receiver = resolved.receiver.clone();
            let is_public = matches!(
                template.declaration_identity,
                Some(GeneratedDeclarationIdentity::Public(_))
            );
            if is_public && let Some(owner) = owner_for_identity {
                self.register_exported_generic_parameter_identities(
                    generic_parameter_list_id,
                    &owner,
                    receiver.as_ref(),
                )?;
            }
        }

        let signatures = self
            .resolved_function_signatures_by_path
            .iter()
            .map(|(path, resolved)| (path.clone(), resolved.clone()))
            .collect::<Vec<_>>();
        let mut private_executables = Vec::new();

        for (path, resolved) in signatures {
            if self.imported_functions_by_local_path.contains_key(&path)
                || self.generic_function_templates_by_path.contains_key(&path)
            {
                continue;
            }

            let target = if let Some(origin) = public_origins_by_path.get(&path) {
                SourceFunctionTarget::Imported {
                    origin: origin.clone(),
                    local_path: path.clone(),
                }
            } else {
                let category = if resolved.receiver.is_some() {
                    ModulePrivateExecutableCategory::ReceiverMethod
                } else {
                    ModulePrivateExecutableCategory::FreeFunction
                };
                let identity =
                    self.private_executable_identity(module_origin, &path, &resolved, category)?;
                private_executables.push((path.clone(), identity.clone()));
                SourceFunctionTarget::ModulePrivate {
                    identity,
                    local_path: path.clone(),
                }
            };

            let fallible_carrier_type_id =
                fallible_carrier_for_signature(&resolved.signature, &mut self.type_environment);
            self.imported_functions_by_local_path.insert(
                path,
                AstImportedFunctionContract {
                    target,
                    summary: bootstrap_call_summary_from_signature(&resolved.signature),
                    fallible_carrier_type_id,
                },
            );
        }

        Ok(private_executables)
    }

    fn register_exported_generic_parameter_identities(
        &mut self,
        template_parameter_list_id: GenericParameterListId,
        owner: &GenericDeclarationOrigin,
        receiver: Option<&ReceiverKey>,
    ) -> Result<(), CompilerError> {
        let generic_parameter_list_id = if owner.nominal_type_origin().is_some() {
            let receiver_path = match receiver {
                Some(ReceiverKey::Struct(path) | ReceiverKey::Choice(path)) => path,
                Some(ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_)) | None => {
                    return Err(CompilerError::compiler_error(
                        "Nominal generic-parameter owner has no source receiver path",
                    ));
                }
            };
            let receiver_type_id = self
                .nominal_type_ids_by_path
                .get(receiver_path)
                .copied()
                .or_else(|| {
                    self.type_environment
                        .nominal_id_for_path(receiver_path)
                        .and_then(|nominal_id| {
                            self.type_environment.type_id_for_nominal_id(nominal_id)
                        })
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public generic receiver owner has no local nominal type handle",
                    )
                })?;
            match self.type_environment.get(receiver_type_id) {
                Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
                Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
                _ => None,
            }
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Public generic receiver owner has no generic parameter list",
                )
            })?
        } else {
            template_parameter_list_id
        };
        let parameters = self
            .type_environment
            .generic_parameters(generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Public generic template references a missing parameter list",
                )
            })?
            .parameters
            .iter()
            .enumerate()
            .map(|(position, parameter)| {
                (
                    position as u32,
                    parameter.id,
                    self.string_table.resolve(parameter.name).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        for (position, parameter_id, authored_name) in parameters {
            let type_id = self
                .type_environment
                .type_id_for_generic_parameter(parameter_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public generic template parameter has no local type handle",
                    )
                })?;
            self.type_environment.register_canonical_identity(
                CanonicalTypeIdentity::GenericParameter(ExportedGenericParameterIdentity::new(
                    owner.clone(),
                    position,
                    authored_name,
                )),
                type_id,
            )?;
        }
        Ok(())
    }

    fn install_private_semantic_identities(
        &mut self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_nominal_origins_by_path: &FxHashMap<PathId, OriginTypeId>,
    ) -> Result<(), CompilerError> {
        let nominal_types = self
            .nominal_type_ids_by_path
            .iter()
            .map(|(path, type_id)| (path.clone(), *type_id))
            .collect::<Vec<_>>();
        for (path, type_id) in nominal_types {
            if self
                .type_environment
                .canonical_identity_for_type_id(type_id)
                .is_some()
            {
                continue;
            }
            if let Some(origin) = public_nominal_origins_by_path.get(&path) {
                self.type_environment.register_canonical_identity(
                    CanonicalTypeIdentity::SourceNominal(origin.clone()),
                    type_id,
                )?;
                continue;
            }
            if !self.source_nominal_paths.contains(&path) {
                continue;
            }
            let category = match self.type_environment.get(type_id) {
                Some(TypeDefinition::Struct(_)) => OriginTypeCategory::Struct,
                Some(TypeDefinition::Choice(_)) => OriginTypeCategory::Choice,
                _ => continue,
            };
            let defining_path = {
                let identity = self.frozen_identity_handle.get().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "materialisation preparation has no frozen identity context",
                    )
                })?;
                let mut scratch = Vec::new();
                identity.paths().render_portable(
                    path,
                    &self.string_table,
                    &mut scratch,
                )
            };
            let identity = ModulePrivateNominalIdentity::new(
                module_origin.clone(),
                defining_path,
                category,
            );
            self.type_environment.register_canonical_identity(
                CanonicalTypeIdentity::ModulePrivateNominal(identity.clone()),
                type_id,
            )?;
        }

        let private_traits = self
            .public_trait_paths
            .iter()
            .map(|path| {
                let trait_id = self.trait_environment.id_for_path(path).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public materialisation trait path has no resolved trait definition",
                    )
                })?;
                let identity = self.frozen_identity_handle.get().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "materialisation preparation has no frozen identity context",
                    )
                })?;
                let defining_name = identity.paths().component(*path).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Public materialisation trait path has no defining name",
                    )
                })?;
                Ok((
                    trait_id,
                    self.string_table.resolve(defining_name).to_owned(),
                ))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        for (trait_id, defining_name) in private_traits {
            if self
                .trait_environment
                .canonical_identity_for_id(trait_id)
                .is_none()
            {
                self.trait_environment.register_canonical_identity(
                    CanonicalTraitIdentity::Source(OriginTraitId::new(
                        module_origin.clone(),
                        defining_name,
                    )),
                    trait_id,
                )?;
            }
        }

        let private_traits = self
            .trait_environment
            .definitions()
            .filter(|definition| {
                self.trait_environment
                    .canonical_identity_for_id(definition.id)
                    .is_none()
            })
            .map(|definition| {
                let defining_path = {
                    let identity = self.frozen_identity_handle.get().ok_or_else(|| {
                        CompilerError::compiler_error(
                            "materialisation preparation has no frozen identity context",
                        )
                    })?;
                    let mut scratch = Vec::new();
                    identity.paths().render_portable(
                        definition.canonical_path,
                        &self.string_table,
                        &mut scratch,
                    )
                };
                Ok((definition.id, defining_path))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        for (trait_id, defining_path) in private_traits {
            self.trait_environment.register_canonical_identity(
                CanonicalTraitIdentity::ModulePrivate(ModulePrivateTraitIdentity::new(
                    module_origin.clone(),
                    defining_path,
                )),
                trait_id,
            )?;
        }

        Ok(())
    }

    fn private_executable_identity(
        &self,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        path: &PathId,
        resolved: &ResolvedFunctionSignature,
        category: ModulePrivateExecutableCategory,
    ) -> Result<ModulePrivateExecutableIdentity, CompilerError> {
        let identity = self.frozen_identity_handle.get().ok_or_else(|| {
            CompilerError::compiler_error(
                "materialisation preparation has no frozen identity context",
            )
        })?;
        let paths = identity.paths();
        let receiver_path = match resolved.receiver.as_ref() {
            Some(ReceiverKey::Struct(receiver) | ReceiverKey::Choice(receiver)) => {
                let mut scratch = Vec::new();
                Some(paths.render_portable(
                    *receiver,
                    &self.string_table,
                    &mut scratch,
                ))
            }
            Some(ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_)) => {
                return Err(CompilerError::compiler_error(
                    "Module-private source receiver method has a non-source receiver",
                ));
            }
            None => None,
        };
        let name = paths
            .component(*path)
            .map(|name| self.string_table.resolve(name).to_owned())
            .ok_or_else(|| {
                CompilerError::compiler_error("Module-private executable path has no defining name")
            })?;
        let declaring_source = {
            let parent = paths.try_parent(*path).unwrap_or(PathId::ROOT);
            let mut scratch = Vec::new();
            paths.render_portable(parent, &self.string_table, &mut scratch)
        };

        Ok(ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            declaring_source,
            category,
            name,
            receiver_path,
        ))
    }

    fn from_environment(
        input: ModuleMaterialisationEnvironmentInput<'_>,
    ) -> Result<Self, CompilerError> {
        let ModuleMaterialisationEnvironmentInput {
            lookups,
            const_values,
            type_environment,
            public_trait_roots,
            default_const_templates_by_path,
            entry_dir,
            module_origin,
            stage0_resolution_facts,
            frozen_identity_handle,
            module_resources,
            string_table,
            template_const_loop_iteration_limit,
            capacity_estimate,
        } = input;

        Ok(Self {
            string_table: string_table.clone_preserving_inherited_prefix(),
            string_table_fork_source: OnceCell::new(),
            entry_dir,
            module_origin,
            module_resources,
            stage0_resolution_facts,
            frozen_identity_handle,
            type_environment: type_environment.fork_for_generated(),
            declaration_table: declaration_table_without_module_values(
                &lookups.declaration_table,
                const_values,
            )?,
            binding_environment: lookups.binding_environment.clone(),
            imported_functions_by_local_path: lookups.imported_functions_by_local_path.clone(),
            imported_struct_definitions: lookups.imported_struct_definitions.clone(),
            imported_choice_definitions: lookups.imported_choice_definitions.clone(),
            const_values: const_values.clone(),
            default_const_templates_by_path,
            builtin_struct_ast_nodes: lookups.builtin_struct_ast_nodes.clone(),
            resolved_struct_fields_by_path: (*lookups.resolved_struct_fields_by_path).clone(),
            resolved_function_signatures_by_path: (*lookups.resolved_function_signatures_by_path)
                .clone(),
            generic_function_templates_by_path: lookups.generic_function_templates_by_path.clone(),
            generic_template_paths_by_identity: FxHashMap::default(),
            resolved_type_aliases_by_path: (*lookups.resolved_type_aliases_by_path).clone(),
            choice_variant_shells_by_path: (*lookups.choice_variant_shells_by_path).clone(),
            declaration_semantics: (*lookups.declaration_semantics).clone(),
            generic_declarations_by_path: (*lookups.generic_declarations_by_path).clone(),
            nominal_type_ids_by_path: (*lookups.nominal_type_ids_by_path).clone(),
            source_nominal_paths: (*lookups.source_nominal_paths).clone(),
            public_trait_paths: public_trait_roots
                .iter()
                .map(|root| root.canonical_path.clone())
                .collect(),
            nominal_blueprints: FxHashMap::default(),
            receiver_methods: (*lookups.receiver_methods).clone(),
            trait_environment: (*lookups.trait_environment).clone(),
            trait_evidence_environment: (*lookups.trait_evidence_environment).clone(),
            external_package_registry: Arc::clone(&lookups.external_package_registry),
            style_directives: lookups.style_directives.clone(),
            build_profile: lookups.build_profile,
            template_const_loop_iteration_limit,
            capacity_estimate,
        })
    }
}
