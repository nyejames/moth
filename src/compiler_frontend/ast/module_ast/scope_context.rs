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
//! limited to emission side channels, completed callable proofs and the AST-local TIR cache.
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
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstanceKey, GenericFunctionInstantiationRequest, GenericRequestRange,
    GenericRequestSlot, GenericRequestTicket,
};
use crate::compiler_frontend::ast::module_ast::environment::{
    AstModuleLookups, DeclarationSemanticKind, ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::numeric_construction::{
    NumericBindingId, NumericClassId, NumericConstructionState, NumericContinuation,
    NumericContinuationId, NumericPin, NumericRecipe, PendingBindingRow,
    join_protected_catch_class, raw_fragment_result_is_independently_fixed,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeId, pin_numeric_shape,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ParameterContract};
use crate::compiler_frontend::ast::templates::template_folding::TirFoldContext;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAlias;
use crate::compiler_frontend::build_config::{
    BuildInputName, ConfigResolutionServices, ResolvedBuildConfigMap,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticDraft};
use crate::compiler_frontend::datatypes::ReceiverKey;
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
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::instrumentation::{
    AstCounter, increment_ast_counter, record_ast_counter_max,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReferenceFailure, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget, ResourceSourceId,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::source::{
    FrozenIdentityHandle, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) use crate::compiler_frontend::ast::receiver_methods::{
    ReceiverMethodCatalog, ReceiverMethodEntry,
};
pub(crate) use lookup::{RouteParameterReference, ScopeDeclarationRef, ValueDeclarationRef};

mod builders;
mod diagnostic_sinks;
mod local_declarations;
mod lookup;
mod required_services;
mod scope_frame;

pub(crate) use scope_frame::{LocalDeclarationSlot, ScopeArena, ScopeFrameId};

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
        source_files: Arc<SourceDatabase>,
    },
    FrozenGeneric {
        /// Explicit owning source identity retained from `StableBodySyntax::donor_file_id`.
        ///
        /// WHAT: the concrete `SourceId` that owns every materialised token span and path row.
        /// WHY: frozen lookups take the retained owner and reject any other identity so donor
        ///      ranges never alias a requester call-site source and independent package handles
        ///      stay distinct.
        owner: SourceId,
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
/// expose an ephemeral logical path for declaration lookup, while frozen rows expose the captured
/// value and no longer retain a donor declaration path.
pub(crate) struct Stage0ResolvedFileReferenceView<'a> {
    pub(crate) class: PreparedFileReferenceClass,
    pub(crate) outcome: Stage0ResolvedFileReferenceOutcome<'a>,
}

pub(crate) enum Stage0ResolvedFileReferenceOutcome<'a> {
    NoPhysicalTarget,
    Content {
        logical_path: Option<PathId>,
        value: Option<&'a OwnedFoldedString>,
    },
    Resource {
        source: Option<&'a ResourceSourceId>,
        owner_relative_path: &'a PortableResourcePath,
    },
    IdentifiedSourceKind,
    Diagnostic(ResolvedFileReferenceFailure),
}

impl Stage0ResolutionFacts {
    pub(crate) fn ordinary(
        resolved_file_references: ResolvedFileReferenceTable,
        source_files: Arc<SourceDatabase>,
    ) -> Self {
        Self {
            backing: Stage0ResolutionFactsBacking::Ordinary {
                resolved_file_references,
                source_files,
            },
        }
    }

    pub(crate) fn frozen_generic(
        owner: SourceId,
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
                owner,
                references: indexed,
            },
        })
    }

    pub(crate) fn lookup(
        &self,
        source_file: SourceId,
        path_syntax: PathSyntaxId,
    ) -> Result<Option<Stage0ResolvedFileReferenceView<'_>>, CompilerError> {
        match &self.backing {
            Stage0ResolutionFactsBacking::Ordinary {
                resolved_file_references,
                source_files,
            } => {
                let Some(reference) = resolved_file_references.get(source_file, path_syntax) else {
                    return Ok(None);
                };
                ordinary_reference_view(reference, source_files).map(Some)
            }
            Stage0ResolutionFactsBacking::FrozenGeneric { owner, references } => {
                if source_file != *owner {
                    return Err(CompilerError::compiler_error(
                        "frozen generic Stage 0 lookup used a non-owning SourceId; donor and requester handles stay distinct",
                    ));
                }
                Ok(references.get(&path_syntax).map(frozen_reference_view))
            }
        }
    }
}

fn ordinary_reference_view<'a>(
    reference: &'a crate::compiler_frontend::paths::file_references::ResolvedFileReference,
    source_files: &'a SourceDatabase,
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
            let logical_path = Some(source_identity.logical_path);
            Stage0ResolvedFileReferenceOutcome::Content {
                logical_path,
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
            Stage0ResolvedFileReferenceOutcome::Diagnostic(*diagnostic)
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
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
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
            frozen_identity_handle: self.frozen_identity_handle.clone(),
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
    pub(crate) nominal_type_ids_by_path: Rc<FxHashMap<PathId, TypeId>>,

    // External package and frontend services.
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,

    // File-local visibility and resolved declarations.
    pub(crate) file_visibility: Option<Arc<FileVisibility>>,
    pub(crate) resolved_type_aliases: Option<Rc<FxHashMap<PathId, ResolvedTypeAlias>>>,
    pub(crate) generic_declarations_by_path: Option<Rc<FxHashMap<PathId, GenericDeclarationKind>>>,
    pub(crate) resolved_struct_fields_by_path: Option<Rc<FxHashMap<PathId, Vec<Declaration>>>>,
    pub(crate) choice_variant_shells_by_path: Option<Rc<FxHashMap<PathId, Vec<ChoiceVariant>>>>,
    pub(crate) resolved_module_constants_override: Option<Rc<ResolvedConstantSet>>,
    pub(crate) file_value_resolution: Option<Rc<FileValueResolutionServices>>,
    pub(crate) emitted_warnings: Rc<RefCell<Vec<DiagnosticDraft>>>,

    pub(crate) generic_function_instantiation_requests: Rc<RefCell<Vec<GenericRequestSlot>>>,
    /// Emission-owned proof that a completed no-slot body has no pending failure contributors.
    /// Missing entries, including bodies still being parsed, remain conservative candidates.
    pub(crate) known_infallible_functions: Option<Rc<RefCell<FxHashSet<PathId>>>>,
    pub(crate) source_file_scope: Option<PathId>,
    pub(crate) source_build_config_values: Option<Arc<ResolvedBuildConfigMap>>,
    /// Names of source `#Config` contracts declared by this module.
    pub(crate) source_build_config_contract_names: Option<Arc<FxHashSet<BuildInputName>>>,
    /// Optional compiler-owned grouped-project config resolver for constant-header folding.
    pub(crate) config_resolution: Option<Rc<ConfigResolutionServices>>,
    /// Owning source identity for `Stage0` joins and span construction.
    ///
    /// WHAT: the exact `SourceId` that owns this scope's token spans and path-table rows
    ///      (for materialised generics, the retained donor owner).
    /// WHY: every live scope joins `Stage0` facts and builds spans against this identity,
    pub(crate) declaring_file_id: SourceId,
    /// Required late-bound frozen identity for every source-owned scope chain.
    pub(crate) frozen_identity_handle: FrozenIdentityHandle,
    /// Per-scope compile-time template expansion limit shared by child contexts.
    pub(crate) template_const_loop_iteration_limit: usize,

    // Receiver method catalog for dispatch.
    pub(crate) receiver_methods: Rc<ReceiverMethodCatalog>,

    // Constant-header contexts are built before the final module lookup package exists.
    pub(crate) trait_environment: Rc<TraitEnvironment>,
    pub(crate) trait_evidence_environment: Rc<TraitEvidenceEnvironment>,
    pub(crate) trait_environment_override: Option<Rc<TraitEnvironment>>,
}

/// Resolved exit contract installed once by the enclosing callable's body emitter.
pub(crate) struct EnclosingCallableContract {
    pub(crate) owner: EnclosingCallableOwner,
    pub(crate) success_types: Vec<TypeId>,
    pub(crate) authored_success_slot_count: usize,
    pub(crate) error_type: Option<TypeId>,
    pub(crate) parameter_contracts: Vec<(PathId, ParameterContract)>,
}

impl EnclosingCallableContract {
    /// Contract of the enclosing callable's own parameter with this path.
    ///
    /// WHAT: returns the parameter's contract when this path names a parameter of the callable
    ///       whose body is being parsed, or `None` for every other binding.
    /// WHY: `Wire` forwarding must distinguish the callable's own `Wire` parameter from an
    ///      ordinary parameter and from a body local; the installed signature contracts are the
    ///      one owner of that fact.
    pub(crate) fn parameter_contract(&self, parameter_path: PathId) -> Option<ParameterContract> {
        self.parameter_contracts
            .iter()
            .find(|(path, _)| *path == parameter_path)
            .map(|(_, contract)| *contract)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnclosingCallableOwner {
    Function,
    EntryStart,
}

/// Shared parser/lowering context for one active AST scope.
pub struct ScopeContext {
    // Core scope identity.
    pub kind: ContextKind,
    pub scope: PathId,

    /// Preserves an enclosing match arm's line boundary in child expression contexts.
    pub(crate) match_arm_boundary: bool,

    /// Defers typed producer rejection until the complete root expression is known.
    /// This is parser control state, not an expression failure fact.
    pub(crate) defer_typed_error_validation: bool,

    // Immutable shared services are cheap to clone into child scopes.
    pub(crate) shared: Rc<ScopeShared>,

    // Typed Vec arena that owns every frame for this parse context.
    //
    // WHAT: all scope frames for one AST parse context live in one contiguous allocation.
    //       The arena is shared across all clones/children through `Rc<RefCell<_>>`,
    //       but borrow guards are never exposed through parser APIs.
    // WHY: replaces per-frame `Rc<ScopeFrame>` allocations with stable IDs and
    //       index-based parent chains.
    pub(crate) arena: Rc<RefCell<ScopeArena>>,

    /// Module-local TIR store shared by all scope contexts in this AST build.
    pub(crate) template_ir_store: Rc<RefCell<TemplateIrStore>>,

    // Stable ID of the frame that owns this scope layer's local declarations.
    pub(crate) current_frame_id: ScopeFrameId,

    unavailable_assignment_targets: FxHashSet<StringId>,
    pending_catch_assignment_targets: FxHashSet<StringId>,

    // Optional file-local visibility gate over declarations.
    pub visible_declaration_ids: Option<Arc<FxHashSet<PathId>>>,
    /// Result types of the immediate receiving site, such as an annotated declaration
    /// or a call argument. Empty when the receiver infers its own type.
    ///
    /// WHY: this is parse-time context for one receiver only. Function bodies leave it
    /// empty so the function's result slots never leak into unrelated local receivers.
    pub expected_result_type_ids: Vec<TypeId>,
    /// Callable exits use this inherited fact, never the immediate receiver or scope kind.
    pub(crate) enclosing_callable: Option<Rc<EnclosingCallableContract>>,

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

    /// Body-owned numeric construction domain shared by this body and its child blocks.
    ///
    /// WHAT: one dense origin/equality/binding/continuation table per independent function,
    ///       root or concrete-generic body. Child control-flow, expression and template scopes
    ///       share the enclosing body's handle; each independent body installs a fresh one.
    /// WHY: a binding's single static type is selected from every permitted use in its body, so
    ///      construction state must outlive any single statement and must never be shared with
    ///      an independent body; the value concept exists only in this construction vocabulary,
    ///      never as a semantic `TypeId` on a statement or expression.
    pub(crate) numeric_construction: Rc<RefCell<NumericConstructionState>>,
    /// Whether this context owns the numeric construction domain it carries.
    ///
    /// WHAT: true only for a context that begins an independent body (module/root construction
    ///       and each function body). Body completion releases the domain exactly once, at
    ///       that root.
    /// WHY: nested branches, loops, handlers and expression scopes share the enclosing domain
    ///      and must never complete or release it themselves.
    pub(crate) numeric_body_root: bool,

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
            scope: self.scope,
            match_arm_boundary: self.match_arm_boundary,
            defer_typed_error_validation: self.defer_typed_error_validation,
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: new_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids: self.expected_result_type_ids.clone(),
            enclosing_callable: self.enclosing_callable.clone(),
            active_value_target: self.active_value_target.clone(),
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            // A clone is a sibling child of the same body; it shares the domain but never owns it.
            numeric_construction: Rc::clone(&self.numeric_construction),
            numeric_body_root: false,
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
    /// WHAT: returns a provisional boundary for the requests owned by one source construct.
    /// WHY: known-true assertion messages retain semantic validation requests without activating
    ///      their executable work.
    pub(crate) fn generic_request_checkpoint(&self) -> usize {
        self.shared
            .generic_function_instantiation_requests
            .borrow()
            .len()
    }

    /// Retains a known-true assertion's request suffix for semantic validation only.
    ///
    /// Pending tickets keep their positions and validation class when they complete. Concrete
    /// requests move in place without a second payload. Repeated retention is idempotent, and
    /// tombstoned work is never revived. Callers must pass a checkpoint from this shared ledger
    /// while the assertion's own requests still form its suffix.
    pub(crate) fn retain_assertion_validation_requests_since(&self, checkpoint: usize) {
        let range = GenericRequestRange::new(checkpoint, self.generic_request_checkpoint());
        self.retain_assertion_validation_requests_in_range(range);
    }

    /// Retains only an assertion's original parse-time slots after numeric completion.
    ///
    /// Unlike a suffix checkpoint, this range excludes later siblings whose tickets were already
    /// reserved when a retained assertion's condition first became statically true.
    pub(crate) fn retain_assertion_validation_requests_in_range(&self, range: GenericRequestRange) {
        let mut slots = self
            .shared
            .generic_function_instantiation_requests
            .borrow_mut();
        debug_assert!(
            range.start <= range.end && range.end <= slots.len(),
            "generic request range must belong to the current ledger"
        );
        if let Some(retained) = slots.get_mut(range.start..range.end) {
            for slot in retained {
                *slot = match std::mem::replace(slot, GenericRequestSlot::Dropped) {
                    GenericRequestSlot::Pending => GenericRequestSlot::AssertionValidationPending,
                    GenericRequestSlot::Concrete(request) => {
                        GenericRequestSlot::AssertionValidation(request)
                    }
                    retained => retained,
                };
            }
        }
    }

    /// Records a request whose exact type and evidence are already concrete.
    pub(crate) fn record_generic_function_instantiation_request(
        &self,
        request: GenericFunctionInstantiationRequest,
    ) {
        self.shared
            .generic_function_instantiation_requests
            .borrow_mut()
            .push(GenericRequestSlot::Concrete(request));
    }

    /// Reserves an ordered ledger slot for a call whose request needs numeric completion first.
    ///
    /// WHAT: the reservation fixes the call's source-order position now. Completion publishes into
    ///       the same slot with `publish_pending_generic_request`.
    /// WHY: publication order follows parse order, not completion order, and checkpoints taken
    ///      while the call is pending still bracket it.
    pub(crate) fn reserve_pending_generic_request(&self) -> GenericRequestTicket {
        let mut slots = self
            .shared
            .generic_function_instantiation_requests
            .borrow_mut();
        let ticket = GenericRequestTicket::new(slots.len());
        slots.push(GenericRequestSlot::Pending);
        ticket
    }

    /// Publishes a resolved request into the slot reserved for it.
    ///
    /// WHAT: pending validation completes as validation-only. A `Dropped` slot accepts the
    ///       completed request without publishing it. Republishing either concrete class is a bug.
    pub(crate) fn publish_pending_generic_request(
        &self,
        ticket: GenericRequestTicket,
        request: GenericFunctionInstantiationRequest,
    ) -> Result<(), CompilerError> {
        let mut slots = self
            .shared
            .generic_function_instantiation_requests
            .borrow_mut();
        let Some(slot) = slots.get_mut(ticket.index()) else {
            return Err(CompilerError::compiler_error(
                "generic request ticket is outside the ordered request ledger",
            ));
        };
        match *slot {
            GenericRequestSlot::Pending => {
                *slot = GenericRequestSlot::Concrete(request);
                Ok(())
            }
            GenericRequestSlot::AssertionValidationPending => {
                *slot = GenericRequestSlot::AssertionValidation(request);
                Ok(())
            }
            GenericRequestSlot::Dropped => Ok(()),
            GenericRequestSlot::Concrete(_) | GenericRequestSlot::AssertionValidation(_) => Err(
                CompilerError::compiler_error("generic request ticket was published twice"),
            ),
        }
    }

    /// Abandons the slot reserved for a call whose completion failed before publication.
    ///
    /// WHAT: either pending class becomes `Dropped`, keeping its source-order position without
    ///       publishing a request. A `Dropped` slot is already inactive and stays as it is.
    /// WHY: a failed retained call must not leave a pending slot behind, and discarding every
    ///      later slot would drop unrelated work.
    pub(crate) fn abandon_pending_generic_request(
        &self,
        ticket: GenericRequestTicket,
    ) -> Result<(), CompilerError> {
        let mut slots = self
            .shared
            .generic_function_instantiation_requests
            .borrow_mut();
        let Some(slot) = slots.get_mut(ticket.index()) else {
            return Err(CompilerError::compiler_error(
                "generic request ticket is outside the ordered request ledger",
            ));
        };
        match *slot {
            GenericRequestSlot::Pending | GenericRequestSlot::AssertionValidationPending => {
                *slot = GenericRequestSlot::Dropped;
                Ok(())
            }
            GenericRequestSlot::Dropped => Ok(()),
            GenericRequestSlot::Concrete(_) | GenericRequestSlot::AssertionValidation(_) => {
                Err(CompilerError::compiler_error(
                    "generic request ticket was abandoned after it was published",
                ))
            }
        }
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
        scope: PathId,
        top_level_declarations: Rc<TopLevelDeclarationTable>,
        external_package_registry: Arc<ExternalPackageRegistry>,
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
            known_infallible_functions: None,
            source_file_scope: None,
            file_value_resolution: None,
            source_build_config_values: None,
            source_build_config_contract_names: None,
            config_resolution: None,
            declaring_file_id: SourceId::COMPILATION_ROOT,
            frozen_identity_handle: FrozenIdentityHandle::new(),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            receiver_methods: Rc::new(ReceiverMethodCatalog::default()),
            nominal_type_ids_by_path: Rc::new(FxHashMap::default()),
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
            match_arm_boundary: false,
            defer_typed_error_validation: false,
            scope,
            shared,
            arena,
            template_ir_store,
            current_frame_id: root_frame_id,
            unavailable_assignment_targets: FxHashSet::default(),
            pending_catch_assignment_targets: FxHashSet::default(),
            visible_declaration_ids: None,
            expected_result_type_ids: Vec::new(),
            enclosing_callable: None,
            active_value_target: None,
            active_generic_type_context: None,
            generic_template_validation: false,
            generic_function_instantiation_stack: Vec::new(),
            // An independently constructed context begins its own inference boundary. It owns
            // the fresh domain whether or not it later parses a body.
            numeric_construction: Rc::new(RefCell::new(NumericConstructionState::new())),
            numeric_body_root: true,
            loop_depth: 0,
        }
    }
    pub fn new_child_control_flow(
        &self,
        kind: ContextKind,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> ScopeContext {
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
        let scope_component = string_table.get_or_intern(format!("__scope_{scope_id}"));
        let scope = path_fork
            .try_intern_child(self.scope, scope_component)
            .expect("path table exhausted while creating child scope");
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
        let match_arm_boundary = self.match_arm_boundary || matches!(&kind, ContextKind::MatchArm);

        ScopeContext {
            kind,
            match_arm_boundary,
            defer_typed_error_validation: false,
            scope,
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids,
            enclosing_callable: self.enclosing_callable.clone(),
            // Branch-like child scopes inherit value production so ordinary nested
            // `if`/match paths can produce for the nearest active value block.
            // Barriers such as loops, functions, conditions, and templates keep
            // clearing the target by constructing non-branch child contexts.
            active_value_target,
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            // Block scopes join the enclosing body's numeric inference boundary.
            numeric_construction: Rc::clone(&self.numeric_construction),
            numeric_body_root: false,
            loop_depth,
        }
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
            match_arm_boundary: self.match_arm_boundary,
            defer_typed_error_validation: self.defer_typed_error_validation,
            scope: self.scope,
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids,
            enclosing_callable: self.enclosing_callable.clone(),
            active_value_target: None,
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            numeric_construction: Rc::clone(&self.numeric_construction),
            numeric_body_root: false,
            loop_depth: self.loop_depth,
        }
    }

    /// Apply expression policy in an empty child frame rather than copying visible locals.
    ///
    /// WHAT: preserves the receiving scope's control state while isolating parser captures.
    /// WHY: policy-only scopes do not introduce declarations; cloning the growing body frame
    ///      for every numeric expression retains quadratic declaration storage.
    pub(crate) fn new_expression_policy_context(&self) -> ScopeContext {
        let mut context = self.new_child_expression(self.expected_result_type_ids.clone());
        context.kind = self.kind.clone();
        context.active_value_target = self.active_value_target.clone();
        context
    }

    /// Keeps the receiving scope's policy while nested operands retain typed producers.
    pub(crate) fn new_pending_typed_error_expression_context(&self) -> ScopeContext {
        let mut context = self.new_expression_policy_context();
        context.defer_typed_error_validation = true;
        context
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
            match_arm_boundary: self.match_arm_boundary,
            defer_typed_error_validation: false,
            scope: self.scope,
            shared: Rc::clone(&self.shared),
            arena: Rc::clone(&self.arena),
            template_ir_store: Rc::clone(&self.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: self.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: self.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: self.visible_declaration_ids.clone(),
            expected_result_type_ids: vec![],
            enclosing_callable: self.enclosing_callable.clone(),
            active_value_target: None,
            active_generic_type_context: self.active_generic_type_context.clone(),
            generic_template_validation: self.generic_template_validation,
            generic_function_instantiation_stack: self.generic_function_instantiation_stack.clone(),
            numeric_construction: Rc::clone(&self.numeric_construction),
            numeric_body_root: false,
            loop_depth: self.loop_depth,
        }
    }

    /// Builds a constant child context that preserves project-aware folding/path state.
    ///
    /// WHAT: shares the parent visibility/declaration environment and forces
    ///       resolver + source file scope propagation into constant parsing paths.
    /// WHY: resolver-less constant contexts are invalid for template folding and
    ///      template-head path coercion.
    ///
    pub fn new_constant(scope: PathId, parent: &ScopeContext) -> ScopeContext {
        increment_ast_counter(AstCounter::ScopeContextsCreated);

        let child_frame_id = parent
            .arena
            .borrow_mut()
            .alloc_child_frame(parent.current_frame_id);
        record_scope_frame_depth(parent.arena.borrow().frame(child_frame_id).depth());

        ScopeContext {
            kind: ContextKind::Constant,
            match_arm_boundary: parent.match_arm_boundary,
            defer_typed_error_validation: false,
            scope,
            shared: Rc::clone(&parent.shared),
            arena: Rc::clone(&parent.arena),
            template_ir_store: Rc::clone(&parent.template_ir_store),
            current_frame_id: child_frame_id,
            unavailable_assignment_targets: parent.unavailable_assignment_targets.clone(),
            pending_catch_assignment_targets: parent.pending_catch_assignment_targets.clone(),
            visible_declaration_ids: parent.visible_declaration_ids.clone(),
            expected_result_type_ids: Vec::new(),
            enclosing_callable: parent.enclosing_callable.clone(),
            active_value_target: None,
            active_generic_type_context: parent.active_generic_type_context.clone(),
            generic_template_validation: parent.generic_template_validation,
            generic_function_instantiation_stack: parent
                .generic_function_instantiation_stack
                .clone(),
            numeric_construction: Rc::clone(&parent.numeric_construction),
            numeric_body_root: false,
            loop_depth: parent.loop_depth,
        }
    }
}

/// Authored facts for one pending numeric binding, shared by every registration of that binding.
///
/// WHAT: carries the name, interned path, source and binding spans, value mode and mutability
///       the author wrote, before the scope owner derives the binding's numeric class and frame.
/// WHY: declarations, multi-bind slots and loop items register the same authored facts, so one
///      record keeps the registration signatures short and gives the row one construction path.
pub(crate) struct AuthoredBindingFacts {
    pub(crate) name: StringId,
    pub(crate) path: PathId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) binding_span: Option<SourceSpan>,
    pub(crate) value_mode: ValueMode,
    pub(crate) mutable: bool,
}

impl ScopeContext {
    /// Share the body-owned numeric construction domain.
    ///
    /// WHAT: returns the one handle every scope of this body (and its child blocks) uses.
    /// WHY: statement builders register origins, witnesses and continuations through this handle;
    ///      only the owning body root completes and releases it.
    pub(crate) fn numeric_construction_handle(&self) -> Rc<RefCell<NumericConstructionState>> {
        Rc::clone(&self.numeric_construction)
    }

    /// The identity of the construction domain this scope belongs to.
    ///
    /// WHAT: stable for every scope of one body root, including child blocks and nested
    ///       functions declared by it never share it.
    /// WHY: module-wide stores retain pending work from several bodies, and frame lookup walks
    ///      parent chains; consumers must compare this key before using a dense binding ID.
    pub(crate) fn numeric_domain_id(
        &self,
    ) -> crate::compiler_frontend::ast::numeric_construction::NumericConstructionDomainId {
        self.numeric_construction.borrow().domain_id()
    }

    /// True when this context began an independent body and owns the numeric domain.
    pub(crate) fn is_numeric_body_root(&self) -> bool {
        self.numeric_body_root
    }

    /// Borrow the live construction state.
    ///
    /// Borrows must stay short and must not re-enter parsing: no parser call may run while a
    /// guard from this accessor is alive.
    pub(crate) fn numeric_state(&self) -> std::cell::Ref<'_, NumericConstructionState> {
        self.numeric_construction.borrow()
    }

    /// Mutable borrow of the live construction state.
    ///
    /// Same re-entrancy rule as [`ScopeContext::numeric_state`].
    pub(crate) fn numeric_state_mut(&self) -> std::cell::RefMut<'_, NumericConstructionState> {
        self.numeric_construction.borrow_mut()
    }

    /// Register one retained numeric-dependent statement in this body's construction domain.
    ///
    /// WHAT: returns the dense continuation ID the statement's authored draft position records.
    /// WHY: the frame slot is deliberately left unresolved until the body root has every use, so
    ///      statement construction resumes through one ordered continuation list owned by the
    ///      body root rather than a statement-local retry.
    pub(crate) fn register_numeric_continuation(
        &self,
        continuation: NumericContinuation,
    ) -> Result<NumericContinuationId, CompilerError> {
        self.numeric_construction
            .borrow_mut()
            .register_continuation(continuation)
    }

    /// The authored value mode of a pending numeric binding row, if the row is live.
    ///
    /// WHAT: a checked lookup into the body-owned binding table. A row that does not belong to
    ///       this body's live construction state returns `None`.
    /// WHY: template heads must read a pending binding's declared value mode without defaulting
    ///      it and without fabricating a `Declaration`. `None` means the ID is foreign or the
    ///      domain has already completed, which callers must treat as a broken compiler
    ///      invariant rather than as an immutable binding.
    pub(crate) fn pending_numeric_value_mode(
        &self,
        binding: NumericBindingId,
    ) -> Option<crate::compiler_frontend::value_mode::ValueMode> {
        self.numeric_construction
            .borrow()
            .binding_value_mode(binding)
    }

    /// The authored mutability of a pending numeric binding row, if the row is live.
    ///
    /// WHAT: a checked lookup into the body-owned binding table mirroring
    ///       [`ScopeContext::pending_numeric_value_mode`]. A row that does not belong to this
    ///       body's live construction state returns `None`.
    /// WHY: assignment validation must read a pending binding's authored mutability without a
    ///      declaration; `None` means the ID is foreign or the domain has already completed,
    ///      which callers must treat as a broken compiler invariant rather than as an immutable
    ///      binding.
    pub(crate) fn pending_numeric_is_mutable(&self, binding: NumericBindingId) -> Option<bool> {
        self.numeric_construction
            .borrow()
            .bindings()
            .get(binding.as_usize())
            .map(|row| row.mutable)
    }

    /// Register one unannotated eligible numeric binding in this scope and body domain.
    ///
    /// WHAT: creates the body-owned binding row, then appends a `Pending` slot under the
    ///       authored name in the current frame so the existing name index and parent walk
    ///       resolve the binding exactly like a ready local.
    /// WHY: a binding's single static type is only selected at the body root, so the binding must
    ///      stay visible to later statements without a parallel pending-name lookup, without a
    ///      fabricated `Declaration` and without a premature default value. The returned row ID is
    ///      the only handle later constraints use; the frame slot is upgraded in place at
    ///      completion.
    pub(crate) fn declare_pending_numeric_binding(
        &self,
        facts: AuthoredBindingFacts,
        recipe: NumericRecipe,
    ) -> Result<NumericBindingId, CompilerError> {
        // A raw literal's authored family is its origin family, except when the fragment's result
        // is independently fixed (a comparison or real whole division): those resolve their type
        // from the operator, so their origin keeps no family and cannot be pinned.
        let authored_family = match &recipe {
            NumericRecipe::Raw { items, family }
                if !raw_fragment_result_is_independently_fixed(items, *family) =>
            {
                Some(*family)
            }
            _ => None,
        };
        let mut state = self.numeric_construction.borrow_mut();
        let class = state.create_class()?;
        if let Some(family) = authored_family {
            state.note_class_family(class, family);
        }
        // A named directed origin is a forward producer: its natural result is registered on its
        // class once it resolves, so later entries read that result instead of a default. A
        // declared literal shape pins its bound numeric leaves at the origin, and a catch-protected
        // literal joins the class its fallbacks already reached.
        match &recipe {
            NumericRecipe::Directed { shape: None, .. } => state.mark_forward(class),
            NumericRecipe::Directed {
                items,
                shape: Some(shape),
            } => {
                pin_numeric_shape(&mut state, *shape, NumericPin::Bound);
                join_protected_catch_class(&mut state, class, items);
            }
            NumericRecipe::MultiBindSlot {
                shape: Some(shape), ..
            } => {
                pin_numeric_shape(&mut state, *shape, NumericPin::Bound);
            }
            NumericRecipe::Alias { source, .. }
                if state.class_is_forward(state.binding(*source).class) =>
            {
                state.mark_forward(class);
            }
            _ => {}
        }
        self.register_pending_numeric_row(state, facts, class, recipe)
    }

    /// Register one pending loop item whose element descriptor comes from a source collection.
    ///
    /// WHAT: creates the item's body-owned row with a `CollectionElement` recipe over the source
    ///       shape's element, then appends the same `Pending` frame slot a declaration uses.
    /// WHY: a numeric element shares its leaf class with the source literal, so the item and the
    ///      source constrain one class. A whole source is pinned as a loop item, so its integer
    ///      family is fixed before any later peer. A non-numeric element keeps its own class.
    pub(crate) fn declare_pending_collection_element(
        &self,
        source_collection: KnownShapeId,
        facts: AuthoredBindingFacts,
    ) -> Result<NumericBindingId, ExpressionParseError> {
        let mut state = self.numeric_construction.borrow_mut();
        let element = state.collection_element_shape(source_collection)?;
        pin_numeric_shape(&mut state, element, NumericPin::LoopItem);
        let class = state.collection_element_class(element)?;
        self.register_pending_numeric_row(
            state,
            facts,
            class,
            NumericRecipe::CollectionElement { element },
        )
        .map_err(ExpressionParseError::from)
    }

    /// Register the pending error binding of a `catch` handler over a numeric-dependent error.
    ///
    /// WHAT: pins the protected call's generic error shape as a bound numeric value, then
    ///       creates a type-only row whose recipe names that shape, and appends the same `Pending`
    ///       frame slot a declaration uses.
    /// WHY: the handler's error type is the producer's error type, which a numeric origin decides
    ///      only after the protected call completes. A named catch binding is an ordinary immutable
    ///      value bound to the existing origin, so it pins the shape like a named forwarded result:
    ///      later reads cannot select a different width. A numeric error shares its leaf class, so
    ///      `err` constrains the same origin as its producer; any other error keeps a fresh class
    ///      that completion materialises once from the shape.
    pub(crate) fn declare_pending_caught_error(
        &self,
        facts: AuthoredBindingFacts,
        shape: KnownShapeId,
    ) -> Result<NumericBindingId, ExpressionParseError> {
        let mut state = self.numeric_construction.borrow_mut();
        pin_numeric_shape(&mut state, shape, NumericPin::Bound);
        let class = match state.shape(shape) {
            KnownShape::Numeric(class) => class,
            _ => state.create_class()?,
        };
        self.register_pending_numeric_row(state, facts, class, NumericRecipe::CaughtError { shape })
            .map_err(ExpressionParseError::from)
    }

    /// Declare one payload capture of a pending `match` arm as a pending numeric binding.
    ///
    /// WHAT: the capture's class is the payload field's envelope class when that envelope is numeric,
    ///       otherwise a fresh class; its type is the envelope shape materialised at body completion.
    /// WHY: a numeric payload shares the subject's generic parameter class, so a later read of the
    ///      capture witnesses the origin's width. Unlike a caught error, a capture does not pin the
    ///      shape: the arm must not select the origin's width before a later witness does.
    pub(crate) fn declare_pending_pattern_capture(
        &self,
        facts: AuthoredBindingFacts,
        shape: KnownShapeId,
    ) -> Result<NumericBindingId, ExpressionParseError> {
        let mut state = self.numeric_construction.borrow_mut();
        let class = match state.shape(shape) {
            KnownShape::Numeric(class) => class,
            _ => state.create_class()?,
        };
        self.register_pending_numeric_row(
            state,
            facts,
            class,
            NumericRecipe::PatternPayload { shape },
        )
        .map_err(ExpressionParseError::from)
    }

    /// Append a pending row's frame slot once its construction row is registered.
    fn register_pending_numeric_row(
        &self,
        mut state: std::cell::RefMut<'_, NumericConstructionState>,
        facts: AuthoredBindingFacts,
        class: NumericClassId,
        recipe: NumericRecipe,
    ) -> Result<NumericBindingId, CompilerError> {
        let name = facts.name;
        let binding_span = facts.binding_span;
        let binding = state.register_binding(PendingBindingRow {
            name,
            path: facts.path,
            span: facts.span,
            binding_span,
            value_mode: facts.value_mode,
            mutable: facts.mutable,
            class,
            recipe: Some(recipe),
            origin: Default::default(),
            frame: self.current_frame_id,
        })?;
        let domain = state.domain_id();
        drop(state);
        self.arena
            .borrow_mut()
            .frame_mut(self.current_frame_id)
            .add_pending_numeric_var(name, binding, domain, binding_span);
        Ok(binding)
    }
}

/// Update the recorded maximum scope-frame depth.
/// WHAT: records the deepest parent-linked frame observed during AST construction.
/// WHY: the no-shadowing frame depth is an objective signal for how nested the
///      current input is, and it helps validate capacity estimates later.
fn record_scope_frame_depth(depth: usize) {
    record_ast_counter_max(AstCounter::ScopeMaxFrameDepth, depth);
}
