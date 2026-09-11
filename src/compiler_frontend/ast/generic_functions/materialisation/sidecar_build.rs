//! Generated sidecar environment construction, emission and evidence installation.
use super::super::MaterialisedGenericAst;
use super::super::{
    GenericFunctionBody, GenericFunctionInstantiationRequest, GenericFunctionTemplate,
};
use super::frozen_syntax::StableBodySyntax;
use super::nominal_blueprints::intern_generated_canonical_type;
use super::preparation_freeze::ModuleMaterialisationPreparation;
use super::stable_types::{
    GeneratedFoldedValueMaterialiser, MaterialisationNominalSource,
    generated_file_value_resolution_services,
};
use crate::compiler_frontend::ast::AstBuildContext;
use crate::compiler_frontend::ast::AstBuildResult;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::const_values::store::ConstValueMetadata;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_public_const_template;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleEnvironment, AstModuleLookups, ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::finalization::AstFinalizer;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::folded_value::PublicConstTemplate;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallTransferEffect,
    PublicCallTransferEligibility,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModuleRootRole,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::evidence::{
    TraitEvidenceDefinition, TraitEvidenceEnvironment,
};
use crate::compiler_frontend::traits::ids::TraitEvidenceId;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
impl ModuleMaterialisationPreparation {
    pub(crate) fn build_environment(
        &self,
        phase_context: &AstPhaseContext<'_>,
        module_resources: Rc<RefCell<ModuleResourceTable>>,
        string_table: &mut StringTable,
    ) -> Result<AstModuleEnvironment, CompilerError> {
        let mut declaration_table =
            TopLevelDeclarationTable::fork_for_generated(Rc::clone(&self.declaration_table));
        let mut resolved_module_constants = ResolvedConstantSet::default();
        let mut generated_type_environment = self.type_environment.clone();
        let mut template_materialiser = GeneratedFoldedValueMaterialiser {
            type_environment: &mut generated_type_environment,
            external_registry: &self.external_package_registry,
            nominal_source: self,
            template_ir_store: Rc::clone(&phase_context.template_ir_store),
            module_resources: Rc::clone(&module_resources),
        };
        for path in self.const_values.module_constant_paths() {
            let value_id = self.const_values.value_for_path(path).ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated materialisation module-constant path has no store value.",
                )
            })?;
            let Some(declaration_id) = declaration_table.declaration_id_by_path(path) else {
                return Err(CompilerError::compiler_error(
                    "Generated materialisation store row has no declaration-table entry",
                ));
            };
            let Some(declaration) = declaration_table.get_mut_by_id(declaration_id) else {
                return Err(CompilerError::compiler_error(
                    "Generated materialisation store row has no declaration-table value",
                ));
            };
            let mut template_builder =
                |projected: &PublicConstTemplate, metadata: &ConstValueMetadata| {
                    let template = materialize_public_const_template(
                        &mut template_materialiser,
                        projected,
                        &phase_context.template_ir_store,
                        string_table,
                        metadata.span,
                    )?;
                    Ok(ExpressionKind::Template(Box::new(template)))
                };
            declaration.value = self
                .const_values
                .expression_for_materialisation(value_id, &mut template_builder)?;
            resolved_module_constants.insert(declaration_id);
        }

        let lookups = AstModuleLookups {
            module_symbols: ModuleSymbols::empty(),
            binding_environment: self.binding_environment.clone(),
            warnings: Vec::new(),
            declaration_table: Rc::new(declaration_table),
            imported_functions_by_local_path: self.imported_functions_by_local_path.clone(),
            imported_struct_definitions: self.imported_struct_definitions.clone(),
            imported_choice_definitions: self.imported_choice_definitions.clone(),
            resolved_module_constants: Rc::new(resolved_module_constants),
            builtin_struct_ast_nodes: self.builtin_struct_ast_nodes.clone(),
            resolved_struct_fields_by_path: Rc::new(self.resolved_struct_fields_by_path.clone()),
            resolved_function_signatures_by_path: Rc::new(
                self.resolved_function_signatures_by_path.clone(),
            ),
            generic_function_templates_by_path: self.generic_function_templates_by_path.clone(),
            resolved_type_aliases_by_path: Rc::new(self.resolved_type_aliases_by_path.clone()),
            choice_variant_shells_by_path: Rc::new(self.choice_variant_shells_by_path.clone()),
            declaration_semantics: Rc::new(self.declaration_semantics.clone()),
            receiver_methods: Rc::new(self.receiver_methods.clone()),
            trait_environment: Rc::new(self.trait_environment.clone()),
            trait_evidence_environment: Rc::new(self.trait_evidence_environment.clone()),
            generic_declarations_by_path: Rc::new(self.generic_declarations_by_path.clone()),
            nominal_type_ids_by_path: Rc::new(self.nominal_type_ids_by_path.clone()),
            source_nominal_paths: Rc::new(self.source_nominal_paths.clone()),
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: self.style_directives.clone(),
            build_profile: self.build_profile,
        };

        Ok(AstModuleEnvironment {
            lookups: Rc::new(lookups),
            generated_evidence_pairs: Rc::new(FxHashSet::default()),
            type_environment: self.type_environment.clone(),
            resolved_public_type_roots: Default::default(),
            resolved_public_trait_roots: Vec::new(),
        })
    }

    pub(crate) fn generic_function_templates(
        &self,
    ) -> &FxHashMap<InternedPath, GenericFunctionTemplate> {
        &self.generic_function_templates_by_path
    }

    pub(crate) fn trait_environment(&self) -> &TraitEnvironment {
        &self.trait_environment
    }

    pub(crate) fn trait_evidence_environment(&self) -> &TraitEvidenceEnvironment {
        &self.trait_evidence_environment
    }

    pub(crate) fn template_for_identity(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<&GenericFunctionTemplate> {
        let path = self.generic_template_paths_by_identity.get(identity)?;
        let template = self.generic_function_templates_by_path.get(path)?;
        (template.declaration_identity.as_ref() == Some(identity) && template.body_tokens.is_some())
            .then_some(template)
    }

    pub(super) fn rebuild_generic_template_identity_index(&mut self) -> Result<(), CompilerError> {
        self.generic_template_paths_by_identity =
            Self::generic_template_identity_index(&self.generic_function_templates_by_path)?;
        Ok(())
    }

    pub(super) fn generic_template_identity_index(
        templates: &FxHashMap<InternedPath, GenericFunctionTemplate>,
    ) -> Result<FxHashMap<GeneratedDeclarationIdentity, InternedPath>, CompilerError> {
        let mut paths_by_identity = FxHashMap::default();
        for (path, template) in templates {
            if template.body_tokens.is_none() {
                continue;
            }
            let Some(identity) = template.declaration_identity.as_ref() else {
                continue;
            };
            if let Some(previous_path) = paths_by_identity.insert(identity.clone(), path.clone()) {
                return Err(CompilerError::compiler_error(format!(
                    "Generic template identity {identity:?} is retained at both {previous_path:?} and {path:?}",
                )));
            }
        }
        Ok(paths_by_identity)
    }

    pub(crate) fn materialise_ast(
        &self,
        identity: &GeneratedFunctionIdentity,
        requester_context: &ModuleMaterialisationPreparation,
        requester_call_span: Option<SourceSpan>,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let template = self
            .template_for_identity(identity.declaration())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Generated request has no retained declaring-module generic template",
                    ),
                    &self.string_table,
                )
            })?;
        let content_value_at_path = |logical_path: &InternedPath| {
            let content_path = self.content_constant_path_for_capture(logical_path)?;
            let resources = self.module_resources.as_ref().ok_or_else(|| {
                CompilerError::compiler_error(
                    "nested generic content capture has no module resource table",
                )
            })?;
            let resources = resources.borrow();
            self.stable_folded_value_at_path(&content_path, &resources)
        };
        let body = template.body_tokens.as_ref().ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "Generated request's retained generic template has no body syntax",
                ),
                &self.string_table,
            )
        })?;
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
        let stable_body = StableBodySyntax::capture(
            body.tokens(),
            &template.source_file,
            &self.string_table,
            stage0_resolution_facts,
            frozen_identity_handle,
            &content_value_at_path,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
        let mut stable_nested_bodies = Vec::new();
        for (path, nested_template) in &self.generic_function_templates_by_path {
            if path == &template.function_path {
                continue;
            }
            let Some(nested_body) = nested_template.body_tokens.as_ref() else {
                continue;
            };
            let nested_stage0_resolution_facts = match nested_body {
                GenericFunctionBody::Source(_) => self.stage0_resolution_facts.as_deref(),
                GenericFunctionBody::Materialised {
                    resolution_facts, ..
                } => Some(resolution_facts.as_ref()),
            };
            let nested_frozen_identity_handle = nested_body
                .frozen_identity_handle()
                .cloned()
                .unwrap_or_else(|| self.frozen_identity_handle.clone());
            let stable_nested_body = StableBodySyntax::capture(
                nested_body.tokens(),
                &nested_template.source_file,
                &self.string_table,
                nested_stage0_resolution_facts,
                nested_frozen_identity_handle,
                &content_value_at_path,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
            stable_nested_bodies.push((
                path.clone(),
                nested_template.source_file.clone(),
                stable_nested_body,
            ));
        }

        let (mut string_table, requester_string_remap) =
            requester_context.fork_materialisation_string_table();
        let source_file = template.source_file.clone();
        let materialised_body = stable_body
            .materialise(&source_file, &mut string_table)
            .map_err(|error| CompilerMessages::from_error_ref(error, &string_table))?;
        let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
        let file_value_resolution = generated_file_value_resolution_services(
            Rc::clone(&module_resources),
            self.module_origin.clone(),
            Arc::clone(&materialised_body.resolution_facts),
        );
        let build_context = AstBuildContext {
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: &self.style_directives,
            string_table: &mut string_table,
            entry_dir: self.entry_dir.clone(),
            root_role: ModuleRootRole::Support,
            build_profile: self.build_profile,
            file_value_resolution: Some(file_value_resolution),
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit: self.template_const_loop_iteration_limit,
            capacity_estimate: self.capacity_estimate,
            #[cfg(feature = "timers")]
            timing_context,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Generated,
        };
        let (phase_context, string_table_ref) =
            AstPhaseContext::from_build_context(build_context, Arc::new(Default::default()));
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_total,
            crate::timing::TimingMetric::FrontendGeneratedAstTotal,
            timing_context
        );
        let mut environment = self
            .build_environment(
                &phase_context,
                Rc::clone(&module_resources),
                string_table_ref,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
        {
            let lookups = Rc::make_mut(&mut environment.lookups);
            let generated_template = lookups
                .generic_function_templates_by_path
                .get_mut(&template.function_path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated materialisation lost its selected generic template",
                    )
                })
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;
            generated_template.body_tokens = Some(materialised_body.into_generic_body());
            for (path, source_file, stable_nested_body) in stable_nested_bodies {
                let materialised_nested_body = stable_nested_body
                    .materialise(&source_file, string_table_ref)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;
                let nested_template = lookups
                    .generic_function_templates_by_path
                    .get_mut(&path)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Generated materialisation lost a nested generic template",
                        )
                    })
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?;
                nested_template.body_tokens = Some(materialised_nested_body.into_generic_body());
            }
        }
        let (build_result, instance_path) = emit_materialised_sidecar(
            &phase_context,
            environment,
            GeneratedSidecarRequest {
                identity,
                function_path: template.function_path.clone(),
                requester_context,
                requester_string_remap: &requester_string_remap,
                requester_call_span,
            },
            self,
            string_table_ref,
        )?;
        Ok(MaterialisedGenericAst {
            build_result,
            string_table,
            instance_path,
        })
    }
}
/// One generated sidecar request as described by the lane that asked for it.
///
/// WHAT: groups the generated identity, its instance path and the requester evidence that
///       both materialisation lanes supply unchanged.
/// WHY: the two lanes differ only in how the body and environment are reconstructed, so the
///       request description belongs to the shared contract rather than its parameter list.
pub(super) struct GeneratedSidecarRequest<'a> {
    pub identity: &'a GeneratedFunctionIdentity,
    pub function_path: InternedPath,
    pub requester_context: &'a ModuleMaterialisationPreparation,
    pub requester_string_remap: &'a StringIdRemap,
    pub requester_call_span: Option<SourceSpan>,
}

/// Emit one generated sidecar after its lane-specific environment has been prepared.
///
/// WHAT: reconstructs generated type arguments and requester evidence, emits the generated
///       request, finalises its AST and carries both source blueprint sets into the result.
/// WHY: the frozen artefact and live preparation lanes differ only in how their body and
///       environment are reconstructed; sidecar emission must remain one contract.
pub(super) fn emit_materialised_sidecar<PrimarySource>(
    phase_context: &AstPhaseContext<'_>,
    mut environment: AstModuleEnvironment,
    request: GeneratedSidecarRequest<'_>,
    primary_source: &PrimarySource,
    string_table: &mut StringTable,
) -> Result<(AstBuildResult, InternedPath), CompilerMessages>
where
    PrimarySource: MaterialisationNominalSource,
{
    let GeneratedSidecarRequest {
        identity,
        function_path,
        requester_context,
        requester_string_remap,
        requester_call_span,
    } = request;
    let type_arguments = identity.type_arguments();
    let mut materialised_type_arguments = Vec::with_capacity(type_arguments.len());
    for canonical_identity in type_arguments {
        let type_id = intern_generated_canonical_type(
            canonical_identity,
            &mut environment.type_environment,
            phase_context.external_package_registry.as_ref(),
            requester_context,
            string_table,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        materialised_type_arguments.push(type_id);
    }
    install_generated_request_evidence(
        identity,
        requester_context,
        requester_string_remap,
        &mut environment,
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let request = GenericFunctionInstantiationRequest::generated(
        identity.declaration(),
        function_path,
        materialised_type_arguments.into_boxed_slice(),
        string_table,
        requester_call_span,
    );
    let instance_path = request.instance_path.clone();
    let emitted = {
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_emit,
            crate::timing::TimingMetric::FrontendGeneratedAstEmit,
            phase_context.timing_context
        );
        AstEmitter::new(phase_context, &mut environment, 1)
            .with_generic_call_site_identity_handle(
                requester_context.frozen_identity_handle.clone(),
            )
            .emit_generated_request(request, string_table)?
    };

    let mut build_result = {
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_finalise,
            crate::timing::TimingMetric::FrontendGeneratedAstFinalise,
            phase_context.timing_context
        );
        AstFinalizer::new(phase_context, environment).finalize(emitted, &[], string_table)?
    };
    // The declaring source owns authored field provenance. Imported or synthetic requester
    // blueprints omit those spans, so donor-first merging keeps source ranges stable.
    build_result
        .materialisation_context
        .inherit_nominal_blueprints(primary_source)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    build_result
        .materialisation_context
        .inherit_nominal_blueprints(requester_context)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    Ok((build_result, instance_path))
}

fn install_generated_request_evidence(
    identity: &GeneratedFunctionIdentity,
    requester_context: &ModuleMaterialisationPreparation,
    requester_string_remap: &StringIdRemap,
    environment: &mut AstModuleEnvironment,
    string_table: &mut StringTable,
) -> Result<(), CompilerError> {
    for evidence_identity in identity.evidence() {
        let generated_target_type_id = environment
            .type_environment
            .type_id_for_canonical_identity(evidence_identity.target_type_identity())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated evidence target type was not interned in the generated environment",
                )
            })?;
        let generated_trait_id = if let Some(trait_id) = environment
            .lookups
            .trait_environment
            .id_for_canonical_identity(evidence_identity.trait_identity())
        {
            trait_id
        } else {
            return Err(CompilerError::compiler_error(format!(
                "Generated evidence trait {:?} is absent from the declaring context",
                evidence_identity.trait_identity()
            )));
        };
        let generated_evidence_pairs = Rc::make_mut(&mut environment.generated_evidence_pairs);
        generated_evidence_pairs.insert((generated_target_type_id, generated_trait_id));
        if let Some(TypeDefinition::GenericInstance(instance)) =
            environment.type_environment.get(generated_target_type_id)
            && let Some(base_type_id) = environment
                .type_environment
                .type_id_for_nominal_id(instance.base)
        {
            generated_evidence_pairs.insert((base_type_id, generated_trait_id));
        }
        let requester_trait_id = requester_context
            .trait_environment
            .id_for_canonical_identity(evidence_identity.trait_identity())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated evidence trait is absent from the requester context",
                )
            })?;
        let requester_target_type_id = requester_type_id_for_canonical_identity(
            evidence_identity.target_type_identity(),
            requester_context,
        )?;
        let requester_evidence_id = requester_context
            .trait_evidence_environment
            .canonical_for(requester_target_type_id, requester_trait_id)
            .or_else(|| {
                requester_context
                    .trait_evidence_environment
                    .builtin_for(requester_target_type_id, requester_trait_id)
            })
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request selected evidence absent from the requester context",
                )
            })?;
        let requester_evidence = requester_context
            .trait_evidence_environment
            .get(requester_evidence_id)
            .ok_or_else(|| {
                CompilerError::compiler_error("Generated requester evidence is missing")
            })?;
        let requester_trait = requester_context
            .trait_environment
            .get(requester_trait_id)
            .ok_or_else(|| CompilerError::compiler_error("Generated requester trait is missing"))?;
        let generated_trait = environment
            .lookups
            .trait_environment
            .get(generated_trait_id)
            .ok_or_else(|| CompilerError::compiler_error("Generated declaring trait is missing"))?;

        let mut requirements = Vec::with_capacity(generated_trait.requirements.len());
        let mut imported_contracts = Vec::with_capacity(generated_trait.requirements.len());
        let executable_requirements = if requester_evidence.kind == TraitEvidenceKind::Canonical {
            generated_trait.requirements.as_slice()
        } else {
            // Compiler-owned builtin cast evidence proves the bound directly. It deliberately
            // has no source receiver-method mapping because `cast` lowers through builtin cast
            // semantics rather than an evidence method call.
            &[]
        };
        for generated_requirement in executable_requirements {
            let requester_requirement = requester_trait
                .requirements
                .iter()
                .find(|requirement| {
                    requester_context.string_table.resolve(requirement.name)
                        == string_table.resolve(generated_requirement.name)
                })
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence could not match a canonical trait requirement",
                    )
                })?;
            let requester_mapping = requester_evidence
                .requirements
                .iter()
                .find(|mapping| mapping.requirement_id == requester_requirement.id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence has no executable target for a trait requirement",
                    )
                })?;
            let mut method_path = requester_mapping.method_path.clone();
            method_path.remap_string_ids(requester_string_remap);
            requirements.push(TraitRequirementEvidence {
                requirement_id: generated_requirement.id,
                method_path: method_path.clone(),
            });

            let (source_target, source_summary) = if let Some(source_contract) = requester_context
                .imported_functions_by_local_path
                .get(&requester_mapping.method_path)
            {
                (
                    source_contract.target.clone(),
                    source_contract.summary.clone(),
                )
            } else if let Some(template) = requester_context
                .generic_function_templates_by_path
                .get(&requester_mapping.method_path)
            {
                let target = match template.declaration_identity.as_ref() {
                    Some(GeneratedDeclarationIdentity::Public(origin)) => {
                        SourceFunctionTarget::Imported {
                            origin: origin.clone(),
                            local_path: requester_mapping.method_path.clone(),
                        }
                    }
                    Some(GeneratedDeclarationIdentity::ModulePrivate(identity)) => {
                        SourceFunctionTarget::ModulePrivate {
                            identity: identity.clone(),
                            local_path: requester_mapping.method_path.clone(),
                        }
                    }
                    None => {
                        return Err(CompilerError::compiler_error(
                            "Generated evidence generic method has no frozen executable identity",
                        ));
                    }
                };
                (
                    target,
                    bootstrap_call_summary_from_signature(&template.signature),
                )
            } else {
                return Err(CompilerError::compiler_error(
                    "Generated evidence method has no frozen executable target",
                ));
            };
            let target = match source_target {
                SourceFunctionTarget::Imported { origin, .. } => SourceFunctionTarget::Imported {
                    origin,
                    local_path: method_path.clone(),
                },
                SourceFunctionTarget::ModulePrivate { identity, .. } => {
                    SourceFunctionTarget::ModulePrivate {
                        identity,
                        local_path: method_path.clone(),
                    }
                }
                SourceFunctionTarget::Local(_) | SourceFunctionTarget::Generated { .. } => {
                    return Err(CompilerError::compiler_error(
                        "Generated evidence method retained an invalid executable target",
                    ));
                }
            };
            imported_contracts.push((
                method_path.clone(),
                AstImportedFunctionContract {
                    target,
                    summary: source_summary,
                    fallible_carrier_type_id: None,
                },
            ));
        }

        let mut source_file = requester_evidence.source_file.clone();
        source_file.remap_string_ids(requester_string_remap);
        let declaration_span = requester_evidence.declaration_span;
        let generated_evidence = TraitEvidenceDefinition {
            id: TraitEvidenceId(0),
            kind: requester_evidence.kind,
            target_type_id: generated_target_type_id,
            trait_id: generated_trait_id,
            source_file,
            declaration_span,
            requirements,
        };
        let lookups = Rc::make_mut(&mut environment.lookups);
        match requester_evidence.kind {
            TraitEvidenceKind::Canonical => Rc::make_mut(&mut lookups.trait_evidence_environment)
                .insert_validated(generated_evidence),
            TraitEvidenceKind::Builtin => Rc::make_mut(&mut lookups.trait_evidence_environment)
                .insert_builtin(generated_evidence),
        }
        for (path, contract) in imported_contracts {
            lookups
                .imported_functions_by_local_path
                .insert(path, contract);
        }
    }

    Ok(())
}
pub(super) fn fallible_carrier_from_type_ids(
    success_type_ids: &[TypeId],
    error_type_id: Option<TypeId>,
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    let error_type_id = error_type_id?;
    let success_type_id = match success_type_ids {
        [] => builtin_type_ids::NONE,
        [single] => *single,
        many => type_environment.intern_tuple(many.to_vec()),
    };
    Some(type_environment.intern_fallible_carrier(success_type_id, error_type_id))
}
pub(super) fn fallible_carrier_for_signature(
    signature: &crate::compiler_frontend::ast::statements::functions::FunctionSignature,
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    let error_type_id = signature
        .returns
        .iter()
        .find(|slot| {
            slot.channel
                == crate::compiler_frontend::ast::statements::functions::ReturnChannel::Error
        })?
        .type_id?;
    let success_types = signature.success_return_type_ids();
    fallible_carrier_from_type_ids(&success_types, Some(error_type_id), type_environment)
}

pub(crate) fn bootstrap_call_summary_from_signature(
    signature: &crate::compiler_frontend::ast::statements::functions::FunctionSignature,
) -> PublicCallSummary {
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            let access = if parameter.value.reactive_source.is_some() {
                PublicCallParameterAccess::Reactive
            } else if parameter.value.value_mode.is_mutable() {
                PublicCallParameterAccess::Mutable
            } else {
                PublicCallParameterAccess::Shared
            };
            PublicCallParameterSummary {
                access,
                mutation: PublicCallMutationEffect::NoWrite,
                transfer_eligibility: if access == PublicCallParameterAccess::Reactive {
                    PublicCallTransferEligibility::Ineligible
                } else {
                    PublicCallTransferEligibility::Eligible
                },
                transfer_effect: if access == PublicCallParameterAccess::Reactive {
                    PublicCallTransferEffect::NeverConsumes
                } else {
                    PublicCallTransferEffect::MayConsume
                },
                reactive_effect: PublicCallReactiveEffect::None,
            }
        })
        .collect();
    PublicCallSummary {
        parameters,
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn requester_type_id_for_canonical_identity(
    identity: &CanonicalTypeIdentity,
    requester_context: &ModuleMaterialisationPreparation,
) -> Result<TypeId, CompilerError> {
    if let Some(type_id) = requester_context
        .type_environment
        .type_id_for_canonical_identity(identity)
    {
        return Ok(type_id);
    }
    match identity {
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool) => Ok(builtin_type_ids::BOOL),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int) => Ok(builtin_type_ids::INT),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float) => Ok(builtin_type_ids::FLOAT),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Decimal) => {
            Ok(builtin_type_ids::DECIMAL)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String) => {
            Ok(builtin_type_ids::STRING)
        }
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Char) => Ok(builtin_type_ids::CHAR),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Range) => Ok(builtin_type_ids::RANGE),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::None) => Ok(builtin_type_ids::NONE),
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error)
        | CanonicalTypeIdentity::ModulePrivateNominal(_)
        | CanonicalTypeIdentity::ExternalOpaque(_)
        | CanonicalTypeIdentity::Collection(_)
        | CanonicalTypeIdentity::OrderedMap(_)
        | CanonicalTypeIdentity::Option(_)
        | CanonicalTypeIdentity::FallibleCarrier(_)
        | CanonicalTypeIdentity::GenericInstance(_)
        | CanonicalTypeIdentity::ModulePrivateGenericInstance(_)
        | CanonicalTypeIdentity::GenericParameter(_)
        | CanonicalTypeIdentity::AnonymousConstRecord => Err(CompilerError::compiler_error(
            "Generated evidence target has no requester-local canonical type handle",
        )),
        CanonicalTypeIdentity::SourceNominal(_) => Err(CompilerError::compiler_error(
            "Generated source evidence target has no requester-local canonical type handle",
        )),
    }
}
