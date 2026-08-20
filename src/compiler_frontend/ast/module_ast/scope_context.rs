//! Shared parser/lowering context for one active AST scope.
//!
//! WHAT: `ScopeContext` carries all state needed to parse/lower a single scope — declarations,
//! visibility gates, type expectations, and optional path-resolution capability.
//!
//! WHY: passing it as one struct avoids large parameter lists across recursive parsing calls,
//! and makes clone-to-child cheap without rebuilding immutable semantic lookup tables.
//!
//! ## Relationship to AST emission
//!
//! `AstEmitter` creates `ScopeContext` fresh for each function/template body after the semantic
//! environment is complete. `ScopeContext` owns only local scope growth through parent-linked
//! frames, loop depth, and type expectations.
//!
//! `ScopeContext` receives shared state from the completed environment (for example
//! `Rc<TopLevelDeclarationTable>` for top-level symbols and `Rc<ReceiverMethodCatalog>` for
//! method lookup) so body parsing is self-contained without referencing the mutable environment
//! builder directly.
//! Semantic lookups are immutable after environment construction. Interior-mutable shared state is
//! limited to emission side channels plus the AST-local TIR cache tied to the shared module store.
//!
//! ## External symbol visibility
//!
//! File-local visibility originates from the header-built `FileVisibility` struct and is
//! applied to each `ScopeContext` via `with_file_visibility`. This includes same-file
//! declarations, imported source symbols, type aliases, and external package symbols.
//!
//! `visible_external_symbols` stores source-visible names mapped to already-resolved
//! `ExternalSymbolId` values. Expression and type resolution must use these IDs directly;
//! they must never re-resolve names globally through the registry.

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstanceKey, GenericFunctionInstantiationRequest,
};
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleLookups, DeclarationSemanticKind, DeclarationSemanticTable, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template_folding::TirFoldContext;
use crate::compiler_frontend::ast::templates::tir::{TemplateIrStore, TirFoldCache};
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAnnotation;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::ActiveGenericTypeContext;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::{
    ExternalConstantDef, ExternalConstantId, ExternalFunctionDef, ExternalFunctionId,
    ExternalPackageRegistry, ExternalSymbolId, ExternalTypeDef, ExternalTypeId,
};
use crate::compiler_frontend::headers::binding_environment::{
    FileVisibility, HeaderBindingEnvironment, SourceDeclarationTarget,
};
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationMetadata, ModuleSymbols,
};
use crate::compiler_frontend::instrumentation::{
    AstCounter, increment_ast_counter, record_ast_counter_max,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;
use crate::return_compiler_error;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) use crate::compiler_frontend::ast::receiver_methods::{
    ReceiverMethodCatalog, ReceiverMethodEntry,
};

mod builders;
mod diagnostic_sinks;
mod local_declarations;
mod lookup;
mod required_services;
mod scope_frame;

use scope_frame::{ScopeArena, ScopeFrameId};

/// Global counter for generating unique synthetic scope paths in child control-flow contexts.
pub(super) static CONTROL_FLOW_SCOPE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Shared state common to a scope and all its cloned children.
///
/// WHAT: bundles all state that is identical across child scopes so cloning a
/// `ScopeContext` only copies per-scope mutable fields and one `Rc` pointer.
/// WHY: eliminates deep cloning of visibility maps, registries, and lookup tables
/// every time a child control-flow or expression scope is created.
#[derive(Clone)]
pub struct ScopeShared {
    // Immutable semantic lookup tables.
    pub(crate) lookups: Rc<AstModuleLookups>,
    pub(crate) top_level_declarations: Rc<TopLevelDeclarationTable>,
    pub(crate) nominal_type_ids_by_path: Rc<FxHashMap<InternedPath, TypeId>>,
    pub(crate) generated_evidence_pairs: Rc<FxHashSet<(TypeId, TraitId)>>,

    // External package and frontend services.
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,

    // File-local visibility and resolved declarations.
    pub(crate) file_visibility: Option<Rc<FileVisibility>>,
    pub(crate) resolved_type_aliases: Option<Rc<FxHashMap<InternedPath, ResolvedTypeAnnotation>>>,
    pub(crate) generic_declarations_by_path:
        Option<Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>>,
    pub(crate) resolved_struct_fields_by_path:
        Option<Rc<FxHashMap<InternedPath, Vec<Declaration>>>>,
    pub(crate) choice_variant_shells_by_path:
        Option<Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>>,

    // Emission side channels (diagnostics, path usages, generic instantiation requests).
    pub(crate) emitted_warnings: Rc<RefCell<Vec<CompilerDiagnostic>>>,
    pub(crate) rendered_path_usages: Rc<RefCell<Vec<RenderedPathUsage>>>,
    pub(crate) generic_function_instantiation_requests:
        Rc<RefCell<Vec<GenericFunctionInstantiationRequest>>>,
    // Path resolution and source identity.
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) source_file_scope: Option<InternedPath>,
    pub(crate) path_format_config: PathStringFormatConfig,
    pub(crate) template_const_loop_iteration_limit: usize,

    // Receiver method catalog for dispatch.
    pub(crate) receiver_methods: Rc<ReceiverMethodCatalog>,

    // Constant-header contexts are built before the final module lookup package exists, but
    // trait names still need to be recognized and rejected in ordinary type positions there.
    pub(crate) trait_environment_override: Option<Rc<TraitEnvironment>>,
}

/// Shared parser/lowering context for one active AST scope.
pub struct ScopeContext {
    // Core scope identity.
    pub kind: ContextKind,
    pub scope: InternedPath,

    // Immutable shared services are cheap to clone into child scopes.
    pub(crate) shared: Rc<ScopeShared>,

    // Typed Vec arena that owns every frame for this parse context.
    //
    // WHAT: all scope frames for one AST parse context live in one contiguous allocation.
    //       The arena is shared across all clones/children through `Rc<RefCell<_>>`,
    //       but borrow guards are never exposed through parser APIs.
    // WHY: replaces per-frame `Rc<ScopeFrame>` allocations with stable IDs and
    //      index-based parent chains.
    pub(crate) arena: Rc<RefCell<ScopeArena>>,

    /// Module-local TIR store shared by all scope contexts in this AST build.
    ///
    /// WHAT: carries the direct `Rc<RefCell<TemplateIrStore>>` handle used by
    ///       every module-local TIR reference.
    /// WHY: child scope constructors clone one shared handle, so exact root,
    ///      phase, and overlay identity stays coherent across the scope tree.
    pub(crate) template_ir_store: Rc<RefCell<TemplateIrStore>>,

    // Stable ID of the frame that owns this scope layer's local declarations.
    //
    // WHAT: `current_frame_id` points to the arena frame that receives `add_var` calls.
    //       Child contexts get a new frame whose parent is the parent's current frame.
    // WHY: explicit frame identity makes clone/child semantics clear and prevents
    //      accidental mutation of a shared `Rc<ScopeFrame>` from multiple contexts.
    pub(crate) current_frame_id: ScopeFrameId,

    // Assignment targets are readable on the success side of an assignment expression, but not from
    // catch recovery subtrees attached to that assignment. The pending set is activated only when
    // the parser enters a `catch` handler body.
    unavailable_assignment_targets: FxHashSet<StringId>,
    pending_catch_assignment_targets: FxHashSet<StringId>,

    // Optional file-local visibility gate over declarations.
    // When present, references must be in this set, which enforces dependency boundaries.
    // Kept directly on ScopeContext (not in ScopeShared) because add_var mutates it.
    pub visible_declaration_ids: Option<FxHashSet<InternedPath>>,

    // Type expectations.
    pub expected_result_type_ids: Vec<TypeId>,
    pub expected_error_type: Option<TypeId>,

    /// Success return slots for the nearest enclosing function-like body.
    ///
    /// WHAT: unlike `expected_result_type_ids`, this remains stable through
    /// expression-local expected-type contexts such as call arguments.
    /// WHY: postfix option propagation returns from the current function, so it
    /// must validate against the function return contract rather than the
    /// immediate expression receiver.
    pub current_function_return_type_ids: Vec<TypeId>,

    /// Active value-production target for `then` statements in the current scope.
    ///
    /// WHAT: when present, `then` statements must produce values matching these types.
    /// WHY: one target shape lets catch, value `if`, and value match handlers
    /// share arity/coercion validation and HIR result-local lowering.
    pub active_value_target: Option<
        crate::compiler_frontend::ast::statements::value_production::ActiveValueProductionTarget,
    >,

    active_generic_type_context: Option<ActiveGenericTypeContext>,
    pub(crate) generic_template_validation: bool,
    pub(crate) generic_function_instantiation_stack: Vec<GenericFunctionInstanceKey>,

    // Control flow state.
    pub loop_depth: usize,
}

impl Clone for ScopeContext {
    /// Clone a scope context for a sibling branch or catch handler.
    ///
    /// WHAT: copies every non-frame field and allocates a new arena frame that is a
    ///       shallow copy of the current frame. The new frame shares the same parent
    ///       chain and existing declaration IDs, but its own `add_var` calls mutate
    ///       only the copy.
    /// WHY: match/if arms and catch handlers must not add captures to the original
    ///      context's frame.
    fn clone(&self) -> Self {
        let new_frame_id = self.arena.borrow_mut().clone_frame(self.current_frame_id);

        Self {
            kind: self.kind.clone(),
            scope: self.scope.clone(),
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: new_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids: self.expected_result_type_ids.clone(),
            expected_error_type: self.expected_error_type,
            current_function_return_type_ids: self.current_function_return_type_ids.clone(),
            active_value_target: self.active_value_target.clone(),
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            loop_depth: self.loop_depth,
        }
    }
}

impl std::ops::Deref for ScopeContext {
    type Target = ScopeShared;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

/// High-level scope categories used by parser/lowering rules.
#[derive(Debug, PartialEq, Clone)]
pub enum ContextKind {
    /// The top-level scope of each file in the module.
    Module,

    Expression,

    /// An expression enforced to be evaluated at compile time;
    /// cannot contain non-constant references.
    Constant,

    /// Top-level compile-time constant declaration context (`name #= ...`).
    ConstantHeader,

    Function,

    /// For loops and if statements.
    Condition,

    Loop,
    Block,
    Branch,
    CatchHandler,

    /// Statement body of one `<pattern> =>` or `else =>` arm in a match block.
    MatchArm,

    Template,
}

impl ContextKind {
    pub fn is_constant_context(&self) -> bool {
        matches!(self, ContextKind::Constant | ContextKind::ConstantHeader)
    }

    pub fn allows_const_record_coercion(&self) -> bool {
        self.is_constant_context()
    }
}

impl ScopeContext {
    pub(crate) fn record_generic_function_instantiation_request(
        &self,
        request: GenericFunctionInstantiationRequest,
    ) {
        self.shared
            .generic_function_instantiation_requests
            .borrow_mut()
            .push(request);
    }

    pub(crate) fn is_generic_function_instantiation_active(
        &self,
        key: &GenericFunctionInstanceKey,
    ) -> bool {
        self.generic_function_instantiation_stack
            .iter()
            .any(|active_key| active_key == key)
    }

    pub(crate) fn active_generic_type_context(&self) -> Option<&ActiveGenericTypeContext> {
        self.active_generic_type_context.as_ref()
    }

    #[cfg(test)]
    /// Return the declarations declared in the current scope frame.
    ///
    /// WHAT: exposes the current frame's local declarations for tests and diagnostics.
    ///       Ancestor declarations remain accessible through `get_reference`.
    pub fn local_declarations(&self) -> Vec<scope_frame::LocalDeclaration> {
        self.arena
            .borrow()
            .frame(self.current_frame_id)
            .local_declarations()
            .to_vec()
    }

    #[cfg(test)]
    /// Return the total number of visible declarations across the frame chain.
    ///
    /// WHAT: counts declarations in the current frame plus every ancestor frame.
    /// WHY: useful for tests and instrumentation that need the effective scope size.
    pub fn total_declaration_count(&self) -> usize {
        let arena = self.arena.borrow();
        arena
            .frame(self.current_frame_id)
            .total_declaration_count(&arena)
    }
}

// --------------------------
//  Constructors
// --------------------------

impl ScopeContext {
    /// Build a context with the minimum synthetic lookup package needed before
    /// the completed AST environment is available.
    ///
    /// WHAT: seeds shared services with empty/default lookup tables plus the
    /// provided declaration table, external package registry, and the shared
    /// module TIR store.
    /// WHY: constant-header parsing runs while the environment is still being
    /// built, so it supplies visibility, aliases, and nominal type maps through
    /// builder setters. Body emission must replace these synthetic lookups with
    /// `with_lookups` before parsing function/start/template bodies.
    ///
    /// The TIR store is a required input, not a scratch default: every production
    /// context must share the one module-level store allocated by
    /// `AstPhaseContext::from_build_context`.
    pub(crate) fn new(
        kind: ContextKind,
        scope: InternedPath,
        top_level_declarations: Rc<TopLevelDeclarationTable>,
        external_package_registry: Arc<ExternalPackageRegistry>,
        expected_result_type_ids: Vec<TypeId>,
        scope_frame_capacity: usize,
        template_ir_store: Rc<RefCell<TemplateIrStore>>,
    ) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        let lookups = Rc::new(AstModuleLookups {
            module_symbols: ModuleSymbols::empty(),
            binding_environment: HeaderBindingEnvironment::default(),
            warnings: Vec::new(),
            declaration_table: top_level_declarations,
            imported_functions_by_local_path: FxHashMap::default(),
            imported_struct_definitions: Vec::new(),
            imported_choice_definitions: Vec::new(),
            module_constants: Vec::new(),
            rendered_path_usages: Rc::new(RefCell::new(Vec::new())),
            builtin_struct_ast_nodes: Vec::new(),
            resolved_struct_fields_by_path: Rc::new(FxHashMap::default()),
            resolved_function_signatures_by_path: Rc::new(FxHashMap::default()),
            generic_function_templates_by_path: FxHashMap::default(),
            resolved_type_aliases_by_path: Rc::new(FxHashMap::default()),
            choice_variant_shells_by_path: Rc::new(FxHashMap::default()),
            declaration_semantics: Rc::new(DeclarationSemanticTable::empty()),
            receiver_methods: Rc::new(ReceiverMethodCatalog::default()),
            trait_environment: Rc::new(TraitEnvironment::new()),
            trait_evidence_environment: Rc::new(TraitEvidenceEnvironment::new()),
            generic_declarations_by_path: Rc::new(FxHashMap::default()),
            nominal_type_ids_by_path: Rc::new(FxHashMap::default()),
            source_nominal_paths: Rc::new(Default::default()),
            external_package_registry,
            style_directives: StyleDirectiveRegistry::built_ins(),
            build_profile: FrontendBuildProfile::Dev,
            project_path_resolver: None,
            path_format_config: PathStringFormatConfig::default(),
        });

        let shared = Rc::new(ScopeShared {
            lookups: Rc::clone(&lookups),
            top_level_declarations: Rc::clone(&lookups.declaration_table),
            external_package_registry: lookups.external_package_registry.clone(),
            style_directives: lookups.style_directives.clone(),
            build_profile: lookups.build_profile,
            file_visibility: None,
            resolved_type_aliases: None,
            generic_declarations_by_path: None,
            resolved_struct_fields_by_path: None,
            choice_variant_shells_by_path: None,
            emitted_warnings: Rc::new(RefCell::new(Vec::new())),
            rendered_path_usages: Rc::clone(&lookups.rendered_path_usages),
            generic_function_instantiation_requests: Rc::new(RefCell::new(Vec::new())),
            project_path_resolver: lookups.project_path_resolver.clone(),
            source_file_scope: None,
            path_format_config: lookups.path_format_config.clone(),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            receiver_methods: Rc::clone(&lookups.receiver_methods),
            nominal_type_ids_by_path: Rc::clone(&lookups.nominal_type_ids_by_path),
            generated_evidence_pairs: Rc::new(FxHashSet::default()),
            trait_environment_override: None,
        });

        let arena = Rc::new(RefCell::new(ScopeArena::with_capacity(
            scope_frame_capacity,
        )));
        let root_frame_id = arena.borrow_mut().alloc_root_frame_with_capacity(0);
        record_scope_frame_depth(0);

        ScopeContext {
            kind,
            scope,
            shared,
            arena,
            template_ir_store,
            current_frame_id: root_frame_id,
            unavailable_assignment_targets: FxHashSet::default(),
            pending_catch_assignment_targets: FxHashSet::default(),
            visible_declaration_ids: None,
            expected_result_type_ids,
            expected_error_type: None,
            current_function_return_type_ids: Vec::new(),
            active_value_target: None,
            active_generic_type_context: None,
            generic_template_validation: false,
            generic_function_instantiation_stack: Vec::new(),
            loop_depth: 0,
        }
    }

    pub fn new_child_control_flow(
        &self,
        kind: ContextKind,
        string_table: &mut StringTable,
    ) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        let child_frame_id = self
            .arena
            .borrow_mut()
            .alloc_child_frame(self.current_frame_id);
        record_scope_frame_depth(self.arena.borrow().frame(child_frame_id).depth());

        let loop_depth = if matches!(kind, ContextKind::Loop) {
            self.loop_depth + 1
        } else {
            self.loop_depth
        };

        let scope_id = CONTROL_FLOW_SCOPE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let scope = self
            .scope
            .join_str(&format!("__scope_{scope_id}"), string_table);
        let active_value_target = if matches!(kind, ContextKind::Branch | ContextKind::MatchArm) {
            self.active_value_target.clone()
        } else {
            None
        };
        // Conditions are not receiving sites. They validate against Bool through
        // parse-context expectations, but they must not let a surrounding
        // declaration or return type solve a nested generic call.
        let expected_result_type_ids = if matches!(kind, ContextKind::Condition) {
            Vec::new()
        } else {
            self.expected_result_type_ids.clone()
        };

        ScopeContext {
            kind,
            scope,
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids,
            expected_error_type: self.expected_error_type,
            current_function_return_type_ids: self.current_function_return_type_ids.clone(),
            // Branch-like child scopes inherit value production so ordinary nested
            // `if`/match paths can produce for the nearest active value block.
            // Barriers such as loops, functions, conditions, and templates keep
            // clearing the target by constructing non-branch child contexts.
            active_value_target,
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            loop_depth,
        }
    }

    pub fn new_child_function(
        &self,
        function_name: StringId,
        signature: FunctionSignature,
        _string_table: &mut StringTable,
    ) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        // Body-local functions are not closures. They receive the completed
        // top-level/dependency visibility through `shared`, but their local frame starts
        // fresh with parameters only so outer locals cannot be captured implicitly.
        let child_frame_id = self
            .arena
            .borrow_mut()
            .alloc_root_frame_with_capacity(signature.parameters.len());
        record_scope_frame_depth(0);

        let expected_result_type_ids = signature.success_return_type_ids();
        let expected_error_type = signature.error_return_type_id();

        let mut new_context = ScopeContext {
            kind: ContextKind::Function,
            scope: self.scope.append(function_name),
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids: expected_result_type_ids.clone(),
            expected_error_type,
            current_function_return_type_ids: expected_result_type_ids,
            active_value_target: None,
            active_generic_type_context: None,
            generic_template_validation: false,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            loop_depth: 0,
        };

        // Share the top-level declaration table (cheap Rc clone); reset locals to params only.
        new_context.set_local_declarations(signature.parameters);

        new_context
    }

    pub fn new_child_expression(&self, expected_result_type_ids: Vec<TypeId>) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        let child_frame_id = self
            .arena
            .borrow_mut()
            .alloc_child_frame(self.current_frame_id);
        record_scope_frame_depth(self.arena.borrow().frame(child_frame_id).depth());

        ScopeContext {
            kind: ContextKind::Expression,
            scope: self.scope.clone(),
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids,
            expected_error_type: self.expected_error_type,
            current_function_return_type_ids: self.current_function_return_type_ids.clone(),
            active_value_target: None,
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            loop_depth: self.loop_depth,
        }
    }

    /// Build the context used while parsing template expressions.
    ///
    /// Constant contexts stay constant so template-head captures can inline
    /// compile-time values. All other contexts parse templates as runtime-capable.
    pub fn new_template_parsing_context(&self) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        let child_frame_id = self
            .arena
            .borrow_mut()
            .alloc_child_frame(self.current_frame_id);
        record_scope_frame_depth(self.arena.borrow().frame(child_frame_id).depth());

        let template_kind = if self.kind.is_constant_context() {
            self.kind.clone()
        } else {
            ContextKind::Template
        };

        ScopeContext {
            kind: template_kind,
            scope: self.scope.clone(),
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids: vec![],
            expected_error_type: self.expected_error_type,
            current_function_return_type_ids: self.current_function_return_type_ids.clone(),
            active_value_target: None,
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            loop_depth: self.loop_depth,
        }
    }

    /// Builds a constant child context that preserves project-aware folding/path state.
    ///
    /// WHAT: shares the parent visibility/declaration environment and forces
    ///       resolver + source file scope propagation into constant parsing paths.
    /// WHY: resolver-less constant contexts are invalid for template folding and
    ///      template-head path coercion.
    pub fn new_constant(scope: InternedPath, parent: &ScopeContext) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        let child_frame_id = parent
            .arena
            .borrow_mut()
            .alloc_child_frame(parent.current_frame_id);
        record_scope_frame_depth(parent.arena.borrow().frame(child_frame_id).depth());

        ScopeContext {
            kind: ContextKind::Constant,
            scope,
            shared: Rc::clone(&parent.shared),
            arena: Rc::clone(&parent.arena),
            template_ir_store: Rc::clone(&parent.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: parent.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: parent.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: parent.visible_declaration_ids.clone(),
            expected_result_type_ids: Vec::new(),
            expected_error_type: parent.expected_error_type,
            current_function_return_type_ids: parent.current_function_return_type_ids.clone(),
            active_value_target: None,
            active_generic_type_context: parent.active_generic_type_context.clone(),
            generic_template_validation: parent.generic_template_validation,
            generic_function_instantiation_stack: parent
                .generic_function_instantiation_stack
                .clone(),
            loop_depth: parent.loop_depth,
        }
    }
}

/// Update the recorded maximum scope-frame depth.
///
/// WHAT: records the deepest parent-linked frame observed during AST construction.
/// WHY: the no-shadowing frame depth is an objective signal for how nested the
///      current input is, and it helps validate capacity estimates later.
fn record_scope_frame_depth(depth: usize) {
    record_ast_counter_max(AstCounter::ScopeMaxFrameDepth, depth);
}
