//! Declaring-module preparation capture and publication freeze.
use super::super::GenericFunctionTemplate;
use super::artefact_emit::ModuleMaterialisationContext;
use super::frozen_syntax::{SharedDonorIdentity, StableBodySyntax};
use super::nominal_blueprints::NominalMaterialisationBlueprint;
use super::semantic_closure::{StableSemanticClosure, stable_body_symbol_names};
use super::sidecar_build::bootstrap_call_summary_from_signature;
use super::stable_types::{
    GenericTemplateArtefact, MaterialisationNominalOriginResolver, MaterialisationNominalSource,
    StableCallableBinding, StableFunctionParameter, StableFunctionSignature,
    StableGenericCallableContract, StableGenericParameter, StableNominalBinding,
    StableParameterContract, StableReceiverKey,
};
use super::visibility::{
    StableExternalSymbol, StableFileVisibility, StableNamespaceBinding, StableReceiverMethod,
    StableVisibleDeclaration,
};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::AstImportedFunctionContract;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleLookups, ResolvedPublicTraitRoot,
};
use crate::compiler_frontend::ast::module_ast::scope_context::Stage0ResolutionFacts;
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ParameterContract};
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
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, FoldedValueProjectionContext, OwnedFoldedString,
    PublicConstTemplate, PublicFoldedValue, convert_const_value_to_folded_value,
};
use crate::compiler_frontend::headers::binding_environment::{
    HeaderBindingEnvironment, SourceDeclarationTarget, SourceFunctionTarget,
};

use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_interface::{
    ConcreteCallSummaryRecord, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicEvidenceRecord, PublicReceiverMethodCategory, PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;
/// Construction-only snapshot used to project stable generic inputs before executable lowering.
pub(crate) struct ModuleMaterialisationCapture {
    pub(crate) string_table: StringTable,
    /// Shares one frozen declaring identity across every retained body captured at detach.
    donor_identity_cache: Rc<OnceCell<SharedDonorIdentity>>,
    /// Original declaring facts remain authoritative when generated builds retain nested bodies.
    inherited_stable_context: Option<ModuleMaterialisationContext>,
    /// Requester-owned declaring modules remain separate from the generated body's provider.
    retained_declaring_contexts: Vec<Arc<ModuleMaterialisationContext>>,
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
    pub(crate) stage0_resolution_facts: Option<Arc<Stage0ResolutionFacts>>,
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) binding_environment: Rc<HeaderBindingEnvironment>,
    pub(crate) imported_functions_by_local_path: FxHashMap<PathId, AstImportedFunctionContract>,
    pub(super) default_const_templates_by_path:
        FxHashMap<PathId, (PublicConstTemplate, Option<OwnedFoldedString>)>,
    // Stable projection borrows the completed lookup owners only until detach consumes capture.
    // Generated builds reconstruct fresh AST tables from the detached canonical facts.
    pub(crate) resolved_struct_fields_by_path: Rc<FxHashMap<PathId, Vec<Declaration>>>,
    pub(crate) resolved_function_signatures_by_path:
        Rc<FxHashMap<PathId, ResolvedFunctionSignature>>,
    pub(crate) generic_function_templates_by_path: FxHashMap<PathId, GenericFunctionTemplate>,
    pub(super) generic_template_paths_by_identity: FxHashMap<GeneratedDeclarationIdentity, PathId>,
    pub(crate) resolved_type_aliases_by_path: Rc<FxHashMap<PathId, ResolvedTypeAlias>>,
    pub(crate) nominal_type_ids_by_path: Rc<FxHashMap<PathId, TypeId>>,
    pub(super) source_nominal_paths: Rc<FxHashSet<PathId>>,
    pub(super) public_trait_paths: Vec<PathId>,
    pub(super) nominal_blueprints:
        FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
    pub(crate) trait_environment: Rc<TraitEnvironment>,
    pub(crate) trait_evidence_environment: TraitEvidenceEnvironment,
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) template_const_loop_iteration_limit: usize,
}

/// Accumulates declaring identities and contracts before the expression-free detach boundary.
pub(crate) struct ModuleMaterialisationPreparationBuilder {
    pub(super) context: ModuleMaterialisationCapture,
}

/// Detached generic inputs. No declaration, default expression or AST lookup owner survives here.
#[derive(Clone)]
pub(crate) struct ModuleMaterialisationPreparation {
    pub(super) stable_context: Arc<ModuleMaterialisationContext>,
    retained_declaring_contexts: Vec<Arc<ModuleMaterialisationContext>>,
    pub(super) callable_contracts: Box<[StableCallableBinding]>,
    pub(super) generic_callable_contracts: Box<[StableGenericCallableContract]>,
    pub(super) nominal_bindings: Box<[StableNominalBinding]>,
    pub(crate) string_table: StringTable,
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) trait_environment: Rc<TraitEnvironment>,
    pub(crate) trait_evidence_environment: TraitEvidenceEnvironment,
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) template_const_loop_iteration_limit: usize,
    pub(super) nominal_blueprints:
        FxHashMap<CanonicalTypeIdentity, NominalMaterialisationBlueprint>,
}

/// Summary convergence mutates only stable facts after the executable AST has been released.
pub(crate) struct ModuleMaterialisationPreparedBuilder {
    pub(super) context: ModuleMaterialisationPreparation,
}

/// Declaring-module facts captured together before AST finalisation releases its local owners.
pub(crate) struct ModuleMaterialisationEnvironmentInput<'a> {
    pub(crate) lookups: &'a AstModuleLookups,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) public_trait_roots: &'a [ResolvedPublicTraitRoot],
    pub(crate) default_const_templates_by_path:
        FxHashMap<PathId, (PublicConstTemplate, Option<OwnedFoldedString>)>,
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
    pub(crate) stage0_resolution_facts: Option<Arc<Stage0ResolutionFacts>>,
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    pub(crate) string_table: &'a StringTable,
    pub(crate) template_const_loop_iteration_limit: usize,
}

/// Transient installer roots shared by closure projection and the artefact that consumes it.
struct TemplateDeclarationSelection<'template> {
    template: &'template GenericFunctionTemplate,
    generic_parameters: Box<[StableGenericParameter]>,
    referenced_names: FxHashSet<String>,
    selected_paths: FxHashSet<PathId>,
    local_declarations: Box<[PathId]>,
}

impl ModuleMaterialisationPreparationBuilder {
    pub(crate) fn from_environment(
        input: ModuleMaterialisationEnvironmentInput<'_>,
    ) -> Result<Self, CompilerError> {
        Ok(Self {
            context: ModuleMaterialisationCapture::from_environment(input)?,
        })
    }

    pub(super) fn inherit_stable_context(
        &mut self,
        context: &ModuleMaterialisationContext,
    ) -> Result<(), CompilerError> {
        for artefact in &context.artefacts {
            self.inherit_nominal_blueprints(artefact)?;
        }
        self.context.inherited_stable_context = Some(context.clone());
        Ok(())
    }

    pub(super) fn inherit_requester_declaring_contexts(
        &mut self,
        requester: &ModuleMaterialisationPreparation,
    ) {
        for context in
            std::iter::once(&requester.stable_context).chain(&requester.retained_declaring_contexts)
        {
            if !self
                .context
                .retained_declaring_contexts
                .iter()
                .any(|retained| Arc::ptr_eq(retained, context))
            {
                self.context
                    .retained_declaring_contexts
                    .push(Arc::clone(context));
            }
        }
    }

    pub(crate) fn context(&self) -> &ModuleMaterialisationCapture {
        &self.context
    }

    pub(crate) fn finish_preparation(
        self,
        const_values: &ConstValueStore,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<ModuleMaterialisationPreparation, CompilerError> {
        Ok(self
            .detach(const_values, &[], &[], resources, path_fork)?
            .context)
    }

    pub(crate) fn finalize_generic_template_identity_index(&mut self) -> Result<(), CompilerError> {
        self.context.rebuild_generic_template_identity_index()
    }

    pub(crate) fn detach(
        self,
        const_values: &ConstValueStore,
        declarations: &[PublicDeclarationRecord],
        evidence: &[PublicEvidenceRecord],
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<ModuleMaterialisationPreparedBuilder, CompilerError> {
        self.context
            .detach(const_values, declarations, evidence, resources, path_fork)
    }

    pub(crate) fn install_concrete_executable_contracts(
        &mut self,
        const_values: &ConstValueStore,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<PathId, OriginFunctionId>,
        public_nominal_origins_by_path: &FxHashMap<PathId, OriginTypeId>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<Vec<(PathId, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.context.install_concrete_executable_contracts(
            const_values,
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

impl ModuleMaterialisationPreparedBuilder {
    pub(crate) fn context(&self) -> &ModuleMaterialisationPreparation {
        &self.context
    }

    pub(crate) fn callables(&self) -> &[StableCallableBinding] {
        &self.context.callable_contracts
    }

    pub(crate) fn update_callable_summary(&mut self, path: &PathId, summary: &PublicCallSummary) {
        self.context.update_callable_summary(path, summary);
    }

    pub(crate) fn freeze(
        mut self,
        public_interface: &PublicSemanticInterface,
    ) -> Result<Option<ModuleMaterialisationContext>, CompilerError> {
        // Public convergence replaces owned facts only. AST/default projection ended at detach.
        let context = Arc::make_mut(&mut self.context.stable_context);
        let mut declarations = std::mem::take(&mut context.declaration_closure).into_vec();
        for declaration in &public_interface.declarations {
            if let Some(existing) = declarations
                .iter_mut()
                .find(|existing| existing.origin == declaration.origin)
            {
                *existing = declaration.clone();
            } else {
                declarations.push(declaration.clone());
            }
        }
        declarations.sort_by(|left, right| left.origin.cmp(&right.origin));
        context.declaration_closure = declarations.into_boxed_slice();
        let mut evidence_records = std::mem::take(&mut context.evidence).into_vec();
        for evidence in &public_interface.reusable_evidence {
            if let Some(existing) = evidence_records
                .iter_mut()
                .find(|existing| existing.identity == evidence.identity)
            {
                *existing = evidence.clone();
            } else {
                evidence_records.push(evidence.clone());
            }
        }
        evidence_records.sort_by(|left, right| left.identity.cmp(&right.identity));
        context.evidence = evidence_records.into_boxed_slice();
        let mut imported_summaries = FxHashMap::default();
        for record in &context.concrete_call_summaries {
            if !public_interface
                .concrete_call_summaries
                .iter()
                .any(|published| published.origin == record.origin)
            {
                imported_summaries.insert(record.origin.clone(), record.summary.clone());
            }
        }
        context.concrete_call_summaries = retained_concrete_call_summaries(
            &imported_summaries,
            &public_interface.concrete_call_summaries,
            &context.declaration_closure,
            &context.evidence,
        )?;
        if context.artefacts.is_empty() {
            return Ok(None);
        }
        Ok(Some(Arc::unwrap_or_clone(self.context.stable_context)))
    }
}

impl ModuleMaterialisationPreparation {
    /// Generated requests intern into the live boundary domain, so retained requester IDs must
    /// still address an exact prefix after provider rebasing extends that domain.
    pub(super) fn validate_requester_string_prefix(
        &self,
        live_string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        let requester_len = self.string_table.len();
        let compatible_prefix = live_string_table.len() >= requester_len
            && self.string_table.iter().zip(live_string_table.iter()).all(
                |((requester_id, requester), (boundary_id, boundary))| {
                    requester_id == boundary_id && requester == boundary
                },
            );
        if !compatible_prefix {
            return Err(CompilerError::compiler_error(
                "generated materialisation requester string table is not an exact prefix of the live boundary string table",
            ));
        }
        Ok(())
    }

    /// Select the exact declaring module, not a bodyless callable restored in another module.
    ///
    /// These contexts already use the live request transaction's identity domain. Keeping the
    /// whole owner preserves its source/resource/frozen authority without merging artefacts
    /// under the generated provider's module origin.
    pub(crate) fn declaring_context_for_identity(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Option<(&ModuleMaterialisationContext, usize)> {
        std::iter::once(&self.stable_context)
            .chain(&self.retained_declaring_contexts)
            .find_map(|context| {
                context.declaration_rows().find_map(|(declaration, index)| {
                    (declaration == identity).then_some((context.as_ref(), index))
                })
            })
    }

    pub(super) fn callable_for_path(&self, path: &PathId) -> Option<&StableCallableBinding> {
        self.callable_contracts
            .iter()
            .find(|contract| &contract.local_path == path)
    }

    pub(super) fn generic_callable_for_path(
        &self,
        path: &PathId,
    ) -> Option<&StableGenericCallableContract> {
        self.generic_callable_contracts
            .iter()
            .find(|contract| &contract.function_path == path)
    }

    pub(crate) fn update_callable_summary(&mut self, path: &PathId, summary: &PublicCallSummary) {
        let stable_context = Arc::make_mut(&mut self.stable_context);
        for contract in &mut self.callable_contracts {
            if &contract.local_path == path {
                contract.summary = summary.clone();
                if let super::stable_types::StableFunctionTarget::Imported(origin) =
                    &contract.target
                {
                    for record in &mut stable_context.concrete_call_summaries {
                        if &record.origin == origin {
                            record.summary = summary.clone();
                        }
                    }
                }
            }
        }
        for artefact in &mut stable_context.artefacts {
            for callable in &mut artefact.callables {
                if &callable.local_path == path {
                    callable.summary = summary.clone();
                }
            }
            for method in &mut artefact.visibility.receiver_methods {
                if &method.local_path == path {
                    method.summary = Some(summary.clone());
                }
            }
        }
        // Private evidence rows keep a copy of the callable they dispatch to, so refresh it with
        // the other summary owners; a stale copy would hide return aliases from consumers that
        // install the evidence.
        stable_context
            .semantic_closure
            .update_callable_summary(path, summary);
    }
}

impl ModuleMaterialisationCapture {
    /// Freeze the declaring string domain once for the stable bodies captured at detach.
    pub(super) fn donor_identity(&self) -> &SharedDonorIdentity {
        self.donor_identity_cache
            .get_or_init(|| SharedDonorIdentity::freeze(&self.string_table))
    }

    fn detach(
        mut self,
        const_values: &ConstValueStore,
        declarations: &[PublicDeclarationRecord],
        public_evidence: &[PublicEvidenceRecord],
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<ModuleMaterialisationPreparedBuilder, CompilerError> {
        self.rebuild_generic_template_identity_index()?;
        let inherited_declarations = self
            .inherited_stable_context
            .iter()
            .flat_map(|context| context.artefacts.iter())
            .map(|artefact| &artefact.declaration_identity)
            .collect::<FxHashSet<_>>();
        let mut templates = self
            .generic_function_templates_by_path
            .values()
            .filter(|template| template.body_tokens.is_some())
            .filter(|template| {
                !template
                    .declaration_identity
                    .as_ref()
                    .is_some_and(|identity| inherited_declarations.contains(identity))
            })
            .collect::<Vec<_>>();
        templates.sort_by(|left, right| left.declaration_identity.cmp(&right.declaration_identity));
        let template_selections = templates
            .into_iter()
            .map(|template| self.template_declaration_selection(const_values, template))
            .collect::<Result<Vec<_>, CompilerError>>()?;
        let retained_local_paths = template_selections
            .iter()
            .flat_map(|selection| selection.local_declarations.iter().copied())
            .collect();

        let mut declaration_closure = self
            .inherited_stable_context
            .as_ref()
            .map(|context| context.declaration_closure.to_vec())
            .unwrap_or_default();
        declaration_closure.extend(
            self.binding_environment
                .imported_declarations_by_origin
                .values()
                .cloned(),
        );
        declaration_closure.extend(declarations.iter().cloned());
        declaration_closure.sort_by(|left, right| left.origin.cmp(&right.origin));
        declaration_closure.dedup_by(|left, right| left.origin == right.origin);

        let mut evidence = self
            .inherited_stable_context
            .as_ref()
            .map(|context| context.evidence.to_vec())
            .unwrap_or_default();
        evidence.extend(
            self.binding_environment
                .imported_evidence_by_identity
                .values()
                .cloned(),
        );
        evidence.extend(public_evidence.iter().cloned());
        evidence.sort_by(|left, right| left.identity.cmp(&right.identity));
        evidence.dedup_by(|left, right| left.identity == right.identity);
        let mut bootstrap_summaries = self
            .binding_environment
            .imported_call_summaries_by_origin
            .clone();
        if let Some(context) = &self.inherited_stable_context {
            for summary in &context.concrete_call_summaries {
                bootstrap_summaries.insert(summary.origin.clone(), summary.summary.clone());
            }
        }
        for contract in self.imported_functions_by_local_path.values() {
            if let SourceFunctionTarget::Imported { origin, .. } = &contract.target {
                bootstrap_summaries
                    .entry(origin.clone())
                    .or_insert_with(|| contract.summary.clone());
            }
        }
        let concrete_call_summaries = retained_concrete_call_summaries(
            &bootstrap_summaries,
            &[],
            &declaration_closure,
            &evidence,
        )?;
        // Private evidence rows carry their method's concrete binding, so the closure reads this
        // table. It is captured once here and reused by the callable detach below.
        let all_callable_paths = self
            .imported_functions_by_local_path
            .keys()
            .copied()
            .collect();
        let callable_contracts =
            self.stable_callable_bindings(const_values, &all_callable_paths, resources, path_fork)?;
        // Empty source modules still supply requester/provider metadata below. Only the closure
        // that freeze would discard is skipped, and bodyless generic contracts keep their lane.
        let mut semantic_closure = if self.generic_function_templates_by_path.is_empty()
            && self.inherited_stable_context.is_none()
        {
            StableSemanticClosure::default()
        } else {
            self.stable_semantic_closure(
                const_values,
                &retained_local_paths,
                &callable_contracts,
                resources,
                path_fork,
            )?
        };
        if let Some(context) = &self.inherited_stable_context {
            semantic_closure.inherit_original(&context.semantic_closure);
        }

        let mut artefacts = template_selections
            .into_iter()
            .map(|selection| {
                self.freeze_template(
                    const_values,
                    selection,
                    declarations,
                    &semantic_closure,
                    resources,
                    path_fork,
                )
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        if let Some(context) = &self.inherited_stable_context {
            artefacts.extend(context.artefacts.iter().cloned());
        }
        artefacts.sort_by(|left, right| left.declaration_identity.cmp(&right.declaration_identity));
        let generic_callable_contracts = self
            .generic_function_templates_by_path
            .values()
            .map(|template| {
                let parameter_slots = self.generic_parameter_slots(template)?;
                let receiver = self
                    .resolved_function_signatures_by_path
                    .get(&template.function_path)
                    .and_then(|resolved| resolved.receiver.as_ref())
                    .map(|receiver| StableReceiverKey::capture(receiver, &self.string_table))
                    .transpose()?;
                Ok(StableGenericCallableContract {
                    declaration_identity: template.declaration_identity.clone().ok_or_else(
                        || {
                            CompilerError::compiler_error(
                                "Generic callable contract has no stable identity",
                            )
                        },
                    )?,
                    generic_parameter_owner: template.generic_parameter_owner.clone(),
                    receiver,
                    function_path: template.function_path,
                    source_file: template.source_file,
                    declaration_span: template.declaration_span,
                    signature: self.stable_function_signature(
                        const_values,
                        &template.signature,
                        &parameter_slots,
                        resources,
                        path_fork,
                    )?,
                    generic_parameters: self.stable_generic_parameters(template)?,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        let nominal_bindings = self.stable_nominal_bindings(&self.nominal_blueprints);
        let stable_context = ModuleMaterialisationContext {
            declaration_closure: declaration_closure.into_boxed_slice(),
            evidence: evidence.into_boxed_slice(),
            concrete_call_summaries,
            semantic_closure,
            artefacts: artefacts.into_boxed_slice(),
            module_origin: self.module_origin.clone(),
            frozen_identity_handle: self.frozen_identity_handle.clone(),
            path_table: None,
            source_string_table: None,
        };
        Ok(ModuleMaterialisationPreparedBuilder {
            context: ModuleMaterialisationPreparation {
                stable_context: Arc::new(stable_context),
                retained_declaring_contexts: self.retained_declaring_contexts,
                string_table: self.string_table,
                callable_contracts,
                generic_callable_contracts,
                nominal_bindings,
                frozen_identity_handle: self.frozen_identity_handle,
                type_environment: self.type_environment,
                trait_environment: self.trait_environment,
                trait_evidence_environment: self.trait_evidence_environment,
                external_package_registry: self.external_package_registry,
                style_directives: self.style_directives,
                build_profile: self.build_profile,
                template_const_loop_iteration_limit: self.template_const_loop_iteration_limit,
                nominal_blueprints: self.nominal_blueprints,
            },
        })
    }

    fn freeze_template(
        &self,
        const_values: &ConstValueStore,
        selection: TemplateDeclarationSelection<'_>,
        declarations: &[PublicDeclarationRecord],
        semantic_closure: &StableSemanticClosure,
        resources: &ModuleResourceTable,
        path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Result<GenericTemplateArtefact, CompilerError> {
        let TemplateDeclarationSelection {
            template,
            generic_parameters,
            referenced_names,
            selected_paths,
            local_declarations,
        } = selection;
        // Share one lazily frozen declaring-domain owner across every artefact in this freeze.
        let donor_identity = self.donor_identity();
        let declaration_identity = template.declaration_identity.clone().ok_or_else(|| {
            CompilerError::compiler_error(
                "Retained generic template has no stable declaration identity",
            )
        })?;
        let body = template.body_tokens.as_ref().ok_or_else(|| {
            CompilerError::compiler_error("Retained generic template has no body syntax")
        })?;
        let generic_parameter_owner = template.generic_parameter_owner.clone();
        let receiver = self
            .resolved_function_signatures_by_path
            .get(&template.function_path)
            .and_then(|resolved| resolved.receiver.as_ref())
            .map(|receiver| StableReceiverKey::capture(receiver, &self.string_table))
            .transpose()?;
        let parameter_slots = self.generic_parameter_slots(template)?;
        let signature = self.stable_function_signature(
            const_values,
            &template.signature,
            &parameter_slots,
            resources,
            path_fork,
        )?;
        let visibility = self.stable_file_visibility(
            const_values,
            &template.source_file,
            &referenced_names,
            resources,
            path_fork,
        )?;
        let declarations = self.stable_declaration_bindings(&selected_paths, declarations)?;
        let callable_paths = selected_paths
            .iter()
            .copied()
            .filter(|path| {
                self.resolved_function_signatures_by_path
                    .get(path)
                    .is_some_and(|resolved| resolved.receiver.is_none())
            })
            .collect();
        let callables =
            self.stable_callable_bindings(const_values, &callable_paths, resources, path_fork)?;
        // Retained contracts can mention nominals never named in the generic body's syntax.
        let retained_signatures = std::iter::once(&signature)
            .chain(callables.iter().map(|callable| &callable.signature))
            .chain(
                visibility
                    .receiver_methods
                    .iter()
                    .map(|method| &method.signature),
            );
        let nominal_blueprints = self.stable_nominal_blueprints(
            const_values,
            &selected_paths,
            retained_signatures,
            semantic_closure,
            resources,
            path_fork,
        )?;
        // Keep every already-resolved spelling of the retained nominals, not just body-visible
        // names. Imported canonical paths must survive nested sidecars and global publication.
        let nominals = self.stable_nominal_bindings(&nominal_blueprints);
        let content_value_at_path = |logical_path: &PathId| {
            let content_path =
                self.content_constant_path_for_capture(const_values, logical_path, path_fork)?;
            self.stable_folded_value_at_path(const_values, &content_path, resources, path_fork)
        };
        let stage0_resolution_facts = body
            .resolution_facts()
            .map(std::sync::Arc::as_ref)
            .or(self.stage0_resolution_facts.as_deref());
        let frozen_identity_handle = body
            .frozen_identity_handle()
            .cloned()
            .unwrap_or_else(|| self.frozen_identity_handle.clone());

        Ok(GenericTemplateArtefact {
            declaration_identity,
            generic_parameter_owner,
            receiver,
            function_path: template.function_path,
            source_file: template.source_file,
            declaration_span: template.declaration_span,
            body: StableBodySyntax::capture(
                body,
                template.source_file,
                path_fork,
                Some(donor_identity),
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

    fn template_declaration_selection<'template>(
        &self,
        const_values: &ConstValueStore,
        template: &'template GenericFunctionTemplate,
    ) -> Result<TemplateDeclarationSelection<'template>, CompilerError> {
        let body = template.body_tokens.as_ref().ok_or_else(|| {
            CompilerError::compiler_error("Retained generic template has no body syntax")
        })?;
        let generic_parameters = self.stable_generic_parameters(template)?;
        let capture_string_table =
            body.capture_string_table(self.donor_identity().strings().as_ref());
        let mut referenced_names = stable_body_symbol_names(body, capture_string_table)?;
        self.retain_generic_bound_trait_names(
            &template.source_file,
            &generic_parameters,
            &mut referenced_names,
        )?;
        let selection_before_bounds =
            self.selected_visible_paths(&template.source_file, &referenced_names)?;
        let names_before_bounds = referenced_names.len();
        self.retain_selected_bound_trait_names(
            &template.source_file,
            &selection_before_bounds,
            &mut referenced_names,
        )?;
        // Retained names only grow, so an unchanged count means the selection above still holds.
        // Bound spellings can select more paths, so only then is the selection rebuilt.
        let selected_paths = if referenced_names.len() == names_before_bounds {
            selection_before_bounds
        } else {
            self.selected_visible_paths(&template.source_file, &referenced_names)?
        };
        let local_declarations =
            self.stable_local_declaration_bindings(const_values, &selected_paths);
        Ok(TemplateDeclarationSelection {
            template,
            generic_parameters,
            referenced_names,
            selected_paths,
            local_declarations,
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
        const_values: &ConstValueStore,
        value_id: ConstValueId,
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
        convert_const_value_to_folded_value(const_values, value_id, &folded_value_context)
    }
    pub(super) fn content_constant_path_for_capture(
        &self,
        const_values: &ConstValueStore,
        logical_path: &PathId,
        path_fork: &PathInternerFork,
    ) -> Result<PathId, CompilerError> {
        const_values
            .module_constant_paths()
            .find(|path| {
                let candidate = **path;
                path_fork.try_parent(candidate) == Some(*logical_path)
                    && path_fork
                        .component(candidate)
                        .map(|name| self.string_table.resolve(name) == "content")
                        .unwrap_or(false)
            })
            .copied()
            .ok_or_else(|| {
                let mut scratch = Vec::new();
                let rendered =
                    path_fork.render_portable(*logical_path, &self.string_table, &mut scratch);
                CompilerError::compiler_error(format!(
                    "synthetic content constant for {} was not present before generic capture",
                    rendered
                ))
            })
    }
    pub(super) fn stable_folded_value_at_default_path(
        &self,
        const_values: &ConstValueStore,
        path: &PathId,
        value_id: ConstValueId,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<PublicFoldedValue, CompilerError> {
        if self.default_const_templates_by_path.contains_key(path) {
            return self
                .default_const_templates_by_path
                .get(path)
                .cloned()
                .map(|(template, folded)| PublicFoldedValue::ConstTemplate { template, folded })
                .ok_or_else(|| {
                    let mut scratch = Vec::new();
                    let rendered =
                        path_fork.render_portable(*path, &self.string_table, &mut scratch);
                    CompilerError::compiler_error(format!(
                        "Retained const template at {} has no stable folded template value",
                        rendered
                    ))
                });
        }
        self.stable_folded_value(const_values, value_id, resources, path_fork)
    }

    pub(super) fn stable_folded_value_at_path(
        &self,
        const_values: &ConstValueStore,
        path: &PathId,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<PublicFoldedValue, CompilerError> {
        let value_id = const_values.value_for_path(path).ok_or_else(|| {
            let mut scratch = Vec::new();
            let rendered = path_fork.render_portable(*path, &self.string_table, &mut scratch);
            CompilerError::compiler_error(format!(
                "Retained module constant at {} has no stable folded store value",
                rendered
            ))
        })?;
        const_values.metadata(value_id).ok_or_else(|| {
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
        convert_const_value_to_folded_value(const_values, value_id, &folded_value_context)
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
                if !self.retain_visible_trait_spellings(source_file, trait_id, referenced_names)? {
                    return Err(CompilerError::compiler_error(
                        "Retained generic bound is not visible in its declaration file",
                    ));
                }
            }
        }

        Ok(())
    }

    /// Retains the trait spellings that generic declarations selected for the body need in the
    /// body's file. Generic callees and generic nominal constructors are both covered.
    ///
    /// Checking a declaration's bounds needs the spellings its bounds were written with, in the
    /// file of the body that uses it. Those spellings sit in the declaration's header, outside the
    /// retained body tokens, so they are retained here. Referenced names over-approximate the uses:
    /// a shadowing local, or a field with the same name, can select a declaration the body never
    /// uses, and that declaration's bound may be unnamed in this file. Such bounds are skipped, not
    /// rejected, because a use the body really makes had to name its bound in the original file.
    ///
    /// `selected_paths` is the selection taken before these spellings were added. Bound spellings
    /// name traits, which carry no generic bounds of their own, so one reselection by the caller
    /// is enough.
    fn retain_selected_bound_trait_names(
        &self,
        source_file: &PathId,
        selected_paths: &FxHashSet<PathId>,
        referenced_names: &mut FxHashSet<String>,
    ) -> Result<(), CompilerError> {
        for path in selected_paths {
            let parameter_list_id = self
                .generic_function_templates_by_path
                .get(path)
                .map(|callee| callee.generic_parameter_list_id)
                .or_else(|| {
                    let type_id = self.nominal_type_ids_by_path.get(path)?;
                    self.type_environment
                        .generic_parameter_list_id_for_type(*type_id)
                });
            let Some(parameter_list_id) = parameter_list_id else {
                continue;
            };
            let parameters = self
                .type_environment
                .generic_parameters(parameter_list_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Retained generic declaration references a missing parameter list",
                    )
                })?;
            for trait_id in parameters
                .parameters
                .iter()
                .flat_map(|parameter| parameter.trait_bounds.iter())
            {
                // An unnamed bound is skipped on purpose; see the doc comment above.
                self.retain_visible_trait_spellings(source_file, *trait_id, referenced_names)?;
            }
        }

        Ok(())
    }

    /// Inserts each spelling the declaring file gives `trait_id`. Reports whether there was one.
    fn retain_visible_trait_spellings(
        &self,
        source_file: &PathId,
        trait_id: TraitId,
        referenced_names: &mut FxHashSet<String>,
    ) -> Result<bool, CompilerError> {
        let visibility = self.binding_environment.visibility_for(source_file)?;
        let mut retained_visible_name = false;
        for (visible_name, target) in &visibility.visible_trait_names {
            if self
                .trait_environment
                .has_path(trait_id, target.local_path())
            {
                referenced_names.insert(self.string_table.resolve(*visible_name).to_owned());
                retained_visible_name = true;
            }
        }
        Ok(retained_visible_name)
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
        const_values: &ConstValueStore,
        signature: &FunctionSignature,
        parameter_slots: &FxHashMap<GenericParameterId, usize>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<StableFunctionSignature, CompilerError> {
        let paths = path_fork;
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
                    value_mode: parameter.value_mode,
                    folded_default: parameter
                        .default
                        .map(|value_id| {
                            self.stable_folded_value_at_default_path(
                                const_values,
                                &parameter.id,
                                value_id,
                                resources,
                                path_fork,
                            )
                        })
                        .transpose()?,
                    parameter_type: self
                        .materialisation_type_blueprint(parameter.type_id, parameter_slots)?,
                    contract: match parameter.contract {
                        ParameterContract::Value => StableParameterContract::Value,
                        ParameterContract::Wire => StableParameterContract::Wire,
                        ParameterContract::Route { input } => StableParameterContract::Route {
                            input: input
                                .map(|type_id| {
                                    self.materialisation_type_blueprint(type_id, parameter_slots)
                                })
                                .transpose()?,
                        },
                    },
                    span: parameter.binding_span,
                })
            })
            .collect::<Result<Box<[_]>, CompilerError>>()?;
        let mut returns = Vec::new();
        let mut error_return = None;
        for returned in &signature.returns {
            let type_id = returned.type_id.ok_or_else(|| {
                CompilerError::compiler_error(
                    "Materialisation function return has no resolved type",
                )
            })?;
            let blueprint = self.materialisation_type_blueprint(type_id, parameter_slots)?;
            match returned.channel {
                crate::compiler_frontend::ast::statements::functions::ReturnChannel::Success => {
                    returns.push(blueprint);
                }
                crate::compiler_frontend::ast::statements::functions::ReturnChannel::Error => {
                    error_return = Some(blueprint);
                }
            }
        }
        Ok(StableFunctionSignature {
            parameters,
            returns: returns.into_boxed_slice(),
            error_return,
        })
    }

    fn stable_file_visibility(
        &self,
        const_values: &ConstValueStore,
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
                            local_path: *target.local_path(),
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
                        const_values,
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

    /// Freeze stable targets for every concrete executable visible to generated bodies.
    ///
    /// Public functions retain their interface identity. Concrete local helpers receive a
    /// distinct artefact-scoped identity and are projected as imported contracts when a generic
    /// body is materialised in an independent sidecar.
    fn install_concrete_executable_contracts(
        &mut self,
        const_values: &ConstValueStore,
        module_origin: &crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity,
        public_origins_by_path: &FxHashMap<PathId, OriginFunctionId>,
        public_nominal_origins_by_path: &FxHashMap<PathId, OriginTypeId>,
        resources: &ModuleResourceTable,
        path_fork: &PathInternerFork,
    ) -> Result<Vec<(PathId, ModulePrivateExecutableIdentity)>, CompilerError> {
        self.install_private_semantic_identities(
            module_origin,
            public_nominal_origins_by_path,
            path_fork,
        )?;
        self.install_nominal_blueprints(const_values, resources, path_fork)?;
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
                    path_fork,
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
            .map(|(path, resolved)| (*path, resolved.clone()))
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
                    local_path: path,
                }
            } else {
                let category = if resolved.receiver.is_some() {
                    ModulePrivateExecutableCategory::ReceiverMethod
                } else {
                    ModulePrivateExecutableCategory::FreeFunction
                };
                let identity = self.private_executable_identity(
                    module_origin,
                    &path,
                    &resolved,
                    category,
                    path_fork,
                )?;
                private_executables.push((path, identity.clone()));
                SourceFunctionTarget::ModulePrivate {
                    identity,
                    local_path: path,
                }
            };

            self.imported_functions_by_local_path.insert(
                path,
                AstImportedFunctionContract {
                    target,
                    summary: bootstrap_call_summary_from_signature(&resolved.signature),
                    returns: resolved.signature.success_return_type_ids(),
                    error_return: resolved.signature.error_return_type_id(),
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
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        let nominal_types = self
            .nominal_type_ids_by_path
            .iter()
            .map(|(path, type_id)| (*path, *type_id))
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
                let mut scratch = Vec::new();
                path_fork.render_portable(path, &self.string_table, &mut scratch)
            };
            let identity =
                ModulePrivateNominalIdentity::new(module_origin.clone(), defining_path, category);
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
                let defining_name = path_fork.component(*path).ok_or_else(|| {
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
                Rc::make_mut(&mut self.trait_environment).register_canonical_identity(
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
                    let mut scratch = Vec::new();
                    path_fork.render_portable(
                        definition.canonical_path,
                        &self.string_table,
                        &mut scratch,
                    )
                };
                Ok((definition.id, defining_path))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        for (trait_id, defining_path) in private_traits {
            Rc::make_mut(&mut self.trait_environment).register_canonical_identity(
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
        path_fork: &PathInternerFork,
    ) -> Result<ModulePrivateExecutableIdentity, CompilerError> {
        let paths = path_fork;
        let receiver_path = match resolved.receiver.as_ref() {
            Some(ReceiverKey::Struct(receiver) | ReceiverKey::Choice(receiver)) => {
                let mut scratch = Vec::new();
                Some(paths.render_portable(*receiver, &self.string_table, &mut scratch))
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
            type_environment,
            public_trait_roots,
            default_const_templates_by_path,
            module_origin,
            stage0_resolution_facts,
            frozen_identity_handle,
            string_table,
            template_const_loop_iteration_limit,
        } = input;

        Ok(Self {
            string_table: string_table.clone_preserving_inherited_prefix(),
            donor_identity_cache: Rc::new(OnceCell::new()),
            inherited_stable_context: None,
            retained_declaring_contexts: Vec::new(),
            module_origin,
            stage0_resolution_facts,
            frozen_identity_handle,
            type_environment: type_environment.fork_for_generated(),
            binding_environment: Rc::clone(&lookups.binding_environment),
            imported_functions_by_local_path: lookups.imported_functions_by_local_path.clone(),
            default_const_templates_by_path,
            // Completed immutable tables keep the environment's existing `Rc` owner.
            resolved_struct_fields_by_path: Rc::clone(&lookups.resolved_struct_fields_by_path),
            resolved_function_signatures_by_path: Rc::clone(
                &lookups.resolved_function_signatures_by_path,
            ),
            generic_function_templates_by_path: lookups.generic_function_templates_by_path.clone(),
            generic_template_paths_by_identity: FxHashMap::default(),
            resolved_type_aliases_by_path: Rc::clone(&lookups.resolved_type_aliases_by_path),
            nominal_type_ids_by_path: Rc::clone(&lookups.nominal_type_ids_by_path),
            source_nominal_paths: Rc::clone(&lookups.source_nominal_paths),
            public_trait_paths: public_trait_roots
                .iter()
                .map(|root| root.canonical_path)
                .collect(),
            nominal_blueprints: FxHashMap::default(),
            // Trait identities may fork this owner during preparation; evidence mutates per request.
            trait_environment: Rc::clone(&lookups.trait_environment),
            trait_evidence_environment: (*lookups.trait_evidence_environment).clone(),
            external_package_registry: Arc::clone(&lookups.external_package_registry),
            style_directives: lookups.style_directives.clone(),
            build_profile: lookups.build_profile,
            template_const_loop_iteration_limit,
        })
    }
}

/// Retain the exact published summaries needed to reconstruct reusable evidence dispatch.
///
/// Generic receiver templates have no base summary. Concrete methods keep the provider's
/// analysed summary, independently of the template's ordinary file visibility.
fn retained_concrete_call_summaries(
    imported_summaries: &FxHashMap<OriginFunctionId, PublicCallSummary>,
    published_summaries: &[ConcreteCallSummaryRecord],
    declaration_closure: &[PublicDeclarationRecord],
    evidence: &[PublicEvidenceRecord],
) -> Result<Box<[ConcreteCallSummaryRecord]>, CompilerError> {
    let mut evidenced_origins = evidence
        .iter()
        .flat_map(|record| record.requirement_mappings.iter())
        .map(|mapping| &mapping.method_origin)
        .collect::<Vec<_>>();
    evidenced_origins.sort_unstable();
    evidenced_origins.dedup();
    if evidenced_origins.is_empty() {
        return Ok(Box::new([]));
    }

    let generic_templates = generic_template_receiver_origins(declaration_closure);
    let mut retained = Vec::with_capacity(evidenced_origins.len());
    for origin in evidenced_origins {
        let imported_summary = imported_summaries.get(origin);
        let published_summary = published_summaries
            .binary_search_by(|record| record.origin.cmp(origin))
            .ok()
            .map(|index| &published_summaries[index].summary);
        if let (Some(imported), Some(published)) = (imported_summary, published_summary)
            && imported != published
        {
            return Err(CompilerError::compiler_error(format!(
                "materialisation concrete call summary {origin:?} disagrees between imported provider records and this module's published interface"
            )));
        }
        let Some(summary) = imported_summary.or(published_summary) else {
            if generic_templates.contains(origin) {
                continue;
            }
            return Err(CompilerError::compiler_error(format!(
                "retained reusable evidence names receiver method {origin:?} but neither an imported provider interface nor this module's published interface carries its call summary"
            )));
        };
        retained.push(ConcreteCallSummaryRecord {
            origin: origin.clone(),
            summary: summary.clone(),
        });
    }
    Ok(retained.into_boxed_slice())
}

/// Collect every receiver-method origin the closed declaration records mark as a generic template.
///
/// WHAT: walks the module-wide declaration closure's struct and choice records for
///       [`PublicReceiverMethodCategory::GenericTemplate`] methods.
/// WHY: a generic template never receives a base concrete summary, so it is the one legitimate
///      evidence method origin without a retained summary. Keeping that classification explicit
///      lets summary retention reject a genuinely missing concrete summary instead of treating
///      it as another template.
fn generic_template_receiver_origins(
    declaration_closure: &[PublicDeclarationRecord],
) -> FxHashSet<&OriginFunctionId> {
    let mut origins = FxHashSet::default();
    for record in declaration_closure {
        let methods = match &record.semantics {
            PublicDeclarationSemantics::Struct(structure) => &structure.receiver_methods,
            PublicDeclarationSemantics::Choice(choice) => &choice.receiver_methods,
            _ => continue,
        };
        for method in methods {
            if matches!(
                method.category,
                PublicReceiverMethodCategory::GenericTemplate(_)
            ) {
                origins.insert(&method.method_origin);
            }
        }
    }
    origins
}
