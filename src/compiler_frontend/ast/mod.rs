//! AST stage — Stage 4 of the frontend pipeline.
//!
//! WHAT: constructs a typed AST from pre-sorted, pre-shaped top-level headers produced by
//! Stages 2–3 (header parsing and dependency sorting).
//!
//! WHY: separating executable-body parsing from header discovery keeps the pipeline
//! deterministic and lets AST focus on semantic resolution, type checking, and body
//! lowering without rediscovering top-level symbols.
//!
//! ## Ownership contract
//!
//! **Header parsing + dependency sorting own:**
//! - top-level declaration discovery
//! - top-level declaration shell parsing
//! - strict top-level dependency ordering
//! - appending the normal root's implicit `start` header last (outside dependency graph)
//!
//! **AST owns:**
//! - lowering sorted header payloads (no top-level shell reparsing)
//! - resolving and validating top-level types/symbols
//! - parsing function bodies and other executable/body-local declarations
//! - template composition, compile-time folding, and runtime render-plan preparation
//!
//! ## Pipeline
//!
//! 1. `AstModuleEnvironmentBuilder::build` — resolves type aliases, nominal types, function
//!    signatures, receiver catalog, and constants from sorted headers.
//! 2. `AstEmitter::emit` — lowers function/start/template bodies into typed AST nodes.
//! 3. `AstFinalizer::finalize` — normalizes templates/constants and assembles [`Ast`] output.
//!
//! Entry point: [`Ast::new`].

// Internal AST implementation modules.
//
// `module_ast` contains the environment, emission, finalization, and scope-context helpers that
// implement the AST pipeline. The rest of the AST surface is split by concern
// (expressions, statements, templates, field access, etc.).
pub(crate) mod ast_nodes;
pub(crate) mod const_eval;
pub(crate) mod const_values;
pub(crate) mod cursor;
pub(crate) mod file_value_resolution;
pub(crate) mod generic_bounds;
pub(crate) mod generic_functions;
mod module_ast;
mod receiver_methods;

/// Body-owned numeric construction state for the scalar inference path.
pub(crate) mod numeric_construction;
pub(crate) mod numeric_known_shape;

#[cfg(feature = "timers")]
pub(crate) use module_ast::build_context::AstTimingMetricFamily;

pub(crate) mod type_interner;
pub(crate) mod type_resolution;

pub(crate) mod expressions {
    //! Expression parsing, evaluation, and AST-owned value contracts.
    //!
    //! Completed semantic values use one phase-owned dense expression store.
    //! Parser RPN and body-local deferred expressions remain transient construction scratch.

    pub(crate) mod anonymous_const_record;
    pub(crate) mod assertion_message_effects;
    pub(crate) mod call_argument;
    pub(crate) mod call_arguments;
    pub(crate) mod call_validation;
    pub(crate) mod choice_constructor;
    pub(crate) mod constructor_views;
    pub(crate) mod error;
    pub(crate) mod eval_expression;
    pub(crate) mod expression;
    pub(crate) mod expression_kind;
    pub(crate) mod expression_rewrite;
    pub(crate) mod expression_rpn;
    pub(crate) mod expression_store;
    pub(crate) mod expression_types;
    pub(crate) mod external_namespace_members;
    pub(crate) mod failure_classification;
    pub(crate) mod failure_facts;
    pub(crate) mod function_calls;
    pub(crate) mod generic_nominal_inference;
    pub(crate) mod generic_nominal_retention;
    pub(crate) mod mutation;
    pub(crate) mod namespace_access;
    pub(crate) mod option_propagation;
    pub(crate) mod parse_expression;
    pub(crate) mod parse_expression_dispatch;
    pub(crate) mod parse_expression_identifiers;
    pub(crate) mod parse_expression_input;
    pub(crate) mod parse_expression_literals;
    pub(crate) mod parse_expression_places;
    pub(crate) mod parse_expression_qualified_references;
    pub(crate) mod parse_expression_templates;
    pub(crate) mod source_function_calls;
    pub(crate) mod struct_instance;
}

pub(crate) mod statements {
    pub(crate) mod asserts;
    pub(crate) mod body_dispatch;
    pub(crate) mod body_expr_stmt;
    pub(crate) mod body_return;
    pub(crate) mod body_symbol;
    pub(crate) mod branching;
    pub(crate) mod collections;
    pub(crate) mod condition_validation;
    pub(crate) mod control_flow_exits;
    pub(crate) mod declarations;
    pub(crate) mod declared_regions;
    pub(crate) mod diagnostics;
    pub(crate) mod fallible_handling;
    pub(crate) mod functions;
    pub(crate) mod if_headers;
    pub(crate) mod loop_headers;
    pub(crate) mod loops;
    pub(crate) mod match_arm_boundaries;
    pub(crate) mod match_exhaustiveness;
    pub(crate) mod match_headers;
    pub(crate) mod match_patterns;
    pub(crate) mod multi_bind;
    pub(crate) mod never_call;
    pub(crate) mod numeric_body_completion;
    pub(crate) mod terminality;
    pub(crate) mod value_production;
}

mod field_access;
mod place_access;
pub(crate) mod templates;

// Public surface — minimal API consumed by later compiler stages.
//
// WHY: the AST module should expose one obvious entry surface. Internal helpers,
// pass implementations, and parser submodules stay private to `ast/`.
pub(in crate::compiler_frontend) use module_ast::build_context::AstBuildContext;
pub(crate) use module_ast::environment::AstPublicInterfaceProjectionInput;
pub(crate) use module_ast::environment::ResolvedPublicTypeRootTable;
pub(crate) use module_ast::environment::TopLevelDeclarationTable;
pub(crate) use module_ast::environment::{
    ResolvedPublicTraitRoot, ResolvedTraitRequirementFact, TraitReceiverAccessKind,
};
pub(crate) use module_ast::environment::{
    ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind, ResolvedTraitSourceFact,
};
#[cfg(test)]
pub(crate) use module_ast::environment::{ResolvedTraitParameterFact, ResolvedTraitReceiverFact};
pub(crate) use module_ast::scope_context::ScopeDeclarationRef;
pub use module_ast::scope_context::{ContextKind, ScopeContext};
pub(crate) use module_ast::scope_context::{FileValueResolutionServices, Stage0ResolutionFacts};
pub(crate) use receiver_methods::{ReceiverMethodCatalog, ReceiverMethodEntry};

// Imports for the AST entry point and body-parsing helper.
use crate::compiler_frontend::ast::ast_nodes::{AstFunctionIdentity, AstNode};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionStore,
};
use crate::compiler_frontend::ast::expressions::failure_classification::PendingFunctionFailureFacts;
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironmentBuilder,
};
use crate::compiler_frontend::ast::module_ast::finalization::{
    AstFinalizer, MaterialisationContextRetention,
};
use crate::compiler_frontend::ast::statements::body_dispatch::{
    ParsedBody, parse_function_body_statements,
};
use crate::compiler_frontend::ast::statements::terminality::NeverProofNode;
use crate::compiler_frontend::ast::templates::top_level_templates::AstConstTopLevelFragment;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::instrumentation::{log_ast_counters, reset_ast_counters};

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, ModuleMaterialisationPreparationBuilder,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDraft, PreparedFailure,
};
use crate::compiler_frontend::datatypes::FunctionReturnContract;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::headers::binding_environment::HeaderBindingEnvironment;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::parse_file_headers::{
    Header, HeaderKind, TopLevelConstFragment,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::{ConstraintDirectiveSpans, SourceConstraintSet};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::line_scanning::ScopeOpening;
use crate::compiler_frontend::traits::evidence::environment::InfallibleCastEvidenceContract;
use crate::timing_scope_attributed;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// Resolved choice definition carried from AST to HIR for pre-registration.
///
/// WHAT: identifies every concrete choice declaration by canonical path.
/// WHY: HIR registers choice IDs from paths, then queries variant and payload
/// metadata from the canonical `TypeEnvironment` instead of receiving a second
/// AST-shaped copy of the same semantic data.
#[derive(Clone, Debug)]
pub struct AstChoiceDefinition {
    pub nominal_path: PathId,
}

/// Consumer-local imported struct definition required by HIR declaration registration.
#[derive(Clone, Debug)]
pub(crate) struct AstImportedStructDefinition {
    pub(crate) nominal_path: PathId,
}

/// One compile-time documentation record attached to its declaration.
///
/// WHAT: carries the target declaration path, the folded text and the directive operand span.
/// WHY: documentation is declaration metadata, not an unattached page fragment. The declaration
/// path is the stable module-local identity later consumers join on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AstDocFragment {
    pub target: PathId,
    pub value: StringId,
    pub span: Option<SourceSpan>,
}

/// One lexical source-constraint attachment retained past AST construction.
///
/// WHAT: names the constrained callable and, for a statement attachment, the exact authored token
///       range the constraint covers.
/// WHY: constraint verification runs after AST construction. This is the one record that lets a
///      later stage map a directive attachment onto HIR operations without re-walking directive
///      syntax, and it deliberately keeps span endpoints as authored token spans instead of
///      inventing a joined byte range that the source's extended-span table does not own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AstConstraintBoundary {
    /// The callable body whose lexical scope owns this attachment.
    pub(crate) function: AstFunctionIdentity,
    /// Lexical scope where the constraint attached.
    ///
    /// WHY: a discarded static-`if` branch is exactly a child scope subtree. Keeping the scope
    ///      lets projection exclude evidence recorded after folding, whose operator span may no
    ///      longer be inside the branch's surviving authored extent.
    pub(crate) scope: PathId,
    /// The annotated statement's authored token range; `None` means the whole callable body.
    pub(crate) statement: Option<AstConstraintStatementSpan>,
    /// Constraints active at this boundary, including those inherited from enclosing annotations.
    pub(crate) constraints: SourceConstraintSet,
    /// Authored directive spans of the guarantees this boundary declares itself.
    ///
    /// WHAT: the run's `$safe_math` and `$infallible` spans, each `None` when that guarantee was
    ///       inherited here or the boundary came from a reconstructed constraint set.
    /// WHY: verification runs after directive syntax is gone, so each label must survive with the
    ///      boundary. A statement that merely inherits a guarantee leaves that field empty, letting
    ///      the enclosing owner keep its own directive instead of reusing this run's first span.
    pub(crate) directive_spans: ConstraintDirectiveSpans,
}

/// Inclusive authored token range of one constrained statement.
///
/// WHAT: the first and last token spans of the attached statement.
/// WHY: joining two token spans into one byte range needs the source's extended-span builder,
///      which the statement parser does not own. Two authored endpoints let later span-based
///      verification test containment without manufacturing an unresolvable span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AstConstraintStatementSpan {
    pub(crate) start: SourceSpan,
    pub(crate) end: SourceSpan,
}

/// One discarded static-`if` branch that must not contribute constraint evidence.
///
/// WHAT: the owning callable and the inclusive authored token range of the inactive branch body.
/// WHY: the fast-use ledger and constraint boundaries are recorded while both branches are
///      resolved, but an inactive branch is not selected code. Recording one conservative region
///      per discarded branch lets HIR projection drop those rows without retaining the removed
///      subtree or re-walking the expression graph. Authored node spans are the only endpoints
///      available at selection time; `None` spans (generated nodes) contribute nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AstConstraintPrunedRegion {
    /// The callable body whose static branch was discarded.
    pub(crate) function: AstFunctionIdentity,
    /// The discarded branch's lexical scope, when that scope is exclusive to the branch.
    ///
    /// WHAT: `Some` only when the two branches were parsed under distinct child scopes.
    /// WHY: scope identity is exact even when a folded expression's surviving span no longer
    ///      reaches the operator the fast-use ledger recorded. Inline value-`if` shares its
    ///      enclosing scope between both branches, so scope filtering is unsound there and the
    ///      span fallback must decide instead.
    pub(crate) scope: Option<PathId>,
    /// First authored span in the discarded branch.
    pub(crate) start: SourceSpan,
    /// Last authored span in the discarded branch.
    pub(crate) end: SourceSpan,
}

/// One selected fast numeric operation recorded while resolving an authored expression.
///
/// WHAT: the callable that owns the operation, its lexical scope and the authored operator span.
/// WHY: `$safe_math` must still see a fast use after constant folding or dead-result elimination
///      removes the executable operation. Resolution records the selection before either
///      optimisation runs, so this sparse evidence outlives the node it describes. One record per
///      distinct operation keeps the ledger proportional to authored fast arithmetic rather than
///      to the expression graph, and the owning callable lets a later verifier attribute the use
///      without re-reading directives or lexical scopes.
///
/// The derived order is the ledger's canonical order: owning callable, lexical scope, then
/// operator span, with the entry body first and an absent span before any real one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AstFastOperationWitness {
    /// The callable body whose authored expression selected fast arithmetic.
    pub(crate) function: AstFunctionIdentity,
    /// The lexical scope that resolved the operation.
    ///
    /// WHY: inactive static branches are scope subtrees. A folded operation's recorded operator
    ///      span can outlive its expression node, but its scope still names the discarded branch.
    pub(crate) scope: PathId,
    /// The authored operator span; `None` for a synthetic operation without source text.
    pub(crate) span: Option<SourceSpan>,
}

/// Canonicalises a fast-operation witness ledger.
///
/// WHAT: sorts by [`AstFastOperationWitness`]'s order and drops duplicate rows.
/// WHY: recording is one push per selected operation, so an expression resolved twice by deferred
///      completion would otherwise carry two rows for one authored operation. Normalising once
///      after every producer has contributed, rather than at each record, keeps the recorder
///      allocation-free per selection and replaces a linear membership scan per record with a
///      single sort.
pub(crate) fn normalize_fast_operation_witnesses(witnesses: &mut Vec<AstFastOperationWitness>) {
    witnesses.sort_unstable();
    witnesses.dedup();
}

/// One selected ordinary call that release constant folding replaced with its value.
///
/// WHAT: the owning callable, the callee's AST declaration path and the authored call span.
/// WHY: release speculation runs on the completed AST before HIR, so a folded call site is
///      absent from every HIR block. The erased call is still selected source, though: its callee's
///      summary bits and fast-use witness must reach the caller exactly as they do in a debug build
///      that keeps the call. HIR projects each row through the same call-target mapping and
///      inactive-static-branch filter as authored and skipped call evidence. The span is mandatory
///      because every projected row carries its authored call site; the release producer declines
///      to erase a spanless call instead. Catch handler bodies and handled subjects are never
///      folded, so every recorded row belongs to source HIR lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AstReleaseErasedCallSite {
    /// The callable body whose expression release speculation folded.
    pub(crate) function: AstFunctionIdentity,
    /// The callee's ordered declaration path in this module's AST.
    pub(crate) target: PathId,
    /// The authored span of the erased call expression.
    pub(crate) span: SourceSpan,
}

/// Unified AST output for all source files in one compilation unit.
///
/// WHAT: the fully resolved, typed executable AST produced by Stage 4, ready for HIR lowering.
/// WHY: one container keeps the pipeline contract explicit — HIR receives exactly this
/// struct and nothing else from the AST stage. Semantic projection side results live on
/// [`AstBuildResult`] and never reach HIR.
pub struct Ast {
    pub nodes: Vec<AstNode>,
    /// The module-local folded-value graph consumed by public projection, HIR and short
    /// compiler services. Authored declarations remain in the declaration table during
    /// finalization and are not duplicated into an AST declaration vector.
    pub const_values: ConstValueStore,
    /// Immutable semantic expression authority for all executable roots.
    pub(crate) expressions: ExpressionStore,
    pub doc_fragments: Vec<AstDocFragment>,
    /// Lexical source-constraint attachments, in authored attachment order.
    ///
    /// WHAT: one record per constrained function body and per constrained statement.
    /// WHY: constraint verification after AST construction must attach guarantees to HIR callables
    ///      and statement ranges without re-reading source directives.
    pub(crate) constraint_boundaries: Vec<AstConstraintBoundary>,

    /// Selected fast numeric operations, in canonical ledger order.
    ///
    /// WHAT: one sparse record per selected fast operation, kept from resolution through folding
    ///       and dead-result elimination.
    /// WHY: `$safe_math` and `$infallible` verification run after optimisation, so evidence of a
    ///      selected fast operation cannot live on the (possibly folded) expression node. The
    ///      ledger is recorded beside the executable AST and ordered and deduplicated once in
    ///      finalization after every producer has contributed. HIR then projects it per function
    ///      into constraint evidence before the AST is dropped.
    pub(crate) fast_operation_witnesses: Vec<AstFastOperationWitness>,

    /// Selected ordinary calls erased by release constant folding, in recording order.
    ///
    /// WHAT: one sparse row per folded call site, keyed by its owning callable.
    /// WHY: `$safe_math` and `$infallible` verification run after release speculation, so a folded
    ///      call has no HIR block that could evidence its callee. The ledger keeps the call site
    ///      visible to HIR projection without retaining the erased expression tree or disabling a
    ///      profitable fold. Catch handler bodies and handled subjects stay at runtime, so every
    ///      row is executable source HIR lowered. It is empty in debug builds because no call is
    ///      erased there.
    pub(crate) release_erased_call_sites: Vec<AstReleaseErasedCallSite>,

    /// Pre-fold failure facts for every concrete callable in `nodes`, keyed by callable identity.
    ///
    /// WHAT: the implicit contributors, deferred custom catches and assertion-message calls the
    ///       AST classifier decided before release folding. Authored-dead statement tails were
    ///       excluded from executable contributors per occurrence, so each body's contributor
    ///       lists keep authored order, but those tails still supply their validation-only
    ///       obligations (deferred custom catches and assertion-message calls). The map itself is
    ///       only a lookup. Dormant generic-template identities are dropped because HIR never
    ///       reads them.
    /// WHY: release folding must not add, remove or reorder failure facts. Finalization always
    ///      computes these facts, even when no dormant template needs boundary validation, and HIR
    ///      projects them instead of reclassifying the post-fold AST, so debug and release publish
    ///      the same contributors and the same witness order.
    pub(crate) pre_fold_failure_facts: HashMap<AstFunctionIdentity, PendingFunctionFailureFacts>,

    /// Statement-owned `Bool` conditions the folded-Bool authority decided before release folding.
    ///
    /// WHAT: one row per `assert`, `if`, value-`if` and conditional-loop condition, keyed by its
    ///       expression root.
    /// WHY: HIR lowering classifies these conditions by the pre-optimisation selection fact. A
    ///      release fold can replace an ordinary call with its constant result, and reading the
    ///      post-fold store would then omit a loop body or a trap that a debug build keeps. A
    ///      condition no profile folded has no row.
    pub(crate) pre_fold_bool_conditions: FxHashMap<ExprId, bool>,

    /// Inactive static-`if` branches, retained so HIR projection can drop their evidence rows.
    ///
    /// WHAT: one conservative authored range per discarded branch, keyed by owning callable.
    /// WHY: both static branches are fully resolved, so their fast-use evidence and statement
    ///      boundaries exist before selection. Inactive branches produce no HIR operations, and
    ///      their ledger rows must not make an annotation invalid. The ranges stay as authored
    ///      spans because a joined byte range needs the source's extended-span table, which the
    ///      statement parser does not own.
    pub(crate) constraint_pruned_regions: Vec<AstConstraintPrunedRegion>,

    /// Whether preparation synthesized the compiler-owned entry `start` body.
    ///
    /// HIR uses this to require exactly one entry only when the module owns a start-bearing root.
    /// API-only modules carry executable function bodies but no implicit entry function.
    pub synthesizes_entry_start: bool,

    /// Const top-level fragments with their runtime insertion indices.
    /// Builders merge these with the runtime fragment list returned by entry `start()`.
    pub const_top_level_fragments: Vec<AstConstTopLevelFragment>,
    pub warnings: Vec<DiagnosticDraft>,

    /// Resolved choice definitions for HIR pre-registration.
    ///
    /// WHAT: every choice declaration (local and imported) with fully resolved payload field types.
    /// WHY: HIR registers choices deterministically before function lowering.
    pub choice_definitions: Vec<AstChoiceDefinition>,
    pub(crate) imported_struct_definitions: Vec<AstImportedStructDefinition>,

    /// Frontend type environment with all interned types and substituted generic instance views.
    ///
    /// WHAT: carries the canonical TypeEnvironment from AST construction to HIR lowering.
    /// WHY: HIR needs to query substituted fields/variants for generic struct/choice instances.
    pub type_environment: TypeEnvironment,

    pub(crate) imported_functions_by_local_path: FxHashMap<PathId, AstImportedFunctionContract>,
    /// Canonical infallible cast requirements, projected onto local executable method paths.
    pub(crate) infallible_cast_evidence: FxHashMap<PathId, InfallibleCastEvidenceContract>,
    /// Provenance introduced by static configuration branch selection, keyed by function identity
    /// until HIR allocates the build-local function IDs.
    pub(crate) static_if_function_provenance:
        FxHashMap<AstFunctionIdentity, SyntheticInterfaceProvenance>,
}

/// Consumer-local semantic contract for one imported concrete source function.
///
/// Header binding owns the stable target and public summary. AST adds consumer-local ordered
/// success slots and the separate error channel needed by HIR without leaking `TypeId` back
/// into the provider interface or header-stage binding vocabulary.
#[derive(Clone, Debug)]
pub(crate) struct AstImportedFunctionContract {
    pub(crate) target: crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget,
    pub(crate) summary: crate::compiler_frontend::public_call_summary::PublicCallSummary,
    pub(crate) return_contract: FunctionReturnContract,
}

/// Closed typed AST construction result carrying executable `Ast` and the three semantic
/// side results consumed before HIR lowering.
///
/// WHAT: the single result of AST construction. `ast` holds executable state only;
///       `public_interface_projection_input` feeds public-interface projection,
///       `module_resources` retains the module-local resource table for the HIR executable lane,
///       and `materialisation_context` retains validated generic templates with their closed
///       declaring-module semantic context.
/// WHY: these side results have distinct downstream owners and do not belong to executable `Ast`;
///      direct Moth-template compilation discards them after folded AST data is produced.
pub struct AstBuildResult {
    /// Executable AST state consumed by HIR lowering.
    pub ast: Ast,

    /// Direct public-interface projection input consumed by the public-interface draft builder.
    pub public_interface_projection_input: AstPublicInterfaceProjectionInput,
    /// The paired support surface projection input, when this module owns a support root.
    pub support_public_interface_projection_input: Option<AstPublicInterfaceProjectionInput>,

    /// Construction-only declaring-module capture consumed before HIR lowering.
    ///
    /// Module builds always retain it. A generated sidecar retains it for nested executable or
    /// assertion-validation requests. The caller resolves stable callable identities, then
    /// detaches owned syntax, canonical signatures/defaults and constants before AST release.
    pub(crate) materialisation_context: Option<ModuleMaterialisationPreparationBuilder>,
    /// The same module-local resource table used while resolving file-valued expressions.
    ///
    /// Public projection borrows this handle to translate local `ResourceId`s into portable
    /// origins. The table remains shared with AST services and is never copied into the draft.
    pub(crate) module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,

    /// Imported generic requests inferred against provider contracts. The requester carries only
    /// stable declaration/type evidence into the compiler-owned generated-function transaction.
    pub(crate) deferred_generic_requests: Vec<GenericFunctionInstantiationRequest>,

    /// Generic instances needed only to validate retained inactive assertion-message witnesses.
    /// These requests own no executable demand and exclude instances already requested above.
    pub(crate) assertion_validation_generic_requests: Vec<GenericFunctionInstantiationRequest>,

    /// Exact-target divergence-proof nodes for this module's own declared `-> !` functions.
    ///
    /// WHAT: one node per declared Never function in the executable AST, whose dependencies are
    ///       the raw standalone Never call targets, including concrete generic instance paths.
    /// WHY: the declaring module's template graph remaps instances to template declarations, so it
    ///      cannot see a cycle that closes through a generated instance's trait-bound call. The
    ///      requester unions these nodes with every generated sidecar's nodes after materialisation
    ///      and rejects a cycle that its own source participates in.
    pub(crate) exact_never_proofs: Vec<NeverProofNode>,
}

/// Complete header-stage output consumed by AST construction.
///
/// WHAT: bundles everything produced by header parsing and dependency sorting that AST needs.
/// WHY: `Ast::new` should receive one named contract, not a loose list of parameters.
pub(in crate::compiler_frontend) struct AstBuildInput {
    pub headers: Vec<Header>,
    pub source_token_owners: crate::compiler_frontend::headers::SourceTokenOwners,
    pub module_symbols: ModuleSymbols,
    pub binding_environment: HeaderBindingEnvironment,
    pub top_level_const_fragments: Vec<TopLevelConstFragment>,
    /// Recovery budgets for ordinary body diagnostics in this module.
    ///
    /// The project configuration owns the error count; the compiler owns the additional-body token
    /// allowance. Carrying it here keeps the emitter from reading either owner.
    pub recovery_policy: crate::compiler_frontend::module_compilation::AstRecoveryPolicy,
    /// Names of this module's own source `$config` contracts.
    ///
    /// The boundary value map is project-wide, but source qualifier materialization must remain
    /// module-local so a namesake declaration in another source cannot borrow its value.
    pub source_build_config_contract_names:
        Arc<FxHashSet<crate::compiler_frontend::build_config::BuildInputName>>,
}

impl Ast {
    /// Constructs a complete typed AST from header-stage outputs and build services.
    ///
    /// WHAT: Orchestrates all AST construction passes in sequence, consuming sorted headers
    /// and header-built visibility through finalization, then assembles the final
    /// [`AstBuildResult`] carrying executable `Ast`, public-interface projection input,
    /// module-local resources and generic materialisation context.
    ///
    /// WHY: Centralizes the pass sequence so the full compilation pipeline is readable in
    /// one place without implementation details. Symbol discovery and dependency visibility are
    /// owned by the header/dependency stages and passed in via `AstBuildInput`.
    // The entry point keeps the obvious `new` name while returning the closed build result that
    // bundles executable `Ast` with public-interface projection input, module-local resources and
    // generic materialisation context; renaming would lose the canonical AST construction entry
    // point expected by callers.
    #[allow(clippy::new_ret_no_self)]
    pub(in crate::compiler_frontend) fn new(
        input: AstBuildInput,
        context: AstBuildContext,
    ) -> Result<AstBuildResult, PreparedFailure> {
        let AstBuildInput {
            headers,
            source_token_owners,
            module_symbols,
            binding_environment,
            top_level_const_fragments,
            recovery_policy,
            source_build_config_contract_names,
        } = input;
        reset_ast_counters();

        let header_count = headers.len();
        let ast_header_counts = AstHeaderCounterSnapshot::from_headers(&headers);
        let (mut phase_context, string_table, path_fork) =
            AstPhaseContext::from_build_context(context, source_build_config_contract_names)?;

        timing_scope_attributed!(
            timing_guard_ast_total,
            phase_context.timing_metric_family.total(),
            phase_context.timing_context
        );

        let mut environment = AstModuleEnvironmentBuilder::new(&mut phase_context, path_fork)
            .build(
                &headers,
                AstEnvironmentInput {
                    module_symbols,
                    binding_environment,
                    source_token_owners: source_token_owners.clone(),
                },
                string_table,
            )?;
        let generic_template_count = environment.lookups.generic_declarations_by_path.len();
        let receiver_method_count = environment.lookups.receiver_methods.by_function_path.len();

        let emitted = {
            timing_scope_attributed!(
                timing_guard,
                phase_context.timing_metric_family.emit(),
                phase_context.timing_context
            );
            AstEmitter::new(
                &mut phase_context,
                &mut environment,
                header_count,
                path_fork,
                source_token_owners,
            )
            .emit(headers, string_table, recovery_policy)?
        };
        let generic_instance_count = emitted.generic_instance_count;
        let build_result = {
            timing_scope_attributed!(
                timing_guard,
                phase_context.timing_metric_family.finalise(),
                phase_context.timing_context
            );
            AstFinalizer::new(phase_context, environment, path_fork).finalize(
                emitted,
                &top_level_const_fragments,
                MaterialisationContextRetention::Always,
                string_table,
            )?
        };

        ast_header_counts.record();
        add_frontend_counter(
            FrontendCounter::AstReceiverMethodCount,
            receiver_method_count,
        );
        add_frontend_counter(
            FrontendCounter::AstGenericTemplateCount,
            generic_template_count,
        );
        add_frontend_counter(
            FrontendCounter::AstGenericInstanceCount,
            generic_instance_count,
        );

        log_ast_counters();

        Ok(build_result)
    }
}

/// Cheap AST boundary counts derived from the sorted headers AST already consumes.
///
/// WHY: these counters explain AST work volume without walking emitted AST nodes or rebuilding
/// declaration metadata owned by earlier stages.
struct AstHeaderCounterSnapshot {
    header_count: usize,
    function_count: usize,
    struct_count: usize,
    choice_count: usize,
    constant_count: usize,
    trait_declaration_count: usize,
    trait_conformance_count: usize,
}

impl AstHeaderCounterSnapshot {
    fn from_headers(headers: &[Header]) -> Self {
        let mut function_count = 0usize;
        let mut struct_count = 0usize;
        let mut choice_count = 0usize;
        let mut constant_count = 0usize;
        let mut trait_declaration_count = 0usize;
        let mut trait_conformance_count = 0usize;

        for header in headers {
            match &header.kind {
                HeaderKind::Function { .. } => function_count += 1,

                HeaderKind::Struct { .. } => struct_count += 1,

                HeaderKind::Choice { .. } => choice_count += 1,

                HeaderKind::Constant { .. } => constant_count += 1,

                HeaderKind::Trait { .. } => trait_declaration_count += 1,

                HeaderKind::TraitConformance { .. } => trait_conformance_count += 1,

                HeaderKind::TypeAlias { .. }
                | HeaderKind::ConstTemplate { .. }
                | HeaderKind::StartFunction
                | HeaderKind::TraitIncompatibility { .. } => {}
            }
        }

        Self {
            header_count: headers.len(),
            function_count,
            struct_count,
            choice_count,
            constant_count,
            trait_declaration_count,
            trait_conformance_count,
        }
    }

    fn record(&self) {
        add_frontend_counter(FrontendCounter::AstHeaderCount, self.header_count);
        add_frontend_counter(FrontendCounter::AstFunctionCount, self.function_count);
        add_frontend_counter(FrontendCounter::AstStructCount, self.struct_count);
        add_frontend_counter(FrontendCounter::AstChoiceCount, self.choice_count);
        add_frontend_counter(FrontendCounter::AstConstantCount, self.constant_count);
        add_frontend_counter(
            FrontendCounter::AstTraitDeclarationCount,
            self.trait_declaration_count,
        );
        add_frontend_counter(
            FrontendCounter::AstTraitConformanceCount,
            self.trait_conformance_count,
        );
    }
}

// WHAT: public(crate) entrypoint for function/start-function body parsing.
// WHY: callers should import one obvious `ast`-root function while detailed statement parsing
//      lives in focused helper modules.
//
// The result distinguishes the two body kinds explicitly:
// - a caller that passed an independent body root receives `ParsedBody::Complete` (nodes only,
//   after every numeric constraint, ordered resumption and construction-table release);
// - a caller that passed a nested construct context receives `ParsedBody::Draft`, whose pending
//   positions resolve when the enclosing root completes. Nested callers must retain the draft
//   (`DeferredBody`/`DeferredConstruct`) or reject the position with `into_nodes_for_host`; they
//   must never unwrap a draft into published nodes.
/// Parse one function, entry or handler body.
///
/// `body_opening` is the effective opening site of an authored block body, so end of file inside
/// it is reported as a missing `;` labelled on that opener. Callers pass `None` when end of file is
/// not a missing closer: the compiler-owned entry start, and function bodies whose token range
/// header capture already balanced.
#[allow(
    clippy::too_many_arguments,
    reason = "body entry forwards the cursor, scope and mutable parser state unchanged"
)]
pub(crate) fn function_body_to_ast(
    token_stream: &mut AstCursor,
    context: ScopeContext,
    body_opening: Option<ScopeOpening>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedBody, ExpressionParseError> {
    parse_function_body_statements(
        token_stream,
        context,
        body_opening,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
    )
}

#[cfg(test)]
#[path = "tests/parser_error_recovery_tests.rs"]
mod parser_error_recovery_tests;

#[cfg(test)]
#[path = "tests/type_resolution_tests.rs"]
mod type_resolution_tests;

#[cfg(test)]
#[path = "tests/type_alias_resolution_tests.rs"]
mod type_alias_resolution_tests;

#[cfg(test)]
#[path = "tests/file_value_resolution_tests.rs"]
mod file_value_resolution_tests;

#[cfg(test)]
#[path = "tests/cursor_tests.rs"]
mod cursor_tests;

#[cfg(test)]
#[path = "tests/receiver_method_tests.rs"]
mod receiver_method_tests;

#[cfg(test)]
#[path = "tests/numeric_construction_tests.rs"]
mod numeric_construction_tests;

#[cfg(test)]
#[path = "tests/numeric_known_shape_tests.rs"]
mod numeric_known_shape_tests;

#[cfg(test)]
#[path = "tests/fast_use_witness_tests.rs"]
mod fast_use_witness_tests;
