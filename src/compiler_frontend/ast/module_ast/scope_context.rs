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
    AstModuleLookups, DeclarationSemanticKind, ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template_folding::TirFoldContext;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAlias;
use crate::compiler_frontend::build_config::{
    BuildInputName, ConfigResolutionServices, ResolvedBuildConfigMap,
};
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
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationMetadata;
use crate::compiler_frontend::instrumentation::{
    AstCounter, increment_ast_counter, record_ast_counter_max,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReferenceOutcome, ResolvedFileReferenceTable,
    ResolvedFileReferenceTarget, ResourceSourceId,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;

use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;

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

thread_local! {
    static EMPTY_TRAIT_ENVIRONMENT: Rc<TraitEnvironment> = Rc::new(TraitEnvironment::new());

    static EMPTY_TRAIT_EVIDENCE_ENVIRONMENT: Rc<TraitEvidenceEnvironment> =
        Rc::new(TraitEvidenceEnvironment::new());
}
/// Immutable Stage 0 resolution facts shared by AST value semantics.
///
/// WHAT: exposes one lookup over either a prepared module's real Stage 0 tables or the compact
///       rows captured by one persistent generic body.
/// WHY: AST readers must interpret the same resolved target vocabulary after a generated body has
///      left its declaring module. Keeping the backing choice here prevents readers from
///      reopening source tables or learning which materialisation lane supplied a row.
#[derive(Clone)]
pub(crate) struct Stage0ResolutionFacts {
    backing: Stage0ResolutionFactsBacking,
}

#[derive(Clone)]
enum Stage0ResolutionFactsBacking {
    Ordinary {
        resolved_file_references: ResolvedFileReferenceTable,
        source_files: SourceFileTable,
    },
    FrozenGeneric {
        references: FxHashMap<PathSyntaxId, FrozenResolvedFileReference>,
    },
}

/// The row has already crossed the materialisation boundary: it contains no donor file, path,
/// resource-source or filesystem identity. Content targets retain their folded value in the owned
/// portable vocabulary, and resource targets retain only their owner-relative portable path.
#[derive(Clone)]
pub(crate) struct FrozenResolvedFileReference {
    pub(crate) path_syntax: PathSyntaxId,
    pub(crate) class: PreparedFileReferenceClass,
    pub(crate) outcome: FrozenResolvedFileReferenceOutcome,
}

#[derive(Clone)]
pub(crate) enum FrozenResolvedFileReferenceOutcome {
    NoPhysicalTarget,
    Content {
        value: OwnedFoldedString,
    },
    Resource {
        owner_relative_path: PortableResourcePath,
    },
    IdentifiedSourceKind,
}

/// One reader-facing resolved file-reference view.
///
/// Ordinary and frozen generic backings both project into this vocabulary. Ordinary content rows
/// expose their logical path for declaration lookup, while frozen rows expose the captured value
/// and no longer retain a donor declaration path.
pub(crate) struct Stage0ResolvedFileReferenceView<'a> {
    pub(crate) class: PreparedFileReferenceClass,
    pub(crate) outcome: Stage0ResolvedFileReferenceOutcome<'a>,
}

pub(crate) enum Stage0ResolvedFileReferenceOutcome<'a> {
    NoPhysicalTarget,
    Content {
        logical_path: Option<&'a InternedPath>,
        value: Option<&'a OwnedFoldedString>,
    },
    Resource {
        source: Option<&'a ResourceSourceId>,
        owner_relative_path: &'a PortableResourcePath,
    },
    IdentifiedSourceKind,
    Diagnostic(&'a CompilerDiagnostic),
}

impl Stage0ResolutionFacts {
    pub(crate) fn ordinary(
        resolved_file_references: ResolvedFileReferenceTable,
        source_files: SourceFileTable,
    ) -> Self {
        Self {
            backing: Stage0ResolutionFactsBacking::Ordinary {
                resolved_file_references,
                source_files,
            },
        }
    }

    pub(crate) fn frozen_generic(
        references: Vec<FrozenResolvedFileReference>,
    ) -> Result<Self, CompilerError> {
        let mut indexed = FxHashMap::with_capacity_and_hasher(references.len(), Default::default());
        for reference in references {
            if reference.path_syntax.is_none() {
                return Err(CompilerError::compiler_error(
                    "frozen generic resolved-reference row has an absent PathSyntaxId marker",
                ));
            }
            validate_frozen_reference(&reference)?;
            if indexed.insert(reference.path_syntax, reference).is_some() {
                return Err(CompilerError::compiler_error(
                    "frozen generic resolved-reference table contains duplicate path handles",
                ));
            }
        }
        Ok(Self {
            backing: Stage0ResolutionFactsBacking::FrozenGeneric {
                references: indexed,
            },
        })
    }

    pub(crate) fn lookup(
        &self,
        source_file: Option<FileId>,
        path_syntax: PathSyntaxId,
    ) -> Result<Option<Stage0ResolvedFileReferenceView<'_>>, CompilerError> {
        match &self.backing {
            Stage0ResolutionFactsBacking::Ordinary {
                resolved_file_references,
                source_files,
            } => {
                let source_file = source_file.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "ordinary Stage 0 file-reference lookup has no declaring FileId",
                    )
                })?;
                let Some(reference) = resolved_file_references.get(source_file, path_syntax) else {
                    return Ok(None);
                };
                ordinary_reference_view(reference, source_files).map(Some)
            }
            Stage0ResolutionFactsBacking::FrozenGeneric { references } => {
                Ok(references.get(&path_syntax).map(frozen_reference_view))
            }
        }
    }
}

fn ordinary_reference_view<'a>(
    reference: &'a crate::compiler_frontend::paths::file_references::ResolvedFileReference,
    source_files: &'a SourceFileTable,
) -> Result<Stage0ResolvedFileReferenceView<'a>, CompilerError> {
    let outcome = match &reference.outcome {
        ResolvedFileReferenceOutcome::NoPhysicalTarget => {
            Stage0ResolvedFileReferenceOutcome::NoPhysicalTarget
        }
        ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
            source,
        }) => {
            let source_identity = source_files.get(*source).ok_or_else(|| {
                CompilerError::compiler_error(
                    "content file reference target was absent from the source identity table",
                )
            })?;
            Stage0ResolvedFileReferenceOutcome::Content {
                logical_path: Some(&source_identity.logical_path),
                value: None,
            }
        }
        ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
            source,
            owner_relative_path,
        }) => Stage0ResolvedFileReferenceOutcome::Resource {
            source: Some(source),
            owner_relative_path,
        },
        ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::IdentifiedSourceKind) => {
            Stage0ResolvedFileReferenceOutcome::IdentifiedSourceKind
        }
        ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic)
        }
    };
    Ok(Stage0ResolvedFileReferenceView {
        class: reference.class,
        outcome,
    })
}

fn frozen_reference_view(
    reference: &FrozenResolvedFileReference,
) -> Stage0ResolvedFileReferenceView<'_> {
    let outcome = match &reference.outcome {
        FrozenResolvedFileReferenceOutcome::NoPhysicalTarget => {
            Stage0ResolvedFileReferenceOutcome::NoPhysicalTarget
        }
        FrozenResolvedFileReferenceOutcome::Content { value } => {
            Stage0ResolvedFileReferenceOutcome::Content {
                logical_path: None,
                value: Some(value),
            }
        }
        FrozenResolvedFileReferenceOutcome::Resource {
            owner_relative_path,
        } => Stage0ResolvedFileReferenceOutcome::Resource {
            source: None,
            owner_relative_path,
        },
        FrozenResolvedFileReferenceOutcome::IdentifiedSourceKind => {
            Stage0ResolvedFileReferenceOutcome::IdentifiedSourceKind
        }
    };
    Stage0ResolvedFileReferenceView {
        class: reference.class,
        outcome,
    }
}

fn validate_frozen_reference(reference: &FrozenResolvedFileReference) -> Result<(), CompilerError> {
    let valid = match reference.class {
        PreparedFileReferenceClass::SiteRoot | PreparedFileReferenceClass::Extensionless => {
            matches!(
                reference.outcome,
                FrozenResolvedFileReferenceOutcome::NoPhysicalTarget
            )
        }
        PreparedFileReferenceClass::ContentSource => matches!(
            reference.outcome,
            FrozenResolvedFileReferenceOutcome::Content { .. }
        ),
        PreparedFileReferenceClass::ResourceFile => matches!(
            reference.outcome,
            FrozenResolvedFileReferenceOutcome::Resource { .. }
        ),
        PreparedFileReferenceClass::SourceKindNoFileValue => matches!(
            reference.outcome,
            FrozenResolvedFileReferenceOutcome::IdentifiedSourceKind
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(CompilerError::compiler_error(
            "frozen generic resolved-reference class does not match its outcome",
        ))
    }
}

/// Shared Stage 0 and module-resource services used by AST value semantics.
#[derive(Clone)]
pub(crate) struct FileValueResolutionServices {
    pub(crate) stage0_resolution_facts: Option<Arc<Stage0ResolutionFacts>>,
    pub(crate) module_resources: Rc<RefCell<ModuleResourceTable>>,
    pub(crate) module_origin: Option<StableModuleOriginIdentity>,
}

impl FileValueResolutionServices {
    /// Fork these services for one materialised body without changing its resource authority.
    ///
    /// WHAT: pairs a body's compact Stage 0 facts with the shared generated-sidecar resource table.
    /// WHY: compact path handles are body-local; recursive body parsing must switch facts while
    /// preserving the one sidecar resource identity owner.
    pub(crate) fn with_stage0_resolution_facts(
        &self,
        stage0_resolution_facts: Arc<Stage0ResolutionFacts>,
    ) -> Rc<Self> {
        Rc::new(Self {
            stage0_resolution_facts: Some(stage0_resolution_facts),
            module_resources: Rc::clone(&self.module_resources),
            module_origin: self.module_origin.clone(),
        })
    }
}

/// Shared state common to a scope and all its cloned children.
///
/// WHAT: bundles all state that is identical across child scopes so cloning a
/// `ScopeContext` only copies per-scope mutable fields and one `Rc` pointer.
/// WHY: eliminates deep cloning of visibility maps, registries, and lookup tables
/// every time a child control-flow or expression scope is created.
#[derive(Clone)]
pub struct ScopeShared {
    // Immutable semantic lookup tables.
    pub(crate) lookups: Option<Rc<AstModuleLookups>>,
    pub(crate) top_level_declarations: Rc<TopLevelDeclarationTable>,
    pub(crate) nominal_type_ids_by_path: Rc<FxHashMap<InternedPath, TypeId>>,
    pub(crate) generated_evidence_pairs: Rc<FxHashSet<(TypeId, TraitId)>>,

    // External package and frontend services.
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,

    // File-local visibility and resolved declarations.
    pub(crate) file_visibility: Option<Arc<FileVisibility>>,
    pub(crate) resolved_type_aliases: Option<Rc<FxHashMap<InternedPath, ResolvedTypeAlias>>>,
    pub(crate) generic_declarations_by_path:
        Option<Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>>,
    pub(crate) resolved_struct_fields_by_path:
        Option<Rc<FxHashMap<InternedPath, Vec<Declaration>>>>,
    pub(crate) choice_variant_shells_by_path:
        Option<Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>>,
    pub(crate) resolved_module_constants_override: Option<Rc<ResolvedConstantSet>>,
    pub(crate) emitted_warnings: Rc<RefCell<Vec<CompilerDiagnostic>>>,

    pub(crate) generic_function_instantiation_requests:
        Rc<RefCell<Vec<GenericFunctionInstantiationRequest>>>,
    pub(crate) source_file_scope: Option<InternedPath>,
    pub(crate) file_value_resolution: Option<Rc<FileValueResolutionServices>>,
    /// Immutable project/package source `#Config` values for constant-header materialization.
    pub(crate) source_build_config_values: Option<Arc<ResolvedBuildConfigMap>>,
    /// Names of source `#Config` contracts declared by this module.
    pub(crate) source_build_config_contract_names: Option<Arc<FxHashSet<BuildInputName>>>,
    /// Optional compiler-owned direct-project config resolver for constant-header folding.
    pub(crate) config_resolution: Option<Rc<ConfigResolutionServices>>,
    pub(crate) declaring_file_id: Option<FileId>,
    pub(crate) template_const_loop_iteration_limit: usize,

    // Receiver method catalog for dispatch.
    pub(crate) receiver_methods: Rc<ReceiverMethodCatalog>,

    // Constant-header contexts are built before the final module lookup package exists.
    pub(crate) trait_environment: Rc<TraitEnvironment>,
    pub(crate) trait_evidence_environment: Rc<TraitEvidenceEnvironment>,
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
    //
    // Kept directly on `ScopeContext` rather than in `ScopeShared` because `add_var` extends it.
    // The set is shared copy-on-write: child scopes and header-pass scopes clone the handle, and
    // only a scope that actually declares a local pays for a private copy.
    pub visible_declaration_ids: Option<Arc<FxHashSet<InternedPath>>>,

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

    /// True while parsing a field value of an anonymous const record.
    ///
    /// Nested `|...|` literals are rejected so pipe counting stays unambiguous.
    pub(crate) inside_anonymous_const_record: bool,
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
            inside_anonymous_const_record: self.inside_anonymous_const_record,
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
    /// Marks the current end of the compiler-owned generic-instantiation request sink.
    ///
    /// WHAT: returns a provisional request boundary that a parser can commit or roll back after
    ///       it has validated a source construct and determined whether its work is active.
    /// WHY: inactive static assertion messages still need frontend inference and evidence checks,
    ///       but must not publish generated-function requests to later stages.
    pub(crate) fn generic_request_checkpoint(&self) -> usize {
        self.shared
            .generic_function_instantiation_requests
            .borrow()
            .len()
    }

    /// Discards requests appended after a compiler-owned provisional boundary.
    ///
    /// The sink is shared by child scopes, so the operation is intentionally defined here rather
    /// than in build orchestration. Callers must only pass a checkpoint obtained from the same
    /// shared sink and must invoke this after validation has completed.
    pub(crate) fn discard_generic_requests_since(&self, checkpoint: usize) {
        let mut requests = self
            .shared
            .generic_function_instantiation_requests
            .borrow_mut();
        debug_assert!(
            checkpoint <= requests.len(),
            "generic request checkpoint must belong to the current sink"
        );
        if checkpoint <= requests.len() {
            requests.truncate(checkpoint);
        }
    }

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
    /// Build a context before the completed AST environment is available.
    ///
    /// WHAT: seeds only the provided declaration table, external package registry and shared
    /// module TIR store. Environment passes attach their narrow side tables explicitly; body
    /// emission installs the completed lookup package with `with_lookups`.
    /// WHY: constant-header parsing runs while the environment is still being
    /// built, so it supplies visibility, aliases and nominal type maps through builder setters.
    /// No synthetic `AstModuleLookups` package is constructed for this path.
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

        let trait_environment = EMPTY_TRAIT_ENVIRONMENT.with(Rc::clone);
        let trait_evidence_environment = EMPTY_TRAIT_EVIDENCE_ENVIRONMENT.with(Rc::clone);
        let shared = Rc::new(ScopeShared {
            lookups: None,
            top_level_declarations,
            external_package_registry,
            style_directives: StyleDirectiveRegistry::built_ins(),
            build_profile: FrontendBuildProfile::Dev,
            file_visibility: None,
            resolved_type_aliases: None,
            generic_declarations_by_path: None,
            resolved_struct_fields_by_path: None,
            choice_variant_shells_by_path: None,
            resolved_module_constants_override: None,
            emitted_warnings: Rc::new(RefCell::new(Vec::new())),
            generic_function_instantiation_requests: Rc::new(RefCell::new(Vec::new())),
            source_file_scope: None,
            file_value_resolution: None,
            source_build_config_values: None,
            source_build_config_contract_names: None,
            config_resolution: None,
            declaring_file_id: None,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            receiver_methods: Rc::new(ReceiverMethodCatalog::default()),
            nominal_type_ids_by_path: Rc::new(FxHashMap::default()),
            generated_evidence_pairs: Rc::new(FxHashSet::default()),
            trait_environment,
            trait_evidence_environment,
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
            inside_anonymous_const_record: false,
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
            inside_anonymous_const_record: self.inside_anonymous_const_record,
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
            inside_anonymous_const_record: false,
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
            inside_anonymous_const_record: self.inside_anonymous_const_record,
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
            inside_anonymous_const_record: self.inside_anonymous_const_record,
        }
    }

    /// Builds a constant child context that preserves project-aware folding/path state.
    ///
    /// WHAT: shares the parent visibility/declaration environment and forces
    ///       resolver + source file scope propagation into constant parsing paths.
    /// WHY: resolver-less constant contexts are invalid for template folding and
    ///      template-head path coercion.
    ///
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
            inside_anonymous_const_record: parent.inside_anonymous_const_record,
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
