//! Final AST assembly.
//!
//! WHAT: owns HIR-boundary cleanup and final [`AstBuildResult`] construction, separating the
//! executable [`Ast`] from the public-interface and generic-template side results.
//! WHY: environment building and node emission should finish before template/module-constant
//! normalization mutates or packages the final AST output.
//!
//! [`AstBuildResult`]: crate::compiler_frontend::ast::AstBuildResult

use super::super::build_context::AstPhaseContext;
use super::super::emission::AstEmission;
use super::super::environment::AstModuleEnvironment;
use super::const_fact_collection::ConstFactCollector;
use super::normalize_ast::{TemplateNormalizationError, discard_inactive_assertion_messages};
use super::static_if_specialization::{StaticIfCandidate, StaticIfSpecialization};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStoreError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionFreezeRoots, PlaceId,
};
use crate::compiler_frontend::ast::expressions::failure_classification::AstBuiltinFailureSummaries;
use crate::compiler_frontend::ast::expressions::failure_facts::ImplicitFailureSource;
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestSlot, ModuleMaterialisationEnvironmentInput,
    ModuleMaterialisationPreparationBuilder,
};
use crate::compiler_frontend::ast::statements::terminality::{
    terminality_policy_for_signature, validate_function_body_terminality,
};
use crate::compiler_frontend::ast::templates::top_level_templates::{
    collect_and_strip_comment_templates, collect_const_top_level_fragments,
};
use crate::compiler_frontend::ast::{
    Ast, AstBuildResult, AstChoiceDefinition, AstPublicInterfaceProjectionInput,
};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerMessages, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::CoreTraitKind;
use crate::compiler_frontend::traits::evidence::environment::InfallibleCastEvidenceContract;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use crate::timing_scope_attributed_opt;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

#[cfg(debug_assertions)]
use super::debug_type_validation::debug_validate_type_ids_for_hir;

/// Orchestrates the final AST assembly phase.
///
/// WHAT: consumes the resolved module environment and emitted AST nodes to
/// produce a fully normalized, validated [`Ast`] ready for HIR lowering.
///
/// WHY: separates finalization orchestration from environment building and
/// node emission so each phase has a single, clear responsibility.
pub(in crate::compiler_frontend::ast) struct AstFinalizer<'context, 'services> {
    pub(super) context: AstPhaseContext<'services>,
    pub(super) environment: AstModuleEnvironment,
    pub(super) path_fork:
        &'context mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
}

/// When AST finalization builds the declaring-module generic materialisation context.
///
/// WHAT: modules always retain the context, while a generated sidecar retains it only as the
///       requester of the nested generic requests its own body deferred.
/// WHY: the context copies module-wide lookup state. Building it for every generated sidecar
///      made generic instantiation quadratic in module size, although only nested requests read it.
pub(in crate::compiler_frontend::ast) enum MaterialisationContextRetention {
    Always,
    ForDeferredRequests,
}

impl ExpressionFreezeRoots for Vec<AstNode> {
    fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId)) {
        for node in self {
            node.visit_expression_ids(visitor);
        }
    }

    fn visit_expression_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut ExprId)) {
        for node in self {
            node.visit_expression_ids_mut(visitor);
        }
    }

    fn visit_place_ids(&self, visitor: &mut dyn FnMut(PlaceId)) {
        for node in self {
            node.visit_place_ids(visitor);
        }
    }

    fn visit_place_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut PlaceId)) {
        for node in self {
            node.visit_place_ids_mut(visitor);
        }
    }
}

impl<'context, 'services> AstFinalizer<'context, 'services> {
    /// Creates a finalizer owning the phase context and borrowing the path table.
    pub(in crate::compiler_frontend::ast) fn new(
        context: AstPhaseContext<'services>,
        environment: AstModuleEnvironment,
        path_fork: &'context mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    ) -> Self {
        Self {
            context,
            environment,
            path_fork,
        }
    }

    /// Assembles the final [`AstBuildResult`] from emitted nodes and the resolved environment.
    ///
    /// WHAT: runs template normalization, module-constant normalization, type-boundary
    /// validation, const-fact collection, builtin merging, and choice-definition gathering
    /// in dependency order.
    ///
    /// WHY: each step mutates or consumes intermediate state that later steps depend on,
    /// so they must run sequentially in a single orchestration function.
    ///
    /// [`AstBuildResult`]: crate::compiler_frontend::ast::AstBuildResult
    pub(in crate::compiler_frontend::ast) fn finalize(
        mut self,
        mut emitted: AstEmission,
        top_level_const_fragments: &[TopLevelConstFragment],
        materialisation_retention: MaterialisationContextRetention,
        string_table: &mut StringTable,
    ) -> Result<AstBuildResult, CompilerMessages> {
        // ----------------------------
        //  Collect doc fragments
        // ----------------------------
        let doc_fragments = collect_and_strip_comment_templates(
            &mut emitted.ast,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
            string_table,
            Some(self.context.external_package_registry.as_ref()),
            self.context.template_const_loop_iteration_limit,
            Rc::clone(&self.context.template_ir_store),
        )
        .map_err(TemplateNormalizationError::from)
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;

        // ----------------------------
        //  Collect const top-level fragments
        // ----------------------------
        let const_top_level_fragments = collect_const_top_level_fragments(
            top_level_const_fragments,
            &emitted.const_templates_by_path,
        )
        .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;

        // ----------------------------
        //  Normalize module constants
        // ----------------------------
        timing_scope_attributed_opt!(
            _module_constant_guard,
            self.context.timing_metric_family.module_constant(),
            self.context.timing_context
        );
        let default_const_templates =
            self.project_default_const_templates(string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
        // Content has been owned by the phase since declaration/default construction. Module
        // rows bind header-resolved values into that same owner, never a second graph.
        let mut const_values = std::mem::take(&mut self.context.const_values);
        const_values
            .populate_from_declaration_table(
                &self.environment.lookups.declaration_table,
                &self.environment.lookups.resolved_module_constants,
            )
            .map_err(|error| {
                self.const_value_store_error_messages(error, &emitted.warnings, string_table)
            })?;
        self.context.const_values = const_values;

        // ----------------------------
        //  Prepare static-control-flow candidate
        // ----------------------------
        // Selection runs transactionally on a projection. When it finds a known Bool, the
        // authored AST remains the validation authority and the candidate becomes durable only
        // after every authored template and type boundary has passed below.
        let mut static_candidate = StaticIfCandidate::prepare(
            &emitted.ast,
            &mut self.context.const_values,
            &mut self.context.expression_builder,
            Rc::clone(&self.context.template_ir_store),
            string_table,
            &self.environment.type_environment,
            Some(self.context.external_package_registry.as_ref()),
        )
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;

        if static_candidate.has_selections() {
            // Normalize and validate the untouched authored tree first. This copy is never
            // published, but must still satisfy the ordinary AST-to-HIR boundary.
            self.normalize_ast_templates_for_hir(&mut emitted.ast, string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
            self.validate_no_unresolved_executable_types(&emitted.ast, &self.context.const_values)
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;

            // The candidate owns its exact annotated TIR contexts. Normalize those active views,
            // validate the completed HIR-boundary shape and publish the candidate atomically.
            self.normalize_ast_templates_for_hir(static_candidate.ast_mut(), string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
            self.validate_no_unresolved_executable_types(
                static_candidate.ast(),
                &self.context.const_values,
            )
            .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
        } else {
            // The common runtime-only path retains the existing single normalization owner. The
            // unchanged projection is discarded without allocating normalization overlays.
            self.normalize_ast_templates_for_hir(&mut emitted.ast, string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
            self.validate_no_unresolved_executable_types(&emitted.ast, &self.context.const_values)
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
        }

        let mut specialization = static_candidate.publish(&mut emitted.ast);
        let static_if_function_provenance = specialization.function_provenance().clone();

        // The authored assertion message must remain available to the authoritative AST
        // type/TIR boundary validation above. Only after that validation succeeds may AST
        // finalization discard compile-time-inactive executable message state.
        discard_inactive_assertion_messages(
            &mut emitted.ast,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
        )
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;

        let start_function_path = self.context.root_role.has_implicit_start().then(|| {
            self.path_fork
                .try_intern_child(
                    self.context.entry_dir,
                    string_table.intern(IMPLICIT_START_FUNC_NAME),
                )
                .expect("path table exhausted while creating implicit start function path")
        });

        // ----------------------------
        //  Specialise static Bool control flow
        // ----------------------------
        let template_specialization = StaticIfSpecialization::run(
            &mut emitted.validated_generic_template_bodies,
            &mut self.context.const_values,
            &mut self.context.expression_builder,
            Rc::clone(&self.context.template_ir_store),
            string_table,
            &self.environment.type_environment,
            Some(self.context.external_package_registry.as_ref()),
        )
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;
        specialization.merge(template_specialization);
        self.validate_specialized_function_terminality(
            &emitted.ast,
            start_function_path,
            &emitted.warnings,
            string_table,
        )?;
        self.validate_specialized_function_terminality(
            &emitted.validated_generic_template_bodies,
            None,
            &emitted.warnings,
            string_table,
        )?;
        let infallible_cast_evidence = self
            .collect_infallible_cast_evidence(&emitted.ast)
            .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
        let has_assertion_validation_candidates = emitted
            .deferred_generic_requests
            .iter()
            .any(|slot| matches!(slot, GenericRequestSlot::AssertionValidation(_)));
        let assertion_validation_paths = self.validate_generic_builtin_failure_boundaries(
            &emitted.validated_generic_template_bodies,
            &emitted.ast,
            &infallible_cast_evidence,
            has_assertion_validation_candidates,
            &emitted.warnings,
            string_table,
        )?;
        let committed_generic_requests = specialization
            .commit_generic_requests(
                std::mem::take(&mut emitted.deferred_generic_requests),
                &assertion_validation_paths,
            )
            .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;

        // ----------------------------
        //  Synchronize finalized public defaults
        // ----------------------------
        // Defaults are already folded handles from their declaration boundary. Synchronize
        // those canonical handles into public and receiver contracts, without folding again.
        self.synchronize_normalized_public_defaults(&emitted.ast)
            .map_err(|error| {
                self.template_normalization_error_messages(error, &emitted.warnings, string_table)
            })?;

        // ----------------------------
        //  Collect active const facts
        // ----------------------------
        // Const-fact collection reads template values through their exact module-local effective
        // views after static selection, so inactive bodies publish no executable advisory facts.
        let const_facts = ConstFactCollector::new(
            string_table,
            &mut self.context.const_values,
            &mut self.context.expression_builder,
            Rc::clone(&self.context.template_ir_store),
            &self.environment.type_environment,
            Some(self.context.external_package_registry.as_ref()),
        )
        .collect(&emitted.ast, start_function_path.as_ref())
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;

        let mut choice_definitions = self.collect_choice_definitions();
        for imported in &self.environment.lookups.imported_choice_definitions {
            if !choice_definitions
                .iter()
                .any(|definition| definition.nominal_path == imported.nominal_path)
            {
                choice_definitions.push(imported.clone());
            }
        }

        let AstModuleEnvironment {
            lookups,
            type_environment,
            resolved_public_type_roots,
            resolved_public_trait_roots,
        } = self.environment;

        // Emission is complete, so the environment lookups `Rc` is the sole strong reference.
        // Recovering owned access lets the donor-local generic-template map and the builtin
        // struct nodes move directly into the build result without cloning body tokens or an
        // `Rc::try_unwrap` dance in semantic orchestration.
        let owned_lookups = match Rc::try_unwrap(lookups) {
            Ok(lookups) => lookups,
            Err(shared) => {
                let error = CompilerError::compiler_error(format!(
                    "AST finalization: the module environment lookups has {} remaining \
                     reference(s) after emission; a non-unique environment is an internal \
                     invariant violation",
                    Rc::strong_count(&shared)
                ));
                return Err(CompilerMessages::from_error_with_warnings(
                    error,
                    emitted.warnings,
                    string_table,
                )
                .with_type_context_for_all_diagnostics(type_environment));
            }
        };

        let retain_materialisation_context = match materialisation_retention {
            MaterialisationContextRetention::Always => true,
            MaterialisationContextRetention::ForDeferredRequests => {
                !committed_generic_requests.executable.is_empty()
                    || !committed_generic_requests.assertion_validation.is_empty()
            }
        };
        let materialisation_context = if retain_materialisation_context {
            let context = ModuleMaterialisationPreparationBuilder::from_environment(
                ModuleMaterialisationEnvironmentInput {
                    lookups: &owned_lookups,
                    type_environment: &type_environment,
                    public_trait_roots: &resolved_public_trait_roots,
                    default_const_templates_by_path: default_const_templates,
                    module_origin: self
                        .context
                        .file_value_resolution
                        .as_ref()
                        .and_then(|services| services.module_origin.clone()),
                    stage0_resolution_facts: self
                        .context
                        .file_value_resolution
                        .as_ref()
                        .and_then(|services| services.stage0_resolution_facts.clone()),
                    frozen_identity_handle: self
                        .context
                        .file_value_resolution
                        .as_ref()
                        .map(|services| services.frozen_identity_handle.clone())
                        .unwrap_or_else(FrozenIdentityHandle::new),
                    string_table,
                    template_const_loop_iteration_limit: self
                        .context
                        .template_const_loop_iteration_limit,
                },
            )
            .map_err(|error| {
                CompilerMessages::from_error_with_warnings(
                    error,
                    emitted.warnings.clone(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(type_environment.clone())
            })?;
            Some(context)
        } else {
            None
        };

        // ----------------------------
        //  Merge builtin AST nodes
        // ----------------------------
        // Generic capture projects nominal inputs from resolved facts, not builtin AST nodes.
        // Those definitions have only the executable AST reader now, so move them directly.
        // Builtin definitions stay ahead of every authored node.
        if !owned_lookups.builtin_struct_ast_nodes.is_empty() {
            let mut ast_nodes = owned_lookups.builtin_struct_ast_nodes;
            ast_nodes.extend(emitted.ast);
            emitted.ast = ast_nodes;
        }

        #[cfg(feature = "benchmark_counters")]
        super::expression_census::record_published_expression_census(
            &self.context.expression_builder,
        );

        #[cfg(debug_assertions)]
        {
            // Runs after the builtin merge so the boundary check walks the complete
            // builtin+source AST. The shared module store is the authority for finalized views.
            let template_ir_store = self.context.template_ir_store.borrow();
            debug_validate_type_ids_for_hir(
                &emitted.ast,
                &self.context.const_values,
                &choice_definitions,
                &type_environment,
                &template_ir_store,
                &self.context.expression_builder,
            );
        }
        // Validation-only roots stay available through every TIR, failure and debug reader.
        // Only completed executable roots enter the consuming freeze after this barrier.
        emitted.validated_generic_template_bodies.clear();
        let AstPhaseContext {
            template_ir_store,
            expression_builder,
            const_values,
            file_value_resolution,
            entry_dir,
            root_role,
            ..
        } = self.context;
        let template_ir_store = Rc::try_unwrap(template_ir_store).map_err(|shared| {
            CompilerMessages::from_error_with_warnings(
                CompilerError::compiler_error(format!(
                    "AST finalization: {} TIR references remain at the last-reader barrier",
                    Rc::strong_count(&shared)
                )),
                emitted.warnings.clone(),
                string_table,
            )
            .with_type_context_for_all_diagnostics(type_environment.clone())
        })?;
        drop(template_ir_store);
        let expressions = expression_builder
            .freeze(&mut emitted.ast)
            .map_err(|error| {
                CompilerMessages::from_error_with_warnings(
                    error,
                    emitted.warnings.clone(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(type_environment.clone())
            })?;

        let public_interface_projection_input = AstPublicInterfaceProjectionInput {
            root_table: resolved_public_type_roots,
            trait_roots: resolved_public_trait_roots,
            trait_environment: Some(owned_lookups.trait_environment),
            trait_evidence_environment: Some(owned_lookups.trait_evidence_environment),
        };
        let module_resources = file_value_resolution
            .as_ref()
            .map(|services| Rc::clone(&services.module_resources));
        Ok(AstBuildResult {
            ast: Ast {
                nodes: emitted.ast,
                expressions,
                const_values,
                doc_fragments,
                entry_path: entry_dir,
                root_role,
                const_top_level_fragments,
                warnings: emitted.warnings,
                choice_definitions,
                imported_struct_definitions: owned_lookups.imported_struct_definitions,
                type_environment,
                const_facts,
                imported_functions_by_local_path: owned_lookups.imported_functions_by_local_path,
                infallible_cast_evidence,
                static_if_function_provenance,
            },
            module_resources,
            public_interface_projection_input,
            materialisation_context,
            deferred_generic_requests: committed_generic_requests.executable,
            assertion_validation_generic_requests: committed_generic_requests.assertion_validation,
        })
    }

    /// Wraps a [`CompilerError`] into [`CompilerMessages`] with the current environment's
    /// type information attached for diagnostic rendering.
    pub(in crate::compiler_frontend::ast) fn error_messages(
        &self,
        error: impl Into<Box<InfrastructureErrors>>,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> CompilerMessages {
        let mut messages =
            CompilerMessages::from_diagnostics(warnings.to_owned(), string_table.clone());
        messages.append_infrastructure_errors(error.into());
        messages.with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
    }

    /// Converts a [`TemplateNormalizationError`] into [`CompilerMessages`], routing
    /// diagnostic and infrastructure errors through their respective constructors.
    fn template_normalization_error_messages(
        &self,
        error: TemplateNormalizationError,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> CompilerMessages {
        match error {
            TemplateNormalizationError::Diagnostic(diagnostic) => {
                CompilerMessages::from_diagnostic_with_warnings(
                    diagnostic,
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
            }
            TemplateNormalizationError::Infrastructure(error) => {
                self.error_messages(error, warnings, string_table)
            }
        }
    }

    fn const_value_store_error_messages(
        &self,
        error: ConstValueStoreError,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> CompilerMessages {
        match error {
            ConstValueStoreError::Diagnostic(diagnostic) => {
                CompilerMessages::from_diagnostic_with_warnings(
                    diagnostic,
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
            }
            ConstValueStoreError::Infrastructure(error) => {
                self.error_messages(error, warnings, string_table)
            }
        }
    }

    /// Evidence owns the requirement method identity. Generic instances are direct children of
    /// their retained template path, including generated sidecars' `__generated_instance`.
    fn collect_infallible_cast_evidence(
        &self,
        concrete_nodes: &[crate::compiler_frontend::ast::ast_nodes::AstNode],
    ) -> Result<FxHashMap<PathId, InfallibleCastEvidenceContract>, CompilerError> {
        let lookups = &self.environment.lookups;
        let mut contracts = FxHashMap::default();
        for evidence in lookups.trait_evidence_environment.canonical_evidence() {
            if !matches!(
                lookups.trait_environment.core_trait_kind(evidence.trait_id),
                Some(CoreTraitKind::Castable {
                    fallibility: BuiltinCastFallibility::Infallible,
                    ..
                })
            ) {
                continue;
            }
            let definition = lookups
                .trait_environment
                .get(evidence.trait_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error("Canonical cast evidence has no resolved trait.")
                })?;
            let target_name = self
                .environment
                .type_environment
                .nominal_path(evidence.target_type_id)
                .and_then(|path| self.path_fork.component(*path))
                .ok_or_else(|| {
                    CompilerError::compiler_error("Canonical cast evidence has no nominal name.")
                })?;
            for mapping in &evidence.requirements {
                let requirement = definition
                    .requirements
                    .iter()
                    .find(|requirement| requirement.id == mapping.requirement_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Canonical cast evidence has no resolved requirement.",
                        )
                    })?;
                contracts.insert(
                    mapping.method_path,
                    InfallibleCastEvidenceContract {
                        target_name,
                        trait_name: definition.name,
                        requirement_name: requirement.name,
                        span: evidence.declaration_span,
                    },
                );
            }
        }
        // Generated evidence can still name the template. Carry its obligation onto each
        // emitted concrete instance without replacing any directly selected method mapping.
        for node in concrete_nodes {
            let NodeKind::Function(path, _, _) = &node.kind else {
                continue;
            };
            let Some(template_path) = self.path_fork.parent(*path) else {
                continue;
            };
            if !lookups
                .generic_function_templates_by_path
                .contains_key(&template_path)
            {
                continue;
            }
            if let Some(contract) = contracts.get(&template_path).copied() {
                contracts.entry(*path).or_insert(contract);
            }
        }
        Ok(contracts)
    }

    /// Validate dormant contracts after AST-local call and catch dependencies reach closure.
    /// Empty provisional facts cannot discharge evidence or prune recovery. Concrete instances
    /// still undergo HIR convergence independently of these retained template verdicts.
    fn validate_generic_builtin_failure_boundaries(
        &self,
        nodes: &[crate::compiler_frontend::ast::ast_nodes::AstNode],
        concrete_nodes: &[crate::compiler_frontend::ast::ast_nodes::AstNode],
        infallible_cast_evidence: &FxHashMap<PathId, InfallibleCastEvidenceContract>,
        has_assertion_validation_candidates: bool,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> Result<FxHashSet<PathId>, CompilerMessages> {
        if nodes.is_empty() && !has_assertion_validation_candidates {
            return Ok(FxHashSet::default());
        }

        let builtin_error_type = self
            .environment
            .type_environment
            .type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                CanonicalBuiltinType::Error,
            ));
        let template_ir_store = self.context.template_ir_store.borrow();
        let summaries = AstBuiltinFailureSummaries::compute(
            &[nodes, concrete_nodes],
            &template_ir_store,
            &self.context.expression_builder,
        )
        .map_err(|error| self.error_messages(error, warnings, string_table))?;
        for node in nodes {
            let NodeKind::Function(path, signature, _) = &node.kind else {
                continue;
            };
            let Some(pending) = summaries.function_facts(*path) else {
                continue;
            };
            if let Some(contract) = infallible_cast_evidence.get(path)
                && let Some(contributor) = summaries.first_active(&pending.body.implicit)
            {
                let witness = summaries.witness(contributor);
                return Err(CompilerMessages::from_diagnostic_with_warnings(
                    CompilerDiagnostic::infallible_cast_evidence_failure(contract, &witness),
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone()));
            }
            for check in &pending.body.deferred_custom_catches {
                let candidate = check
                    .candidates
                    .iter()
                    .find(|candidate| summaries.is_active(candidate));
                if check.eligibility_only {
                    if candidate.is_some() {
                        continue;
                    }
                    return Err(CompilerMessages::from_diagnostic_with_warnings(
                        CompilerDiagnostic::invalid_fallible_handling(
                            InvalidFallibleHandlingReason::CatchOnNonFallible,
                            check.catch_span,
                        ),
                        warnings.to_owned(),
                        string_table,
                    ));
                }
                if let Some(candidate) = candidate {
                    return Err(CompilerMessages::from_diagnostic_with_warnings(
                        CompilerDiagnostic::invalid_fallible_handling(
                            InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                                error_type_id: check.error_type_id,
                                typed_producer_span: check.typed_producer_span,
                                implicit_producer_span: candidate.span,
                            },
                            check.catch_span,
                        ),
                        warnings.to_owned(),
                        string_table,
                    )
                    .with_type_context_for_all_diagnostics(
                        self.environment.type_environment.clone(),
                    ));
                }
            }
            if let Some(contributor) = pending
                .assertion_message_calls
                .iter()
                .find(|contributor| summaries.is_active(contributor))
            {
                return Err(CompilerMessages::from_diagnostic_with_warnings(
                    CompilerDiagnostic::invalid_fallible_handling(
                        InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
                        contributor.span.or(node.span),
                    ),
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone()));
            }
            let custom_error = match signature.error_return_type_id() {
                Some(error_type) if Some(error_type) == builtin_error_type => continue,
                Some(error_type) => Some(error_type),
                None if self
                    .environment
                    .lookups
                    .exported_callable_paths
                    .contains(path) =>
                {
                    None
                }
                None => continue,
            };
            let contributor = summaries.first_active(&pending.body.implicit);
            let Some(contributor) = contributor else {
                continue;
            };
            let witness = summaries.witness(contributor);
            let reason = match custom_error {
                Some(error_type_id) => {
                    InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                        error_type_id,
                        witness: Box::new(witness),
                    }
                }
                None => InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                    witness: Box::new(witness),
                },
            };
            return Err(CompilerMessages::from_diagnostic_with_warnings(
                CompilerDiagnostic::invalid_fallible_handling(
                    reason,
                    node.span.or(contributor.span),
                ),
                warnings.to_owned(),
                string_table,
            )
            .with_type_context_for_all_diagnostics(self.environment.type_environment.clone()));
        }
        // Only witnesses on completed concrete roots can demand inactive generic validation.
        // Dormant template checks above and eliminated static branches own no such demand.
        let mut assertion_validation_paths = FxHashSet::default();
        if has_assertion_validation_candidates {
            for node in concrete_nodes {
                let NodeKind::Function(path, ..) = node.kind else {
                    continue;
                };
                let Some(pending) = summaries.function_facts(path) else {
                    continue;
                };
                for contributor in &pending.assertion_message_calls {
                    if let ImplicitFailureSource::PrivateCall(path) = contributor.source {
                        assertion_validation_paths.insert(path);
                    }
                }
            }
        }
        Ok(assertion_validation_paths)
    }

    fn validate_specialized_function_terminality(
        &self,
        ast_nodes: &[crate::compiler_frontend::ast::ast_nodes::AstNode],
        start_function_path: Option<PathId>,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
        for node in ast_nodes {
            let NodeKind::Function(path, signature, body) = &node.kind else {
                continue;
            };
            let policy = terminality_policy_for_signature(
                signature,
                start_function_path.is_some_and(|start_path| start_path == *path),
            );
            let terminality_diagnostic = validate_function_body_terminality(
                body,
                policy,
                node.span,
                &self.context.expression_builder,
                &self.context.const_values,
            )
            .map_err(|error| self.error_messages(error, warnings, string_table))?;
            if let Some(diagnostic) = terminality_diagnostic {
                return Err(CompilerMessages::from_diagnostic_with_warnings(
                    diagnostic,
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone()));
            }
        }
        Ok(())
    }

    /// Collects all non-generic choice definitions from the resolved environment.
    ///
    /// WHAT: iterates the declaration table and extracts choice definitions that
    /// have no generic parameters, so HIR can emit them as concrete nominal types.
    ///
    /// WHY: generic choices are templates, not concrete types, and must not be
    /// emitted as standalone definitions.
    fn collect_choice_definitions(&self) -> Vec<AstChoiceDefinition> {
        let mut choice_definitions = vec![];
        for entry in self.environment.lookups.declaration_table.iter() {
            let type_id = entry.type_id;
            let Some(choice_def) = self
                .environment
                .type_environment
                .choice_definition_for(type_id)
            else {
                continue;
            };

            // Skip generic choice declarations (they have type parameters).
            if choice_def.generic_parameters.is_some() {
                continue;
            }

            choice_definitions.push(AstChoiceDefinition {
                nominal_path: choice_def.path.to_owned(),
            });
        }

        choice_definitions
    }
}
