//! Generated sidecar environment construction, emission and evidence installation.
use super::super::MaterialisedGenericAst;
use super::super::{GenericFunctionInstantiationRequest, GenericFunctionTemplate};
use super::frozen_syntax::StableBodySyntax;
use super::nominal_blueprints::{
    intern_generated_canonical_type, intern_materialisation_type_blueprint,
};
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
use crate::compiler_frontend::ast::generic_bounds::{BoundEvidenceSelection, evidence_for_type};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_public_const_template;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleEnvironment, AstModuleLookups, ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::finalization::{
    AstFinalizer, MaterialisationContextRetention,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnChannel};
use crate::compiler_frontend::builtins::casts::evidence::builtin_cast_proves_core_trait;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::GenericTypeBindings;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::PublicConstTemplate;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallTransferEffect,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModuleRootRole,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitParameter, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement,
};
use crate::compiler_frontend::traits::environment::{TraitEnvironment, trait_this_name};
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::evidence::{
    TraitEvidenceDefinition, TraitEvidenceEnvironment,
};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

type MaterialisedSidecarOutcome = (AstBuildResult, PathId);

impl ModuleMaterialisationPreparation {
    pub(crate) fn build_environment(
        &self,
        phase_context: &AstPhaseContext<'_>,
        module_resources: Rc<RefCell<ModuleResourceTable>>,
        string_table: &mut StringTable,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<AstModuleEnvironment, CompilerError> {
        let mut declaration_table =
            TopLevelDeclarationTable::fork_for_generated(Rc::clone(&self.declaration_table));
        let mut resolved_module_constants = ResolvedConstantSet::default();
        let mut generated_type_environment = self.type_environment.fork_for_generated();
        let mut template_materialiser = GeneratedFoldedValueMaterialiser {
            type_environment: &mut generated_type_environment,
            external_registry: &self.external_package_registry,
            nominal_source: self,
            template_ir_store: Rc::clone(&phase_context.template_ir_store),
            module_resources: Rc::clone(&module_resources),
            path_fork,
        };
        for row in self.const_values.iter_module_constant_views() {
            let value_id = row.id;
            let Some(declaration_id) = declaration_table.declaration_id_by_path(row.path) else {
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
            exported_callable_paths: FxHashSet::default(),
            binding_environment: Rc::clone(&self.binding_environment),
            warnings: Vec::new(),
            declaration_table: Rc::new(declaration_table),
            imported_functions_by_local_path: self.imported_functions_by_local_path.clone(),
            imported_struct_definitions: self.imported_struct_definitions.clone(),
            imported_choice_definitions: self.imported_choice_definitions.clone(),
            resolved_module_constants: Rc::new(resolved_module_constants),
            builtin_struct_ast_nodes: self.builtin_struct_ast_nodes.clone(),
            // Preparing sidecars inherit compatible type IDs and immutable lookup owners.
            // Request-local templates and evidence remain independently owned; generated evidence
            // installation writes each sidecar's evidence environment, so sharing one owner would
            // only move the deep copy into that first write.
            resolved_struct_fields_by_path: Rc::clone(&self.resolved_struct_fields_by_path),
            resolved_function_signatures_by_path: Rc::clone(
                &self.resolved_function_signatures_by_path,
            ),
            generic_function_templates_by_path: self.generic_function_templates_by_path.clone(),
            resolved_type_aliases_by_path: Rc::clone(&self.resolved_type_aliases_by_path),
            choice_variant_shells_by_path: Rc::clone(&self.choice_variant_shells_by_path),
            declaration_semantics: Rc::clone(&self.declaration_semantics),
            receiver_methods: Rc::clone(&self.receiver_methods),
            trait_environment: Rc::clone(&self.trait_environment),
            trait_evidence_environment: Rc::new(self.trait_evidence_environment.clone()),
            generic_declarations_by_path: Rc::clone(&self.generic_declarations_by_path),
            nominal_type_ids_by_path: Rc::clone(&self.nominal_type_ids_by_path),
            source_nominal_paths: Rc::clone(&self.source_nominal_paths),
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: self.style_directives.clone(),
            build_profile: self.build_profile,
        };

        Ok(AstModuleEnvironment {
            lookups: Rc::new(lookups),
            generated_evidence_pairs: Rc::new(FxHashSet::default()),
            type_environment: generated_type_environment,
            resolved_public_type_roots: Default::default(),
            resolved_public_trait_roots: Vec::new(),
        })
    }

    pub(crate) fn generic_function_templates(&self) -> &FxHashMap<PathId, GenericFunctionTemplate> {
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
        templates: &FxHashMap<PathId, GenericFunctionTemplate>,
    ) -> Result<FxHashMap<GeneratedDeclarationIdentity, PathId>, CompilerError> {
        let mut paths_by_identity = FxHashMap::default();
        for (path, template) in templates {
            if template.body_tokens.is_none() {
                continue;
            }
            let Some(identity) = template.declaration_identity.as_ref() else {
                continue;
            };
            if let Some(previous_path) = paths_by_identity.insert(identity.clone(), *path) {
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
        string_table: &mut StringTable,
        path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
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
        let content_value_at_path = |logical_path: &PathId| {
            let content_path = self.content_constant_path_for_capture(logical_path, path_fork)?;
            let resources = self.module_resources.as_ref().ok_or_else(|| {
                CompilerError::compiler_error(
                    "nested generic content capture has no module resource table",
                )
            })?;
            let resources = resources.borrow();
            self.stable_folded_value_at_path(&content_path, &resources, path_fork)
        };
        let body = template.body_tokens.as_ref().ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "Generated request's retained generic template has no body syntax",
                ),
                &self.string_table,
            )
        })?;
        // Share the declaring preparation's lazily frozen donor owner across every request.
        // Source bodies share the cached owner; a nested body that retained its own donor pair
        // keeps that pair.
        let donor_identity = self.donor_identity().clone();
        let stage0_resolution_facts = body
            .resolution_facts()
            .map(Arc::as_ref)
            .or(self.stage0_resolution_facts.as_deref());
        let frozen_identity_handle = body
            .frozen_identity_handle()
            .cloned()
            .unwrap_or_else(|| self.frozen_identity_handle.clone());
        let stable_body = StableBodySyntax::capture(
            body,
            template.source_file,
            path_fork,
            Some(&donor_identity),
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
            let nested_stage0_resolution_facts = nested_body
                .resolution_facts()
                .map(Arc::as_ref)
                .or(self.stage0_resolution_facts.as_deref());
            let nested_frozen_identity_handle = nested_body
                .frozen_identity_handle()
                .cloned()
                .unwrap_or_else(|| self.frozen_identity_handle.clone());
            let stable_nested_body = StableBodySyntax::capture(
                nested_body,
                nested_template.source_file,
                path_fork,
                Some(&donor_identity),
                nested_stage0_resolution_facts,
                nested_frozen_identity_handle,
                &content_value_at_path,
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &self.string_table))?;
            stable_nested_bodies.push((*path, nested_template.source_file, stable_nested_body));
        }

        self.validate_requester_string_prefix(string_table)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        let source_file = template.source_file;
        let identity_tables = body.source_identity_tables();
        let materialised_body = stable_body
            .materialise(source_file, path_fork, string_table, identity_tables)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
        let file_value_resolution = generated_file_value_resolution_services(
            Rc::clone(&module_resources),
            self.module_origin.clone(),
            Arc::clone(&materialised_body.resolution_facts),
        );
        let build_context = AstBuildContext {
            external_package_registry: Arc::clone(&self.external_package_registry),
            style_directives: &self.style_directives,
            string_table,
            path_fork,
            entry_dir: self.entry_dir,
            root_role: ModuleRootRole::Support,
            build_profile: self.build_profile,
            numeric_profile: self.numeric_profile,
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
        let (phase_context, string_table_ref, path_fork_ref) =
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
                path_fork_ref,
            )
            // `build_environment` uses the path fork retained by the AST build context.
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
            generated_template.body_tokens = Some(
                materialised_body
                    .into_generic_body()
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table_ref))?,
            );
            for (path, source_file, stable_nested_body) in stable_nested_bodies {
                let materialised_nested_body = stable_nested_body
                    .materialise(source_file, path_fork_ref, string_table_ref, None)
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
                nested_template.body_tokens = Some(
                    materialised_nested_body
                        .into_generic_body()
                        .map_err(|error| {
                            CompilerMessages::from_error_ref(error, string_table_ref)
                        })?,
                );
            }
        }
        let (build_result, instance_path) = emit_materialised_sidecar(
            &phase_context,
            environment,
            GeneratedSidecarRequest {
                identity,
                function_path: template.function_path,
                requester_context,
                requester_call_span,
            },
            self,
            path_fork_ref,
            string_table_ref,
        )?;
        Ok(MaterialisedGenericAst {
            build_result,
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
    pub function_path: PathId,
    pub requester_context: &'a ModuleMaterialisationPreparation,
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
    path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    string_table: &mut StringTable,
) -> Result<MaterialisedSidecarOutcome, CompilerMessages>
where
    PrimarySource: MaterialisationNominalSource,
{
    let GeneratedSidecarRequest {
        identity,
        function_path,
        requester_context,
        requester_call_span,
    } = request;
    let type_arguments = identity.type_arguments();
    let nominal_source = (primary_source, requester_context);
    let mut materialised_type_arguments = Vec::with_capacity(type_arguments.len());
    for canonical_identity in type_arguments {
        let type_id = intern_generated_canonical_type(
            canonical_identity,
            &mut environment.type_environment,
            phase_context.external_package_registry.as_ref(),
            &nominal_source,
            string_table,
            path_fork,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        materialised_type_arguments.push(type_id);
    }
    install_generated_request_evidence(
        identity,
        &materialised_type_arguments,
        requester_context,
        primary_source,
        &mut environment,
        string_table,
        path_fork,
        phase_context.external_package_registry.as_ref(),
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let request = GenericFunctionInstantiationRequest::generated(
        identity.declaration(),
        function_path,
        materialised_type_arguments.into_boxed_slice(),
        path_fork,
        string_table,
        requester_call_span,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let instance_path = request.instance_path;
    let emitted = {
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_emit,
            crate::timing::TimingMetric::FrontendGeneratedAstEmit,
            phase_context.timing_context
        );
        AstEmitter::new(
            phase_context,
            &mut environment,
            1,
            path_fork,
            FxHashMap::default(),
        )
        .with_generic_call_site_identity_handle(requester_context.frozen_identity_handle.clone())
        .emit_generated_request(request, string_table)?
    };

    let mut build_result = {
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_finalise,
            crate::timing::TimingMetric::FrontendGeneratedAstFinalise,
            phase_context.timing_context
        );
        AstFinalizer::new(phase_context, environment, path_fork).finalize(
            emitted,
            &[],
            MaterialisationContextRetention::ForDeferredRequests,
            string_table,
        )?
    };
    // The declaring source owns authored field provenance. Imported or synthetic requester
    // blueprints omit those spans, so donor-first merging keeps source ranges stable.
    if let Some(context) = &mut build_result.materialisation_context {
        context
            .inherit_nominal_blueprints(primary_source)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        context
            .inherit_nominal_blueprints(requester_context)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    Ok((build_result, instance_path))
}

/// One evidence row a generated request still has to install.
///
/// WHAT: pairs the canonical requester evidence identity with the generated-domain receiver the
///       reinstated template's own type arguments are solved against.
/// WHY: the request's own selections carry no receiver, while evidence discovered through a
///       reinstated template's concrete bounds always does; one optional receiver lets both kinds
///       share a single worklist instead of inventing a receiver for the request's own rows.
///       Worklist items are moved out of the queue, so the item itself is never cloned.
#[derive(Debug)]
struct PendingGeneratedEvidence {
    identity: CanonicalEvidenceIdentity,
    concrete_receiver: Option<TypeId>,
}

/// Install every evidence row one generated request's evidence closure needs.
///
/// WHAT: walks a queue of canonical requester evidence identities and installs each row's pair,
///       trait, evidence definition and imported contracts once per identity, while nested bound
///       discovery runs once per `(identity, receiver)`.
/// WHY: the request's own selections seed the closure, and a reinstated template's concrete bounds
///      can require requester rows the declaring module never projected, so those rows are appended
///      while the template is still in hand. A row is rediscovered for every receiver that solves
///      the template's own arguments, so installation must be idempotent per identity or the same
///      pair, evidence definition and contracts would be rebuilt; discovery must still run per
///      receiver or the nested rows that receiver unlocks would be missed.
#[allow(
    clippy::too_many_arguments,
    reason = "evidence installation keeps the request identity, its materialised type arguments, the requester context, the declaring nominal source, and the generated domain's collaborators as separate borrows"
)]
fn install_generated_request_evidence<PrimarySource>(
    identity: &GeneratedFunctionIdentity,
    materialised_type_arguments: &[TypeId],
    requester_context: &ModuleMaterialisationPreparation,
    primary_source: &PrimarySource,
    environment: &mut AstModuleEnvironment,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    external_registry: &ExternalPackageRegistry,
) -> Result<(), CompilerError>
where
    PrimarySource: MaterialisationNominalSource,
{
    // Declaring provenance stays authoritative; requester blueprints fill only the identities the
    // declaring source does not retain, exactly as the generated type-argument lane composes them.
    let nominal_source = (primary_source, requester_context);
    // The request's own selections come first. A reinstated template whose concrete arguments
    // require further requester evidence appends those rows here, so nested requests the body
    // resolves inside this domain find every row the requester already selected outside it.
    let mut pending = identity
        .evidence()
        .iter()
        .cloned()
        .map(|evidence| PendingGeneratedEvidence {
            identity: evidence,
            concrete_receiver: None,
        })
        .collect::<VecDeque<_>>();
    // A row's pair, trait, evidence definition and imported contracts are identical whoever
    // discovered it, so one canonical identity installs them once. Bound discovery is deduplicated
    // per `(identity, receiver)` instead: the same row discovered again for a different receiver
    // may still unlock nested rows there, but it must not rebuild anything already installed.
    let mut installed_rows: FxHashSet<CanonicalEvidenceIdentity> = FxHashSet::default();
    let mut discovered: FxHashSet<(CanonicalEvidenceIdentity, Option<TypeId>)> =
        FxHashSet::default();
    while let Some(PendingGeneratedEvidence {
        identity: evidence_identity,
        concrete_receiver,
    }) = pending.pop_front()
    {
        if !discovered.insert((evidence_identity.clone(), concrete_receiver)) {
            continue;
        }
        let install_row = installed_rows.insert(evidence_identity.clone());
        // Request type arguments are interned before installation, so their rows hit here; a row
        // discovered through a reinstated template's bounds is interned on first use. A repeat for
        // another receiver reuses the interned target, the installed trait and the installed row.
        let generated_target_type_id = if install_row {
            Some(intern_generated_canonical_type(
                evidence_identity.target_type_identity(),
                &mut environment.type_environment,
                external_registry,
                &nominal_source,
                string_table,
                path_fork,
            )?)
        } else {
            None
        };
        let requester_trait_id = requester_context
            .trait_environment
            .id_for_canonical_identity(evidence_identity.trait_identity())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated evidence trait is absent from the requester context",
                )
            })?;
        let generated_trait_id = if let Some(generated_target_type_id) = generated_target_type_id {
            let generated_trait_id = install_generated_requester_bound_trait(
                evidence_identity.trait_identity(),
                requester_trait_id,
                requester_context,
                environment,
                external_registry,
                &nominal_source,
                string_table,
                path_fork,
            )?;
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
            generated_trait_id
        } else {
            // The first installation already registered this canonical identity's trait here.
            environment
                .lookups
                .trait_environment
                .id_for_canonical_identity(evidence_identity.trait_identity())
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated reinstated evidence trait is missing from the generated context",
                    )
                })?
        };
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
            });
        let requester_evidence = match requester_evidence_id {
            Some(evidence_id) => Some(
                requester_context
                    .trait_evidence_environment
                    .get(evidence_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error("Generated requester evidence is missing")
                    })?,
            ),
            None => None,
        };
        // The first installation proves the requester still holds a real row (or a builtin core
        // cast proof) for this pair; a repeat discovery reuses that already proven row.
        if install_row
            && requester_evidence.is_none()
            && !builtin_cast_proves_core_trait(
                requester_target_type_id,
                requester_trait_id,
                &requester_context.type_environment,
                &requester_context.trait_environment,
                requester_context.numeric_profile,
            )
        {
            return Err(CompilerError::compiler_error(
                "Generated request selected evidence absent from the requester context",
            ));
        }
        let evidence_kind =
            requester_evidence.map_or(TraitEvidenceKind::Builtin, |evidence| evidence.kind);
        // Template installation mutates the lookup tables, not the immutable trait definitions.
        // Keep that definition owner borrowed independently throughout the requirement loop.
        let generated_trait_environment = Rc::clone(&environment.lookups.trait_environment);
        let generated_trait = generated_trait_environment
            .get(generated_trait_id)
            .ok_or_else(|| CompilerError::compiler_error("Generated declaring trait is missing"))?;
        let requester_trait = if evidence_kind == TraitEvidenceKind::Canonical {
            Some(
                requester_context
                    .trait_environment
                    .get(requester_trait_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error("Generated requester trait is missing")
                    })?,
            )
        } else {
            None
        };

        // Only a first installation builds the row's requirement list and contracts; a repeat
        // discovery reuses the installed row instead of rebuilding either.
        let (mut requirements, mut imported_contracts) =
            if install_row && evidence_kind == TraitEvidenceKind::Canonical {
                let requirement_capacity = generated_trait.requirements.len();
                (
                    Vec::with_capacity(requirement_capacity),
                    Vec::with_capacity(requirement_capacity),
                )
            } else {
                (Vec::new(), Vec::new())
            };
        let executable_requirements = if evidence_kind == TraitEvidenceKind::Canonical {
            generated_trait.requirements.as_slice()
        } else {
            // Compiler-owned builtin cast evidence proves the bound directly. It deliberately
            // has no source receiver-method mapping because `cast` lowers through builtin cast
            // semantics rather than an evidence method call.
            &[]
        };
        for generated_requirement in executable_requirements {
            let requester_trait = requester_trait.ok_or_else(|| {
                CompilerError::compiler_error("Generated requester trait is missing")
            })?;
            let requester_evidence = requester_evidence.ok_or_else(|| {
                CompilerError::compiler_error("Generated requester evidence is missing")
            })?;
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
            let method_path = requester_mapping.method_path;
            if install_row {
                requirements.push(TraitRequirementEvidence {
                    requirement_id: generated_requirement.id,
                    method_path,
                });
            }

            if let Some(template) = requester_context
                .generic_function_templates_by_path
                .get(&method_path)
            {
                // A template origin is the only case that still has to become a concrete callable
                // inside the materialised body, so it is reinstalled into this domain before the
                // body is emitted; the instance itself materialises later through the shared
                // request lane. A generic evidence method is a non-executable placeholder here:
                // deriving a carrier from the retained template signature would invent one for a
                // call shape that must never reach HIR, because the instance's own contract owns
                // the ordinary call carrier.
                install_generated_requester_template(
                    method_path,
                    template,
                    requester_context,
                    environment,
                    external_registry,
                    &nominal_source,
                    string_table,
                    path_fork,
                )?;
                // The reinstated template's own concrete arguments decide which further requester
                // evidence its nested requests will select inside this domain, so they are solved
                // against the concrete receiver while the template is still in hand. This runs
                // for every distinct receiver; the contract below belongs to the row itself.
                discover_generated_requester_bound_evidence(
                    &mut pending,
                    method_path,
                    template,
                    &evidence_identity,
                    concrete_receiver,
                    materialised_type_arguments,
                    requester_context,
                    environment,
                )?;
                if install_row {
                    let target = match template.declaration_identity.as_ref() {
                        Some(GeneratedDeclarationIdentity::Public(origin)) => {
                            SourceFunctionTarget::Imported {
                                origin: origin.clone(),
                                local_path: method_path,
                            }
                        }
                        Some(GeneratedDeclarationIdentity::ModulePrivate(identity)) => {
                            SourceFunctionTarget::ModulePrivate {
                                identity: identity.clone(),
                                local_path: method_path,
                            }
                        }
                        None => {
                            return Err(CompilerError::compiler_error(
                                "Generated evidence generic method has no frozen executable identity",
                            ));
                        }
                    };
                    imported_contracts.push((
                        method_path,
                        AstImportedFunctionContract {
                            target,
                            summary: bootstrap_call_summary_from_signature(&template.signature),
                            fallible_carrier_type_id: None,
                        },
                    ));
                }
            } else if let Some(source_contract) = requester_context
                .imported_functions_by_local_path
                .get(&method_path)
            {
                // The first installation installed this contract; a repeat discovery reuses it
                // and must not re-materialise the same carrier.
                if !install_row {
                    continue;
                }
                let fallible_carrier_type_id = match requester_context
                    .resolved_function_signatures_by_path
                    .get(&method_path)
                {
                    Some(resolved_signature) => materialise_generated_fallible_carrier(
                        &resolved_signature.signature,
                        requester_context,
                        &mut environment.type_environment,
                        external_registry,
                        &nominal_source,
                        string_table,
                        path_fork,
                    )?,
                    // A requester that is itself a generated sidecar installed this contract
                    // without the requester's own signature projection. That contract already
                    // carries the requester-domain carrier for the same method, so it is
                    // re-interned through the stable-identity bridge instead of being replaced.
                    None => source_contract
                        .fallible_carrier_type_id
                        .map(|carrier_type_id| {
                            intern_generated_requester_type(
                                carrier_type_id,
                                requester_context,
                                &mut environment.type_environment,
                                external_registry,
                                &nominal_source,
                                string_table,
                                path_fork,
                            )
                        })
                        .transpose()?,
                };
                let source_target = source_contract.target.clone();
                let target = match source_target {
                    SourceFunctionTarget::Imported { origin, .. } => {
                        SourceFunctionTarget::Imported {
                            origin,
                            local_path: method_path,
                        }
                    }
                    SourceFunctionTarget::ModulePrivate { identity, .. } => {
                        SourceFunctionTarget::ModulePrivate {
                            identity,
                            local_path: method_path,
                        }
                    }
                    SourceFunctionTarget::Local(_) | SourceFunctionTarget::Generated { .. } => {
                        return Err(CompilerError::compiler_error(
                            "Generated evidence method retained an invalid executable target",
                        ));
                    }
                };
                imported_contracts.push((
                    method_path,
                    AstImportedFunctionContract {
                        target,
                        summary: source_contract.summary.clone(),
                        fallible_carrier_type_id,
                    },
                ));
            } else {
                return Err(CompilerError::compiler_error(
                    "Generated evidence method has no frozen executable target",
                ));
            }
        }

        // A repeat for another receiver only discovered nested rows for it: the pair, trait,
        // evidence definition and imported contracts were installed by the row's first walk.
        if !install_row {
            continue;
        }
        let Some(generated_target_type_id) = generated_target_type_id else {
            return Err(CompilerError::compiler_error(
                "Generated evidence row was installed without an interned target type",
            ));
        };

        let source_file = requester_evidence
            .map(|evidence| evidence.source_file)
            .unwrap_or(PathId::ROOT);
        let declaration_span = requester_evidence.and_then(|evidence| evidence.declaration_span);
        let generated_evidence = TraitEvidenceDefinition {
            id: TraitEvidenceId(0),
            kind: evidence_kind,
            target_type_id: generated_target_type_id,
            trait_id: generated_trait_id,
            source_file,
            declaration_span,
            requirements,
        };
        let lookups = Rc::make_mut(&mut environment.lookups);
        match evidence_kind {
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

/// Reinstalls one requester-projected generic template into the generated sidecar domain.
///
/// WHAT: registers the requester's generic parameter list with its canonical trait bounds,
///       re-interns the template signature's slots through the requester's materialisation
///       blueprints, and inserts the template without body syntax at the evidence method's
///       projected path.
/// WHY: a materialised body resolves its evidence and receiver calls against the sidecar's own
///      environment, so an evidence template the declaring module never projected must exist
///      there before the body is emitted. The instance itself is materialised later from the
///      provider's frozen body, so this placeholder carries only the identity, parameters and
///      signature that inference reads; the body stays absent by construction.
///
/// `method_path` is the evidence requirement's projected path, which is also the key every later
/// lookup uses; it is passed explicitly so the installation cannot silently key elsewhere.
#[allow(
    clippy::too_many_arguments,
    reason = "template reinstallation keeps the evidence path, the requester template, its context, and the generated domain's collaborators as separate borrows"
)]
fn install_generated_requester_template<N: MaterialisationNominalSource>(
    method_path: PathId,
    template: &GenericFunctionTemplate,
    requester_context: &ModuleMaterialisationPreparation,
    environment: &mut AstModuleEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    // A same-module requester already injected this template with its own materialised body, and
    // its type IDs are already compatible with the generated environment.
    if environment
        .lookups
        .generic_function_templates_by_path
        .contains_key(&method_path)
    {
        return Ok(());
    }

    let parameter_list = requester_context
        .type_environment
        .generic_parameters(template.generic_parameter_list_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Generated evidence template has no requester-local generic parameter list",
            )
        })?;
    let mut parameter_slots: FxHashMap<GenericParameterId, usize> = FxHashMap::default();
    let mut bounds_by_local = FxHashMap::default();
    for (slot, parameter) in parameter_list.parameters.iter().enumerate() {
        parameter_slots.insert(parameter.id, slot);
        let mut bounds = Vec::with_capacity(parameter.trait_bounds.len());
        for trait_id in &parameter.trait_bounds {
            let identity = requester_context
                .trait_environment
                .canonical_identity_for_id(*trait_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence template bound has no canonical requester identity",
                    )
                })?;
            bounds.push(install_generated_requester_bound_trait(
                identity,
                *trait_id,
                requester_context,
                environment,
                external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?);
        }
        bounds_by_local.insert(TypeParameterId(slot as u32), bounds);
    }

    let registered = environment
        .type_environment
        .register_generic_parameter_list(
            parameter_list
                .parameters
                .iter()
                .enumerate()
                .map(|(slot, parameter)| (TypeParameterId(slot as u32), parameter.name)),
            &bounds_by_local,
        );
    let mut generic_parameter_type_ids = Vec::with_capacity(parameter_list.parameters.len());
    for slot in 0..parameter_list.parameters.len() as u32 {
        let parameter_id = registered
            .canonical_by_local
            .get(&TypeParameterId(slot))
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated evidence template parameter registration omitted a slot",
                )
            })?;
        generic_parameter_type_ids.push(
            environment
                .type_environment
                .type_id_for_generic_parameter(parameter_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated evidence template parameter registration omitted its type handle",
                    )
                })?,
        );
    }

    let mut parameters = Vec::with_capacity(template.signature.parameters.len());
    for parameter in &template.signature.parameters {
        let type_id = intern_generated_template_slot(
            parameter.value.type_id,
            &parameter_slots,
            &generic_parameter_type_ids,
            requester_context,
            environment,
            external_registry,
            nominal_source,
            string_table,
            path_fork,
        )?;
        let mut declaration = parameter.clone();
        declaration.value.type_id = type_id;
        declaration.value.diagnostic_type =
            diagnostic_type_spelling(type_id, &environment.type_environment);
        parameters.push(declaration);
    }
    let mut returns = Vec::with_capacity(template.signature.returns.len());
    for slot in &template.signature.returns {
        let Some(type_id) = slot.type_id else {
            returns.push(slot.clone());
            continue;
        };
        let type_id = intern_generated_template_slot(
            type_id,
            &parameter_slots,
            &generic_parameter_type_ids,
            requester_context,
            environment,
            external_registry,
            nominal_source,
            string_table,
            path_fork,
        )?;
        let mut remapped = slot.clone();
        remapped.value = diagnostic_type_spelling(type_id, &environment.type_environment);
        remapped.type_id = Some(type_id);
        returns.push(remapped);
    }

    Rc::make_mut(&mut environment.lookups)
        .generic_function_templates_by_path
        .insert(
            method_path,
            GenericFunctionTemplate {
                function_path: method_path,
                source_file: template.source_file,
                declaration_identity: template.declaration_identity.clone(),
                generic_parameter_owner: template.generic_parameter_owner.clone(),
                generic_parameter_list_id: registered.list_id,
                signature: FunctionSignature {
                    parameters,
                    returns,
                },
                body_tokens: None,
                declaration_span: template.declaration_span,
            },
        );

    Ok(())
}

/// Re-interns one requester template signature slot into the generated sidecar domain.
///
/// WHAT: projects the requester-local slot through its stable materialisation blueprint, where the
///       template's own generic parameters map to their generated-domain slots, and interns the
///       result in the generated environment.
/// WHY: the template's signature is requester-domain data, so every slot must cross the boundary
///      through the same stable bridge nominal members use; a raw requester `TypeId` cannot be
///      interpreted against the declaring module's environment.
#[allow(
    clippy::too_many_arguments,
    reason = "slot interning keeps the requester blueprint context, its parameter slots, the generated parameter handles, and the generated domain's collaborators as separate borrows"
)]
fn intern_generated_template_slot<N: MaterialisationNominalSource>(
    type_id: TypeId,
    parameter_slots: &FxHashMap<GenericParameterId, usize>,
    generic_parameter_type_ids: &[TypeId],
    requester_context: &ModuleMaterialisationPreparation,
    environment: &mut AstModuleEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TypeId, CompilerError> {
    let blueprint = requester_context.materialisation_type_blueprint(type_id, parameter_slots)?;
    intern_materialisation_type_blueprint(
        &blueprint,
        generic_parameter_type_ids,
        nominal_source,
        &mut environment.type_environment,
        external_registry,
        string_table,
        path_fork,
    )
}

/// Materialises one evidence method's fallible carrier into the generated type domain.
///
/// WHAT: reverse-projects the resolved signature's success and error slots through the requester's
///       stable type identities, re-interns them in the generated environment, and applies the
///       shared carrier shape rule for zero, single and multiple success returns.
/// WHY: a generated sidecar lowers the evidence call against its own module-local type domain, so
///      the requester's carrier `TypeId` cannot be reused across the boundary. A signature without
///      an error slot is genuinely infallible and keeps no carrier. Zero and single success
///      returns stay on the stack; only a multi-return signature pays for its tuple list.
fn materialise_generated_fallible_carrier<N: MaterialisationNominalSource>(
    signature: &FunctionSignature,
    requester_context: &ModuleMaterialisationPreparation,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<TypeId>, CompilerError> {
    let Some(error_type_id) = signature.error_return_type_id() else {
        return Ok(None);
    };
    let error_type_id = intern_generated_requester_type(
        error_type_id,
        requester_context,
        type_environment,
        external_registry,
        nominal_source,
        string_table,
        path_fork,
    )?;
    let mut intern_success = |success_type_id: TypeId| {
        intern_generated_requester_type(
            success_type_id,
            requester_context,
            type_environment,
            external_registry,
            nominal_source,
            string_table,
            path_fork,
        )
    };
    let mut success_slots = signature
        .returns
        .iter()
        .filter(|slot| slot.channel == ReturnChannel::Success)
        .filter_map(|slot| slot.type_id);
    let first_success = success_slots.next().map(&mut intern_success).transpose()?;
    let second_success = success_slots.next().map(&mut intern_success).transpose()?;
    Ok(match (first_success, second_success) {
        (None, _) => fallible_carrier_from_type_ids(&[], Some(error_type_id), type_environment),
        (Some(success_type_id), None) => fallible_carrier_from_type_ids(
            &[success_type_id],
            Some(error_type_id),
            type_environment,
        ),
        (Some(first_success), Some(second_success)) => {
            let mut success_type_ids = Vec::with_capacity(signature.returns.len());
            success_type_ids.push(first_success);
            success_type_ids.push(second_success);
            for success_type_id in success_slots {
                success_type_ids.push(intern_success(success_type_id)?);
            }
            // The slice entry point would copy this already-translated list, so the tuple shape
            // interns it through the same `intern_tuple` API that entry point uses.
            let success_type_id = type_environment.intern_tuple(success_type_ids);
            Some(type_environment.intern_fallible_carrier(success_type_id, error_type_id))
        }
    })
}

/// Re-interns one requester-local signature slot in the generated sidecar's type domain.
///
/// WHAT: projects the requester `TypeId` to its stable identity, then interns that identity into
///       the generated environment.
/// WHY: the two modules own disjoint module-local `TypeId` spaces even though they share the
///      evidence method's signature, so the stable identity is the only safe bridge.
fn intern_generated_requester_type<N: MaterialisationNominalSource>(
    type_id: TypeId,
    requester_context: &ModuleMaterialisationPreparation,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TypeId, CompilerError> {
    let canonical_identity = requester_context.stable_type_identity(type_id)?;
    intern_generated_canonical_type(
        &canonical_identity,
        type_environment,
        external_registry,
        nominal_source,
        string_table,
        path_fork,
    )
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
            let access = if parameter.value.value_mode.is_mutable() {
                PublicCallParameterAccess::Mutable
            } else {
                PublicCallParameterAccess::Shared
            };
            PublicCallParameterSummary {
                access,
                mutation: PublicCallMutationEffect::NoWrite,
                transfer_effect: PublicCallTransferEffect::MayConsume,
            }
        })
        .collect();
    PublicCallSummary {
        parameters,
        return_alias: FunctionReturnAliasSummary::Fresh,
        escapes_builtin_failure: false,
    }
}

/// Resolves an evidence target identity against the frozen requester environment.
///
/// WHAT: existing-type lookup only. The requester context is frozen shared semantic context, so
///       seeded builtins resolve through the environment's builtin key map and a `Dec` scale
///       resolves only when the requester already interned that scale.
/// WHY: the requester lookup proves what the requester already has for evidence reuse; new
///      consumer-local types are interned by the mutable generated-side materialisers instead.
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
        CanonicalTypeIdentity::SourceNominal(_) => Err(CompilerError::compiler_error(
            "Generated source evidence target has no requester-local canonical type handle",
        )),
        _ => Err(CompilerError::compiler_error(
            "Generated evidence target has no requester-local canonical type handle",
        )),
    }
}

/// Collect the requester evidence a reinstated template's concrete arguments require.
///
/// WHAT: solves the reinstated template's type arguments from the concrete receiver the generated
///       body uses through the shared binding walk, then appends every evidence row the
///       requester's registered bounds resolve for those arguments to the pending worklist.
/// WHY: a generated sidecar re-resolves a reinstated template's nested requests in its own domain,
///      so the rows those requests select must already exist there. Discovering them beside the
///      template that needs them keeps the closure inside the existing install loop and reuses the
///      shared receiver inference owner instead of adding a second inference pass; nothing is
///      added for a receiver the template shape does not match, because the sidecar's own bound
///      validation still reports a genuinely missing row.
#[allow(
    clippy::too_many_arguments,
    reason = "bound discovery keeps the pending worklist, the reinstated template and its evidence identity, the concrete receiver, the request's materialised type arguments, and the declaring/requester domains as separate borrows"
)]
fn discover_generated_requester_bound_evidence(
    pending: &mut VecDeque<PendingGeneratedEvidence>,
    method_path: PathId,
    requester_template: &GenericFunctionTemplate,
    evidence_identity: &CanonicalEvidenceIdentity,
    concrete_receiver: Option<TypeId>,
    materialised_type_arguments: &[TypeId],
    requester_context: &ModuleMaterialisationPreparation,
    environment: &AstModuleEnvironment,
) -> Result<(), CompilerError> {
    match concrete_receiver {
        // Evidence discovered through a template already carries the exact receiver it was
        // selected for.
        Some(receiver) => push_generated_template_bound_evidence(
            pending,
            method_path,
            requester_template,
            receiver,
            requester_context,
            environment,
        ),
        // A request's own selection records no receiver, so every type argument that instantiates
        // the row's target is solved against the template; the request's own bound parameter is
        // among them by construction.
        None => {
            for candidate in materialised_type_arguments {
                if generated_argument_instantiates_evidence_target(
                    *candidate,
                    evidence_identity.target_type_identity(),
                    &environment.type_environment,
                ) {
                    push_generated_template_bound_evidence(
                        pending,
                        method_path,
                        requester_template,
                        *candidate,
                        requester_context,
                        environment,
                    )?;
                }
            }
            Ok(())
        }
    }
}

/// Add the requester evidence one concrete receiver requires from a reinstated template.
///
/// WHAT: solves the template's own type arguments the concrete receiver determines through the
///       shared binding walk and appends the requester row each declared bound resolves for them.
/// WHY: the sidecar re-selects that evidence when the body resolves the template's nested
///      requests, so the rows have to exist in this domain first. A receiver the template shape
///      does not match adds nothing: the sidecar's own bound validation still reports a genuinely
///      missing row instead of the install inventing one.
fn push_generated_template_bound_evidence(
    pending: &mut VecDeque<PendingGeneratedEvidence>,
    method_path: PathId,
    requester_template: &GenericFunctionTemplate,
    receiver: TypeId,
    requester_context: &ModuleMaterialisationPreparation,
    environment: &AstModuleEnvironment,
) -> Result<(), CompilerError> {
    let Some(installed_template) = environment
        .lookups
        .generic_function_templates_by_path
        .get(&method_path)
    else {
        return Ok(());
    };
    let Some(receiver_parameter) = installed_template.signature.parameters.first() else {
        return Ok(());
    };

    let mut bindings = GenericTypeBindings::default();
    // A structural mismatch or a conflicting repeated parameter leaves the template unsolved. The
    // body's own nested request then reports the real diagnostic, so nothing is added here.
    if !environment
        .type_environment
        .try_collect_type_parameter_bindings_typeid(
            receiver_parameter.value.type_id,
            receiver,
            &mut bindings,
        )
        .unwrap_or(false)
    {
        return Ok(());
    }
    let Some(generated_parameters) = environment
        .type_environment
        .generic_parameters(installed_template.generic_parameter_list_id)
    else {
        return Ok(());
    };
    let Some(requester_parameters) = requester_context
        .type_environment
        .generic_parameters(requester_template.generic_parameter_list_id)
    else {
        return Ok(());
    };

    for (requester_parameter, generated_parameter) in requester_parameters
        .parameters
        .iter()
        .zip(generated_parameters.parameters.iter())
    {
        // Only a parameter the receiver itself determines can be solved here; the body's own
        // inference resolves the rest from its authored arguments and reports any real failure.
        let Some(concrete_argument) = bindings.get(generated_parameter.id) else {
            continue;
        };
        for bound_trait_id in &requester_parameter.trait_bounds {
            let Some(requester_argument_type_id) = requester_evidence_argument_type_id(
                concrete_argument,
                requester_context,
                &environment.type_environment,
            ) else {
                continue;
            };
            let Some(BoundEvidenceSelection::Registered(evidence_id)) = evidence_for_type(
                requester_argument_type_id,
                *bound_trait_id,
                &requester_context.type_environment,
                &requester_context.trait_environment,
                &requester_context.trait_evidence_environment,
                requester_context.numeric_profile,
            ) else {
                continue;
            };
            let Some(row) = requester_context
                .trait_evidence_environment
                .get(evidence_id)
            else {
                continue;
            };
            // The row itself carries the requester's own target, so the closure reuses that
            // identity instead of reconstructing it from the concrete argument. A target the
            // requester cannot canonicalise cannot name a request either, so it is left to the
            // sidecar's own bound validation rather than failing the install.
            let Ok(target_identity) = requester_context.stable_type_identity(row.target_type_id)
            else {
                continue;
            };
            let Some(trait_identity) = requester_context
                .trait_environment
                .canonical_identity_for_id(row.trait_id)
            else {
                return Err(CompilerError::compiler_error(
                    "Generated requester bound evidence trait has no canonical identity",
                ));
            };
            pending.push_back(PendingGeneratedEvidence {
                identity: CanonicalEvidenceIdentity::new(target_identity, trait_identity.clone()),
                concrete_receiver: Some(concrete_argument),
            });
        }
    }

    Ok(())
}

/// Whether one materialised request type argument can be the receiver of an evidence row.
///
/// WHAT: accepts the exact target type and a generic instance whose nominal constructor is that
///       target, mirroring the constructor retry the requester's bound evidence selection uses.
/// WHY: only a proven instance of the row's target may seed a reinstated template's own type
///      arguments; any other argument would solve them from an unrelated receiver.
fn generated_argument_instantiates_evidence_target(
    candidate: TypeId,
    target_identity: &CanonicalTypeIdentity,
    type_environment: &TypeEnvironment,
) -> bool {
    if type_environment
        .canonical_identity_for_type_id(candidate)
        .is_some_and(|identity| identity == target_identity)
    {
        return true;
    }

    // A generic instance may use evidence registered on its nominal constructor, exactly as the
    // bound selection retries the constructor when an instance carries no exact row.
    let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(candidate) else {
        return false;
    };
    type_environment
        .type_id_for_nominal_id(instance.base)
        .and_then(|base| type_environment.canonical_identity_for_type_id(base))
        .is_some_and(|identity| identity == target_identity)
}

/// Resolve the requester type one bound evidence selection for a generated argument runs against.
///
/// WHAT: returns the requester handle of the argument itself, or of a generic instance argument's
///       nominal constructor.
/// WHY: the requester's canonical index always holds the nominal and builtin handles, while a
///      nested instance handle may never have been interned there; its constructor identity
///      carries the very row the instance-level selection would fall back to. `None` means the
///      argument has no requester-registered identity at all, which no source conformance can
///      target, so there is no row to install.
fn requester_evidence_argument_type_id(
    concrete_argument: TypeId,
    requester_context: &ModuleMaterialisationPreparation,
    type_environment: &TypeEnvironment,
) -> Option<TypeId> {
    let identity = match type_environment.get(concrete_argument) {
        Some(TypeDefinition::GenericInstance(instance)) => type_environment
            .type_id_for_nominal_id(instance.base)
            .and_then(|base| type_environment.canonical_identity_for_type_id(base))?,
        _ => type_environment.canonical_identity_for_type_id(concrete_argument)?,
    };
    requester_context
        .type_environment
        .type_id_for_canonical_identity(identity)
}

/// Resolve one requester template bound in the generated domain, reconstructing its definition
/// when the declaring context never projected that trait.
///
/// WHAT: returns the generated-domain `TraitId` for a canonical bound identity, installing a
///       definition copied from the requester's own resolved trait when this domain lacks it.
/// WHY: a selected source method's bound traits may be owned by a provider or by the requester
///      rather than by the module whose body is materialised. The generated sidecar still has to
///      resolve those bounds and match their evidence rows, so the bound is preserved through the
///      shared canonical identity instead of failing as an internal error; a trait path the
///      declaring context already bound elsewhere is a genuine conflict and stays one.
#[allow(
    clippy::too_many_arguments,
    reason = "bound trait reconstruction keeps the canonical identity, the requester's resolved trait, and the generated domain's collaborators as separate borrows"
)]
fn install_generated_requester_bound_trait<N: MaterialisationNominalSource>(
    identity: &CanonicalTraitIdentity,
    requester_trait_id: TraitId,
    requester_context: &ModuleMaterialisationPreparation,
    environment: &mut AstModuleEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TraitId, CompilerError> {
    if let Some(trait_id) = environment
        .lookups
        .trait_environment
        .id_for_canonical_identity(identity)
    {
        return Ok(trait_id);
    }

    let source = requester_context
        .trait_environment
        .get(requester_trait_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Generated requester template bound has no resolved trait",
            )
        })?;
    let this_type = environment
        .type_environment
        .register_synthetic_generic_parameter(trait_this_name(string_table));
    let trait_id = environment.lookups.trait_environment.next_trait_id();
    let mut next_requirement_id = environment.lookups.trait_environment.next_requirement_id();
    let mut requirements = Vec::with_capacity(source.requirements.len());
    for requirement in &source.requirements {
        let receiver = match requirement.receiver {
            TraitReceiverRequirement::Immutable { .. } => {
                TraitReceiverRequirement::Immutable { this_type }
            }
            TraitReceiverRequirement::Mutable { .. } => {
                TraitReceiverRequirement::Mutable { this_type }
            }
        };
        let parameters = requirement
            .parameters
            .iter()
            .map(|parameter| {
                Ok(ResolvedTraitParameter {
                    name: parameter.name,
                    value_mode: parameter.value_mode.clone(),
                    type_id: intern_generated_trait_requirement_slot(
                        parameter.type_id,
                        source.this_type,
                        this_type,
                        requester_context,
                        &mut environment.type_environment,
                        external_registry,
                        nominal_source,
                        string_table,
                        path_fork,
                    )?,
                    span: parameter.span,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        let returns = requirement
            .returns
            .iter()
            .map(|returned| {
                Ok(ResolvedTraitReturn {
                    type_id: intern_generated_trait_requirement_slot(
                        returned.type_id,
                        source.this_type,
                        this_type,
                        requester_context,
                        &mut environment.type_environment,
                        external_registry,
                        nominal_source,
                        string_table,
                        path_fork,
                    )?,
                    channel: returned.channel,
                    span: returned.span,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        requirements.push(ResolvedTraitRequirement {
            id: next_requirement_id,
            name: requirement.name,
            receiver,
            parameters,
            returns,
            span: requirement.span,
        });
        next_requirement_id.0 += 1;
    }

    let definition = ResolvedTraitDefinition {
        id: trait_id,
        name: source.name,
        canonical_path: source.canonical_path,
        source_file: source.source_file,
        this_type,
        requirements,
        declaration_span: source.declaration_span,
        visibility: source.visibility.clone(),
    };
    let lookups = Rc::make_mut(&mut environment.lookups);
    let trait_environment = Rc::make_mut(&mut lookups.trait_environment);
    if trait_environment.insert(definition).is_some() {
        return Err(CompilerError::compiler_error(
            "Generated template bound trait path is already registered in the declaring context",
        ));
    }
    trait_environment.register_canonical_identity(identity.clone(), trait_id)?;
    Ok(trait_id)
}

/// Re-interns one requester trait requirement slot for a reconstructed bound.
///
/// WHAT: maps the requester's synthetic `This` handle to the generated domain's own synthetic
///       parameter and projects every other slot through the shared requester type bridge.
/// WHY: `This` is a synthetic anonymous parameter with no canonical identity or blueprint, while
///      every other requirement slot must cross the module boundary exactly as template signature
///      slots do.
#[allow(
    clippy::too_many_arguments,
    reason = "requirement slot interning keeps the requester's synthetic This handle, the generated one, and the bridge collaborators as separate borrows"
)]
fn intern_generated_trait_requirement_slot<N: MaterialisationNominalSource>(
    type_id: TypeId,
    requester_this_type: TypeId,
    generated_this_type: TypeId,
    requester_context: &ModuleMaterialisationPreparation,
    type_environment: &mut TypeEnvironment,
    external_registry: &ExternalPackageRegistry,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<TypeId, CompilerError> {
    if type_id == requester_this_type {
        return Ok(generated_this_type);
    }

    intern_generated_requester_type(
        type_id,
        requester_context,
        type_environment,
        external_registry,
        nominal_source,
        string_table,
        path_fork,
    )
}
