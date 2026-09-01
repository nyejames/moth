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
use super::public_const_templates::const_template_value_from_projection;
use super::static_if_specialization::{StaticIfCandidate, StaticIfSpecialization};
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::body_local::insert_body_local_const_records;
use crate::compiler_frontend::ast::const_values::store::{
    ConstTemplateValue, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::generic_functions::{
    ModuleMaterialisationEnvironmentInput, ModuleMaterialisationPreparationBuilder,
};
use crate::compiler_frontend::ast::statements::terminality::{
    terminality_policy_for_signature, validate_function_body_terminality,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::top_level_templates::{
    collect_and_strip_comment_templates, collect_const_top_level_fragments,
};
use crate::compiler_frontend::ast::{
    Ast, AstBuildResult, AstChoiceDefinition, AstPublicInterfaceProjectionInput,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use crate::timing_scope_attributed_opt;
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
    pub(super) context: &'context AstPhaseContext<'services>,
    pub(super) environment: AstModuleEnvironment,
}

impl<'context, 'services> AstFinalizer<'context, 'services> {
    /// Creates a new finalizer with the given phase context and resolved environment.
    pub(in crate::compiler_frontend::ast) fn new(
        context: &'context AstPhaseContext<'services>,
        environment: AstModuleEnvironment,
    ) -> Self {
        Self {
            context,
            environment,
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
        string_table: &mut StringTable,
    ) -> Result<AstBuildResult, CompilerMessages> {
        // ----------------------------
        //  Collect doc fragments
        // ----------------------------
        let doc_fragments = collect_and_strip_comment_templates(
            &mut emitted.ast,
            string_table,
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
        let projected_const_templates =
            self.project_const_templates(string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
        // The TIR store borrow lives only for store construction. Const-fact collection and
        // generated materialisation below take the same `RefCell` again.
        let const_values = {
            let template_ir_store = self.context.template_ir_store.borrow();
            let mut template_builder =
                |defining_path: Option<&InternedPath>,
                 template: &Template|
                 -> Result<ConstTemplateValue, ConstValueStoreError> {
                    // A root module constant was already projected once by `project_const_templates`.
                    // Only a nested template inside an aggregate constant is projected here, and it
                    // must go through the same owner rather than a second classification rule.
                    let projected = match defining_path {
                    Some(path) => projected_const_templates
                        .module_values
                        .get(path)
                        .cloned()
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "Module constant template has no single finalization projection.",
                            )
                        })?,
                    None => self
                        .project_template_value(template, &template_ir_store, string_table)
                        .map_err(|error| match error {
                            TemplateNormalizationError::Diagnostic(diagnostic) => {
                                ConstValueStoreError::Diagnostic(diagnostic)
                            }
                            TemplateNormalizationError::Infrastructure(error) => {
                                ConstValueStoreError::Infrastructure(error)
                            }
                        })?,
                };

                    const_template_value_from_projection(projected, template)
                };

            ConstValueStore::from_declaration_table(
                &self.environment.lookups.declaration_table,
                &self.environment.lookups.resolved_module_constants,
                &self.environment.type_environment,
                &mut template_builder,
            )
            .and_then(|mut store| {
                insert_body_local_const_records(
                    &mut store,
                    &emitted.ast,
                    &self.environment.type_environment,
                    &mut template_builder,
                )?;
                Ok(store)
            })
        }
        .map_err(|error| {
            self.const_value_store_error_messages(error, &emitted.warnings, string_table)
        })?;

        // ----------------------------
        //  Prepare static-control-flow candidate
        // ----------------------------
        // Selection runs transactionally on a projection. When it finds a known Bool, the
        // authored AST remains the validation authority and the candidate becomes durable only
        // after every authored template and type boundary has passed below.
        let mut static_candidate = StaticIfCandidate::prepare(
            &emitted.ast,
            &const_values,
            Rc::clone(&self.context.template_ir_store),
            string_table,
        )
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;

        if static_candidate.has_selections() {
            // Normalize and validate the untouched authored tree first. Its provisional reactive
            // summaries may include both branches because this copy is never published.
            self.propagate_reactive_template_metadata(&mut emitted.ast)
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
            self.normalize_ast_templates_for_hir(&mut emitted.ast, string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
            self.validate_no_unresolved_executable_types(&emitted.ast, &const_values, string_table)
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;

            // The candidate owns its exact annotated TIR contexts. Normalize those active views,
            // validate the completed HIR-boundary shape and publish the candidate atomically.
            self.propagate_reactive_template_metadata(static_candidate.ast_mut())
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
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
                &const_values,
                string_table,
            )
            .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
        } else {
            // The common runtime-only path retains the existing single normalization owner. The
            // unchanged projection is discarded without allocating reactive or normalization
            // overlays.
            self.propagate_reactive_template_metadata(&mut emitted.ast)
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
            self.normalize_ast_templates_for_hir(&mut emitted.ast, string_table)
                .map_err(|error| {
                    self.template_normalization_error_messages(
                        error,
                        &emitted.warnings,
                        string_table,
                    )
                })?;
            self.validate_no_unresolved_executable_types(&emitted.ast, &const_values, string_table)
                .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;
        }

        let mut specialization = static_candidate.publish(&mut emitted.ast);

        // The authored assertion message must remain available to the authoritative AST
        // type/TIR boundary validation above. Only after that validation succeeds may AST
        // finalization discard compile-time-inactive executable message state.
        discard_inactive_assertion_messages(&mut emitted.ast);

        let start_function_path = self.context.root_role.has_implicit_start().then(|| {
            self.context
                .entry_dir
                .join_str(IMPLICIT_START_FUNC_NAME, string_table)
        });

        // ----------------------------
        //  Specialise static Bool control flow
        // ----------------------------
        let template_specialization = StaticIfSpecialization::run(
            &mut emitted.validated_generic_template_bodies,
            &const_values,
            Rc::clone(&self.context.template_ir_store),
            string_table,
        )
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;
        specialization.merge(template_specialization);
        emitted.deferred_generic_requests =
            specialization.commit_active_generic_requests(emitted.deferred_generic_requests);
        self.validate_specialized_function_terminality(
            &emitted.ast,
            start_function_path.as_ref(),
            &emitted.warnings,
            string_table,
        )?;
        self.validate_specialized_function_terminality(
            &emitted.validated_generic_template_bodies,
            None,
            &emitted.warnings,
            string_table,
        )?;

        // ----------------------------
        //  Publish active reactive metadata
        // ----------------------------
        // Template normalization above validates and materialises both authored branches. The
        // durable flow pass runs only after static selection, so inactive returns cannot pollute
        // function signatures, surviving calls or runtime handoffs.
        self.propagate_reactive_template_metadata(&mut emitted.ast)
            .map_err(|error| self.error_messages(error, &emitted.warnings, string_table))?;

        // ----------------------------
        //  Synchronize finalized public defaults
        // ----------------------------
        // The emitted AST now carries normalized defaults and active reactive return metadata.
        // Synchronize that one completed copy into public roots and receiver indexes. Generic
        // declarations without emitted nodes normalize their retained defaults here as before.
        self.synchronize_normalized_public_defaults(&emitted.ast, string_table)
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
            &const_values,
            Rc::clone(&self.context.template_ir_store),
        )
        .collect(&const_values, &emitted.ast, start_function_path.as_ref())
        .map_err(|error| {
            self.template_normalization_error_messages(error, &emitted.warnings, string_table)
        })?;

        // ----------------------------
        //  Merge builtin AST nodes
        // ----------------------------
        if !self.environment.lookups.builtin_struct_ast_nodes.is_empty() {
            let mut ast_nodes = self.environment.lookups.builtin_struct_ast_nodes.clone();
            ast_nodes.extend(emitted.ast);
            emitted.ast = ast_nodes;
        }

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
            generated_evidence_pairs: _,
            type_environment,
            resolved_public_type_roots,
            resolved_public_trait_roots,
        } = self.environment;

        #[cfg(debug_assertions)]
        {
            // The shared module store is the authority for finalized views.
            let template_ir_store = self.context.template_ir_store.borrow();
            debug_validate_type_ids_for_hir(
                &emitted.ast,
                &const_values,
                &choice_definitions,
                &type_environment,
                &template_ir_store,
            );
        }

        // Emission is complete, so the environment lookups `Rc` is the sole strong reference.
        // Recovering owned access lets the donor-local generic-template map move directly into
        // the build result without cloning body tokens or an `Rc::try_unwrap` dance in semantic
        // orchestration.
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

        let materialisation_context = ModuleMaterialisationPreparationBuilder::from_environment(
            ModuleMaterialisationEnvironmentInput {
                lookups: &owned_lookups,
                const_values: &const_values,
                type_environment: &type_environment,
                public_trait_roots: &resolved_public_trait_roots,
                default_const_templates_by_path: projected_const_templates.by_path,
                entry_dir: self.context.entry_dir.clone(),
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
                module_resources: self
                    .context
                    .file_value_resolution
                    .as_ref()
                    .map(|services| Rc::clone(&services.module_resources)),
                string_table,
                template_const_loop_iteration_limit: self
                    .context
                    .template_const_loop_iteration_limit,
                capacity_estimate: self.context.capacity_estimate,
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
        let public_interface_projection_input = AstPublicInterfaceProjectionInput {
            root_table: resolved_public_type_roots,
            trait_roots: resolved_public_trait_roots,
            trait_environment: Some(owned_lookups.trait_environment),
            trait_evidence_environment: Some(owned_lookups.trait_evidence_environment),
        };
        let module_resources = self
            .context
            .file_value_resolution
            .as_ref()
            .map(|services| Rc::clone(&services.module_resources));
        Ok(AstBuildResult {
            ast: Ast {
                nodes: emitted.ast,
                const_values,
                doc_fragments,
                entry_path: self.context.entry_dir.to_owned(),
                root_role: self.context.root_role,
                const_top_level_fragments,
                warnings: emitted.warnings,
                choice_definitions,
                imported_struct_definitions: owned_lookups.imported_struct_definitions,
                type_environment,
                const_facts,
                imported_functions_by_local_path: owned_lookups.imported_functions_by_local_path,
            },
            module_resources,
            public_interface_projection_input,
            materialisation_context,
            deferred_generic_requests: emitted.deferred_generic_requests,
        })
    }

    /// Wraps a [`CompilerError`] into [`CompilerMessages`] with the current environment's
    /// type information attached for diagnostic rendering.
    pub(in crate::compiler_frontend::ast) fn error_messages(
        &self,
        error: CompilerError,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> CompilerMessages {
        CompilerMessages::from_error_with_warnings(error, warnings.to_owned(), string_table)
            .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
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
                    *diagnostic,
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
            }
            TemplateNormalizationError::Infrastructure(error) => {
                self.error_messages(*error, warnings, string_table)
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
                    *diagnostic,
                    warnings.to_owned(),
                    string_table,
                )
                .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
            }
            ConstValueStoreError::Infrastructure(error) => {
                self.error_messages(*error, warnings, string_table)
            }
        }
    }

    fn validate_specialized_function_terminality(
        &self,
        ast_nodes: &[crate::compiler_frontend::ast::ast_nodes::AstNode],
        start_function_path: Option<&InternedPath>,
        warnings: &[CompilerDiagnostic],
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
        for node in ast_nodes {
            let NodeKind::Function(path, signature, body) = &node.kind else {
                continue;
            };
            let policy = terminality_policy_for_signature(
                signature,
                start_function_path.is_some_and(|start_path| start_path == path),
            );
            if let Some(diagnostic) =
                validate_function_body_terminality(body, policy, node.location.clone())
            {
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
            let type_id = entry.value.type_id;
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
