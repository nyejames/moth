//! Generated sidecar environment construction, emission and evidence installation.
use super::super::MaterialisedGenericAst;
use super::super::{GenericFunctionInstantiationRequest, GenericFunctionTemplate};
use super::artefact_emit::ModuleMaterialisationContext;
use super::nominal_blueprints::intern_generated_canonical_type;
use super::preparation_freeze::{ModuleMaterialisationCapture, ModuleMaterialisationPreparation};
use super::stable_types::{GeneratedValueMaterialisationServices, MaterialisationNominalSource};
use crate::compiler_frontend::ast::AstBuildResult;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::generic_bounds::{BoundEvidenceSelection, evidence_for_type};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::AstModuleEnvironment;
use crate::compiler_frontend::ast::module_ast::finalization::{
    AstFinalizer, MaterialisationContextRetention,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::builtins::casts::evidence::builtin_cast_proves_core_trait;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::GenericTypeBindings;
use crate::compiler_frontend::datatypes::generic_parameters::TypeParameterId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallTransferEffect,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitParameter, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement,
};
use crate::compiler_frontend::traits::environment::trait_this_name;
use crate::compiler_frontend::traits::evidence::TraitEvidenceDefinition;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::rc::Rc;

type MaterialisedSidecarOutcome = (AstBuildResult, PathId);

impl ModuleMaterialisationCapture {
    pub(crate) fn generic_function_templates(&self) -> &FxHashMap<PathId, GenericFunctionTemplate> {
        &self.generic_function_templates_by_path
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
}

impl ModuleMaterialisationPreparation {
    pub(crate) fn materialise_ast(
        &self,
        identity: &GeneratedFunctionIdentity,
        requester_context: &ModuleMaterialisationPreparation,
        requester_call_span: Option<SourceSpan>,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
        #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
    ) -> Result<MaterialisedGenericAst, CompilerMessages> {
        let (declaring_context, template_index) = self
            .declaring_context_for_identity(identity.declaration())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Generated request has no retained declaring-module generic template",
                    ),
                    &self.string_table,
                )
            })?;
        declaring_context.materialise_ast_at(
            template_index,
            super::ModuleMaterialisationInput {
                identity,
                requester_context,
                requester_call_span,
                external_package_registry: &self.external_package_registry,
                string_table,
                path_fork,
                style_directives: &self.style_directives,
                build_profile: self.build_profile,
                template_const_loop_iteration_limit: self.template_const_loop_iteration_limit,
                #[cfg(feature = "timers")]
                timing_context,
            },
        )
    }
}
/// One generated request with the original stable declaring authority and requester evidence.
pub(super) struct GeneratedSidecarRequest<'a> {
    pub identity: &'a GeneratedFunctionIdentity,
    pub function_path: PathId,
    pub requester_context: &'a ModuleMaterialisationPreparation,
    pub declaring_context: &'a ModuleMaterialisationContext,
    pub requester_call_span: Option<SourceSpan>,
}

/// Emit one generated sidecar after its lane-specific environment has been prepared.
///
/// WHAT: reconstructs generated type arguments and requester evidence, emits the generated
///       request, finalises its AST and carries both source blueprint sets into the result.
/// WHY: all bodies use the same stable reconstruction path, and requester evidence must enter
///       that fresh domain before body emission.
pub(super) fn emit_materialised_sidecar<PrimarySource>(
    mut phase_context: AstPhaseContext<'_>,
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
        declaring_context,
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
    // Canonical reconstruction issues new definition paths. Restore the requester's existing
    // lookup spellings from detached rows so publication retains its imported nominal aliases.
    for binding in &requester_context.nominal_bindings {
        let type_id = intern_generated_canonical_type(
            &binding.identity,
            &mut environment.type_environment,
            phase_context.external_package_registry.as_ref(),
            &nominal_source,
            string_table,
            path_fork,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        environment
            .type_environment
            .register_nominal_path_alias(binding.local_path, type_id)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }
    let mut value_services = GeneratedValueMaterialisationServices {
        external_registry: phase_context.external_package_registry.as_ref(),
        template_ir_store: &phase_context.template_ir_store,
        module_resources: Rc::clone(
            &phase_context
                .file_value_resolution
                .as_ref()
                .ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Generated evidence has no file-value services",
                        ),
                        string_table,
                    )
                })?
                .module_resources,
        ),
        expression_builder: &mut phase_context.expression_builder,
        const_values: &mut phase_context.const_values,
    };
    install_generated_request_evidence(
        identity,
        &materialised_type_arguments,
        requester_context,
        primary_source,
        &mut environment,
        string_table,
        path_fork,
        &mut value_services,
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
    #[cfg(feature = "timers")]
    let timing_context = phase_context.timing_context;
    let emitted = {
        crate::timing_scope_attributed!(
            timing_guard_generated_ast_emit,
            crate::timing::TimingMetric::FrontendGeneratedAstEmit,
            timing_context
        );
        AstEmitter::new(
            &mut phase_context,
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
            timing_context
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
            .inherit_stable_context(declaring_context)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        context
            .inherit_nominal_blueprints(primary_source)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        context
            .inherit_nominal_blueprints(requester_context)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        context.inherit_requester_declaring_contexts(requester_context);
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
    services: &mut super::stable_types::GeneratedValueMaterialisationServices<'_>,
) -> Result<(), CompilerError>
where
    PrimarySource: MaterialisationNominalSource,
{
    let external_registry = services.external_registry;
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

            if let Some(template) = requester_context.generic_callable_for_path(&method_path) {
                // Canonical signatures reconstruct owned folded defaults in the generated domain;
                // requester declaration and expression graphs never cross this boundary.
                install_generated_requester_template(
                    method_path,
                    template,
                    requester_context,
                    environment,
                    services,
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
                    let target = match &template.declaration_identity {
                        GeneratedDeclarationIdentity::Public(origin) => {
                            SourceFunctionTarget::Imported {
                                origin: origin.clone(),
                                local_path: method_path,
                            }
                        }
                        GeneratedDeclarationIdentity::ModulePrivate(identity) => {
                            SourceFunctionTarget::ModulePrivate {
                                identity: identity.clone(),
                                local_path: method_path,
                            }
                        }
                    };
                    imported_contracts.push((
                        method_path,
                        generated_requester_template_contract(method_path, target, environment)?,
                    ));
                }
            } else if let Some(source_contract) = requester_context.callable_for_path(&method_path)
            {
                if !install_row {
                    continue;
                }
                if matches!(
                    &source_contract.target,
                    super::stable_types::StableFunctionTarget::Generated(_)
                ) {
                    return Err(CompilerError::compiler_error(
                        "Generated evidence method retained an invalid executable target",
                    ));
                }
                imported_contracts.push((
                    method_path,
                    source_contract.materialise_requester_contract(
                        method_path,
                        environment,
                        services,
                        &nominal_source,
                        string_table,
                        path_fork,
                    )?,
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

/// Builds an evidence contract from the template already translated into this sidecar's domain.
///
/// The retained requester signature is not a valid source of local slot handles. Reusing the
/// installed template also keeps generic slots tied to its generated parameter list.
fn generated_requester_template_contract(
    method_path: PathId,
    target: SourceFunctionTarget,
    environment: &AstModuleEnvironment,
) -> Result<AstImportedFunctionContract, CompilerError> {
    let template = environment
        .lookups
        .generic_function_templates_by_path
        .get(&method_path)
        .ok_or_else(|| {
            CompilerError::compiler_error("Generated evidence template was not installed")
        })?;
    Ok(AstImportedFunctionContract {
        target,
        summary: bootstrap_call_summary_from_signature(&template.signature),
        returns: template.signature.success_return_type_ids(),
        error_return: template.signature.error_return_type_id(),
    })
}

/// Restore requester inference contracts from canonical slots and owned folded defaults.
///
/// Retain the resolved receiver alongside the bodyless template. Public methods reuse the
/// nominal's parameter identities, but each callable owns its declaration-specific bound list.
#[allow(
    clippy::too_many_arguments,
    reason = "requester-template installation keeps method identity, stable callable contract and requester domain, mutable target environment and value services, nominal source and mutable string/path tables separate"
)]
fn install_generated_requester_template<N: MaterialisationNominalSource>(
    method_path: PathId,
    template: &super::stable_types::StableGenericCallableContract,
    requester_context: &ModuleMaterialisationPreparation,
    environment: &mut AstModuleEnvironment,
    services: &mut super::stable_types::GeneratedValueMaterialisationServices<'_>,
    nominal_source: &N,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    if environment
        .lookups
        .generic_function_templates_by_path
        .contains_key(&method_path)
    {
        return Ok(());
    }
    let mut bounds_by_local = FxHashMap::default();
    for (slot, parameter) in template.generic_parameters.iter().enumerate() {
        let mut bounds = Vec::with_capacity(parameter.bounds.len());
        for identity in &parameter.bounds {
            let requester_trait_id = requester_context
                .trait_environment
                .id_for_canonical_identity(identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated requester template bound has no canonical requester trait",
                    )
                })?;
            bounds.push(install_generated_requester_bound_trait(
                identity,
                requester_trait_id,
                requester_context,
                environment,
                services.external_registry,
                nominal_source,
                string_table,
                path_fork,
            )?);
        }
        bounds_by_local.insert(TypeParameterId(slot as u32), bounds);
    }
    let receiver_identity = template.generic_parameter_owner.as_ref()
        .and_then(crate::compiler_frontend::canonical_type_identity::GenericDeclarationOrigin::nominal_type_origin)
        .map(|origin| CanonicalTypeIdentity::SourceNominal(origin.clone()));
    // Public receiver templates share exported parameter identities, not bound contracts.
    // Private receiver methods retain their own substitution handles.
    let (generic_parameter_list_id, canonical_by_local) = if let Some(identity) = &receiver_identity
    {
        let receiver_type = intern_generated_canonical_type(
            identity,
            &mut environment.type_environment,
            services.external_registry,
            nominal_source,
            string_table,
            path_fork,
        )?;
        let list_id = match environment.type_environment.get(receiver_type) {
            Some(TypeDefinition::Struct(definition)) => definition.generic_parameters,
            Some(TypeDefinition::Choice(definition)) => definition.generic_parameters,
            _ => None,
        }
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Generated requester generic receiver has no nominal parameter list",
            )
        })?;
        let parameters = environment
            .type_environment
            .generic_parameters(list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated requester generic receiver parameter list is missing",
                )
            })?;
        if parameters.parameters.len() != template.generic_parameters.len() {
            return Err(CompilerError::compiler_error(
                "Generated requester generic receiver parameter count disagrees with its owner",
            ));
        }
        let registered = environment
            .type_environment
            .register_generic_parameter_contract(
                list_id,
                template
                    .generic_parameters
                    .iter()
                    .enumerate()
                    .map(|(slot, parameter)| {
                        (
                            TypeParameterId(slot as u32),
                            string_table.intern(&parameter.name),
                        )
                    }),
                &bounds_by_local,
            )
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated requester template contract disagrees with its parameter owner",
                )
            })?;
        (registered.list_id, registered.canonical_by_local)
    } else {
        let registered = environment
            .type_environment
            .register_generic_parameter_list(
                template
                    .generic_parameters
                    .iter()
                    .enumerate()
                    .map(|(slot, parameter)| {
                        (
                            TypeParameterId(slot as u32),
                            string_table.intern(&parameter.name),
                        )
                    }),
                &bounds_by_local,
            );
        (registered.list_id, registered.canonical_by_local)
    };
    let mut generic_parameter_type_ids = Vec::with_capacity(template.generic_parameters.len());
    for (slot, parameter) in template.generic_parameters.iter().enumerate() {
        let parameter_id = canonical_by_local
            .get(&TypeParameterId(slot as u32))
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated requester template registration omitted a parameter",
                )
            })?;
        let type_id = environment
            .type_environment
            .type_id_for_generic_parameter(*parameter_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated requester template parameter has no type handle",
                )
            })?;
        if let Some(identity) = &parameter.exported_identity {
            environment.type_environment.register_canonical_identity(
                CanonicalTypeIdentity::GenericParameter(identity.clone()),
                type_id,
            )?;
        }
        generic_parameter_type_ids.push(type_id);
    }
    let (signature, _, _, _) = template.signature.materialise(
        &method_path,
        &mut super::artefact_emit::StableFunctionMaterialisationContext {
            generic_parameter_type_ids: &generic_parameter_type_ids,
            nominal_source,
            type_environment: &mut environment.type_environment,
            const_values: &mut *services.const_values,
            external_package_registry: services.external_registry,
            module_resources: Rc::clone(&services.module_resources),
            path_fork,
            string_table,
        },
    )?;
    let receiver = if template.receiver.is_some() {
        Some(super::artefact_emit::materialised_receiver_key(
            &signature,
            &environment.type_environment,
        )?)
    } else {
        None
    };
    let lookups = Rc::make_mut(&mut environment.lookups);
    Rc::make_mut(&mut lookups.resolved_function_signatures_by_path).insert(
        method_path,
        crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature {
            receiver,
            signature: signature.clone(),
        },
    );
    lookups.generic_function_templates_by_path.insert(
        method_path,
        GenericFunctionTemplate {
            function_path: method_path,
            source_file: template.source_file,
            declaration_identity: Some(template.declaration_identity.clone()),
            generic_parameter_owner: template.generic_parameter_owner.clone(),
            generic_parameter_list_id,
            signature,
            body_tokens: None,
            declaration_span: template.declaration_span,
        },
    );
    Ok(())
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

pub(crate) fn bootstrap_call_summary_from_signature(
    signature: &FunctionSignature,
) -> PublicCallSummary {
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| {
            let access = if parameter.value_mode.is_mutable() {
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
    requester_template: &super::stable_types::StableGenericCallableContract,
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
    requester_template: &super::stable_types::StableGenericCallableContract,
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
            receiver_parameter.type_id,
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

    for (requester_parameter, generated_parameter) in requester_template
        .generic_parameters
        .iter()
        .zip(generated_parameters.parameters.iter())
    {
        // Only a parameter the receiver itself determines can be solved here; the body's own
        // inference resolves the rest from its authored arguments and reports any real failure.
        let Some(concrete_argument) = bindings.get(generated_parameter.id) else {
            continue;
        };
        for bound_identity in &requester_parameter.bounds {
            let bound_trait_id = requester_context
                .trait_environment
                .id_for_canonical_identity(bound_identity)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Generated requester template bound has no canonical requester trait",
                    )
                })?;
            let Some(requester_argument_type_id) = requester_evidence_argument_type_id(
                concrete_argument,
                requester_context,
                &environment.type_environment,
            ) else {
                continue;
            };
            let Some(BoundEvidenceSelection::Registered(evidence_id)) = evidence_for_type(
                requester_argument_type_id,
                bound_trait_id,
                &requester_context.type_environment,
                &requester_context.trait_environment,
                &requester_context.trait_evidence_environment,
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
                    value_mode: parameter.value_mode,
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

#[cfg(test)]
#[path = "../tests/sidecar_build_tests.rs"]
mod sidecar_build_tests;
