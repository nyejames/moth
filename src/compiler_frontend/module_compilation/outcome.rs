//! Typed result of one module's local semantic compilation.
//!
//! WHAT: the success/diagnosed classification the compiler returns for one module job and the
//!       success payload the build boundary merges and publishes.
//! WHY:  a diagnosed source module and an internal compiler failure are different result classes.
//!       Classifying them once here means graph and render consumers never re-classify a mixed
//!       message bag.

use crate::compiler_frontend::compiler_messages::ModuleDiagnostics;
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::module_compilation::generated::GeneratedFunctionDelta;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Typed result of one retained module's semantic compilation.
///
/// `Success` carries the complete unmerged semantic result plus its local string-table delta.
/// `Diagnosed` carries the user-facing diagnostics the renderer surfaces. An infrastructure
/// failure is `Err(CompilerError)` at the call boundary instead.
pub(crate) enum ModuleCompilationOutcome {
    // `ModuleSemanticResult` carries the full unmerged module (HIR, type environment and borrow
    // facts) and is far larger than `ModuleDiagnostics`, so the success payload is boxed to keep
    // the boundary outcome small. The box is transient: the caller unboxes once before merging.
    Success(Box<ModuleSemanticResult>),
    Diagnosed(ModuleDiagnostics),
}

/// Everything one successful module compilation produced, before boundary publication.
///
/// WHAT: the validated base module lanes, the generated delta completed in the same transaction,
///       the closed public interface and the module-local string table carrying every diagnostic
///       render identity.
/// WHY: publication is atomic. Keeping the artefact, its generated delta and its string-table
///      state in one value means the build boundary merges string identities once and commits the
///      whole transaction or none of it. The stable module origin travels through
///      `public_interface`; no dense `ModuleId` crosses this boundary, because standalone
///      compilation has no graph-assigned identity.
pub(crate) struct ModuleSemanticResult {
    /// Validated base HIR, paired type environment, borrow facts, link facts and metadata.
    pub(crate) module: Module,
    /// New generated identities, summaries and sidecars completed for this module.
    pub(crate) generated_delta: GeneratedFunctionDelta,
    /// The module-local string table carrying every diagnostic render identity produced during
    /// semantic compilation. Merged into the build table once per module so downstream consumers
    /// see a single remapped table.
    pub(crate) string_table: StringTable,
    /// The closed and publication-validated semantic interface. Provider-owned re-export facts
    /// have already joined through immutable completed interfaces, so the graph can publish this
    /// value directly after the deterministic string-table merge.
    pub(crate) public_interface: PublicSemanticInterface,
}
