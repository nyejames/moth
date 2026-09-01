//! Boracle feature lane entry for the internal reference solver.
//!
//! WHAT: feature-gated origin flow, typed provenance relations, loan liveness, last-use
//!       queries and deterministic reports over a validated `BorrowProblem`.
//! WHY:  the reference lane must stay inspectable and isolated from the shipped alpha checker.
//!
//! Flow:
//! `problem` builder -> `origins` (flow + typed relation construction) -> `relations`
//! (typed vocabulary and the origin-overlap owner) -> `loans` -> `report` / `service`.
//!
//! The `oracle` submodule is a separate bounded operational semantics, not another reference-solver stage.
//!
//! This module does not run during normal compilation, does not rewrite HIR, and does not
//! decide lifetime topology, retained edges or physical memory strategy.
//!
//! Files:
//! - `origins.rs`: origin flow and typed `OriginRelations` construction
//! - `relations.rs`: typed relation rows, overlap/disjoint/unknown evidence and validation
//! - `loans.rs`: loan derivation and origin-aware conflicts
//! - `oracle/`: bounded operational execution and replayable conflict traces
//! - `differential.rs`: classified comparison of the reference rule set against the experiments
//! - `reducer.rs`: deterministic disagreement-preserving problem reduction
//! - `report.rs` / `service.rs`: solver reports, dumps and the source-service boundary

mod differential;
mod loans;
mod oracle;
mod origins;
mod reducer;
mod relations;
mod report;
mod service;

#[allow(unused_imports)]
pub(crate) use differential::{
    OracleComparison, OracleComparisonClass, OracleComparisonSet, OracleComparisonSeverity,
    compare_problem_parts, compare_reference_and_experiments,
};

#[allow(unused_imports)]
pub(crate) use loans::{
    AccessDecision, ConflictWitness, ExclusiveLoanLiveness, LoanFact, LoanSolution, LoanSolver,
};
#[allow(unused_imports)]
pub(crate) use oracle::{
    ExecutionTrace, OracleBounds, OracleLimitReason, OracleOutcome, execute_bounded,
};
#[allow(unused_imports)]
pub(crate) use origins::{OriginFact, OriginSolution, OriginSolver, OriginTrace, OriginTraceRule};
#[allow(unused_imports)]
pub(crate) use reducer::{
    ReducedProblem, ReductionPass, ReductionSize, reduce_problem, reduction_size,
    render_fixture_skeleton,
};
#[allow(unused_imports)]
pub(crate) use relations::{
    CopyGraphId, DisjointReason, OriginDisjointEvidence, OriginOverlapDecision,
    OriginOverlapEvidence, OriginRegistration, OriginRelation, OriginRelationEvidence,
    OriginRelationKind, OriginRelations, OriginUnknownEvidence, PrecisionLossReason,
};
#[allow(unused_imports)]
pub(crate) use report::{BoracleReport, BoracleSolver, ReactiveObservation};
#[cfg(test)]
pub(crate) use service::solve_hir_module;
#[allow(unused_imports)]
pub(crate) use service::{
    BoracleDump, BoracleExperiment, BoracleExperimentMetadata, BoracleFunctionReport,
    BoracleModuleReport, BoracleReferencePromotionStatus, BoracleReferenceRuleSet,
    BoracleRuleSelection, BoracleServiceOptions, run_hir_module,
};

#[cfg(test)]
mod tests;
