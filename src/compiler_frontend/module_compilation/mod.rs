//! Compiler-owned module compilation boundary.
//!
//! WHAT: the one production service that compiles a ready module, plus the input, option, result
//!       and generated-delta values that service consumes and returns.
//! WHY:  the compiler owns local semantic compilation. Stage 0 decides which module is ready and
//!       what happens to the result; it never sequences interface binding, declaration ordering,
//!       AST semantics, HIR lowering or borrow validation itself.
//!
//! # What this module owns
//! - [`service`]: `compile_module`, the canonical local semantic sequence
//! - [`context`]: the provider interfaces, capability surface and options one module job reads
//! - [`options`]: the exact settings the frontend consumes, replacing any project config container
//! - [`prepared`]: the provider-independent prepared source payload one module job receives
//! - [`artefact`]: the executable, link-fact, compiler-metadata result lanes and the sealed artefact
//! - [`generated`]: generated request canonicalisation, materialisation, convergence and the delta
//! - [`outcome`]: the success/diagnosed classification and the unmerged success payload
//! - [`external_imports`]: provider and builder runtime import candidates for one module
//! - [`stages`]: warning-preserving wrappers over the HIR and borrow stage owners
//!
//! # What this module does NOT own
//! - Stage 0 discovery, scheduling, string-table merging and publication, which stay under
//!   `build_system/create_project_modules`
//! - Project aggregation, entry assembly and output records, which stay under `build_system/build`
//! - The stage implementations themselves, which stay with `headers`, `ast`, `hir`, `analysis` and
//!   `public_interface`

// Only `artefact` and `generated` are reachable by path: build-system and project tests construct
// artefact lanes and generated fixtures directly. Every other submodule is reached through the
// re-exports below, so this module map stays the one way in and `stages` cannot become a second
// raw-stage entry point.
pub(crate) mod artefact;
pub(crate) mod generated;

mod context;
mod external_imports;
mod options;
mod outcome;
mod prepared;
mod service;
mod stages;

pub(crate) use artefact::{CompiledModuleArtifact, ModuleExternalImport, ModuleRootActivity};
pub(crate) use artefact::{Module, ResolvedConstFragment};
pub(crate) use context::ModuleCompilationContext;
pub(crate) use generated::{
    CompletedGeneratedFunction, GeneratedFunctionDelta, GeneratedFunctionId,
    GeneratedFunctionSidecar, KnownGeneratedFunctions, ProviderMaterialisationRegistry,
    validate_completed_generated_record,
};
pub(crate) use options::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
pub(crate) use options::FrontendOptions;
pub(crate) use outcome::{ModuleCompilationOutcome, ModuleSemanticResult};
pub(crate) use prepared::PreparedModuleInput;
pub(crate) use service::compile_module;
