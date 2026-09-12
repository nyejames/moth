//! Frozen artefact reconstruction and generated sidecar input emission.
use super::super::GenericFunctionTemplate;
use super::alias_projection;
use super::frozen_syntax::MaterialisedBody;
use super::nominal_blueprints::{
    intern_generated_canonical_type, intern_materialisation_type_blueprint,
    materialised_nominal_declaration, materialised_struct_fields,
};
use super::semantic_closure::{StableSemanticClosure, install_private_semantic_closure};
use super::sidecar_build::{
    GeneratedSidecarRequest, emit_materialised_sidecar, fallible_carrier_from_type_ids,
};
use super::stable_types::{
    GeneratedFoldedValueMaterialiser, GeneratedValueMaterialisationServices,
    GenericTemplateArtefact, MaterialisationNominalSource, StableFunctionSignature,
    StableFunctionTarget, generated_file_value_resolution_services,
};
use super::{MaterialisedGenericAst, ModuleMaterialisationInput};
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::AstBuildContext;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_public_folded_value;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironment, AstModuleEnvironmentBuilder, AstModuleLookups,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeIdentity, GenericDeclarationOrigin,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::{FunctionTypeKey, GenericParameterListId, TypeId};
use crate::compiler_frontend::datatypes::{DataType, ReceiverKey, diagnostic_type_spelling};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::{
    HeaderBindingEnvironment, ImportedFunctionContract, SourceFunctionTarget,
};
use crate::compiler_frontend::headers::module_symbols::{GenericDeclarationKind, ModuleSymbols};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::{PublicDeclarationRecord, PublicEvidenceRecord};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModuleRootRole,
    StableModuleOriginIdentity,
};
use crate::compiler_frontend::source::{FrozenIdentityContext, FrozenIdentityHandle};
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerFork, PathTable,
};
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
#[cfg(test)]
#[path = "artefact_emit/test_support.rs"]
mod test_support;
///
/// The context is deliberately a compact list rather than a donor-module snapshot. It owns one
/// module-wide stable semantic closure and one retained body per template. Modules without
/// retained bodies publish no context.
#[derive(Clone)]
pub(crate) struct ModuleMaterialisationContext {
    pub(super) declaration_closure: Box<[PublicDeclarationRecord]>,
    pub(super) evidence: Box<[PublicEvidenceRecord]>,
    pub(super) semantic_closure: StableSemanticClosure,
    pub(super) artefacts: Box<[GenericTemplateArtefact]>,
    pub(super) module_origin: Option<StableModuleOriginIdentity>,
    pub(super) frozen_identity_handle: FrozenIdentityHandle,
    /// The complete path table that issued this context's retained `PathId`s.
    ///
    /// A published provider can be consumed by a different project/package boundary. Keeping the
    /// issuing table here lets the requester re-intern those paths into its own live fork instead
    /// of assuming that equal numeric IDs name equal paths.
    pub(super) path_table: Option<Arc<PathTable>>,
    /// The string table whose IDs are stored in `path_table`.
    ///
    /// Path components use the provider boundary's `StringId` domain. Retaining that resolver with
    /// the path table prevents a requester boundary from interpreting the same numeric IDs as its
    /// own strings while rebasing a published context.
    pub(super) source_string_table: Option<Arc<FrozenStringTable>>,
}
impl ModuleMaterialisationContext {
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.semantic_closure.remap_path_ids(remap);
        for artefact in &mut self.artefacts {
            artefact.remap_path_ids(remap);
        }
    }

    pub(crate) fn frozen_identity_handle(&self) -> FrozenIdentityHandle {
        self.frozen_identity_handle.clone()
    }

    /// Iterate every published template with its exact dense row index.
    ///
    /// WHAT: lets the boundary publication index point at one exact template row instead of
    ///       re-searching all artefacts at materialisation time.
    /// WHY: generated materialisation is request-driven; a direct row index keeps lookup
    ///      proportional to one identity and detects duplicates inside one context.
    pub(crate) fn declaration_rows(
        &self,
    ) -> impl Iterator<Item = (&GeneratedDeclarationIdentity, usize)> + '_ {
        self.artefacts
            .iter()
            .enumerate()
            .map(|(index, artefact)| (&artefact.declaration_identity, index))
    }

    /// Materialise the exact template row selected by the boundary publication index.
    pub(crate) fn materialise_ast_at<'a>(
        &self,
        template_index: usize,
        input: ModuleMaterialisationInput<'a>,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let artefact = self.artefacts.get(template_index).ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "Published materialisation context has no template row {template_index}"
                )),
                &input.requester_context.string_table,
            )
        })?;
        check_materialisation_row_identity(artefact, input.identity).map_err(|error| {
            CompilerMessages::from_error_ref(error, &input.requester_context.string_table)
        })?;
        artefact.materialise_ast(self, input)
    }
    pub(crate) fn install_frozen_identity(
        &self,
        identity: Arc<FrozenIdentityContext>,
    ) -> Result<(), CompilerError> {
        self.frozen_identity_handle.install(Arc::clone(&identity))?;
        for artefact in &self.artefacts {
            artefact
                .body
                .frozen_identity_handle
                .install(Arc::clone(&identity))?;
        }
        Ok(())
    }

    /// Attach the complete identity tables that issued this context's retained identities.
    pub(crate) fn install_identity_tables(
        &mut self,
        path_table: Arc<PathTable>,
        source_string_table: Arc<FrozenStringTable>,
    ) {
        self.path_table = Some(path_table);
        self.source_string_table = Some(source_string_table);
    }

    /// Rebase a published provider context into the requester's path/string identity domain.
    ///
    /// Provider contexts are published after their local path delta merges, but a source package
    /// (and a later project module) may own a different numeric path domain. Re-interning the
    /// issuing table through the requester's fork preserves path structure without introducing a
    /// second interner, then the cloned retained context can be used for this request only.
    pub(crate) fn rebased_for_requester(
        &self,
        path_fork: &mut PathInternerFork,
        destination_strings: &mut StringTable,
    ) -> Result<Self, CompilerError> {
        let Some(path_table) = self.path_table.as_deref() else {
            return Ok(self.clone());
        };
        let source_strings = self.source_string_table.as_deref().ok_or_else(|| {
            CompilerError::compiler_error(
                "published materialisation context has no source string table",
            )
        })?;
        let path_remap = path_fork
            .remap_table_from_strings(path_table, source_strings, destination_strings)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "published materialisation context path table could not be re-interned",
                )
            })?;
        let mut rebased = self.clone();
        rebased.remap_path_ids(&path_remap);
        rebased.path_table = Some(Arc::new(path_fork.snapshot_table()));
        Ok(rebased)
    }
}

/// Verify that one indexed template row belongs to the requested generated identity.
///
/// WHAT: a stale but in-range row must fail as an internal invariant error instead of
///       materialising the wrong generic declaration.
/// WHY: the boundary publication index is exact by contract; identity disagreement means the
///      index or the retained artefact lane is corrupt.
pub(super) fn check_materialisation_row_identity(
    artefact: &GenericTemplateArtefact,
    identity: &GeneratedFunctionIdentity,
) -> Result<(), CompilerError> {
    if artefact.declaration_identity != *identity.declaration() {
        return Err(CompilerError::compiler_error(format!(
            "Published materialisation row holds declaration identity {:?} but request {:?} selected it",
            artefact.declaration_identity,
            identity.declaration()
        )));
    }
    Ok(())
}

impl GenericTemplateArtefact {
    fn materialise_ast<'a>(
        &self,
        context: &ModuleMaterialisationContext,
        input: ModuleMaterialisationInput<'a>,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let ModuleMaterialisationInput {
            identity,
            requester_context,
            requester_call_span,
            path_fork,
            external_package_registry,
            style_directives,
            build_profile,
            template_const_loop_iteration_limit,
            #[cfg(feature = "timers")]
            timing_context,
        } = input;
        let (mut string_table, requester_string_remap) =
            requester_context.fork_materialisation_string_table();

        let source_file = self.source_file;
        let function_path = self.function_path;
        let entry_dir = path_fork.parent(source_file).unwrap_or(PathId::ROOT);
        let materialised_body = self
            .body
            .materialise(source_file, path_fork, &mut string_table)
            .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
        let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
        let file_value_resolution = generated_file_value_resolution_services(
            Rc::clone(&module_resources),
            context.module_origin.clone(),
            Arc::clone(&materialised_body.resolution_facts),
        );
        let build_context = AstBuildContext {
            external_package_registry: Arc::new(external_package_registry.clone()),
            style_directives,
            string_table: &mut string_table,
            path_fork,
            entry_dir,
            root_role: ModuleRootRole::Support,
            build_profile,
            file_value_resolution: Some(file_value_resolution),
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit,
            capacity_estimate: FrontendArenaCapacityEstimate::default(),
            #[cfg(feature = "timers")]
            timing_context,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Generated,
        };
        let (phase_context, string_table_ref, path_fork_ref) =
            AstPhaseContext::from_build_context(build_context, Arc::new(Default::default()));
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_total,
            crate::timing::TimingMetric::FrontendGeneratedAstTotal,
            timing_context
        );
        let binding_environment = self
            .materialise_binding_environment(
                context,
                &source_file,
                external_package_registry,
                path_fork_ref,
                string_table_ref,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;
        let builtin_manifest =
            crate::compiler_frontend::builtins::error_type::register_builtin_error_types(
                path_fork_ref,
                string_table_ref,
            );
        let mut module_symbols = ModuleSymbols::empty();
        module_symbols
            .builtin_visible_symbol_paths
            .extend(builtin_manifest.visible_symbol_paths.iter().cloned());
        module_symbols.compiler_owned_declarations = builtin_manifest.declarations;
        module_symbols
            .resolved_struct_fields_by_path
            .extend(builtin_manifest.resolved_struct_fields_by_path);
        module_symbols
            .struct_source_by_path
            .extend(builtin_manifest.struct_source_by_path);
        let mut environment = AstModuleEnvironmentBuilder::new(&phase_context, path_fork_ref).build(
            &[],
            AstEnvironmentInput {
                module_symbols,
                binding_environment,
            },
            string_table_ref,
        )?;
        let value_services = GeneratedValueMaterialisationServices {
            external_registry: external_package_registry,
            template_ir_store: &phase_context.template_ir_store,
            module_resources: Rc::clone(&module_resources),
        };
        self.install_closed_environment(
            context,
            &mut environment,
            &value_services,
            string_table_ref,
            path_fork_ref,
            Some(materialised_body),
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;

        let (build_result, instance_path) = emit_materialised_sidecar(
            &phase_context,
            environment,
            GeneratedSidecarRequest {
                identity,
                function_path,
                requester_context,
                requester_string_remap: &requester_string_remap,
                requester_call_span,
            },
            self,
            path_fork_ref,
            string_table_ref,
        )?;
        Ok(MaterialisedGenericAst {
            build_result,
            string_table,
            instance_path,
        })
    }

    fn materialise_binding_environment(
        &self,
        context: &ModuleMaterialisationContext,
        source_file: &PathId,
        external_package_registry: &ExternalPackageRegistry,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
        string_table: &mut StringTable,
    ) -> Result<HeaderBindingEnvironment, CompilerError> {
        let mut environment = HeaderBindingEnvironment::default();
        environment.file_visibility_by_source.insert(
            source_file.clone(),
            Arc::new(
                self.visibility
                    .materialise(external_package_registry, path_fork, string_table)?,
            ),
        );
        for binding in &self.declarations {
            let local_path = binding.local_path;
            environment
                .imported_declarations_by_local_path
                .insert(local_path, binding.origin.clone());
        }
        for record in &context.declaration_closure {
            environment
                .imported_declarations_by_origin
                .insert(record.origin.clone(), record.clone());
        }
        for record in &context.evidence {
            environment
                .imported_evidence_by_identity
                .insert(record.identity.clone(), record.clone());
        }
        for callable in &self.callables {
            let local_path = callable.local_path;
            let target = callable.target.materialise(local_path.clone());
            let SourceFunctionTarget::Imported { origin, .. } = &target else {
                // Generated and module-private callables materialise through the generated
                // function lanes; only imported provider callables enter the header contract
                // tables.
                continue;
            };
            environment
                .imported_call_summaries_by_origin
                .insert(origin.clone(), callable.summary.clone());
            environment
                .imported_functions_by_local_path
                .insert(local_path, ImportedFunctionContract { target });
        }
        Ok(environment)
    }
    fn install_closed_environment(
        &self,
        context: &ModuleMaterialisationContext,
        environment: &mut AstModuleEnvironment,
        services: &GeneratedValueMaterialisationServices<'_>,
        string_table: &mut StringTable,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
        mut materialised_self_body: Option<MaterialisedBody>,
    ) -> Result<(), CompilerError> {
        let external_package_registry = services.external_registry;
        let template_ir_store = services.template_ir_store;
        let module_resources = Rc::clone(&services.module_resources);
        let value_services = services;
        for nominal in &self.nominals {
            let type_id = intern_generated_canonical_type(
                &nominal.identity,
                &mut environment.type_environment,
                external_package_registry,
                self,
                string_table,
                path_fork,
            )?;
            let local_path = nominal.local_path;
            environment
                .type_environment
                .register_nominal_path_alias(local_path.clone(), type_id)?;
            let generated_nominal_path = environment
                .type_environment
                .nominal_path(type_id)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Materialised nominal binding has no generated environment path",
                    )
                })?;
            let generic_kind = if environment
                .type_environment
                .generic_parameter_list_id_for_type(type_id)
                .is_some()
            {
                Some(match environment.type_environment.get(type_id) {
                    Some(TypeDefinition::Struct(_)) => GenericDeclarationKind::Struct,
                    Some(TypeDefinition::Choice(_)) => GenericDeclarationKind::Choice,
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "Materialised generic nominal has no struct or choice definition",
                        ));
                    }
                })
            } else {
                None
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            Rc::make_mut(&mut lookups.nominal_type_ids_by_path).insert(local_path.clone(), type_id);
            Rc::make_mut(&mut lookups.source_nominal_paths).insert(local_path.clone());
            if let Some(kind) = generic_kind {
                let declarations = Rc::make_mut(&mut lookups.generic_declarations_by_path);
                declarations
                    .entry(local_path.clone())
                    .or_insert_with(|| kind.clone());
                declarations
                    .entry(generated_nominal_path.clone())
                    .or_insert(kind);
            }
            if !lookups
                .resolved_struct_fields_by_path
                .contains_key(&generated_nominal_path)
                && let Some(blueprint) = self.nominal_blueprints.get(&nominal.identity)
                && let Some(fields) = materialised_struct_fields(
                    type_id,
                    &mut environment.type_environment,
                    blueprint,
                    self,
                    value_services,
                    string_table,
                    path_fork,
                )?
            {
                Rc::make_mut(&mut lookups.resolved_struct_fields_by_path)
                    .insert(generated_nominal_path, fields);
            }
            if lookups.declaration_table.get_by_path(&local_path).is_none() {
                append_materialised_declaration(
                    lookups,
                    materialised_nominal_declaration(
                        local_path.clone(),
                        type_id,
                        &environment.type_environment,
                    )?,
                    path_fork,
                )?;
            }
        }

        // Private traits are part of the declaration environment needed to resolve bounds on
        // nested retained templates. Install them after nominal shells exist, but before any
        // nested signature or bound reconstruction consumes the generated trait table.
        install_private_semantic_closure(
            self,
            context,
            environment,
            external_package_registry,
            template_ir_store,
            string_table,
            path_fork,
        )?;

        for callable in &self.callables {
            if self
                .declarations
                .iter()
                .any(|declaration| declaration.local_path == callable.local_path)
            {
                continue;
            }
            let local_path = callable.local_path;
            let (signature, function_type_id, fallible_carrier_type_id) =
                callable.signature.materialise(
                    &local_path,
                    &mut StableFunctionMaterialisationContext {
                        generic_parameter_type_ids: &[],
                        nominal_source: self,
                        path_fork,
                        type_environment: &mut environment.type_environment,
                        external_package_registry,
                        template_ir_store,
                        module_resources: Rc::clone(&module_resources),
                        string_table,
                    },
                )?;
            let declaration = Declaration {
                id: local_path.clone(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    Default::default(),
                    function_type_id,
                    DataType::Function(Box::new(None), signature.clone()),
                    ValueMode::ImmutableReference,
                ),
                binding_span: None,
                config_qualifier: None,
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            append_materialised_declaration(lookups, declaration, path_fork)?;
            Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
                local_path.clone(),
                ResolvedFunctionSignature {
                    receiver: None,
                    signature,
                },
            );
            Rc::make_mut(&mut lookups.declaration_semantics)
                .register_materialised_function(local_path.clone());
            lookups.imported_functions_by_local_path.insert(
                local_path.clone(),
                AstImportedFunctionContract {
                    target: callable.target.materialise(local_path),
                    summary: callable.summary.clone(),
                    fallible_carrier_type_id,
                },
            );
        }

        for local_path_components in &self.local_declarations {
            let local_path = *local_path_components;
            if let Some(constant) = context
                .semantic_closure
                .constants
                .iter()
                .find(|constant| constant.local_path == *local_path_components)
            {
                let type_id = intern_generated_canonical_type(
                    &constant.type_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                    path_fork,
                )?;
                let mut materialiser = GeneratedFoldedValueMaterialiser {
                    type_environment: &mut environment.type_environment,
                    external_registry: external_package_registry,
                    nominal_source: self,
                    template_ir_store: Rc::clone(template_ir_store),
                    module_resources: Rc::clone(&module_resources),
                    path_fork,
                };
                let span = constant.span;
                let mut value = materialize_public_folded_value(
                    &mut materialiser,
                    &constant.value,
                    type_id,
                    string_table,
                    span,
                )?;
                value.value_mode = ValueMode::ImmutableReference;
                let declaration = Declaration {
                    id: local_path.clone(),
                    value,
                    binding_span: None,
                    config_qualifier: None,
                };
                let lookups = Rc::make_mut(&mut environment.lookups);
                let declaration_id = match lookups
                    .declaration_table
                    .declaration_id_by_path(&local_path)
                {
                    Some(declaration_id) => declaration_id,
                    None => append_materialised_declaration(
                        lookups,
                        declaration.clone(),
                        path_fork,
                    )?,
                };
                Rc::make_mut(&mut lookups.resolved_module_constants).insert(declaration_id);
                Rc::make_mut(&mut lookups.declaration_semantics)
                    .register_materialised_constant(local_path);
                continue;
            }
            if alias_projection::restore_generated_local_alias(
                self,
                context,
                environment,
                local_path,
                *local_path_components,
                external_package_registry,
                path_fork,
                string_table,
            )? {
                continue;
            }
        }

        for method in &self.visibility.receiver_methods {
            let method_path = method.local_path;
            if context
                .artefacts
                .iter()
                .any(|nested| nested.function_path == method_path)
            {
                continue;
            }
            if !method.generic_parameters.is_empty() {
                // Imported generic receiver methods are reprojected from their imported nominal
                // declaration. A locally-owned generic receiver method must have a retained
                // artefact, so leaving this path unresolved would hide a broken closure.
                if matches!(method.target, StableFunctionTarget::Imported(_)) {
                    continue;
                }
                return Err(CompilerError::compiler_error(
                    "Generic receiver method has no retained materialisation artefact",
                ));
            }
            let (signature, function_type_id, fallible_carrier_type_id) =
                method.signature.materialise(
                    &method_path,
                    &mut StableFunctionMaterialisationContext {
                        generic_parameter_type_ids: &[],
                        nominal_source: self,
                        type_environment: &mut environment.type_environment,
                        path_fork,
                        external_package_registry,
                        template_ir_store,
                        module_resources: Rc::clone(&module_resources),
                        string_table,
                    },
                )?;
            let receiver = materialised_receiver_key(&signature, &environment.type_environment)
                .or_else(|_| {
                    Ok::<ReceiverKey, CompilerError>(method.receiver.materialise(string_table))
                })?;
            let declaration = Declaration {
                id: method_path.clone(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    Default::default(),
                    function_type_id,
                    DataType::Function(Box::new(Some(receiver.clone())), signature.clone()),
                    ValueMode::ImmutableReference,
                ),
                binding_span: None,
                config_qualifier: None,
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            if lookups
                .declaration_table
                .get_by_path(&method_path)
                .is_none()
            {
                append_materialised_declaration(lookups, declaration, path_fork)?;
            }
            Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
                method_path.clone(),
                ResolvedFunctionSignature {
                    receiver: Some(receiver.clone()),
                    signature: signature.clone(),
                },
            );
            register_materialised_receiver_method(
                lookups,
                method_path,
                receiver,
                signature.clone(),
                path_fork,
            )?;
            Rc::make_mut(&mut lookups.declaration_semantics)
                .register_materialised_function(method_path.clone());
            lookups.imported_functions_by_local_path.insert(
                method_path.clone(),
                AstImportedFunctionContract {
                    target: method.target.materialise(method_path),
                    summary: method.summary.clone().ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Concrete receiver method has no retained call summary",
                        )
                    })?,
                    fallible_carrier_type_id,
                },
            );
        }

        let selected_paths = self.visibility.materialised_selected_paths();
        for nested in &context.artefacts {
            let nested_path = nested.function_path;
            if nested.declaration_identity != self.declaration_identity
                && !selected_paths.contains(&nested_path)
            {
                continue;
            }
            let (generic_parameter_list_id, generic_parameter_type_ids) = if let Some(
                nominal_origin,
            ) = nested
                .generic_parameter_owner
                .as_ref()
                .and_then(GenericDeclarationOrigin::nominal_type_origin)
            {
                // A generic receiver method shares the enclosing nominal's local generic
                // handles. Registering a second list for the method would assign one stable
                // exported parameter identity to two TypeIds and make its generated sidecar
                // internally inconsistent.
                let nominal_identity = CanonicalTypeIdentity::SourceNominal(nominal_origin.clone());
                let nominal_type_id = intern_generated_canonical_type(
                    &nominal_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                    path_fork,
                )?;
                let generic_parameter_list_id =
                    match environment.type_environment.get(nominal_type_id) {
                        Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
                        Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
                        _ => {
                            return Err(CompilerError::compiler_error(
                                "Generic receiver template owner is not a nominal type",
                            ));
                        }
                    }
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generic receiver template owner has no generic parameter list",
                        )
                    })?;
                let generic_parameter_type_ids = environment
                    .type_environment
                    .generic_parameters(generic_parameter_list_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generic receiver template owner has a missing parameter list",
                        )
                    })?
                    .parameters
                    .iter()
                    .map(|parameter| {
                        environment
                            .type_environment
                            .type_id_for_generic_parameter(parameter.id)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Generic receiver template owner parameter has no type handle",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                if generic_parameter_type_ids.len() != nested.generic_parameters.len() {
                    return Err(CompilerError::compiler_error(
                        "Generic receiver template parameter count disagrees with its nominal owner",
                    ));
                }
                for (parameter, type_id) in nested
                    .generic_parameters
                    .iter()
                    .zip(&generic_parameter_type_ids)
                {
                    if let Some(exported_identity) = parameter.exported_identity.as_ref() {
                        let expected_identity =
                            CanonicalTypeIdentity::GenericParameter(exported_identity.clone());
                        environment
                            .type_environment
                            .register_canonical_identity(expected_identity, *type_id)?;
                    }
                }
                (generic_parameter_list_id, generic_parameter_type_ids)
            } else if nested.receiver.is_some() {
                let receiver_identity = nested.receiver_nominal_identity.as_ref().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Materialised generic receiver template {:?} has no retained nominal identity",
                        nested_path
                    ))
                })?;
                let receiver_type_id = intern_generated_canonical_type(
                    receiver_identity,
                    &mut environment.type_environment,
                    external_package_registry,
                    self,
                    string_table,
                    path_fork,
                )?;
                let generic_parameter_list_id =
                    match environment.type_environment.get(receiver_type_id) {
                        Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
                        Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
                        _ => {
                            return Err(CompilerError::compiler_error(
                                "Private generic receiver owner is not a nominal type",
                            ));
                        }
                    }
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Private generic receiver owner has no generic parameter list",
                        )
                    })?;
                let generic_parameter_type_ids = environment
                    .type_environment
                    .generic_parameters(generic_parameter_list_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Private generic receiver owner has a missing parameter list",
                        )
                    })?
                    .parameters
                    .iter()
                    .map(|parameter| {
                        environment
                            .type_environment
                            .type_id_for_generic_parameter(parameter.id)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Private generic receiver owner parameter has no type handle",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                if generic_parameter_type_ids.len() != nested.generic_parameters.len() {
                    return Err(CompilerError::compiler_error(
                        "Private generic receiver template parameter count disagrees with its nominal owner",
                    ));
                }
                self.restore_private_receiver_generic_parameter_bounds(
                    nested,
                    receiver_identity,
                    generic_parameter_list_id,
                    &mut environment.type_environment,
                    &environment.lookups.trait_environment,
                    string_table,
                )?;
                (generic_parameter_list_id, generic_parameter_type_ids)
            } else {
                let registration =
                    environment
                        .type_environment
                        .register_generic_parameter_list(
                            nested.generic_parameters.iter().enumerate().map(
                                |(slot, parameter)| {
                                    (
                                        TypeParameterId(slot as u32),
                                        string_table.intern(&parameter.name),
                                    )
                                },
                            ),
                            &FxHashMap::default(),
                        );
                let generic_parameter_type_ids = (0..nested.generic_parameters.len())
                    .map(|slot| {
                        let parameter_id = registration
                            .canonical_by_local
                            .get(&TypeParameterId(slot as u32))
                            .copied()
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Materialised generic template omitted a parameter slot",
                                )
                            })?;
                        environment
                            .type_environment
                            .type_id_for_generic_parameter(parameter_id)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "Materialised generic template parameter has no type handle",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                let mut resolved_bounds_by_local = FxHashMap::default();
                for (slot, parameter) in nested.generic_parameters.iter().enumerate() {
                    let local_id = TypeParameterId(slot as u32);
                    let parameter_id = registration
                        .canonical_by_local
                        .get(&local_id)
                        .copied()
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialised generic template omitted a stable parameter slot",
                            )
                        })?;
                    let parameter_type_id = environment
                        .type_environment
                        .type_id_for_generic_parameter(parameter_id)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Materialised generic template parameter has no type identity",
                            )
                        })?;
                    if let Some(exported_identity) = &parameter.exported_identity {
                        environment.type_environment.register_canonical_identity(
                            CanonicalTypeIdentity::GenericParameter(exported_identity.clone()),
                            parameter_type_id,
                        )?;
                    }
                    let bounds = parameter
                            .bounds
                            .iter()
                            .map(|identity| {
                                environment
                                    .lookups
                                    .trait_environment
                                    .id_for_canonical_identity(identity)
                                    .ok_or_else(|| {
                                        CompilerError::compiler_error(
                                            "Materialised generic bound is absent from its trait closure",
                                        )
                                    })
                            })
                            .collect::<Result<Vec<_>, CompilerError>>()?;
                    resolved_bounds_by_local.insert(local_id, bounds);
                }
                environment
                    .type_environment
                    .update_generic_parameter_bounds(
                        registration.list_id,
                        &resolved_bounds_by_local,
                        &registration.canonical_by_local,
                    );
                (registration.list_id, generic_parameter_type_ids)
            };
            let (signature, function_type_id, _) = nested.signature.materialise(
                &nested_path,
                &mut StableFunctionMaterialisationContext {
                    generic_parameter_type_ids: &generic_parameter_type_ids,
                    nominal_source: nested,
                    type_environment: &mut environment.type_environment,
                    path_fork,
                    external_package_registry,
                    template_ir_store,
                    module_resources: Rc::clone(&module_resources),
                    string_table,
                },
            )?;
            let receiver = if nested.receiver.is_some() {
                Some(materialised_receiver_key(
                    &signature,
                    &environment.type_environment,
                )?)
            } else {
                None
            };
            let source_file = nested.source_file;
            let body = if nested.declaration_identity == self.declaration_identity {
                materialised_self_body.take().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "generated materialisation consumed its root generic body more than once",
                    )
                })?
                .into_generic_body()
            } else {
                nested
                    .body
                    .materialise(source_file, path_fork, string_table)?
                    .into_generic_body()
            };
            let template = GenericFunctionTemplate {
                function_path: nested_path.clone(),
                source_file,
                declaration_identity: Some(nested.declaration_identity.clone()),
                generic_parameter_owner: nested.generic_parameter_owner.clone(),
                generic_parameter_list_id,
                signature: signature.clone(),
                body_tokens: Some(body),
                declaration_span: nested.declaration_span,
            };
            let lookups = Rc::make_mut(&mut environment.lookups);
            lookups
                .generic_function_templates_by_path
                .insert(nested_path.clone(), template);
            Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
                nested_path.clone(),
                ResolvedFunctionSignature {
                    receiver: receiver.clone(),
                    signature: signature.clone(),
                },
            );
            if let Some(receiver) = receiver.clone() {
                register_materialised_receiver_method(
                    lookups,
                    nested_path,
                    receiver,
                    signature.clone(),
                    path_fork,
                )?;
            }
            if lookups
                .declaration_table
                .get_by_path(&nested_path)
                .is_none()
            {
                append_materialised_declaration(
                    lookups,
                    Declaration {
                        id: nested_path.clone(),
                        value: Expression::new(
                            ExpressionKind::NoValue,
                            Default::default(),
                            function_type_id,
                            DataType::Function(Box::new(receiver), signature),
                            ValueMode::ImmutableReference,
                        ),
                        binding_span: None,
                        config_qualifier: None,
                    },
                    path_fork,
                )?;
            }
            Rc::make_mut(&mut lookups.declaration_semantics)
                .register_materialised_function(nested_path);
        }
        if materialised_self_body.is_some() {
            return Err(CompilerError::compiler_error(
                "generated materialisation did not install its root generic body",
            ));
        }
        Ok(())
    }
}

fn materialised_receiver_key(
    signature: &FunctionSignature,
    type_environment: &TypeEnvironment,
) -> Result<ReceiverKey, CompilerError> {
    let receiver_type_id = signature
        .parameters
        .first()
        .map(|parameter| parameter.value.type_id)
        .ok_or_else(|| {
            CompilerError::compiler_error("Receiver method has no receiver parameter")
        })?;
    type_environment
        .receiver_key_for_type_id(receiver_type_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Receiver method parameter does not resolve to a nominal receiver",
            )
        })
}

fn register_materialised_receiver_method(
    lookups: &mut AstModuleLookups,
    function_path: PathId,
    receiver: ReceiverKey,
    signature: FunctionSignature,
    path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
) -> Result<(), CompilerError> {
    let method_name = path_fork.component(function_path).ok_or_else(|| {
        CompilerError::compiler_error(
            "Materialised receiver method path has no final method-name component",
        )
    })?;
    let entry = ReceiverMethodEntry {
        function_path,
        receiver: receiver.clone(),
        source_file: path_fork.parent(function_path).unwrap_or(PathId::ROOT),
        receiver_mutable: signature
            .parameters
            .first()
            .is_some_and(|parameter| parameter.value.value_mode.is_mutable()),
        signature,
    };
    let receiver_methods = Rc::make_mut(&mut lookups.receiver_methods);
    receiver_methods
        .by_receiver_and_name
        .entry((receiver, method_name))
        .or_default()
        .push(entry.clone());
    receiver_methods
        .by_method_name
        .entry(method_name)
        .or_default()
        .push(entry.clone());
    if receiver_methods
        .by_function_path
        .insert(function_path, entry)
        .is_some()
    {
        return Err(CompilerError::compiler_error(
            "Materialised receiver method path was registered more than once",
        ));
    }
    Ok(())
}

pub(super) fn append_materialised_declaration(
    lookups: &mut AstModuleLookups,
    declaration: Declaration,
    path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
) -> Result<crate::compiler_frontend::ast::module_ast::environment::DeclarationId, CompilerError> {
    let path = declaration.id;
    Rc::make_mut(&mut lookups.declaration_table)
        .append_for_construction(declaration, path_fork)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Materialised declaration path {path:?} was registered more than once",
            ))
        })
}

impl GenericTemplateArtefact {
    /// Restore the declaration-site bounds shared by a private receiver and its nominal owner.
    ///
    /// WHAT: validates ordered names and canonical bound identities, then patches the generated
    ///       nominal parameter handles with consumer-local `TraitId` values.
    /// WHY: nominal reconstruction creates reusable local handles before the generated trait
    ///       table is available. A private receiver template must not inherit an empty bound
    ///       list, because bound dispatch selects both its method and generated evidence identity.
    fn restore_private_receiver_generic_parameter_bounds(
        &self,
        nested: &GenericTemplateArtefact,
        receiver_identity: &CanonicalTypeIdentity,
        generic_parameter_list_id: GenericParameterListId,
        type_environment: &mut TypeEnvironment,
        trait_environment: &TraitEnvironment,
        string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        let nominal_blueprint =
            self.nominal_blueprints
                .get(receiver_identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Private generic receiver owner has no retained nominal blueprint",
                    )
                })?;
        let nominal_parameters = &nominal_blueprint.generic_parameters;
        let local_parameters = type_environment
            .generic_parameters(generic_parameter_list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Private generic receiver owner has no reconstructed parameter list",
                )
            })?
            .parameters
            .clone();
        if nominal_parameters.len() != nested.generic_parameters.len()
            || nominal_parameters.len() != local_parameters.len()
        {
            return Err(CompilerError::compiler_error(
                "Private generic receiver owner and method parameter lists disagree in arity",
            ));
        }

        let mut bounds_by_local = FxHashMap::default();
        let mut canonical_by_local = FxHashMap::default();
        for (slot, ((nominal_parameter, receiver_parameter), local_parameter)) in nominal_parameters
            .iter()
            .zip(&nested.generic_parameters)
            .zip(&local_parameters)
            .enumerate()
        {
            if nominal_parameter.name != receiver_parameter.name
                || string_table.resolve(local_parameter.name) != nominal_parameter.name
                || nominal_parameter.bounds.as_ref() != receiver_parameter.bounds.as_ref()
            {
                return Err(CompilerError::compiler_error(
                    "Private generic receiver owner and method parameter bounds disagree",
                ));
            }

            let bounds = receiver_parameter
                .bounds
                .iter()
                .map(|identity| {
                    trait_environment
                        .id_for_canonical_identity(identity)
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Private generic receiver bound is absent from the generated trait table",
                            )
                        })
                })
                .collect::<Result<Vec<_>, CompilerError>>()?;
            let local_id = TypeParameterId(slot as u32);
            canonical_by_local.insert(local_id, local_parameter.id);
            bounds_by_local.insert(local_id, bounds);
        }

        type_environment.update_generic_parameter_bounds(
            generic_parameter_list_id,
            &bounds_by_local,
            &canonical_by_local,
        );
        Ok(())
    }
}
struct StableFunctionMaterialisationContext<'a, N: MaterialisationNominalSource> {
    generic_parameter_type_ids: &'a [TypeId],
    nominal_source: &'a N,
    type_environment: &'a mut TypeEnvironment,
    external_package_registry: &'a ExternalPackageRegistry,
    template_ir_store:
        &'a Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    module_resources: Rc<RefCell<ModuleResourceTable>>,
    path_fork: &'a mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    string_table: &'a mut StringTable,
}

impl StableFunctionSignature {
    fn materialise<N: MaterialisationNominalSource>(
        &self,
        function_path: &PathId,
        context: &mut StableFunctionMaterialisationContext<'_, N>,
    ) -> Result<(FunctionSignature, TypeId, Option<TypeId>), CompilerError> {
        let mut parameters = Vec::with_capacity(self.parameters.len());
        let mut parameter_type_ids = Vec::with_capacity(self.parameters.len());
        for parameter in &self.parameters {
            let type_id = intern_materialisation_type_blueprint(
                &parameter.parameter_type,
                context.generic_parameter_type_ids,
                context.nominal_source,
                context.type_environment,
                context.external_package_registry,
                context.string_table,
                context.path_fork,
            )?;
            let name = context.string_table.intern(&parameter.name);
            let parameter_path = context
                .path_fork
                .try_intern_child(*function_path, name)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "path table exhausted while materialising a function parameter",
                    )
                })?;
            let parameter_span = parameter.span;
            let mut value = if let Some(default) = parameter.folded_default.as_ref() {
                let mut materialiser = GeneratedFoldedValueMaterialiser {
                    type_environment: &mut *context.type_environment,
                    external_registry: context.external_package_registry,
                    nominal_source: context.nominal_source,
                    template_ir_store: Rc::clone(context.template_ir_store),
                    module_resources: Rc::clone(&context.module_resources),
                    path_fork: context.path_fork,
                };
                materialize_public_folded_value(
                    &mut materialiser,
                    default,
                    type_id,
                    context.string_table,
                    parameter_span,
                )?
            } else {
                Expression::new(
                    ExpressionKind::NoValue,
                    parameter_span,
                    type_id,
                    diagnostic_type_spelling(type_id, context.type_environment),
                    parameter.value_mode.clone(),
                )
            };
            value.value_mode = parameter.value_mode.clone();
            if parameter.reactive {
                value.reactive_source = Some(ReactiveSource {
                    path: parameter_path.clone(),
                    kind: ReactiveSourceKind::Parameter,
                });
            }
            parameters.push(Declaration {
                id: parameter_path,
                value,
                binding_span: None,
                config_qualifier: None,
            });
            parameter_type_ids.push(type_id);
        }
        let mut returns = Vec::with_capacity(self.returns.len());
        let mut success_type_ids = Vec::new();
        let mut error_return = None;
        for returned in &self.returns {
            let type_id = intern_materialisation_type_blueprint(
                &returned.return_type,
                context.generic_parameter_type_ids,
                context.nominal_source,
                context.type_environment,
                context.external_package_registry,
                context.string_table,
                context.path_fork,
            )?;
            let diagnostic_type = diagnostic_type_spelling(type_id, context.type_environment);
            returns.push(ReturnSlot {
                value: diagnostic_type,
                type_id: Some(type_id),
                reactive_template: None,
                channel: returned.channel,
            });
            match returned.channel {
                ReturnChannel::Success => success_type_ids.push(type_id),
                ReturnChannel::Error => error_return = Some(type_id),
            }
        }
        let fallible_carrier_type_id = fallible_carrier_from_type_ids(
            &success_type_ids,
            error_return,
            context.type_environment,
        );
        let function_type_id = context.type_environment.intern_function(FunctionTypeKey {
            parameters: parameter_type_ids.into_boxed_slice(),
            returns: success_type_ids.into_boxed_slice(),
            error_return,
        });
        Ok((
            FunctionSignature {
                parameters,
                returns,
            },
            function_type_id,
            fallible_carrier_type_id,
        ))
    }
}
