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
//! This module does not run during normal compilation, does not rewrite HIR, and does not
//! decide lifetime topology, retained edges or physical memory strategy.
//!
//! Files:
//! - `origins.rs`: origin flow and typed `OriginRelations` construction
//! - `relations.rs`: typed relation rows, overlap/disjoint/unknown evidence and validation
//! - `loans.rs`: loan derivation and origin-aware conflicts
//! - `report.rs` / `service.rs`: solver reports, dumps and the source-service boundary

mod loans;
mod origins;
mod relations;
mod report;
mod service;

#[allow(unused_imports)]
pub(crate) use loans::{
    AccessDecision, ConflictWitness, ExclusiveLoanLiveness, LoanFact, LoanSolution, LoanSolver,
};
#[allow(unused_imports)]
pub(crate) use origins::{OriginFact, OriginSolution, OriginSolver, OriginTrace, OriginTraceRule};
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
    BoracleDump, BoracleExperiment, BoracleFunctionReport, BoracleModuleReport,
    BoracleServiceOptions, run_hir_module,
};

#[cfg(test)]
mod tests;
