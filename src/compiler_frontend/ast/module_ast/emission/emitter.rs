//! AST node emission.
//!
//! WHAT: iterates sorted headers with full context (resolved signatures, receiver catalog,
//! per-file visibility) and lowers each header into typed AST nodes.
//! WHY: emission is the only AST phase that parses executable bodies (function bodies, template
//! bodies, start body). Earlier phases consume header shells without body parsing.
//! Top-level declaration shell reparsing does NOT happen here — shells were fully parsed
//! by the header stage and resolved by environment construction.
//!
//! Constants and choices are handled in earlier passes; they do not emit nodes here.
//! Struct node emission reads the resolved field table produced by environment construction.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind, SourceLocation};
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionBodyValidationInput, GenericFunctionInstance, GenericFunctionInstanceKey,
    GenericFunctionInstantiationRequest, GenericInstantiationDiagnosticContext,
    concrete_argument_mapping, recursive_generic_function_instantiation,
    substitute_function_signature,
    validate_generic_function_body as validate_generic_body_template,
    with_generic_instantiation_context,
};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::environment::AstModuleEnvironment;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::statements::terminality::{
    terminality_policy_for_signature, validate_function_body_terminality,
};
use crate::compiler_frontend::ast::templates::create_template_node::ConstRequiredTemplateConstruction;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult,
};
use crate::compiler_frontend::ast::templates::tir::{
    PreparedTemplate, TemplateTirPhase, TirView, fold_prepared_template,
};
use crate::compiler_frontend::ast::templates::top_level_templates::FoldedConstTemplateResult;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, GenericSubstitutionDiagnostic, InvalidTemplateStructureReason,
};
use std::sync::Arc;

use crate::compiler_frontend::ast::type_resolution::resolve_diagnostic_type_to_type_id_checked;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::generic_parameters::{
    ActiveGenericTypeContext, GenericParameterScope,
};
use crate::compiler_frontend::datatypes::ids::{
    GenericParameterId, GenericParameterListId, TypeId,
};
use crate::compiler_frontend::headers::import_environment::FileVisibility;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::projects::settings::{self, IMPLICIT_START_FUNC_NAME};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "detailed_timers")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
#[cfg(feature = "detailed_timers")]
use crate::timing::{detailed_timer_output_enabled, log_aggregated_duration};
#[cfg(feature = "detailed_timers")]
use std::time::Duration;

pub(in crate::compiler_frontend::ast) struct AstEmission {
    /// Typed AST nodes emitted for this module (functions, structs, generic instances).
    pub(in crate::compiler_frontend::ast) ast: Vec<AstNode>,
    /// Warnings accumulated during emission (unused variables, deprecated uses, etc.).
    pub(in crate::compiler_frontend::ast) warnings: Vec<CompilerDiagnostic>,
    /// Folded top-level const template result records keyed by source file.
    pub(in crate::compiler_frontend::ast) const_templates_by_path:
        FxHashMap<InternedPath, FoldedConstTemplateResult>,
    /// Concrete generic function instances emitted while lowering visible calls.
    pub(in crate::compiler_frontend::ast) generic_instance_count: usize,
    /// Imported generic calls are inferred here but materialised by the build-owned sidecar
    /// worklist from the declaring module's retained context.
    pub(in crate::compiler_frontend::ast) deferred_generic_requests:
        Vec<GenericFunctionInstantiationRequest>,
}

#[cfg(test)]
#[path = "capacity_budget_tests.rs"]
mod capacity_budget_tests;

/// Shared input used by [`AstEmitter::build_base_scope_context`] to create a
/// [`ScopeContext`] that is identical across function, start, and const-template emission.
struct BaseScopeContextInput<'scope> {
    kind: ContextKind,
    scope: InternedPath,
    top_level_declarations: &'scope Rc<TopLevelDeclarationTable>,
    visibility: Rc<FileVisibility>,
    source_file_scope: InternedPath,
    scope_frame_capacity: usize,
}

/// AST-local spender for module-level scope-frame capacity estimates.
///
/// WHAT: divides one module estimate across the root parse contexts known from sorted headers.
/// WHY: `ScopeArena` is owned per parse context, so applying the full module estimate to every
/// root would multiply memory use. This budget spends the estimate once and lets undersized roots
/// grow normally.
struct ScopeFrameCapacityBudget {
    remaining_frames: usize,
    remaining_roots: usize,
}

impl ScopeFrameCapacityBudget {
    fn new(module_frame_estimate: usize, root_count: usize) -> Self {
        Self {
            remaining_frames: module_frame_estimate,
            remaining_roots: root_count,
        }
    }

    fn next_root_capacity(&mut self) -> usize {
        if self.remaining_roots == 0 || self.remaining_frames == 0 {
            self.remaining_roots = self.remaining_roots.saturating_sub(1);
            return 0;
        }

        let capacity = self.remaining_frames.div_ceil(self.remaining_roots);
        self.remaining_frames = self.remaining_frames.saturating_sub(capacity);
        self.remaining_roots -= 1;
        capacity
    }
}

/// Rebase each parameter's [`InternedPath`] from a bare name to a fully qualified path
/// under the given function path.
///
/// WHAT: ensures parameter symbols are module-unique before body parsing.
/// WHY: AST symbol IDs are full [`InternedPath`] values, not local-scope names.
fn rebase_signature_parameters(signature: &mut FunctionSignature, function_path: &InternedPath) {
    for parameter in &mut signature.parameters {
        let Some(parameter_name) = parameter.id.name() else {
            continue;
        };

        let old_parameter_id = parameter.id.clone();
        parameter.id = function_path.append(parameter_name);

        if let Some(source) = &mut parameter.value.reactive_source
            && source.path == old_parameter_id
        {
            source.path = parameter.id.clone();
        }

        if let Some(metadata) = &mut parameter.value.reactive_template {
            for dependency in &mut metadata.template_value_parameters {
                if dependency.parameter == old_parameter_id {
                    dependency.parameter = parameter.id.clone();
                }
            }
        }
    }
}

/// Count sorted-header roots that create independent root `ScopeArena`s during emission.
///
/// WHAT: function headers count once whether they emit a concrete body or validate a generic
/// template body; start headers and top-level const templates also parse in their own root
/// contexts.
/// WHY: the module-wide capacity estimate must be distributed over root parse contexts that can
/// consume it once. Header kinds handled entirely by environment construction or metadata
/// validation do not allocate root body arenas here.
fn count_root_scope_arena_consumers(headers: &[Header]) -> usize {
    headers
        .iter()
        .filter(|header| {
            matches!(
                header.kind,
                HeaderKind::Function { .. }
                    | HeaderKind::StartFunction
                    | HeaderKind::ConstTemplate { .. }
            )
        })
        .count()
}

pub(in crate::compiler_frontend::ast) struct AstEmitter<'context, 'services, 'environment> {
    context: &'context AstPhaseContext<'services>,
    environment: &'environment mut AstModuleEnvironment,
    ast: Vec<AstNode>,
    warnings: Vec<CompilerDiagnostic>,
    const_templates_by_path: FxHashMap<InternedPath, FoldedConstTemplateResult>,
    compatibility_cache: TypeCompatibilityCache,
    generic_function_instantiation_requests: Rc<RefCell<Vec<GenericFunctionInstantiationRequest>>>,
    generic_function_instances_by_key:
        FxHashMap<GenericFunctionInstanceKey, GenericFunctionInstance>,
    deferred_generic_requests: Vec<GenericFunctionInstantiationRequest>,
}

impl<'context, 'services, 'environment> AstEmitter<'context, 'services, 'environment> {
    pub(in crate::compiler_frontend::ast) fn new(
        context: &'context AstPhaseContext<'services>,
        environment: &'environment mut AstModuleEnvironment,
        header_count: usize,
    ) -> Self {
        let warnings = environment.lookups.warnings.clone();
        Self {
            context,
            environment,
            ast: Vec::with_capacity(header_count * settings::TOKEN_TO_NODE_RATIO),
            warnings,
            const_templates_by_path: FxHashMap::default(),
            compatibility_cache: TypeCompatibilityCache::new(),
            generic_function_instantiation_requests: Rc::new(RefCell::new(Vec::new())),
            generic_function_instances_by_key: FxHashMap::default(),
            deferred_generic_requests: Vec::new(),
        }
    }

    pub(in crate::compiler_frontend::ast) fn emit_generated_request(
        mut self,
        request: GenericFunctionInstantiationRequest,
        string_table: &mut StringTable,
    ) -> Result<AstEmission, CompilerMessages> {
        self.emit_generic_function_instance(request, &[], string_table)?;
        self.defer_requested_generic_function_instances();

        Ok(AstEmission {
            ast: self.ast,
            warnings: self.warnings,
            const_templates_by_path: self.const_templates_by_path,
            generic_instance_count: self.generic_function_instances_by_key.len(),
            deferred_generic_requests: self.deferred_generic_requests,
        })
    }

    /// Emits AST nodes for each header kind (functions, structs, templates).
    /// Build a base `ScopeContext` with all shared state that is identical across function,
    /// start, and const-template emission.
    ///
    /// WHAT: centralizes the repeated 11-method `ScopeContext` builder chain so each emission
    /// arm only adds emission-specific configuration (parameters for functions, etc.).
    /// WHY: avoids duplicating the same visibility/alias/field/setup sequence in three match arms.
    fn build_base_scope_context(&self, input: BaseScopeContextInput<'_>) -> ScopeContext {
        ScopeContext::new(
            input.kind,
            input.scope,
            Rc::clone(input.top_level_declarations),
            Arc::clone(&self.context.external_package_registry),
            Vec::<TypeId>::new(),
            input.scope_frame_capacity,
            self.context.template_ir_store.clone(),
        )
        .with_style_directives(self.context.style_directives)
        .with_build_profile(self.context.build_profile)
        .with_file_visibility(input.visibility)
        .with_resolved_type_aliases(Rc::clone(
            &self.environment.lookups.resolved_type_aliases_by_path,
        ))
        .with_generic_declarations(Rc::clone(
            &self.environment.lookups.generic_declarations_by_path,
        ))
        .with_resolved_struct_fields_by_path(Rc::clone(
            &self.environment.lookups.resolved_struct_fields_by_path,
        ))
        .with_project_path_resolver(self.context.project_path_resolver.clone())
        .with_path_format_config(self.context.path_format_config.clone())
        .with_template_const_loop_iteration_limit(self.context.template_const_loop_iteration_limit)
        .with_rendered_path_usage_sink(Rc::clone(&self.environment.lookups.rendered_path_usages))
        .with_generic_function_instantiation_sink(Rc::clone(
            &self.generic_function_instantiation_requests,
        ))
        .with_receiver_methods(Rc::clone(&self.environment.lookups.receiver_methods))
        .with_lookups(Rc::clone(&self.environment.lookups))
        .with_source_file_scope(input.source_file_scope)
    }

    pub(in crate::compiler_frontend::ast) fn emit(
        mut self,
        sorted_headers: Vec<Header>,
        string_table: &mut StringTable,
    ) -> Result<AstEmission, CompilerMessages> {
        // The environment owns the single resolved declaration table. Body contexts clone only
        // the Rc pointer so declaration metadata is not rebuilt during emission.
        let top_level_declarations = Rc::clone(&self.environment.lookups.declaration_table);

        #[cfg(feature = "detailed_timers")]
        let mut total_function_body_parse_time = Duration::default();
        #[cfg(feature = "detailed_timers")]
        let mut total_start_body_parse_time = Duration::default();
        #[cfg(feature = "detailed_timers")]
        let mut total_const_template_parse_time = Duration::default();
        #[cfg(feature = "detailed_timers")]
        let mut total_const_template_fold_time = Duration::default();
        #[cfg(feature = "detailed_timers")]
        let mut function_headers_emitted = 0usize;
        #[cfg(feature = "detailed_timers")]
        let mut start_headers_emitted = 0usize;
        #[cfg(feature = "detailed_timers")]
        let mut struct_headers_emitted = 0usize;
        #[cfg(feature = "detailed_timers")]
        let mut const_templates_emitted = 0usize;

        let root_scope_consumer_count = count_root_scope_arena_consumers(&sorted_headers);
        let mut scope_frame_capacity_budget = ScopeFrameCapacityBudget::new(
            self.context.capacity_estimate.scope_frames,
            root_scope_consumer_count,
        );

        for header in sorted_headers {
            let visibility = Rc::new(
                self.environment
                    .lookups
                    .import_environment
                    .visibility_for(&header.source_file)
                    .map_err(|error| self.error_messages(error, string_table))?
                    .clone(),
            );
            let source_file_scope = header.canonical_source_file(string_table);

            match &header.kind {
                HeaderKind::Function {
                    generic_parameters, ..
                } => {
                    if !generic_parameters.is_empty() {
                        self.validate_generic_function_body(
                            header,
                            visibility,
                            source_file_scope,
                            scope_frame_capacity_budget.next_root_capacity(),
                            string_table,
                        )?;
                        continue;
                    }

                    #[cfg(feature = "detailed_timers")]
                    let start = crate::timing::start_detailed_timer();
                    self.emit_function(
                        header,
                        visibility,
                        source_file_scope,
                        scope_frame_capacity_budget.next_root_capacity(),
                        string_table,
                    )?;
                    #[cfg(feature = "detailed_timers")]
                    {
                        if let Some(elapsed) = start.elapsed() {
                            total_function_body_parse_time += elapsed;
                        }
                        function_headers_emitted += 1;
                    }
                }

                HeaderKind::StartFunction => {
                    #[cfg(feature = "detailed_timers")]
                    let start = crate::timing::start_detailed_timer();
                    self.emit_start(
                        header,
                        visibility,
                        source_file_scope,
                        scope_frame_capacity_budget.next_root_capacity(),
                        string_table,
                    )?;
                    #[cfg(feature = "detailed_timers")]
                    {
                        if let Some(elapsed) = start.elapsed() {
                            total_start_body_parse_time += elapsed;
                        }
                        start_headers_emitted += 1;
                    }
                }

                HeaderKind::Struct {
                    generic_parameters, ..
                } => {
                    if !generic_parameters.is_empty() {
                        continue;
                    }

                    #[cfg(feature = "detailed_timers")]
                    {
                        struct_headers_emitted += 1;
                    }
                    self.emit_struct(header, string_table)?;
                }

                // Constants and choices are fully handled during environment construction.
                HeaderKind::Constant { .. } | HeaderKind::Choice { .. } => {}

                HeaderKind::ConstTemplate { .. } => {
                    let mut template_tokens = header.tokens;
                    let context = self.build_base_scope_context(BaseScopeContextInput {
                        kind: ContextKind::Constant,
                        scope: template_tokens.src_path.to_owned(),
                        top_level_declarations: &top_level_declarations,
                        visibility,
                        source_file_scope,
                        scope_frame_capacity: scope_frame_capacity_budget.next_root_capacity(),
                    });

                    #[cfg(feature = "detailed_timers")]
                    let const_template_parse_start = crate::timing::start_detailed_timer();
                    let template =
                        self.parse_const_template(&mut template_tokens, &context, string_table)?;
                    #[cfg(feature = "detailed_timers")]
                    {
                        if let Some(elapsed) = const_template_parse_start.elapsed() {
                            total_const_template_parse_time += elapsed;
                        }
                    }
                    self.warnings.extend(context.take_emitted_warnings());

                    #[cfg(feature = "detailed_timers")]
                    let const_template_fold_start = crate::timing::start_detailed_timer();
                    let folded_result =
                        self.fold_const_template(template, &context, string_table)?;
                    #[cfg(feature = "detailed_timers")]
                    {
                        if let Some(elapsed) = const_template_fold_start.elapsed() {
                            total_const_template_fold_time += elapsed;
                        }
                        const_templates_emitted += 1;
                    }

                    self.const_templates_by_path
                        .insert(template_tokens.src_path, folded_result);
                }

                // --------------------------
                //  Type aliases (no runtime emission)
                // --------------------------
                HeaderKind::TypeAlias { .. } => {
                    // Type aliases are compile-time-only metadata; they do not emit runtime nodes.
                }

                HeaderKind::Trait { .. }
                | HeaderKind::TraitConformance { .. }
                | HeaderKind::TraitIncompatibility { .. } => {
                    // Trait metadata is compile-time-only. AST environment construction has
                    // already resolved trait identities, evidence, and incompatibility relations
                    // before body emission.
                }
            }
        }

        self.defer_requested_generic_function_instances();

        #[cfg(feature = "detailed_timers")]
        {
            log_aggregated_duration(
                "AST/node emission/function bodies parsed in: ",
                total_function_body_parse_time,
            );
            log_aggregated_duration(
                "AST/node emission/start bodies parsed in: ",
                total_start_body_parse_time,
            );
            log_aggregated_duration(
                "AST/node emission/const templates parsed in: ",
                total_const_template_parse_time,
            );
            log_aggregated_duration(
                "AST/node emission/const templates folded in: ",
                total_const_template_fold_time,
            );
            if detailed_timer_output_enabled() {
                saying::say!(
                    "AST/node emission/headers emitted: \n functions = ", Dark Green function_headers_emitted,
                    Reset "\n starts = ", Dark Green start_headers_emitted,
                    Reset "\n structs = ", Dark Green struct_headers_emitted,
                    Reset "\n const templates = ", Dark Green const_templates_emitted
                );
            }

            add_frontend_counter(
                FrontendCounter::AstFunctionBodyRootCount,
                function_headers_emitted,
            );
            add_frontend_counter(
                FrontendCounter::AstStartBodyRootCount,
                start_headers_emitted,
            );
            add_frontend_counter(
                FrontendCounter::AstConstTemplateFoldedCount,
                const_templates_emitted,
            );
            add_frontend_counter(
                FrontendCounter::AstRootScopeArenaCount,
                root_scope_consumer_count,
            );
        }

        Ok(AstEmission {
            ast: self.ast,
            warnings: self.warnings,
            const_templates_by_path: self.const_templates_by_path,
            generic_instance_count: self.generic_function_instances_by_key.len(),
            deferred_generic_requests: self.deferred_generic_requests,
        })
    }

    // --------------------------
    //  Emit function bodies
    // --------------------------

    /// Move inferred generic calls into the build-owned worklist without parsing their bodies.
    ///
    /// The dedicated generated-function materialiser invokes `emit_generic_function_instance`
    /// for one selected request. Ordinary module emission only records stable requests, including
    /// nested calls discovered while that selected generated body is parsed.
    fn defer_requested_generic_function_instances(&mut self) {
        let requests =
            std::mem::take(&mut *self.generic_function_instantiation_requests.borrow_mut());

        for request in requests {
            if self
                .generic_function_instances_by_key
                .contains_key(&request.key)
            {
                continue;
            }

            self.generic_function_instances_by_key.insert(
                request.key.clone(),
                GenericFunctionInstance {
                    instance_path: request.instance_path.clone(),
                    key: request.key.clone(),
                },
            );
            self.deferred_generic_requests.push(request);
        }
    }

    /// Rebuilds the body-local generic type context from canonical type metadata.
    ///
    /// WHAT: exposes generic parameter names while parsing a generic function
    /// body and optionally supplies concrete substitutions for an emitted
    /// instance.
    /// WHY: signature resolution owns canonical parameter allocation; body
    /// parsing must consume that canonical identity instead of reconstructing
    /// parser-local parameter IDs.
    fn build_active_generic_type_context(
        &self,
        parameter_list_id: GenericParameterListId,
        substitutions: Option<FxHashMap<GenericParameterId, TypeId>>,
        source_parameter_by_rebased_path: FxHashMap<InternedPath, GenericParameterId>,
        string_table: &StringTable,
    ) -> Result<ActiveGenericTypeContext, CompilerMessages> {
        let Some(parameter_list) = self
            .environment
            .type_environment
            .generic_parameters(parameter_list_id)
        else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Generic function body requested an unknown generic parameter list.",
                ),
                string_table,
            ));
        };

        Ok(ActiveGenericTypeContext {
            parameter_scope: GenericParameterScope::from_canonical_parameter_list(parameter_list),
            substitutions,
            source_parameter_by_rebased_path,
        })
    }

    fn source_parameter_origins_for_signature(
        &self,
        source_signature: &FunctionSignature,
        emitted_signature: &FunctionSignature,
    ) -> FxHashMap<InternedPath, GenericParameterId> {
        let mut origins = FxHashMap::default();

        for (source_parameter, emitted_parameter) in source_signature
            .parameters
            .iter()
            .zip(emitted_signature.parameters.iter())
        {
            let Some(TypeDefinition::GenericParameter(parameter)) = self
                .environment
                .type_environment
                .get(source_parameter.value.type_id)
            else {
                continue;
            };

            origins.insert(emitted_parameter.id.clone(), parameter.id);
        }

        origins
    }

    fn generic_substitution_diagnostics(
        &self,
        parameter_list_id: GenericParameterListId,
        type_arguments: &[TypeId],
    ) -> Vec<GenericSubstitutionDiagnostic> {
        let Some(parameter_list) = self
            .environment
            .type_environment
            .generic_parameters(parameter_list_id)
        else {
            return Vec::new();
        };

        parameter_list
            .parameters
            .iter()
            .zip(type_arguments.iter())
            .map(
                |(parameter, concrete_type_id)| GenericSubstitutionDiagnostic {
                    parameter_name: parameter.name,
                    concrete_type_id: *concrete_type_id,
                },
            )
            .collect()
    }

    fn emit_generic_function_instance(
        &mut self,
        request: GenericFunctionInstantiationRequest,
        active_stack: &[GenericFunctionInstanceKey],
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        // --------------------------
        //  Deduplication and recursion guard
        // --------------------------
        if let Some(existing_instance) = self.generic_function_instances_by_key.get(&request.key) {
            debug_assert_eq!(existing_instance.key, request.key);
            debug_assert_eq!(existing_instance.instance_path, request.instance_path);
            return Ok(());
        }

        if active_stack
            .iter()
            .any(|active_key| active_key == &request.key)
        {
            return Err(self.diagnostic_messages(
                recursive_generic_function_instantiation(
                    request.key.function_path.name(),
                    request.call_location,
                ),
                string_table,
            ));
        }

        // --------------------------
        //  Resolve template and substitute signature
        // --------------------------
        let Some(template) = self
            .environment
            .lookups
            .generic_function_templates_by_path
            .get(&request.key.function_path)
            .cloned()
        else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Generic function instance requested for an unknown template.",
                ),
                string_table,
            ));
        };

        let Some(mut token_stream) = template.body_tokens.clone() else {
            let instance = GenericFunctionInstance {
                instance_path: request.instance_path.clone(),
                key: request.key.clone(),
            };
            self.generic_function_instances_by_key
                .insert(request.key.clone(), instance);
            self.deferred_generic_requests.push(request);
            return Ok(());
        };

        let Some(mapping) = concrete_argument_mapping(
            template.generic_parameter_list_id,
            request.key.type_arguments.as_ref(),
            &self.environment.type_environment,
        ) else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Generic function instance request did not match its template parameter list.",
                ),
                string_table,
            ));
        };
        let substitution_diagnostics = self.generic_substitution_diagnostics(
            template.generic_parameter_list_id,
            request.key.type_arguments.as_ref(),
        );

        let mut signature = substitute_function_signature(
            &template.signature,
            &mapping,
            &mut self.environment.type_environment,
        );
        rebase_signature_parameters(&mut signature, &request.instance_path);
        let generic_type_context = self.build_active_generic_type_context(
            template.generic_parameter_list_id,
            Some(mapping),
            self.source_parameter_origins_for_signature(&template.signature, &signature),
            string_table,
        )?;

        // --------------------------
        //  Build body parsing context
        // --------------------------
        let visibility = Rc::new(
            self.environment
                .lookups
                .import_environment
                .visibility_for(&template.source_file)
                .map_err(|error| self.error_messages(error, string_table))?
                .clone(),
        );
        let mut visible_declarations = visibility.visible_declaration_paths.clone();
        for parameter in &signature.parameters {
            visible_declarations.insert(parameter.id.to_owned());
        }

        let mut active_instance_stack = active_stack.to_vec();
        active_instance_stack.push(request.key.clone());

        let mut context = self
            .build_base_scope_context(BaseScopeContextInput {
                kind: ContextKind::Function,
                scope: request.instance_path.clone(),
                top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
                visibility,
                source_file_scope: template.source_file.clone(),
                scope_frame_capacity: 0,
            })
            .with_visible_declarations(visible_declarations)
            .with_active_generic_type_context(generic_type_context)
            .with_generic_function_instantiation_stack(active_instance_stack.clone());
        context.expected_result_type_ids = signature.success_return_type_ids();
        context.expected_error_type = signature.error_return_type_id();
        context.current_function_return_type_ids = context.expected_result_type_ids.clone();
        context.set_local_declarations(signature.parameters.to_owned());

        // --------------------------
        //  Parse body and materialize nested instances
        // --------------------------
        token_stream.src_path = request.instance_path.clone();
        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let body = match function_body_to_ast(
            &mut token_stream,
            context,
            &mut type_interner,
            &mut self.warnings,
            string_table,
        ) {
            Ok(body) => body,
            Err(diagnostic) => {
                let diagnostic = with_generic_instantiation_context(
                    *diagnostic,
                    GenericInstantiationDiagnosticContext {
                        call_location: request.call_location.clone(),
                        declaration_location: template.declaration_location.clone(),
                        substitutions: substitution_diagnostics,
                    },
                );
                return Err(self.diagnostic_messages(diagnostic, string_table));
            }
        };

        // Template validation already proved terminality, so a failure during concrete instance
        // reparse is an internal compiler invariant failure rather than a user-facing diagnostic.
        let policy = terminality_policy_for_signature(&signature, false);
        if let Some(diagnostic) =
            validate_function_body_terminality(&body, policy, template.declaration_location.clone())
        {
            return Err(self.error_messages(
                CompilerError::new(
                    format!(
                        "Generic function instance {} failed terminality validation after template validation succeeded",
                        request.instance_path.to_string(string_table)
                    ),
                    diagnostic.primary_location,
                    ErrorType::Compiler,
                ),
                string_table,
            ));
        }

        // Nested calls remain stable requests. The build-owned worklist materialises their
        // bodies, records dependency edges and detects indirect request cycles.
        self.defer_requested_generic_function_instances();

        // --------------------------
        //  Register instance and emit AST node
        // --------------------------
        self.generic_function_instances_by_key.insert(
            request.key.clone(),
            GenericFunctionInstance {
                instance_path: request.instance_path.clone(),
                key: request.key,
            },
        );
        self.ast.push(AstNode {
            kind: NodeKind::Function(request.instance_path.clone(), signature, body),
            location: template.declaration_location,
            scope: request.instance_path,
        });

        Ok(())
    }

    fn validate_generic_function_body(
        &mut self,
        header: Header,
        visibility: Rc<FileVisibility>,
        source_file_scope: InternedPath,
        scope_frame_capacity: usize,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        // --------------------------
        //  Retrieve resolved signature and template
        // --------------------------
        let Some(resolved_signature) = self
            .environment
            .lookups
            .resolved_function_signatures_by_path
            .get(&header.tokens.src_path)
            .cloned()
        else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Generic function signature was not resolved before body validation.",
                ),
                string_table,
            ));
        };

        let Some(template) = self
            .environment
            .lookups
            .generic_function_templates_by_path
            .get(&header.tokens.src_path)
            .cloned()
        else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Generic function template was not stored before body validation.",
                ),
                string_table,
            ));
        };

        // --------------------------
        //  Build validation context and run check
        // --------------------------
        let mut visible_declarations = visibility.visible_declaration_paths.clone();
        for parameter in &resolved_signature.signature.parameters {
            visible_declarations.insert(parameter.id.to_owned());
        }

        let mut context = self
            .build_base_scope_context(BaseScopeContextInput {
                kind: ContextKind::Function,
                scope: header.tokens.src_path.to_owned(),
                top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
                visibility,
                source_file_scope,
                scope_frame_capacity,
            })
            .with_visible_declarations(visible_declarations);
        let generic_type_context = self.build_active_generic_type_context(
            template.generic_parameter_list_id,
            None,
            FxHashMap::default(),
            string_table,
        )?;
        context = context.with_active_generic_type_context(generic_type_context);
        context.expected_result_type_ids = resolved_signature.signature.success_return_type_ids();
        context.expected_error_type = resolved_signature.signature.error_return_type_id();
        context.current_function_return_type_ids = context.expected_result_type_ids.clone();
        context.set_local_declarations(resolved_signature.signature.parameters);

        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        validate_generic_body_template(GenericFunctionBodyValidationInput {
            template: &template,
            context,
            type_interner: &mut type_interner,
            warnings: &mut self.warnings,
            string_table,
        })
        .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))
    }

    fn emit_function(
        &mut self,
        header: Header,
        visibility: Rc<FileVisibility>,
        source_file_scope: InternedPath,
        scope_frame_capacity: usize,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        // --------------------------
        //  Resolve signature
        // --------------------------
        let Some(resolved_signature) = self
            .environment
            .lookups
            .resolved_function_signatures_by_path
            .get(&header.tokens.src_path)
            .cloned()
        else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Function signature was not resolved before AST emission.",
                ),
                string_table,
            ));
        };

        // --------------------------
        //  Build body parsing context
        // --------------------------
        let mut visible_declarations = visibility.visible_declaration_paths.clone();
        for parameter in &resolved_signature.signature.parameters {
            visible_declarations.insert(parameter.id.to_owned());
        }

        // Top-level declarations are shared via Rc (no data copy);
        // parameters live in the function's current scope frame.
        let mut context = self
            .build_base_scope_context(BaseScopeContextInput {
                kind: ContextKind::Function,
                scope: header.tokens.src_path.to_owned(),
                top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
                visibility,
                source_file_scope,
                scope_frame_capacity,
            })
            .with_visible_declarations(visible_declarations);
        let expected_result_type_ids = resolved_signature.signature.success_return_type_ids();
        let expected_error_type = resolved_signature.signature.error_return_type_id();
        context.expected_result_type_ids = expected_result_type_ids;
        context.expected_error_type = expected_error_type;
        context.current_function_return_type_ids = context.expected_result_type_ids.clone();
        context.set_local_declarations(resolved_signature.signature.parameters.to_owned());

        // --------------------------
        //  Parse body and emit node
        // --------------------------
        let mut token_stream = header.tokens;
        let function_scope = context.scope.clone();

        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let body_result = function_body_to_ast(
            &mut token_stream,
            context,
            &mut type_interner,
            &mut self.warnings,
            string_table,
        );

        let body = body_result.map_err(|error| self.diagnostic_messages(*error, string_table))?;

        self.validate_body_terminality(
            &body,
            &resolved_signature.signature,
            false,
            header.name_location.clone(),
            string_table,
        )?;

        // AST symbol IDs are stored as full InternedPath values and are unique
        // module-wide, not only within a local scope.
        self.ast.push(AstNode {
            kind: NodeKind::Function(token_stream.src_path, resolved_signature.signature, body),
            location: header.name_location,
            scope: function_scope,
        });

        Ok(())
    }

    // --------------------------
    //  Emit start function bodies
    // --------------------------

    fn emit_start(
        &mut self,
        header: Header,
        visibility: Rc<FileVisibility>,
        source_file_scope: InternedPath,
        scope_frame_capacity: usize,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        // --------------------------
        //  Build context and parse body
        // --------------------------
        let context = self.build_base_scope_context(BaseScopeContextInput {
            kind: ContextKind::Module,
            scope: header.tokens.src_path.to_owned(),
            top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
            visibility,
            source_file_scope,
            scope_frame_capacity,
        });

        let mut token_stream = header.tokens;
        let start_scope = context.scope.clone();

        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let body_result = function_body_to_ast(
            &mut token_stream,
            context,
            &mut type_interner,
            &mut self.warnings,
            string_table,
        );

        let body = body_result.map_err(|error| self.diagnostic_messages(*error, string_table))?;

        // --------------------------
        //  Synthesize implicit start signature and emit node
        // --------------------------
        let full_name = token_stream
            .src_path
            .join_str(IMPLICIT_START_FUNC_NAME, string_table);

        // WHAT: entry start() returns Collection(StringSlice, MutableOwned),
        //       which is the Moth frontend type for Vec<String>.
        // WHY: compiler-design-overview.md describes the return type as Vec<String>;
        //      DataType::Collection(StringSlice) is the same contract
        //      expressed in frontend DataType terms. The HIR builder adds the implicit
        //      return of the accumulated fragment vec at function end.
        let start_return_type = DataType::collection(DataType::StringSlice);
        let start_return_type_id = resolve_diagnostic_type_to_type_id_checked(
            &start_return_type,
            &mut self.environment.type_environment,
            &header.name_location,
        )
        .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;
        let start_signature = FunctionSignature {
            parameters: vec![],
            returns: vec![ReturnSlot {
                value: start_return_type,
                type_id: Some(start_return_type_id),
                reactive_template: None,
                channel: ReturnChannel::Success,
            }],
        };

        self.validate_body_terminality(
            &body,
            &start_signature,
            true,
            header.name_location.clone(),
            string_table,
        )?;

        self.ast.push(AstNode {
            kind: NodeKind::Function(full_name, start_signature, body),
            location: header.name_location,
            scope: start_scope,
        });

        Ok(())
    }

    // --------------------------
    //  Emit struct definitions
    // --------------------------

    fn emit_struct(
        &mut self,
        header: Header,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let fields = self
            .environment
            .lookups
            .resolved_struct_fields_by_path
            .get(&header.tokens.src_path)
            .cloned()
            .ok_or_else(|| {
                self.error_messages(
                    CompilerError::compiler_error(
                        "Struct fields were not resolved before AST emission.",
                    ),
                    string_table,
                )
            })?;

        self.ast.push(AstNode {
            kind: NodeKind::StructDefinition(header.tokens.src_path.to_owned(), fields),
            location: header.name_location,
            scope: header.tokens.src_path,
        });

        Ok(())
    }

    // --------------------------
    //  Const template helpers
    // --------------------------

    fn parse_const_template(
        &mut self,
        template_tokens: &mut FileTokens,
        context: &ScopeContext,
        string_table: &mut StringTable,
    ) -> Result<ConstRequiredTemplateConstruction, CompilerMessages> {
        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let template_result = Template::new_const_required_with_type_interner(
            template_tokens,
            context,
            &mut type_interner,
            vec![],
            string_table,
        );

        let template =
            template_result.map_err(|error| self.diagnostic_messages(*error, string_table))?;

        Ok(template)
    }

    fn fold_const_template(
        &mut self,
        construction: ConstRequiredTemplateConstruction,
        context: &ScopeContext,
        string_table: &mut StringTable,
    ) -> Result<FoldedConstTemplateResult, CompilerMessages> {
        let ConstRequiredTemplateConstruction {
            template,
            preparation,
        } = construction;
        let reference = template.tir_reference;
        let store = context.template_ir_store.borrow();
        let view = TirView::with_minimum_phase(
            &store,
            reference.root,
            reference.phase,
            TemplateTirPhase::Composed,
            reference.context,
        )
        .map_err(|error| self.error_messages(error, string_table))?;
        let prepared = match preparation {
            PreparedTemplate::Foldable(prepared) => prepared,
            PreparedTemplate::Helper(_) => {
                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::HelperInConstTemplate,
                        template.location,
                    ),
                    string_table,
                ));
            }
            PreparedTemplate::Runtime(_) => {
                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::NonFoldableConstTemplate,
                        template.location,
                    ),
                    string_table,
                ));
            }
        };
        let mut fold_context = match context
            .new_template_fold_context(string_table, "top-level const template folding")
        {
            Ok(ctx) => ctx,
            Err(error) => {
                return Err(self.error_messages(error, string_table));
            }
        };
        // Top-level const fragments are builder-facing rendered text metadata; no semantic or
        // fingerprint consumer owns provenance at this boundary in the current milestone.
        let TemplateFoldResult {
            emission,
            provenance: _,
        } = match fold_prepared_template(&prepared, view, &mut fold_context) {
            Ok(result) => result,
            Err(error) => {
                drop(fold_context);
                return Err(self.template_error_messages(error, string_table));
            }
        };
        let value = match emission {
            TemplateEmission::Output(value) => value,
            TemplateEmission::NoOutput => fold_context.string_table.intern(""),
            TemplateEmission::Break(_) | TemplateEmission::Continue(_) => {
                drop(fold_context);
                return Err(self.error_messages(
                    CompilerError::compiler_error(
                        "Template loop-control signal escaped the nearest template loop during folding.",
                    ),
                    string_table,
                ));
            }
        };

        let result = FoldedConstTemplateResult::new(value);

        Ok(result)
    }

    /// Wraps an internal [`CompilerError`] into [`CompilerMessages`], preserving current
    /// warnings and the module type environment for render-time type-name resolution.
    fn error_messages(&self, error: CompilerError, string_table: &StringTable) -> CompilerMessages {
        CompilerMessages::from_error_with_warnings(error, self.warnings.clone(), string_table)
            .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
    }

    /// Wraps a user-facing [`CompilerDiagnostic`] into [`CompilerMessages`], preserving current
    /// warnings and the module type environment for render-time type-name resolution.
    fn diagnostic_messages(
        &self,
        diagnostic: CompilerDiagnostic,
        string_table: &StringTable,
    ) -> CompilerMessages {
        CompilerMessages::from_diagnostic_with_warnings(
            diagnostic,
            self.warnings.clone(),
            string_table,
        )
        .with_type_context_for_all_diagnostics(self.environment.type_environment.clone())
    }

    /// Converts a [`TemplateError`] (which may be user-facing or infrastructure) into the
    /// appropriate [`CompilerMessages`] wrapper.
    fn template_error_messages(
        &self,
        error: TemplateError,
        string_table: &StringTable,
    ) -> CompilerMessages {
        match error {
            TemplateError::Diagnostic(diagnostic) => {
                self.diagnostic_messages(*diagnostic, string_table)
            }
            TemplateError::Infrastructure(error) => self.error_messages(*error, string_table),
        }
    }

    /// Runs AST-owned terminality validation for a parsed function body.
    ///
    /// WHAT: converts the optional `FunctionMayFallThrough` diagnostic into the standard
    /// `CompilerMessages` wrapper used by this emitter.
    /// WHY: body parsing is complete at this point; missing-return diagnostics belong to AST,
    /// not to HIR lowering.
    fn validate_body_terminality(
        &self,
        body: &[AstNode],
        signature: &FunctionSignature,
        is_entry_start: bool,
        location: SourceLocation,
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
        let policy = terminality_policy_for_signature(signature, is_entry_start);
        if let Some(diagnostic) = validate_function_body_terminality(body, policy, location) {
            return Err(self.diagnostic_messages(diagnostic, string_table));
        }

        Ok(())
    }
}
