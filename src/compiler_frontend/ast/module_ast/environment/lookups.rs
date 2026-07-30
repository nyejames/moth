//! Completed AST environment lookup contracts.
//!
//! WHAT: holds the immutable lookup bundle produced after AST environment construction finishes.
//! WHY: AST emission, finalization, and `ScopeContext` all read from one shared
//! `Rc<AstModuleEnvironment>` instead of depending on pass-order-specific builder state.
//! Builder state lives in `builder.rs`; this file owns only the finished data contracts.

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration};
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::module_ast::environment::{
    DeclarationSemanticTable, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodCatalog;
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAnnotation;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;

use super::resolved_public_trait_roots::ResolvedPublicTraitRoot;
use super::resolved_public_type_roots::ResolvedPublicTypeRootTable;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::import_environment::HeaderImportEnvironment;
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationMetadata, ModuleSymbols,
};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Immutable resolved lookups shared by AST body emission and downstream stages.
///
/// WHAT: one flat struct that carries every side table produced during environment construction.
/// WHY: `ScopeContext` and AST finalization need random access to declarations, signatures, type
/// aliases, and receiver methods without re-traversing headers or re-resolving paths.
#[derive(Clone)]
pub(crate) struct AstModuleLookups {
    // Header-stage source data.
    // WHY: these are moved straight from the header/dependency-sort phase; AST does not rebuild them.
    pub(crate) module_symbols: ModuleSymbols,
    pub(crate) import_environment: HeaderImportEnvironment,
    pub(crate) warnings: Vec<CompilerDiagnostic>,

    // Declaration tables and resolved constant artifacts.
    // WHY: body emission and type resolution share one indexed declaration source.
    pub(crate) declaration_table: Rc<TopLevelDeclarationTable>,
    pub(crate) imported_functions_by_local_path:
        FxHashMap<InternedPath, crate::compiler_frontend::ast::AstImportedFunctionContract>,
    pub(crate) imported_struct_definitions:
        Vec<crate::compiler_frontend::ast::AstImportedStructDefinition>,
    pub(crate) imported_choice_definitions: Vec<crate::compiler_frontend::ast::AstChoiceDefinition>,
    pub(crate) module_constants: Vec<Declaration>,
    pub(crate) rendered_path_usages: Rc<RefCell<Vec<RenderedPathUsage>>>,
    pub(crate) builtin_struct_ast_nodes: Vec<AstNode>,

    // Resolved nominal-type side tables.
    // WHY: these are populated as declarations are processed and are then frozen for body emission.
    pub(crate) resolved_struct_fields_by_path: Rc<FxHashMap<InternedPath, Vec<Declaration>>>,
    pub(crate) resolved_function_signatures_by_path:
        Rc<FxHashMap<InternedPath, ResolvedFunctionSignature>>,
    // Owned directly: this map is only borrowed (never cloned as an `Rc`) during emission, so
    // AST finalization can move it straight into the `AstBuildResult` generic-template side
    // result without an `Rc::try_unwrap` dance.
    pub(crate) generic_function_templates_by_path: FxHashMap<InternedPath, GenericFunctionTemplate>,
    pub(crate) resolved_type_aliases_by_path: Rc<FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
    pub(crate) choice_variant_shells_by_path: Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>,

    // Semantic declaration classification.
    // WHY: expression dispatch must distinguish functions, nominal types, constants, and values
    // without branching on diagnostic-only `DataType` spelling.
    pub(crate) declaration_semantics: Rc<DeclarationSemanticTable>,

    // Generic declaration metadata.
    pub(crate) generic_declarations_by_path:
        Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>,

    // Canonical TypeId for each nominal struct/choice registered in type_environment.
    // WHY: parsed type resolution and downstream consumers need fast path-to-TypeId lookup.
    pub(crate) nominal_type_ids_by_path: Rc<FxHashMap<InternedPath, TypeId>>,

    // Receiver method catalog built from visible declarations and imports.
    pub(crate) receiver_methods: Rc<ReceiverMethodCatalog>,

    // Resolved trait metadata.
    // WHY: conformance validation, generic bounds, and bound-provided receiver calls need stable
    // trait IDs and requirement TypeIds without querying raw headers.
    pub(crate) trait_environment: Rc<TraitEnvironment>,

    // Validated canonical conformance evidence.
    // WHY: trait-bound checks and bound-provided receiver calls need indexed evidence instead of
    // scanning conformance headers or receiver methods repeatedly.
    pub(crate) trait_evidence_environment: Rc<TraitEvidenceEnvironment>,

    // Environment-wide immutable services copied from AstPhaseContext so ScopeShared can
    // reference everything through one Rc<AstModuleEnvironment>.
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) path_format_config: PathStringFormatConfig,
}

/// Final AST module environment paired with its canonical TypeEnvironment.
///
/// WHAT: the complete semantic context consumed by AST body emission.
/// WHY: body emission needs both the lookup side tables and the canonical type identity table
/// that assigns every semantic TypeId.
pub(crate) struct AstModuleEnvironment {
    pub(crate) lookups: Rc<AstModuleLookups>,

    // Frontend semantic type identity owned by this module.
    // WHY: AST nodes carry compact TypeIds; the environment carries the canonical table.
    pub(crate) type_environment: TypeEnvironment,

    // Transient AST-owned resolved public type-root table (type-only).
    // WHY: carried immediately before HIR lowering so semantic orchestration can project
    // canonical roots without reconstructing public semantics from HIR or source.
    pub(crate) resolved_public_type_roots: ResolvedPublicTypeRootTable,

    // Transient AST-owned resolved public trait-root vector.
    // WHY: carried immediately before HIR lowering so the public-interface draft projection
    // can build trait surfaces without reconstructing trait semantics from HIR or source.
    // The type-root table stays type-only; trait-root facts live in their own owner.
    pub(crate) resolved_public_trait_roots: Vec<ResolvedPublicTraitRoot>,
}
