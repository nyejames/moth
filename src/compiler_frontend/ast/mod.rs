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
pub(crate) mod generic_bounds;
pub(crate) mod generic_functions;
mod module_ast;
mod receiver_methods;
pub(crate) mod type_interner;
pub(crate) mod type_resolution;

pub(crate) mod expressions {
    //! Expression parsing, evaluation, and AST-owned value contracts.
    //!
    //! Runtime expressions and copy targets use the narrowed expression-owned
    //! `ExpressionRpn` and `PlaceExpression` payloads. Broad `AstNode` fragments
    //! must not be stored inside expression variants that survive AST evaluation.

    pub(crate) mod call_argument;
    pub(crate) mod call_validation;
    pub(crate) mod choice_constructor;
    pub(crate) mod constructor_views;
    pub(crate) mod error;
    pub(crate) mod eval_expression;
    pub(crate) mod expression;
    pub(crate) mod expression_kind;
    pub(crate) mod expression_rpn;
    #[cfg(test)]
    #[path = "tests/expression_test_support.rs"]
    pub(crate) mod expression_test_support;
    pub(crate) mod expression_types;
    pub(crate) mod external_namespace_members;
    pub(crate) mod function_calls;
    pub(crate) mod generic_nominal_inference;
    pub(crate) mod mutation;
    pub(crate) mod namespace_access;
    pub(crate) mod option_propagation;
    pub(crate) mod parse_expression;
    pub(crate) mod parse_expression_dispatch;
    pub(crate) mod parse_expression_identifiers;
    pub(crate) mod parse_expression_input;
    pub(crate) mod parse_expression_literals;
    pub(crate) mod parse_expression_places;
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
    pub(crate) mod declarations;
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
    pub(crate) mod scoped_blocks;
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
pub use module_ast::build_context::AstBuildContext;
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
pub(crate) use module_ast::environment::{
    ResolvedTraitParameterFact, ResolvedTraitReceiverFact, ResolvedTraitReturnFact,
};
pub use module_ast::scope_context::{ContextKind, ScopeContext};
pub(crate) use receiver_methods::{ReceiverMethodCatalog, ReceiverMethodEntry};
pub use templates::top_level_templates::AstDocFragment;
pub use templates::top_level_templates::AstDocFragmentKind;

// Imports for the AST entry point and body-parsing helper.
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::const_values::facts::AstConstFacts;
use crate::compiler_frontend::ast::module_ast::build_context::AstPhaseContext;
use crate::compiler_frontend::ast::module_ast::emission::AstEmitter;
use crate::compiler_frontend::ast::module_ast::environment::{
    AstEnvironmentInput, AstModuleEnvironmentBuilder,
};
use crate::compiler_frontend::ast::module_ast::finalization::AstFinalizer;
use crate::compiler_frontend::ast::statements::body_dispatch::parse_function_body_statements;
use crate::compiler_frontend::ast::templates::top_level_templates::AstConstTopLevelFragment;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::instrumentation::{log_ast_counters, reset_ast_counters};

use crate::benchmark_timer_log;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, ModuleMaterialisationPreparationBuilder,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::import_environment::HeaderImportEnvironment;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::parse_file_headers::{
    Header, HeaderKind, TopLevelConstFragment,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use rustc_hash::FxHashMap;
#[cfg(feature = "detailed_timers")]
use std::time::Instant;

/// Resolved choice definition carried from AST to HIR for pre-registration.
///
/// WHAT: identifies every concrete choice declaration by canonical path.
/// WHY: HIR registers choice IDs from paths, then queries variant and payload
/// metadata from the canonical `TypeEnvironment` instead of receiving a second
/// AST-shaped copy of the same semantic data.
#[derive(Clone, Debug)]
pub struct AstChoiceDefinition {
    pub nominal_path: InternedPath,
}

/// Consumer-local imported struct definition required by HIR declaration registration.
#[derive(Clone, Debug)]
pub(crate) struct AstImportedStructDefinition {
    pub(crate) nominal_path: InternedPath,
}

/// Unified AST output for all source files in one compilation unit.
///
/// WHAT: the fully resolved, typed executable AST produced by Stage 4, ready for HIR lowering.
/// WHY: one container keeps the pipeline contract explicit — HIR receives exactly this
/// struct and nothing else from the AST stage. Semantic projection side results live on
/// [`AstBuildResult`] and never reach HIR.
pub struct Ast {
    pub nodes: Vec<AstNode>,
    pub module_constants: Vec<Declaration>,
    pub doc_fragments: Vec<AstDocFragment>,

    /// The path to the original entry point file.
    pub entry_path: InternedPath,

    /// Structural role of the active module root.
    ///
    /// HIR uses this to require `start` only for normal modules. API-only support and facade
    /// modules carry executable function bodies but no implicit entry function.
    pub root_role: ModuleRootRole,

    /// Const top-level fragments with their runtime insertion indices.
    /// Builders merge these with the runtime fragment list returned by entry `start()`.
    pub const_top_level_fragments: Vec<AstConstTopLevelFragment>,
    pub rendered_path_usages: Vec<RenderedPathUsage>,
    pub warnings: Vec<CompilerDiagnostic>,

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

    /// Compile-time const facts for all declarations that resolve to constant values.
    ///
    /// WHAT: records explicit module constants, private inferred top-level constants
    ///       (start body), and body-local constants discovered during AST finalization.
    /// WHY: config validation and HIR metadata need one shared source of truth for
    ///      const-ness without re-walking the AST.
    pub const_facts: AstConstFacts,
    pub(crate) imported_functions_by_local_path:
        FxHashMap<InternedPath, AstImportedFunctionContract>,
}

/// Consumer-local semantic contract for one imported concrete source function.
///
/// Header binding owns the stable target and public summary. AST adds the locally projected
/// function and fallible-carrier handles needed by HIR without leaking `TypeId` back into the
/// provider interface or header-stage binding vocabulary.
#[derive(Clone, Debug)]
pub(crate) struct AstImportedFunctionContract {
    pub(crate) target: crate::compiler_frontend::headers::import_environment::SourceFunctionTarget,
    pub(crate) summary: crate::compiler_frontend::public_call_summary::PublicCallSummary,
    pub(crate) fallible_carrier_type_id: Option<TypeId>,
}

/// Closed typed AST construction result carrying executable `Ast` and the two semantic
/// side results consumed before HIR lowering.
///
/// WHAT: the single result of AST construction. `ast` holds executable state only.
///       `public_interface_projection_input` feeds the public-interface draft projection and
///       `materialisation_context` retains validated generic templates and their closed
///       declaring-module semantic context, both before
///       HIR receives the executable `Ast`. Config and direct Moth-template compilation discard
///       the side results because they stop at folded AST data.
/// WHY: separating the projection side results from executable `Ast` keeps HIR input
///      executable only and removes the take-before-HIR extraction dance from semantic
///      orchestration. The field list is closed: no `Rc`, `RefCell`, trait/evidence environment
///      or generic-template map remains reachable from the `Ast` passed to HIR.
pub struct AstBuildResult {
    /// Executable AST state consumed by HIR lowering.
    pub ast: Ast,

    /// Direct public-interface projection input consumed by the public-interface draft builder.
    pub public_interface_projection_input: AstPublicInterfaceProjectionInput,

    /// Construction-only declaring-module context frozen before successful publication.
    pub(crate) materialisation_context: ModuleMaterialisationPreparationBuilder,

    /// Imported generic requests inferred against provider contracts. The requester carries only
    /// stable declaration/type evidence into the build-owned sidecar worklist.
    pub(crate) deferred_generic_requests: Vec<GenericFunctionInstantiationRequest>,
}

/// Complete header-stage output consumed by AST construction.
///
/// WHAT: bundles everything produced by header parsing and dependency sorting that AST needs.
/// WHY: `Ast::new` should receive one named contract, not a loose list of parameters.
pub struct AstBuildInput {
    pub headers: Vec<Header>,
    pub module_symbols: ModuleSymbols,
    pub import_environment: HeaderImportEnvironment,
    pub top_level_const_fragments: Vec<TopLevelConstFragment>,
}

impl Ast {
    /// Constructs a complete typed AST from header-stage outputs and build services.
    ///
    /// WHAT: Orchestrates all AST construction passes in sequence, consuming sorted headers
    /// and header-built visibility through finalization, then assembles the final
    /// [`AstBuildResult`] carrying executable `Ast` and the two projection side results.
    ///
    /// WHY: Centralizes the pass sequence so the full compilation pipeline is readable in
    /// one place without implementation details. Symbol discovery and import visibility are
    /// owned by the header/dependency stages and passed in via `AstBuildInput`.
    // The entry point keeps the obvious `new` name while returning the closed build result that
    // bundles executable `Ast` with its projection side results; renaming would lose the canonical
    // AST construction entry point expected by callers.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        input: AstBuildInput,
        context: AstBuildContext<'_>,
    ) -> Result<AstBuildResult, CompilerMessages> {
        let AstBuildInput {
            headers,
            module_symbols,
            import_environment,
            top_level_const_fragments,
        } = input;

        reset_ast_counters();

        let header_count = headers.len();
        let ast_header_counts = AstHeaderCounterSnapshot::from_headers(&headers);
        let (phase_context, string_table) = AstPhaseContext::from_build_context(context);

        let mut environment = AstModuleEnvironmentBuilder::new(&phase_context).build(
            &headers,
            AstEnvironmentInput {
                module_symbols,
                import_environment,
            },
            string_table,
        )?;
        let generic_template_count = environment.lookups.generic_declarations_by_path.len();
        let receiver_method_count = environment.lookups.receiver_methods.by_function_path.len();

        #[cfg(feature = "detailed_timers")]
        let node_emission_start = Instant::now();
        let emitted = AstEmitter::new(&phase_context, &mut environment, header_count)
            .emit(headers, string_table)?;
        let generic_instance_count = emitted.generic_instance_count;
        benchmark_timer_log!(
            node_emission_start,
            "ast_emit_nodes_ms",
            "AST/emit nodes completed in: "
        );
        #[cfg(feature = "detailed_timers")]
        let _ = node_emission_start;

        #[cfg(feature = "detailed_timers")]
        let finalization_start = Instant::now();
        let build_result = AstFinalizer::new(&phase_context, environment).finalize(
            emitted,
            &top_level_const_fragments,
            string_table,
        )?;
        benchmark_timer_log!(
            finalization_start,
            "ast_finalize_ms",
            "AST/finalize completed in: "
        );
        #[cfg(feature = "detailed_timers")]
        let _ = finalization_start;

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
// lives in focused helper modules.
pub(crate) fn function_body_to_ast(
    token_stream: &mut FileTokens,
    context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<Vec<AstNode>, Box<CompilerDiagnostic>> {
    parse_function_body_statements(token_stream, context, type_interner, warnings, string_table)
}

#[cfg(test)]
#[path = "tests/const_fact_tests.rs"]
mod const_fact_tests;

#[cfg(test)]
#[path = "tests/parser_error_recovery_tests.rs"]
mod parser_error_recovery_tests;

#[cfg(test)]
#[path = "tests/type_resolution_tests.rs"]
mod type_resolution_tests;
