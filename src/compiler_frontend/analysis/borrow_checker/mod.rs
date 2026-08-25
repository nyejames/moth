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
//! This also owns shared future seams:
//! - `problem`: shared borrow-problem vocabulary for future boracle-style analyses
//! - `last_use`: shared last-use analysis vocabulary for future boracle-style analyses
//! - `boracle`: feature-gated Boracle lane entrypoint, currently isolated from the alpha path

#[cfg(feature = "boracle")]
mod boracle;
mod diagnostics;
mod engine;
mod error;
mod last_use;
mod metadata;
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

#[cfg(feature = "boracle")]
#[allow(unused_imports)]
pub(crate) use boracle::{
    BoracleDump, BoracleExperiment, BoracleModuleReport, BoracleServiceOptions, run_hir_module,
    solve_hir_module,
};
#[cfg(feature = "boracle")]
#[allow(unused_imports)]
pub(crate) use problem::{AccessKind, CallResultProvenance, EventKind, OriginKind};

use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(in crate::compiler_frontend) fn check_borrows(
    module: &HirModule,
    external_package_registry: &ExternalPackageRegistry,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    BorrowChecker::new(module, external_package_registry, string_table).run()
}

#[cfg(test)]
mod tests;
