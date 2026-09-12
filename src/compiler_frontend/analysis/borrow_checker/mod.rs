//! Borrow validation driver and side-table fact production.
//!
//! WHAT: orchestrates borrow checking for a complete immutable HIR module by building metadata,
//! running the fixed-point engine, applying transfer rules, and returning side-table facts.
//! WHY: borrow validation enforces source-language exclusivity before backend lowering while
//! keeping ownership/drop information separate from HIR node shapes.
//!
//! This module must not mutate HIR, perform backend ownership lowering, or use diagnostics as
//! analysis state. External-call access policy belongs in the metadata/transfer owners below.
//!
//! The `problem` and `last_use` seams compile only for the Boracle lane and its tests:
//! they are not part of the default compiler path. `boracle` is feature-gated and currently
//! isolated from the alpha path; the shared seams remain available to that lane and their tests.
//! The normal checker owns the metadata/transfer path below.

#[cfg(feature = "boracle")]
mod boracle;
mod diagnostics;
mod engine;
mod error;
#[cfg(any(feature = "boracle", test))]
mod last_use;
mod metadata;
#[cfg(any(feature = "boracle", test))]
mod problem;
mod state;
mod transfer;
mod types;

pub(crate) use error::BorrowCheckError;
pub(crate) use types::{BorrowAnalysis, BorrowCheckReport, BorrowDropSiteKind, LocalMode};

#[cfg(test)]
pub(crate) use types::{
    BorrowDropSite, BorrowStateSnapshot, LocalBorrowSnapshot, OptionalTransferStatus,
    ReactiveInvalidationFact, ReactiveInvalidationKind,
};
pub(crate) type BorrowFacts = BorrowAnalysis;

// WHY: These optional re-exports expose the Boracle service to project tooling and focused tests.
// The feature-lane rows are consumed by the project command when enabled, while the test-only
// helper is gated separately. Keep allowances local to this optional boundary.
#[cfg(all(feature = "boracle", test))]
#[allow(unused_imports)]
pub(crate) use boracle::solve_hir_module;
#[cfg(feature = "boracle")]
#[allow(unused_imports)]
pub(crate) use boracle::{
    BoracleDump, BoracleExperiment, BoracleExperimentMetadata, BoracleModuleReport,
    BoracleReferencePromotionStatus, BoracleReferenceRuleSet, BoracleRuleSelection,
    BoracleServiceOptions, OriginOverlapDecision, run_hir_module,
};
#[cfg(feature = "boracle")]
#[allow(unused_imports)]
pub(crate) use problem::{
    AccessKind, CallResultProvenance, CallResultUnknownReason, EventKind, OriginKind,
};

use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(in crate::compiler_frontend) fn check_borrows(
    module: &HirModule,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    BorrowChecker::new(module, external_package_registry, path_fork, string_table).run()
}

#[cfg(test)]
mod tests;
