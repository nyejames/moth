//! Compiler-owned generated-function completion for one module compilation.
//!
//! WHAT: everything between "AST emitted a concrete generic request" and "this module transaction
//!       has a complete generated delta".
//! WHY:  generated functions are the sharpest compiler/build seam. Boundary-wide aggregation,
//!       deduplication, storage and publication belong to Stage 0. Canonicalising a request,
//!       materialising its AST, lowering and validating its HIR, borrow-checking it and converging
//!       call summaries to a local fixed point are compiler semantics, and they all live here.
//!
//! # What this module owns
//! - [`artefacts`]: sidecars, completed records and the per-transaction delta
//! - [`known`]: the immutable view of already published generated work a module may reuse
//! - [`transaction`]: the per-module request state machine and its fixed point
//! - [`requests`]: canonicalising a concrete request from requester AST facts
//! - [`provider_materialisations`]: resolving the declaring template for one request
//! - [`materialisation`]: materialising, lowering, borrow-checking and completing one request
//! - [`convergence`]: the HIR-derived call model and monotone summary propagation
//!
//! # What this module does NOT own
//! - The boundary generated store, duplicate prevention and transactional publication, which stay
//!   under `build_system/create_project_modules`
//! - Generic template validation and AST substitution, which stay in `ast::generic_functions`

pub(crate) mod artefacts;
pub(crate) mod convergence;
pub(crate) mod known;
pub(crate) mod materialisation;
pub(crate) mod provider_materialisations;
pub(crate) mod requests;
pub(crate) mod transaction;

pub(crate) use artefacts::{
    CompletedGeneratedFunction, GeneratedFunctionDelta, GeneratedFunctionId,
    GeneratedFunctionSidecar, validate_completed_generated_record,
};
pub(crate) use known::KnownGeneratedFunctions;
pub(crate) use provider_materialisations::ProviderMaterialisationRegistry;

#[cfg(test)]
#[path = "tests/fixtures.rs"]
pub(crate) mod test_fixtures;
