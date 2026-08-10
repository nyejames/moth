//! Builder-style setters for [`ScopeContext`].
//!
//! WHAT: each method consumes the context, updates one field in the shared or
//! per-scope state, and returns the context for chaining. These setters are
//! used by `AstModuleEnvironmentBuilder` to assemble a complete scope context
//! before body emission begins.
//!
//! WHY: header parsing and environment construction run in stages where not
//! all visibility maps, lookup tables, and services are available at once.
//! Builder setters let the environment incrementally populate the context
//! without exposing the internal `Rc::make_mut` dance at every call site.

use super::*;

#[cfg(test)]
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;

#[cfg(test)]
impl ScopeContext {
    /// Build a scope context backed by a fresh isolated TIR store for tests
    /// that do not own the shared module-level store.
    ///
    /// WHAT: adopts an empty `TemplateIrStore` as the fresh module store so the
    ///       context satisfies the required-store constructor invariant of
    ///       `ScopeContext::new` without borrowing a production module store.
    /// WHY: `ScopeContext::new` requires the shared module TIR store; tests that exercise non-template scope
    ///      behaviour need an isolated store without assembling one inline at
    ///      every call site. Tests that must share a specific store should build
    ///      that store explicitly and pass it to `ScopeContext::new`, or swap it
    ///      in with `with_template_ir_store`.
    pub(crate) fn new_for_tests(
        kind: ContextKind,
        scope: InternedPath,
        top_level_declarations: Rc<TopLevelDeclarationTable>,
        external_package_registry: Arc<ExternalPackageRegistry>,
        expected_result_type_ids: Vec<TypeId>,
        scope_frame_capacity: usize,
    ) -> ScopeContext {
        let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
        ScopeContext::new(
            kind,
            scope,
            top_level_declarations,
            external_package_registry,
            expected_result_type_ids,
            scope_frame_capacity,
            template_ir_store,
        )
    }
}

impl ScopeContext {
    /// Set the shared parser-emitted TIR store for this scope tree.
    ///
    /// WHAT: replaces the isolated test store with the explicitly shared module store.
    /// WHY: tests that inspect one store across contexts must pass that handle
    /// directly, just like production contexts do.
    #[cfg(test)]
    pub(crate) fn with_template_ir_store(
        mut self,
        store: Rc<RefCell<TemplateIrStore>>,
    ) -> ScopeContext {
        self.template_ir_store = store;
        self
    }

    /// Clone the shared parser-emitted TIR store handle.
    ///
    /// Tests use this to inspect B6 draft output without making the store a public
    /// language or HIR-facing API.
    #[cfg(test)]
    pub(crate) fn template_ir_store(&self) -> Rc<RefCell<TemplateIrStore>> {
        Rc::clone(&self.template_ir_store)
    }

    // --------------------------
    //  Build profile
    // --------------------------

    pub fn with_build_profile(mut self, profile: FrontendBuildProfile) -> ScopeContext {
        Rc::make_mut(&mut self.shared).build_profile = profile;
        self
    }

    // --------------------------
    //  Visibility and import environment
    // --------------------------

    /// Restrict declaration resolution to the provided path set.
    ///
    /// WHAT: when present, only declarations whose paths are in this set are
    /// resolvable by name. When absent, any declaration in the module may be
    /// resolved.
    /// WHY: file/start contexts set this to enforce import semantics and
    /// prevent same-file references from bypassing the visibility system.
    pub fn with_visible_declarations(mut self, visible: FxHashSet<InternedPath>) -> ScopeContext {
        self.visible_declaration_ids = Some(visible);
        self
    }

    /// Mutate the file-local visibility package without making each setter
    /// repeat the default-or-clone dance.
    ///
    /// WHAT: preserves existing visibility maps and updates one selected field.
    /// WHY: constant-header contexts are assembled from separate visibility
    /// maps before a full `FileVisibility` is available at the call site.
    fn update_file_visibility(&mut self, update: impl FnOnce(&mut FileVisibility)) {
        let shared = Rc::make_mut(&mut self.shared);
        let mut file_visibility = shared
            .file_visibility
            .as_ref()
            .map(|visibility| (**visibility).clone())
            .unwrap_or_default();

        update(&mut file_visibility);
        shared.file_visibility = Some(Rc::new(file_visibility));
    }

    /// Register source-visible external package symbols.
    ///
    /// WHAT: maps source-level names to already-resolved `ExternalSymbolId`
    /// values. Expression and type resolution must use these IDs directly;
    /// they must never re-resolve names globally through the registry.
    pub fn with_visible_external_symbols(
        mut self,
        visible: FxHashMap<StringId, ExternalSymbolId>,
    ) -> ScopeContext {
        self.update_file_visibility(|file_visibility| {
            file_visibility.visible_external_symbols = visible;
        });
        self
    }

    /// Register source-visible bindings for same-file and imported declarations.
    ///
    /// WHAT: maps source-level names to their canonical interned declaration
    /// paths. This is the primary path for resolving source symbols in value
    /// position during expression parsing.
    pub fn with_visible_source_bindings(
        mut self,
        bindings: FxHashMap<StringId, SourceDeclarationTarget>,
    ) -> ScopeContext {
        self.update_file_visibility(|file_visibility| {
            file_visibility.visible_source_names = bindings;
        });
        self
    }

    /// Register source-visible type aliases.
    ///
    /// WHAT: maps source-level names to the interned paths of type alias
    /// declarations. Used to give a precise diagnostic when a type alias is
    /// mistakenly used in value position.
    pub fn with_visible_type_aliases(
        mut self,
        aliases: FxHashMap<StringId, SourceDeclarationTarget>,
    ) -> ScopeContext {
        self.update_file_visibility(|file_visibility| {
            file_visibility.visible_type_alias_names = aliases;
        });
        self
    }

    /// Apply a header-built `FileVisibility` to this scope context.
    ///
    /// WHAT: copies all visibility maps from the prepared header environment.
    /// WHY: AST emission should consume header-built visibility directly instead of
    /// reconstructing import bindings or manually setting each field.
    pub(crate) fn with_file_visibility(mut self, visibility: Rc<FileVisibility>) -> ScopeContext {
        self.visible_declaration_ids = Some(visibility.visible_declaration_paths.clone());
        Rc::make_mut(&mut self.shared).file_visibility = Some(visibility);
        self
    }

    // --------------------------
    //  Type resolution metadata
    // --------------------------

    /// Register resolved type alias metadata.
    ///
    /// WHAT: maps type alias declaration paths to their `ResolvedTypeAnnotation`,
    /// which carries the parsed source ref, diagnostic spelling, and canonical `TypeId`
    /// when available. Used during type checking to expand aliases transparently while
    /// preserving fixed-collection capacity syntax.
    pub(crate) fn with_resolved_type_aliases(
        mut self,
        aliases: Rc<FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).resolved_type_aliases = Some(aliases);
        self
    }

    /// Seed explicit compile-time constants known before final module lookups exist.
    ///
    /// WHAT: records top-level compile-time constants already resolved by the environment builder.
    /// WHY: constant-header and nominal-member contexts run before final `AstModuleLookups`
    ///      contains `module_constants`, but fixed-capacity type syntax still needs to
    ///      distinguish explicit constants from merely foldable runtime bindings.
    pub(crate) fn with_explicit_compile_time_constants(
        self,
        constants: &[Declaration],
    ) -> ScopeContext {
        self.arena
            .borrow_mut()
            .frame_mut(self.current_frame_id)
            .explicit_compile_time_constant_declarations
            .extend(constants.iter().map(|constant| constant.id.clone()));
        self
    }

    /// Register generic declaration metadata by path.
    ///
    /// WHAT: records generic parameter metadata for nominal declarations.
    /// Used during generic function instantiation and type argument validation.
    pub(crate) fn with_generic_declarations(
        mut self,
        declarations: Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).generic_declarations_by_path = Some(declarations);
        self
    }

    /// Register resolved struct field declarations by path.
    ///
    /// WHAT: maps struct declaration paths to their ordered field
    /// declarations. Consumed by expression parsing for field access and
    /// struct literal validation.
    pub(crate) fn with_resolved_struct_fields_by_path(
        mut self,
        fields: Rc<FxHashMap<InternedPath, Vec<Declaration>>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).resolved_struct_fields_by_path = Some(fields);
        self
    }

    /// Register choice variant shells by path.
    ///
    /// WHAT: maps choice declaration paths to their ordered variant shells.
    /// Consumed by expression parsing for choice construction and match
    /// pattern validation.
    pub(crate) fn with_choice_variant_shells_by_path(
        mut self,
        shells: Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).choice_variant_shells_by_path = Some(shells);
        self
    }

    /// Register nominal type identities by declaration path.
    ///
    /// WHAT: maps declaration paths to their canonical `TypeId`. Used to
    /// resolve nominal type references during expression and type parsing.
    pub(crate) fn with_nominal_type_ids_by_path(
        mut self,
        ids: Rc<FxHashMap<InternedPath, TypeId>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).nominal_type_ids_by_path = ids;
        self
    }

    /// Register exact generated-evidence targets for the current materialisation.
    ///
    /// WHAT: supplies transient nominal identities selected by the requester-side evidence map.
    /// WHY: generated bound validation must accept that exact selection without pretending it is
    /// an authored source binding or widening ordinary file visibility.
    pub(crate) fn with_generated_evidence_target_type_ids(
        mut self,
        type_ids: Rc<FxHashSet<TypeId>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).generated_evidence_target_type_ids = type_ids;
        self
    }

    // --------------------------
    //  Project services and directives
    // --------------------------

    pub fn with_style_directives(
        mut self,
        style_directives: &StyleDirectiveRegistry,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).style_directives = style_directives.clone();
        self
    }

    pub(crate) fn with_project_path_resolver(
        mut self,
        resolver: Option<ProjectPathResolver>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).project_path_resolver = resolver;
        self
    }

    pub fn with_source_file_scope(mut self, source_file: InternedPath) -> ScopeContext {
        Rc::make_mut(&mut self.shared).source_file_scope = Some(source_file);
        self
    }

    pub fn with_path_format_config(mut self, config: PathStringFormatConfig) -> ScopeContext {
        Rc::make_mut(&mut self.shared).path_format_config = config;
        self
    }

    pub fn with_template_const_loop_iteration_limit(mut self, limit: usize) -> ScopeContext {
        Rc::make_mut(&mut self.shared).template_const_loop_iteration_limit = limit;
        self
    }

    /// Attach a sink for tracking rendered path usages.
    ///
    /// WHAT: collects path references that appear in template output so the
    /// build system can emit dependency metadata.
    pub fn with_rendered_path_usage_sink(
        mut self,
        sink: Rc<RefCell<Vec<RenderedPathUsage>>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).rendered_path_usages = sink;
        self
    }

    // --------------------------
    //  Generic function tracking
    // --------------------------

    /// Attach a sink for generic function instantiation requests.
    ///
    /// WHAT: collects requests to instantiate concrete generic function
    /// bodies. Consumed by `AstEmitter` after body parsing completes.
    pub(crate) fn with_generic_function_instantiation_sink(
        mut self,
        sink: Rc<RefCell<Vec<GenericFunctionInstantiationRequest>>>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).generic_function_instantiation_requests = sink;
        self
    }

    /// Set the active generic function instantiation stack.
    ///
    /// WHAT: records which generic function instantiations are currently
    /// in progress to detect and prevent infinite recursion.
    pub(crate) fn with_generic_function_instantiation_stack(
        mut self,
        stack: Vec<GenericFunctionInstanceKey>,
    ) -> ScopeContext {
        self.generic_function_instantiation_stack = stack;
        self
    }

    /// Set the active generic type substitution context.
    ///
    /// WHAT: provides type parameter substitutions for the current generic
    /// function body. Used during type checking of generic function bodies.
    pub(crate) fn with_active_generic_type_context(
        mut self,
        generic_context: ActiveGenericTypeContext,
    ) -> ScopeContext {
        self.active_generic_type_context = Some(generic_context);
        self
    }

    // --------------------------
    //  Receiver methods
    // --------------------------

    /// Register the receiver method catalog.
    ///
    /// WHAT: stores all receiver methods visible in the module. Used by
    /// expression parsing for receiver-method dispatch.
    pub(crate) fn with_receiver_methods(
        mut self,
        receiver_methods: Rc<ReceiverMethodCatalog>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).receiver_methods = receiver_methods;
        self
    }

    // --------------------------
    //  Module lookups
    // --------------------------

    /// Replace synthetic lookup services created by `new()` with the real immutable
    /// module lookup package built before body emission.
    pub(crate) fn with_lookups(mut self, lookups: Rc<AstModuleLookups>) -> ScopeContext {
        let shared = Rc::make_mut(&mut self.shared);
        shared.nominal_type_ids_by_path = Rc::clone(&lookups.nominal_type_ids_by_path);
        shared.choice_variant_shells_by_path =
            Some(Rc::clone(&lookups.choice_variant_shells_by_path));
        shared.resolved_type_aliases = Some(Rc::clone(&lookups.resolved_type_aliases_by_path));
        shared.lookups = lookups;
        self
    }

    /// Inject a resolved trait environment into this scope context.
    ///
    /// WHAT: replaces the synthetic empty `TraitEnvironment` created by `ScopeContext::new`
    /// with the real module trait metadata so type resolution in constant headers can
    /// recognize and reject trait names in ordinary type annotations.
    /// WHY: constant-header parsing runs while the AST environment is still being built;
    /// it needs trait awareness without waiting for the full `AstModuleLookups` package.
    pub(crate) fn with_trait_environment(
        mut self,
        trait_environment: Rc<TraitEnvironment>,
    ) -> ScopeContext {
        Rc::make_mut(&mut self.shared).trait_environment_override = Some(trait_environment);
        self
    }
}
