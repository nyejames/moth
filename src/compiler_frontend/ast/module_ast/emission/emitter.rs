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

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionBodyValidationInput, GenericFunctionInstance, GenericFunctionInstanceKey,
    GenericFunctionInstantiationRequest, GenericInstantiationDiagnosticContext, GenericRequestSlot,
    recursive_generic_function_instantiation, substitute_function_signature,
    validate_generic_function_body as validate_generic_body_template,
    with_generic_instantiation_context,
};
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::environment::AstModuleEnvironment;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    ContextKind, EnclosingCallableContract, EnclosingCallableOwner, ScopeContext,
};
use crate::compiler_frontend::ast::statements::body_dispatch::ParsedBody;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionParameter, FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::templates::create_template_node::PreparedTemplateConstruction;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationOutcome, TemplateTirPhase, TirView, fold_prepared_template,
};
use crate::compiler_frontend::ast::templates::top_level_templates::FoldedConstTemplateResult;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    templates, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDraft, GenericSubstitutionDiagnostic, InvalidTemplateStructureReason,
    PreparedFailure,
};
use std::sync::Arc;

use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::generic_parameters::{
    ActiveGenericTypeContext, GenericParameterScope,
};
use crate::compiler_frontend::datatypes::ids::{
    GenericParameterId, GenericParameterListId, TypeId,
};
use crate::compiler_frontend::headers::SyntheticContentPayload;
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceTokens;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::projects::settings::{self, IMPLICIT_START_FUNC_NAME};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(feature = "detailed_timers")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::timed_stage_attributed_opt;

fn enclosing_callable_contract(
    signature: &FunctionSignature,
    owner: EnclosingCallableOwner,
) -> Rc<EnclosingCallableContract> {
    let authored_success_slot_count = signature.success_return_slot_count();
    Rc::new(EnclosingCallableContract {
        owner,
        success_types: signature.success_return_type_ids(),
        authored_success_slot_count,
        error_type: signature.error_return_type_id(),
        parameter_contracts: signature
            .parameters
            .iter()
            .map(|parameter| (parameter.id, parameter.contract))
            .collect(),
    })
}

pub(in crate::compiler_frontend::ast) struct AstEmission {
    /// Typed AST nodes emitted for this module (functions, structs, generic instances).
    pub(in crate::compiler_frontend::ast) ast: Vec<AstNode>,
    /// Warnings accumulated during emission (unused variables, deprecated uses, etc.).
    pub(in crate::compiler_frontend::ast) warnings: Vec<DiagnosticDraft>,
    /// Folded top-level const template result records keyed by source file.
    pub(in crate::compiler_frontend::ast) const_templates_by_path:
        FxHashMap<PathId, FoldedConstTemplateResult>,
    /// Concrete generic function instances emitted while lowering visible calls.
    pub(in crate::compiler_frontend::ast) generic_instance_count: usize,
    /// Every generic request slot recorded during emission, in ledger order, including
    /// assertion-validation candidates. Static specialisation addresses slots by position;
    /// finalization partitions executable survivors from witnessed validation-only requests.
    /// Imported calls materialise through the declaring module's generated-function transaction.
    pub(in crate::compiler_frontend::ast) deferred_generic_requests: Vec<GenericRequestSlot>,
    /// Fully validated generic template bodies retained only through static specialisation and
    /// terminality. They never enter executable HIR.
    pub(in crate::compiler_frontend::ast) validated_generic_template_bodies: Vec<AstNode>,
}

#[cfg(test)]
#[path = "capacity_budget_tests.rs"]
mod capacity_budget_tests;

/// Shared input used by [`AstEmitter::build_base_scope_context`] to create a
/// [`ScopeContext`] that is identical across function, start, and const-template emission.
struct BaseScopeContextInput<'scope> {
    kind: ContextKind,
    scope: PathId,
    top_level_declarations: &'scope Rc<TopLevelDeclarationTable>,
    visibility: Arc<FileVisibility>,
    declaring_file_id: SourceId,
    source_file_scope: PathId,
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

/// Rebase each parameter's [`PathId`] from a bare name to a fully qualified path
/// under the given function path.
///
/// WHAT: ensures parameter symbols are module-unique before body parsing.
/// WHY: AST symbol IDs are complete [`PathId`] values, not local-scope names.
fn rebase_signature_parameters(
    signature: &mut FunctionSignature,
    function_path: PathId,
    path_fork: &mut PathInternerFork,
) -> Result<(), DiagnosticDraft> {
    for parameter in &mut signature.parameters {
        let Some(parameter_name) = path_fork.component(parameter.id) else {
            continue;
        };

        parameter.id = path_fork
            .try_intern_child(function_path, parameter_name)
            .ok_or_else(token_lexical::logical_path_capacity)?;
    }
    Ok(())
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
    ast: Vec<AstNode>,
    warnings: DiagnosticBag,
    context: &'context mut AstPhaseContext<'services>,
    path_fork: &'context mut PathInternerFork,
    environment: &'environment mut AstModuleEnvironment,
    source_token_owners: crate::compiler_frontend::headers::SourceTokenOwners,
    const_templates_by_path: FxHashMap<PathId, FoldedConstTemplateResult>,
    compatibility_cache: TypeCompatibilityCache,
    generic_function_instantiation_requests: Rc<RefCell<Vec<GenericRequestSlot>>>,
    known_infallible_functions: Rc<RefCell<FxHashSet<PathId>>>,
    generic_function_instances_by_key:
        FxHashMap<GenericFunctionInstanceKey, GenericFunctionInstance>,
    validated_generic_template_bodies: Vec<AstNode>,
    generic_call_site_identity_handle: Option<FrozenIdentityHandle>,
}
impl<'context, 'services, 'environment> AstEmitter<'context, 'services, 'environment> {
    pub(in crate::compiler_frontend::ast) fn new(
        context: &'context mut AstPhaseContext<'services>,
        environment: &'environment mut AstModuleEnvironment,
        header_count: usize,
        path_fork: &'context mut PathInternerFork,
        source_token_owners: crate::compiler_frontend::headers::SourceTokenOwners,
    ) -> Self {
        let lookups = std::rc::Rc::get_mut(&mut environment.lookups)
            .expect("AST environment lookups are uniquely owned before emission");
        let warnings = DiagnosticBag::from_diagnostics(std::mem::take(&mut lookups.warnings));
        Self {
            context,
            path_fork,
            source_token_owners,
            environment,
            ast: Vec::with_capacity(header_count * settings::TOKEN_TO_NODE_RATIO),
            warnings,
            const_templates_by_path: FxHashMap::default(),
            compatibility_cache: TypeCompatibilityCache::new(),
            generic_function_instantiation_requests: Rc::new(RefCell::new(Vec::new())),
            known_infallible_functions: Rc::new(RefCell::new(FxHashSet::default())),
            generic_function_instances_by_key: FxHashMap::default(),
            validated_generic_template_bodies: Vec::new(),
            generic_call_site_identity_handle: None,
        }
    }

    pub(in crate::compiler_frontend::ast) fn with_generic_call_site_identity_handle(
        mut self,
        handle: FrozenIdentityHandle,
    ) -> Self {
        self.generic_call_site_identity_handle = Some(handle);
        self
    }

    fn generic_call_site_identity_handle(&self) -> Option<FrozenIdentityHandle> {
        self.generic_call_site_identity_handle.clone().or_else(|| {
            self.context
                .file_value_resolution
                .as_ref()
                .map(|services| services.frozen_identity_handle.clone())
        })
    }

    pub(in crate::compiler_frontend::ast) fn emit_generated_request(
        mut self,
        request: GenericFunctionInstantiationRequest,
        string_table: &mut StringTable,
    ) -> Result<AstEmission, PreparedFailure> {
        self.emit_generic_function_instance(request, &[], string_table)?;
        let deferred_generic_requests = self.take_generic_request_ledger();

        Ok(AstEmission {
            ast: self.ast,
            warnings: self.warnings.into_diagnostics(),
            const_templates_by_path: self.const_templates_by_path,
            generic_instance_count: self.generic_function_instances_by_key.len(),
            deferred_generic_requests,
            validated_generic_template_bodies: self.validated_generic_template_bodies,
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
        let mut context = ScopeContext::new(
            input.kind,
            input.scope,
            Rc::clone(input.top_level_declarations),
            Arc::clone(&self.context.external_package_registry),
            input.scope_frame_capacity,
            self.context.template_ir_store.clone(),
        )
        .with_style_directives(self.context.style_directives)
        .with_build_profile(self.context.build_profile)
        .with_template_const_loop_iteration_limit(self.context.template_const_loop_iteration_limit)
        .with_file_visibility(Arc::clone(&input.visibility))
        .with_resolved_type_aliases(Rc::clone(
            &self.environment.lookups.resolved_type_aliases_by_path,
        ))
        .with_generic_declarations(Rc::clone(
            &self.environment.lookups.generic_declarations_by_path,
        ))
        .with_resolved_struct_fields_by_path(Rc::clone(
            &self.environment.lookups.resolved_struct_fields_by_path,
        ))
        .with_generic_function_instantiation_sink(Rc::clone(
            &self.generic_function_instantiation_requests,
        ))
        .with_known_infallible_functions(Rc::clone(&self.known_infallible_functions))
        .with_receiver_methods(Rc::clone(&self.environment.lookups.receiver_methods))
        .with_lookups(Rc::clone(&self.environment.lookups))
        .with_source_file_scope(input.source_file_scope)
        .with_declaring_file_id(input.declaring_file_id);
        if let Some(services) = &self.context.file_value_resolution {
            context = context.with_file_value_resolution(Rc::clone(services));
        }
        // Body expression evaluation reports source-`#Config` provenance for numeric failures,
        // so the resolved map and the bootstrap services travel with the scope the same way
        // they travel with constant-header scopes. Only the two provenance owners are attached;
        // contract names stay a constant-header concern and cost nothing here.
        if !self.context.build_config_values.is_empty() {
            context = context
                .with_source_build_config_values(Arc::clone(&self.context.build_config_values));
        }
        if let Some(services) = &self.context.config_resolution {
            context = context.with_config_resolution(Rc::clone(services));
        }
        context
    }

    fn source_path_for_header(&self, header: &Header) -> Result<PathId, InfrastructureFailure> {
        let ownerless_synthetic_constant = matches!(
            header.synthetic_content_payload,
            Some(SyntheticContentPayload::RenderedHtml(_))
        ) && header.tokens.is_empty()
            && header.token_sequence.is_none()
            && header.name_span.is_none()
            && matches!(header.kind, HeaderKind::Constant { .. });
        if !self
            .source_token_owners
            .contains_key(&header.tokens.source())
            && !ownerless_synthetic_constant
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "header source path has no canonical prepared source token owner",
            ));
        }
        self.environment
            .lookups
            .module_symbols
            .source_path_for_header(header)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "header source path was not retained by the prepared source identity",
                )
            })
    }

    fn canonical_owner_for_header(
        &self,
        header: &Header,
    ) -> Result<Arc<SourceTokens>, InfrastructureFailure> {
        let owner = self
            .source_token_owners
            .get(&header.tokens.source())
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "header body range has no canonical prepared source stream",
                )
            })?;
        Ok(Arc::clone(owner.tokens()))
    }

    pub(in crate::compiler_frontend::ast) fn emit(
        mut self,
        sorted_headers: Vec<Header>,
        string_table: &mut StringTable,
    ) -> Result<AstEmission, PreparedFailure> {
        // The environment owns the single resolved declaration table. Body contexts clone only
        // the Rc pointer so declaration metadata is not rebuilt during emission.
        let top_level_declarations = Rc::clone(&self.environment.lookups.declaration_table);

        #[cfg(feature = "detailed_timers")]
        let mut function_headers_emitted = 0usize;
        #[cfg(feature = "detailed_timers")]
        let mut start_headers_emitted = 0usize;
        #[cfg(feature = "detailed_timers")]
        let mut const_templates_emitted = 0usize;

        let root_scope_consumer_count = count_root_scope_arena_consumers(&sorted_headers);
        let mut scope_frame_capacity_budget = ScopeFrameCapacityBudget::new(
            self.context.capacity_estimate.scope_frames,
            root_scope_consumer_count,
        );

        for header in sorted_headers {
            let source_file_scope = self
                .source_path_for_header(&header)
                .map_err(|error| self.error_messages(error, string_table))?;
            let visibility = Arc::clone(
                self.environment
                    .lookups
                    .binding_environment
                    .visibility_for(&source_file_scope)
                    .map_err(|error| self.error_messages(error, string_table))?,
            );

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

                    self.emit_function(
                        header,
                        visibility,
                        source_file_scope,
                        scope_frame_capacity_budget.next_root_capacity(),
                        string_table,
                    )?;
                    #[cfg(feature = "detailed_timers")]
                    {
                        function_headers_emitted += 1;
                    }
                }

                HeaderKind::StartFunction => {
                    self.emit_start(
                        header,
                        visibility,
                        source_file_scope,
                        scope_frame_capacity_budget.next_root_capacity(),
                        string_table,
                    )?;
                    #[cfg(feature = "detailed_timers")]
                    {
                        start_headers_emitted += 1;
                    }
                }

                HeaderKind::Struct {
                    generic_parameters, ..
                } => {
                    if !generic_parameters.is_empty() {
                        continue;
                    }

                    self.emit_struct(header)?;
                }

                // Constants and choices are fully handled during environment construction.
                HeaderKind::Constant { .. } | HeaderKind::Choice { .. } => {}
                HeaderKind::ConstTemplate { .. } => {
                    let template_path = header.declaration_path;
                    let template_source = self
                        .canonical_owner_for_header(&header)
                        .map_err(|error| self.error_messages(error, string_table))?;
                    let context = self.build_base_scope_context(BaseScopeContextInput {
                        kind: ContextKind::Constant,
                        scope: template_path,
                        top_level_declarations: &top_level_declarations,
                        visibility,
                        declaring_file_id: header.tokens.source(),
                        source_file_scope,
                        scope_frame_capacity: scope_frame_capacity_budget.next_root_capacity(),
                    });

                    let template = timed_stage_attributed_opt!(
                        self.context.timing_metric_family.const_template_parse(),
                        self.context.timing_context,
                        self.parse_const_template(
                            &header,
                            &template_source,
                            &context,
                            string_table
                        )?
                    );
                    for warning in context.take_emitted_warnings() {
                        self.warnings.push(warning);
                    }

                    let folded_result = timed_stage_attributed_opt!(
                        self.context.timing_metric_family.const_template_fold(),
                        self.context.timing_context,
                        self.fold_const_template(template, &context, string_table)?
                    );
                    #[cfg(feature = "detailed_timers")]
                    {
                        const_templates_emitted += 1;
                    }

                    self.const_templates_by_path
                        .insert(template_path, folded_result);
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

        let deferred_generic_requests = self.take_generic_request_ledger();

        #[cfg(feature = "detailed_timers")]
        {
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
            warnings: self.warnings.into_diagnostics(),
            const_templates_by_path: self.const_templates_by_path,
            generic_instance_count: self.generic_function_instances_by_key.len(),
            deferred_generic_requests,
            validated_generic_template_bodies: self.validated_generic_template_bodies,
        })
    }

    // --------------------------
    //  Emit function bodies
    // --------------------------

    /// Drains the module-wide generic request ledger exactly once, at emission finish.
    ///
    /// WHAT: returns every slot in recording order and registers only executable concrete keys
    ///       as known instances. Assertion-validation candidates stay owned by their ledger slots.
    /// WHY: static-if ranges and tickets address stable positions, so the ledger is never drained
    ///      mid-emission. Every provisional origin survives until specialization removes inactive
    ///      ranges and finalization selects retained assertion witnesses. Deduplicating earlier
    ///      would lose an active duplicate whose first call was inactive or validation-only.
    fn take_generic_request_ledger(&mut self) -> Vec<GenericRequestSlot> {
        let slots = std::mem::take(&mut *self.generic_function_instantiation_requests.borrow_mut());

        for slot in &slots {
            if let GenericRequestSlot::Concrete(request) = slot
                && !self
                    .generic_function_instances_by_key
                    .contains_key(&request.key)
            {
                self.generic_function_instances_by_key.insert(
                    request.key.clone(),
                    GenericFunctionInstance {
                        instance_path: request.instance_path,
                        key: request.key.clone(),
                    },
                );
            }
        }

        slots
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
        source_parameter_by_rebased_path: FxHashMap<PathId, GenericParameterId>,
    ) -> Result<ActiveGenericTypeContext, InfrastructureFailure> {
        let Some(parameter_list) = self
            .environment
            .type_environment
            .generic_parameters(parameter_list_id)
        else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "Generic function body requested an unknown generic parameter list.",
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
    ) -> FxHashMap<PathId, GenericParameterId> {
        let mut origins = FxHashMap::default();

        for (source_parameter, emitted_parameter) in source_signature
            .parameters
            .iter()
            .zip(emitted_signature.parameters.iter())
        {
            let Some(TypeDefinition::GenericParameter(parameter)) = self
                .environment
                .type_environment
                .get(source_parameter.type_id)
            else {
                continue;
            };

            origins.insert(emitted_parameter.id, parameter.id);
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
    ) -> Result<(), PreparedFailure> {
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
            let mut diagnostic = recursive_generic_function_instantiation(
                self.path_fork.component(request.key.function_path),
                request.call_span,
            );
            if request.call_span.is_some()
                && let Some(handle) = self.generic_call_site_identity_handle()
            {
                diagnostic = diagnostic.with_primary_frozen_identity_handle(handle);
            }
            return Err(self.diagnostic_messages(diagnostic, string_table));
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
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Generic function instance requested for an unknown template.",
                ),
                string_table,
            ));
        };

        let Some(body) = template.body_tokens.clone() else {
            let instance = GenericFunctionInstance {
                instance_path: request.instance_path,
                key: request.key.clone(),
            };
            self.generic_function_instances_by_key
                .insert(request.key.clone(), instance);
            self.generic_function_instantiation_requests
                .borrow_mut()
                .push(GenericRequestSlot::Concrete(request));
            return Ok(());
        };
        let parse_owner = body.parse_owner();
        let (mut body_cursor, body_source_id) = parse_owner
            .cursor()
            .map_err(|error| self.error_messages(error, string_table))?;
        let frozen_identity_handle = body.frozen_identity_handle().cloned();

        let Some(mapping) = self.environment.type_environment.build_parameter_mapping(
            template.generic_parameter_list_id,
            request.key.type_arguments.as_ref(),
        ) else {
            return Err(self.error_messages(
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
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
        rebase_signature_parameters(&mut signature, request.instance_path, &mut *self.path_fork)?;
        let generic_type_context = self.build_active_generic_type_context(
            template.generic_parameter_list_id,
            Some(mapping),
            self.source_parameter_origins_for_signature(&template.signature, &signature),
        )?;

        // --------------------------
        //  Build body parsing context
        // --------------------------
        let visibility = Arc::clone(
            self.environment
                .lookups
                .binding_environment
                .visibility_for(&template.source_file)
                .map_err(|error| self.error_messages(error, string_table))?,
        );
        let mut visible_declarations = (*visibility.visible_declaration_paths).clone();
        for parameter in &signature.parameters {
            visible_declarations.insert(parameter.id.to_owned());
        }

        let mut active_instance_stack = active_stack.to_vec();
        active_instance_stack.push(request.key.clone());

        let mut context = self
            .build_base_scope_context(BaseScopeContextInput {
                kind: ContextKind::Function,
                scope: request.instance_path,
                top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
                visibility,
                // Materialised bodies carry their retained donor owner; the scope threads it so
                // Stage 0 lookups validate against the frozen facts owner and never alias the
                // requester call-site source.
                declaring_file_id: body_source_id,
                source_file_scope: template.source_file,
                scope_frame_capacity: 0,
            })
            .with_visible_declarations(Arc::new(visible_declarations))
            .with_active_generic_type_context(generic_type_context)
            .with_generic_function_instantiation_stack(active_instance_stack.clone());
        if let Some(facts) = body.resolution_facts() {
            let Some(services) = self.context.file_value_resolution.as_ref() else {
                return Err(self.error_messages(
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "Materialised generic body has Stage 0 facts but no file-value services",
                    ),
                    string_table,
                ));
            };
            context = context.with_file_value_resolution(
                services.with_stage0_resolution_facts(Arc::clone(facts)),
            );
        }
        if let Some(frozen_identity_handle) = frozen_identity_handle.as_ref() {
            context = context.with_frozen_identity_handle(frozen_identity_handle.clone());
        }
        context.enclosing_callable = Some(enclosing_callable_contract(
            &signature,
            EnclosingCallableOwner::Function,
        ));
        context
            .set_local_declarations(
                signature
                    .parameters
                    .iter()
                    .map(FunctionParameter::to_local_declaration)
                    .collect(),
                &*self.path_fork,
                string_table,
            )
            .map_err(|diagnostic| self.diagnostic_messages(diagnostic, string_table))?;
        // --------------------------
        //  Parse body and materialize nested instances
        // --------------------------
        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let mut body_warnings = DiagnosticBag::new();
        let body_result = function_body_to_ast(
            &mut body_cursor,
            context,
            &mut type_interner,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
            &mut body_warnings,
            string_table,
            &mut *self.path_fork,
        );
        if matches!(&body_result, Ok(ParsedBody::Complete(_)))
            && let Some(frozen_identity_handle) = frozen_identity_handle.as_ref()
        {
            for warning in body_warnings.iter_mut() {
                warning.attach_frozen_identity_handle_if_missing(frozen_identity_handle.clone());
            }
        }
        self.warnings.append(body_warnings);
        let body = match body_result {
            Ok(ParsedBody::Complete(body)) => body,
            Ok(ParsedBody::Draft(_)) => {
                return Err(self.error_messages(
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "a materialised generic function body returned an unresolved numeric draft",
                    ),
                    string_table,
                ));
            }
            Err(ExpressionParseError::Diagnostic(diagnostic))
            | Err(ExpressionParseError::TemplateDirectiveEmptyArguments(diagnostic))
            | Err(ExpressionParseError::NonConstantReferenceInConstant(diagnostic)) => {
                let diagnostic = with_generic_instantiation_context(
                    diagnostic,
                    GenericInstantiationDiagnosticContext {
                        call_span: request.call_span,
                        declaration_span: template.declaration_span,
                        frozen_identity_handle: frozen_identity_handle.clone(),
                        call_site_frozen_identity_handle: if request.call_span.is_some() {
                            self.generic_call_site_identity_handle()
                        } else {
                            None
                        },
                        substitutions: substitution_diagnostics,
                    },
                );
                return Err(self.diagnostic_messages(diagnostic, string_table));
            }
            Err(error) => return Err(self.expression_error_messages(error, string_table)),
        };

        // Nested calls remain stable requests. The generated-function transaction materialises
        // their bodies, records dependency edges and detects indirect request cycles.

        // --------------------------
        //  Register instance and emit AST node
        // --------------------------
        self.generic_function_instances_by_key.insert(
            request.key.clone(),
            GenericFunctionInstance {
                instance_path: request.instance_path,
                key: request.key,
            },
        );
        self.ast.push(AstNode {
            kind: NodeKind::Function(request.instance_path, signature, body),
            // Generated instance has no consumer-local authored declaration range.
            span: None,
            scope: request.instance_path,
        });

        Ok(())
    }

    fn validate_generic_function_body(
        &mut self,
        header: Header,
        visibility: Arc<FileVisibility>,
        source_file_scope: PathId,
        scope_frame_capacity: usize,
        string_table: &mut StringTable,
    ) -> Result<(), PreparedFailure> {
        // --------------------------
        //  Retrieve resolved signature and template
        // --------------------------
        let Some(resolved_signature) = self
            .environment
            .lookups
            .resolved_function_signatures_by_path
            .get(&header.declaration_path)
            .cloned()
        else {
            return Err(self.error_messages(
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Generic function signature was not resolved before body validation.",
                ),
                string_table,
            ));
        };

        let Some(template) = self
            .environment
            .lookups
            .generic_function_templates_by_path
            .get(&header.declaration_path)
            .cloned()
        else {
            return Err(self.error_messages(
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Generic function template was not stored before body validation.",
                ),
                string_table,
            ));
        };

        // --------------------------
        //  Build validation context and run check
        // --------------------------
        let mut visible_declarations = (*visibility.visible_declaration_paths).clone();
        for parameter in &resolved_signature.signature.parameters {
            visible_declarations.insert(parameter.id.to_owned());
        }

        let mut context = self
            .build_base_scope_context(BaseScopeContextInput {
                kind: ContextKind::Function,
                scope: header.declaration_path,
                top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
                visibility,
                declaring_file_id: header.tokens.source(),
                source_file_scope,
                scope_frame_capacity,
            })
            .with_visible_declarations(Arc::new(visible_declarations));
        let generic_type_context = self.build_active_generic_type_context(
            template.generic_parameter_list_id,
            None,
            FxHashMap::default(),
        )?;
        context = context.with_active_generic_type_context(generic_type_context);
        context.enclosing_callable = Some(enclosing_callable_contract(
            &resolved_signature.signature,
            EnclosingCallableOwner::Function,
        ));
        context
            .set_local_declarations(
                resolved_signature
                    .signature
                    .parameters
                    .iter()
                    .map(FunctionParameter::to_local_declaration)
                    .collect(),
                &*self.path_fork,
                string_table,
            )
            .map_err(|diagnostic| self.diagnostic_messages(diagnostic, string_table))?;

        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let body = validate_generic_body_template(GenericFunctionBodyValidationInput {
            template: &template,
            context,
            type_interner: &mut type_interner,
            expression_builder: &mut self.context.expression_builder,
            const_values: &mut self.context.const_values,
            warnings: &mut self.warnings,
            string_table,
            path_fork: &mut *self.path_fork,
        })
        .map_err(|error| self.expression_error_messages(error, string_table))?;

        self.validated_generic_template_bodies.push(AstNode {
            kind: NodeKind::Function(template.function_path, resolved_signature.signature, body),
            // Generic body validation retains no consumer-local authored declaration range.
            span: None,
            scope: header.declaration_path,
        });
        Ok(())
    }

    fn emit_function(
        &mut self,
        header: Header,
        visibility: Arc<FileVisibility>,
        source_file_scope: PathId,
        scope_frame_capacity: usize,
        string_table: &mut StringTable,
    ) -> Result<(), PreparedFailure> {
        // --------------------------
        //  Resolve signature
        // --------------------------
        let Some(resolved_signature) = self
            .environment
            .lookups
            .resolved_function_signatures_by_path
            .get(&header.declaration_path)
            .cloned()
        else {
            return Err(self.error_messages(
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Function signature was not resolved before AST emission.",
                ),
                string_table,
            ));
        };

        // --------------------------
        //  Build body parsing context
        // --------------------------
        let mut visible_declarations = (*visibility.visible_declaration_paths).clone();
        for parameter in &resolved_signature.signature.parameters {
            visible_declarations.insert(parameter.id.to_owned());
        }

        // Top-level declarations are shared via Rc (no data copy);
        // parameters live in the function's current scope frame.
        let mut context = self
            .build_base_scope_context(BaseScopeContextInput {
                kind: ContextKind::Function,
                scope: header.declaration_path,
                top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
                visibility,
                declaring_file_id: header.tokens.source(),
                source_file_scope,
                scope_frame_capacity,
            })
            .with_visible_declarations(Arc::new(visible_declarations));
        context.enclosing_callable = Some(enclosing_callable_contract(
            &resolved_signature.signature,
            EnclosingCallableOwner::Function,
        ));
        context
            .set_local_declarations(
                resolved_signature
                    .signature
                    .parameters
                    .iter()
                    .map(FunctionParameter::to_local_declaration)
                    .collect(),
                &*self.path_fork,
                string_table,
            )
            .map_err(|diagnostic| self.diagnostic_messages(diagnostic, string_table))?;

        // --------------------------
        //  Parse body and emit node
        // --------------------------
        // Direct canonical body cursor: share the prepared-source owner over the
        // bounded canonical source-token range. Segmented headers use the retained
        // sequence; contiguous headers use the range. Loop/template windows and
        // explicit synthetic streams use their own explicit cursors.
        let body_source = self
            .canonical_owner_for_header(&header)
            .map_err(|error| self.error_messages(error, string_table))?;
        let function_scope = context.scope;
        let mut body_cursor = match header.token_sequence {
            Some(sequence) => AstCursor::from_source_sequence(&body_source, sequence),
            None => AstCursor::from_source_tokens(&body_source, header.tokens),
        }
        .map_err(|error| self.error_messages(error, string_table))?;
        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let mut body_warnings = DiagnosticBag::new();
        let body_result = function_body_to_ast(
            &mut body_cursor,
            context,
            &mut type_interner,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
            &mut body_warnings,
            string_table,
            &mut *self.path_fork,
        );
        self.warnings.append(body_warnings);

        let body = body_result
            .and_then(|parsed| parsed.expect_complete_nodes("a source function body"))
            .map_err(|error| self.expression_error_messages(error, string_table))?;

        // AST symbol IDs are stored as complete PathId values and are unique
        // module-wide, not only within a local scope.
        self.ast.push(AstNode {
            kind: NodeKind::Function(header.declaration_path, resolved_signature.signature, body),
            span: header.name_span,
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
        visibility: Arc<FileVisibility>,
        source_file_scope: PathId,
        scope_frame_capacity: usize,
        string_table: &mut StringTable,
    ) -> Result<(), PreparedFailure> {
        // The synthetic signature owns the same propagation context as an authored
        // function. Keep Module parsing rules for root declarations and fragments.
        let mut context = self.build_base_scope_context(BaseScopeContextInput {
            kind: ContextKind::Module,
            scope: header.declaration_path,
            top_level_declarations: &Rc::clone(&self.environment.lookups.declaration_table),
            visibility,
            declaring_file_id: header.tokens.source(),
            source_file_scope,
            scope_frame_capacity,
        });

        let error_type = resolve_builtin_error_type_typed(&context, None, string_table)
            .map_err(|error| self.error_messages(error, string_table))?
            .type_id;
        let builtin_string_type_id = self.environment.type_environment.builtins().string;
        let start_return_type_id = self
            .environment
            .type_environment
            .intern_collection(builtin_string_type_id, None);
        let start_signature = FunctionSignature {
            parameters: vec![],
            returns: vec![
                ReturnSlot {
                    type_id: Some(start_return_type_id),
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    type_id: Some(error_type),
                    channel: ReturnChannel::Error,
                },
            ],
        };
        // Root initializers infer their own success types. The fragment collection
        // is an implicit HIR return, not a receiving expectation for those values.
        context.enclosing_callable = Some(enclosing_callable_contract(
            &start_signature,
            EnclosingCallableOwner::EntryStart,
        ));

        let start_source = self
            .canonical_owner_for_header(&header)
            .map_err(|error| self.error_messages(error, string_table))?;
        let start_scope = context.scope;
        let start_src_path = header.declaration_path;
        let mut body_cursor = match header.token_sequence {
            Some(sequence) => AstCursor::from_source_sequence(&start_source, sequence),
            None => AstCursor::from_source_tokens(&start_source, header.tokens),
        }
        .map_err(|error| self.error_messages(error, string_table))?;
        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let mut body_warnings = DiagnosticBag::new();
        let body_result = function_body_to_ast(
            &mut body_cursor,
            context,
            &mut type_interner,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
            &mut body_warnings,
            string_table,
            &mut *self.path_fork,
        );
        self.warnings.append(body_warnings);

        let body = body_result
            .and_then(|parsed| parsed.expect_complete_nodes("the implicit start function body"))
            .map_err(|error| self.expression_error_messages(error, string_table))?;

        // --------------------------
        // Emit the module-owned start without changing root selection or activation.
        let full_name = self
            .path_fork
            .try_intern_child(
                start_src_path,
                string_table.intern(IMPLICIT_START_FUNC_NAME),
            )
            .ok_or_else(token_lexical::logical_path_capacity)?;

        self.ast.push(AstNode {
            kind: NodeKind::Function(full_name, start_signature, body),
            // The implicit start function has no authored declaration span.
            span: None,
            scope: start_scope,
        });

        Ok(())
    }

    // --------------------------
    //  Emit struct definitions
    // --------------------------

    fn emit_struct(&mut self, header: Header) -> Result<(), InfrastructureFailure> {
        let fields = self
            .environment
            .lookups
            .resolved_struct_fields_by_path
            .get(&header.declaration_path)
            .cloned()
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Struct fields were not resolved before AST emission.",
                )
            })?;

        self.ast.push(AstNode {
            kind: NodeKind::StructDefinition(header.declaration_path.to_owned(), fields),
            span: header.name_span,
            scope: header.declaration_path,
        });

        Ok(())
    }

    // --------------------------
    //  Const template helpers
    // --------------------------

    fn parse_const_template(
        &mut self,
        header: &Header,
        template_source: &Arc<SourceTokens>,
        context: &ScopeContext,
        string_table: &mut StringTable,
    ) -> Result<PreparedTemplateConstruction, PreparedFailure> {
        let source_path = context.scope;
        let mut template_cursor = match header.token_sequence {
            Some(sequence) => AstCursor::from_source_sequence(template_source, sequence),
            None => AstCursor::from_source_tokens(template_source, header.tokens),
        }
        .map_err(|error| self.error_messages(error, string_table))?;
        let mut type_interner = AstTypeInterner::new(
            &mut self.environment.type_environment,
            &mut self.compatibility_cache,
        );
        let template_result = Template::new_const_required_with_type_interner(
            &mut template_cursor,
            source_path,
            context,
            &mut type_interner,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
            vec![],
            string_table,
            &mut *self.path_fork,
        );

        let template = template_result
            .map_err(|error| self.expression_error_messages(error.into(), string_table))?;

        Ok(template)
    }

    fn fold_const_template(
        &mut self,
        construction: PreparedTemplateConstruction,
        context: &ScopeContext,
        string_table: &mut StringTable,
    ) -> Result<FoldedConstTemplateResult, PreparedFailure> {
        let PreparedTemplateConstruction {
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
        if !matches!(preparation.outcome, TemplatePreparationOutcome::Foldable) {
            return Err(self.diagnostic_messages(
                match preparation.outcome {
                    TemplatePreparationOutcome::SlotInsertHelper => {
                        templates::invalid_template_structure(
                            InvalidTemplateStructureReason::HelperInConstTemplate,
                            template.span,
                        )
                    }
                    TemplatePreparationOutcome::Runtime(_) => {
                        templates::invalid_template_structure(
                            InvalidTemplateStructureReason::NonFoldableConstTemplate,
                            template.span,
                        )
                    }
                    TemplatePreparationOutcome::Foldable => crate::compiler_bug!(
                        CompilerStage::AstConstruction,
                        "fold_const_template already returned for Foldable; this match only handles non-Foldable outcomes"
                    ),
                },
                string_table,
            ));
        }
        let prepared = preparation;
        let mut fold_context = context.new_tir_fold_context(
            string_table,
            &mut self.context.expression_builder,
            &mut self.context.const_values,
        );
        // Top-level const fragments are builder-facing rendered text metadata; no semantic or
        // fingerprint consumer owns provenance at this boundary in the current milestone.
        let mut result = match fold_prepared_template(&prepared, view, &mut fold_context) {
            Ok(result) => result,
            Err(error) => {
                drop(fold_context);
                return Err(self.template_error_messages(error, string_table));
            }
        };
        if let Some(diagnostic) = result.take_required_diagnostic() {
            // A top-level const fragment is rendered text metadata: structural text anchored to a
            // resource or site root has no characters at this boundary.
            drop(fold_context);
            return Err(self.diagnostic_messages(diagnostic, string_table));
        }
        let TemplateFoldResult { emission, .. } = result;
        let value = match emission {
            TemplateEmission::Output(value) => value,
            TemplateEmission::NoOutput => {
                ConstStringValue::Text(fold_context.string_table.intern(""))
            }
        };

        let result = FoldedConstTemplateResult::new(value);

        Ok(result)
    }

    /// Routes infrastructure failures through their typed lane.
    fn error_messages(
        &self,
        error: impl Into<Box<InfrastructureErrors>>,
        _string_table: &StringTable,
    ) -> PreparedFailure {
        PreparedFailure::Infrastructure(error.into())
    }

    /// Captures a user-facing diagnostic and accumulated warnings before the environment drops.
    fn diagnostic_messages(
        &mut self,
        diagnostic: DiagnosticDraft,
        string_table: &mut StringTable,
    ) -> PreparedFailure {
        self.diagnostic_bag_messages(DiagnosticBag::from(diagnostic), string_table)
    }

    fn diagnostic_bag_messages(
        &mut self,
        bag: DiagnosticBag,
        string_table: &mut StringTable,
    ) -> PreparedFailure {
        let mut diagnostics = std::mem::take(&mut self.warnings);
        diagnostics.append(bag);
        PreparedFailure::capture_diagnosed(
            diagnostics,
            string_table.take_for_handoff(),
            Some(&self.environment.type_environment),
            Some(Arc::new(self.path_fork.shared_table())),
        )
    }

    /// Converts a [`TemplateError`] (which may be user-facing or infrastructure) into the
    /// appropriate AST diagnostic failure lane.
    fn template_error_messages(
        &mut self,
        error: TemplateError,
        string_table: &mut StringTable,
    ) -> PreparedFailure {
        match error {
            TemplateError::Diagnostic(diagnostic)
            | TemplateError::EmptyArguments(diagnostic)
            | TemplateError::NonConstantReferenceInConstant(diagnostic) => {
                self.diagnostic_messages(diagnostic, string_table)
            }
            TemplateError::Infrastructure(error) => self.error_messages(error, string_table),
        }
    }

    /// Converts the AST parser's two-lane failure without downgrading retained-data errors.
    ///
    /// A numeric deferral must never reach the emitter: every retaining lane completes at its
    /// body root, so a leak here is a precise invariant failure naming the boundary.
    fn expression_error_messages(
        &mut self,
        error: ExpressionParseError,
        string_table: &mut StringTable,
    ) -> PreparedFailure {
        match error.at_boundary_without_numeric_retention("the module emitter body boundary") {
            Ok(diagnostic) => self.diagnostic_messages(diagnostic, string_table),
            Err(ExpressionParseError::Infrastructure(errors)) => {
                self.error_messages(errors, string_table)
            }
            Err(
                ExpressionParseError::Diagnostic(_)
                | ExpressionParseError::TemplateDirectiveEmptyArguments(_)
                | ExpressionParseError::NonConstantReferenceInConstant(_)
                | ExpressionParseError::NumericDeferral(_),
            ) => crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "module emitter body boundary received an unconverted parse error",
            ),
        }
    }
}
