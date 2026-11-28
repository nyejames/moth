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
pub(crate) use types::{BorrowAnalysis, BorrowCheckReport, BorrowDropSiteKind};

#[cfg(test)]
pub(crate) use types::{BorrowDropSite, LocalMode, OptionalTransferStatus};
pub(crate) type BorrowFacts = BorrowAnalysis;

// WHY: These optional re-exports expose the Boracle service to project tooling and focused tests.
// The feature-lane rows are consumed by the project command when enabled, while the test-only
// helper is gated separately. Keep allowances local to this optional boundary.
#[cfg(all(feature = "boracle", test))]
#[allow(unused_imports)]
pub(crate) use boracle::solve_hir_module;
#[cfg(feature = "boracle")]
pub use boracle::{BoracleDump, BoracleExperiment, BoracleReferenceRuleSet, BoracleRuleSelection};
#[cfg(feature = "boracle")]
#[allow(unused_imports)]
pub(crate) use boracle::{
    BoracleExperimentMetadata, BoracleModuleReport, BoracleReferencePromotionStatus,
    BoracleServiceOptions, OriginOverlapDecision, run_hir_module,
};
#[cfg(feature = "boracle")]
#[allow(unused_imports)]
pub(crate) use problem::{
    AccessKind, CallResultProvenance, CallResultUnknownReason, EventKind, OriginKind,
};
#[cfg(all(test, feature = "boracle"))]
pub(crate) use problem::{BindingDestination, RebindValue, TerminatorEventKind, UseKind};

use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
#[cfg(test)]
use crate::compiler_frontend::ast::generic_functions::with_registered_type_projection;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};

/// Validates against the converged contracts, computing local contracts for standalone callers.
pub(in crate::compiler_frontend) fn check_borrows(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    converged: Option<(
        FxHashMap<FunctionId, PublicCallSummary>,
        &CanonicalTypeProjectionContext<'_>,
    )>,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    let checker = BorrowChecker::new(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
    );
    if let Some((summaries, projection)) = converged {
        checker
            .with_type_projection(projection)
            .run(Some(summaries))
    } else {
        checker.run(None)
    }
}

/// Cold allocation extraction used to check the production refresh against final HIR.
#[cfg(test)]
pub(in crate::compiler_frontend) fn summarize_call_allocations(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    with_registered_type_projection(type_environment, external_package_registry, |projection| {
        summarize_call_allocations_with_projection(
            module,
            type_environment,
            external_package_registry,
            path_fork,
            string_table,
            projection,
        )
    })
}

#[cfg(test)]
pub(in crate::compiler_frontend) fn summarize_call_allocations_with_projection(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    projection: &CanonicalTypeProjectionContext<'_>,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    let mut checker = BorrowChecker::new(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
    )
    .with_type_projection(projection);
    checker.build_public_call_summaries()?;
    let mut report = BorrowCheckReport::default();
    report.analysis.public_call_summaries = checker.public_call_summaries;
    Ok(report)
}

#[cfg(test)]
pub(in crate::compiler_frontend) fn check_borrows_with_projection_for_test(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    projection: &CanonicalTypeProjectionContext<'_>,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    BorrowChecker::new(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
    )
    .with_type_projection(projection)
    .run(None)
}

/// Create declaration contracts for failure inference. Extracting from provisional bodies
/// would produce allocation facts that nobody reads before private failure lane installation.
pub(in crate::compiler_frontend) fn seed_call_summaries(
    module: &HirModule,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    let mut report = BorrowCheckReport::default();
    report.analysis.public_call_summaries = BorrowChecker::seed_public_call_summaries(
        module,
        &diagnostics::BorrowDiagnostics::new(module, path_fork, string_table),
    )?;
    report.metadata = metadata::BorrowMetadataCache::bootstrap(module);
    Ok(report)
}

#[allow(
    dead_code,
    reason = "B2b.3b runs the exact queue after applying the installer receipt"
)]
pub(in crate::compiler_frontend) fn stabilize_call_metadata(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    projection: &CanonicalTypeProjectionContext<'_>,
    report: &mut BorrowCheckReport,
) -> Result<(), BorrowCheckError> {
    if !report.metadata.has_work() {
        return Ok(());
    }
    let mut checker = BorrowChecker::new(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
    )
    .with_type_projection(projection);
    checker.public_call_summaries = std::mem::take(&mut report.analysis.public_call_summaries);
    let result = report.metadata.solve(&mut checker);
    report.analysis.public_call_summaries = checker.public_call_summaries;
    result
}

pub(in crate::compiler_frontend) fn refresh_call_allocations(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    inputs: (
        &HirModuleLinkFacts,
        &FxHashSet<FunctionId>,
        &CanonicalTypeProjectionContext<'_>,
    ),
    report: &mut BorrowCheckReport,
) -> Result<(), BorrowCheckError> {
    let mut checker = BorrowChecker::new(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
    )
    .with_type_projection(inputs.2);
    checker.public_call_summaries = std::mem::take(&mut report.analysis.public_call_summaries);
    checker.refresh_return_alias_summaries(inputs.0, inputs.1)?;
    report.analysis.public_call_summaries = checker.public_call_summaries;
    Ok(())
}

/// Widen call mutation effects from already-validated reaches, without replaying transfer.
pub(in crate::compiler_frontend) fn refresh_call_mutation_effects(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    report: &mut BorrowCheckReport,
) -> Result<(), BorrowCheckError> {
    let mut checker = BorrowChecker::without_transfer_indices(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
    );
    checker.public_call_summaries = std::mem::take(&mut report.analysis.public_call_summaries);
    checker.finalize_public_call_summary_effects_to_fixed_point(report)?;
    report.analysis.public_call_summaries = checker.public_call_summaries;
    Ok(())
}

#[cfg(test)]
mod tests;
