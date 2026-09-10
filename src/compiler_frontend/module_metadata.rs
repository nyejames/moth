//! Compiler-frontend-owned module metadata extracted during HIR lowering.
//!
//! WHAT: owns non-HIR compiler metadata that HIR lowering extracts from the AST — resolved
//! documentation fragments — plus the typed lowering result boundary.
//! WHY: these are compiler/builder-facing metadata lanes, not executable semantic HIR state. HIR
//! must carry only executable/semantic IR; documentation fragments belong to the module compilation
//! boundary, not the HIR payload.
//!
//! ## Ownership boundary
//!
//! - `HirLoweringResult` is the typed result boundary returned by HIR lowering. Production
//!   orchestration consumes its named fields and assembles the non-HIR metadata into
//!   `ModuleCompilerMetadata` on the current `Module` payload.
//! - `HirLoweringMetadata` carries only the extracted non-HIR metadata. Successful-module
//!   warnings are not duplicated here: the frontend orchestration `warnings` vector already
//!   merges preparation and AST warnings and remains the single successful-module warning
//!   source. HIR lowering keeps the AST warnings privately in `HirBuilder` for error-context
//!   rendering only.
//! - `ModuleDocFragment` replaces the former `HirDocFragment`. Resolved documentation metadata is
//!   not HIR and uses a non-HIR name and owner.
//!   Documentation metadata carries only optional exact spans; generated fragments remain spanless.

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::source::SourceSpan;

// -------------------------
//  Resolved documentation fragments
// -------------------------

/// Kind of resolved documentation fragment.
///
/// WHY: preserved so the metadata payload carries the full fragment shape. Currently only `Doc`
/// is produced; the enum remains explicit so future fragment kinds stay a single-owner decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleDocFragmentKind {
    Doc,
}

/// WHAT: carries the fully resolved documentation text and its authored source span, when one
///       exists. Generated documentation remains spanless rather than inventing provenance.
/// WHY: builders and documentation tooling consume resolved doc metadata after HIR lowering. This
///      is compiler metadata, not executable HIR state.
#[derive(Debug, Clone)]
pub struct ModuleDocFragment {
    #[allow(dead_code)] // Retained for deferred documentation-metadata consumers.
    pub kind: ModuleDocFragmentKind,
    /// The resolved documentation text.
    ///
    /// WHY: preserved for builder/documentation-metadata consumers. Currently read only in tests;
    /// retained so the struct carries the full fragment shape.
    #[allow(dead_code)] // Retained for deferred documentation-metadata consumers.
    pub rendered_text: String,
    #[allow(dead_code)] // Retained for deferred documentation source diagnostics.
    pub span: Option<SourceSpan>,
}

// -------------------------
//  HIR lowering metadata result boundary
// -------------------------

/// Non-HIR compiler metadata extracted by HIR lowering.
///
/// WHAT: bundles resolved documentation fragments that HIR lowering pulls from the AST but must
/// not store on `HirModule`.
/// WHY: HIR owns executable/semantic IR only. This typed metadata lets the build system assemble
/// the module compiler-metadata lane without HIR temporarily carrying non-HIR state.
///      Successful-module warnings are intentionally not duplicated here.
#[derive(Debug, Clone, Default)]
pub struct HirLoweringMetadata {
    pub doc_fragments: Vec<ModuleDocFragment>,
}

/// Typed HIR lowering result boundary.
///
/// WHAT: bundles the validated `HirModule`, its frontend `TypeEnvironment`, and the extracted
///       non-HIR compiler metadata produced by lowering.
/// WHY: production orchestration consumes these as named fields rather than a positional tuple,
///      keeping the HIR/metadata boundary explicit at the frontend→build-system handoff.
pub struct HirLoweringResult {
    pub hir_module: HirModule,
    pub type_environment: TypeEnvironment,
    pub metadata: HirLoweringMetadata,
}
