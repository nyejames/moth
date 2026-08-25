//! Boracle feature lane entry for the internal reference solver.
//!
//! This module is intentionally isolated behind the `boracle` cargo feature and does
//! not participate in the shipped alpha checker path.

mod loans;
mod origins;
mod report;
mod service;

#[allow(unused_imports)]
pub(crate) use loans::{AccessDecision, ConflictWitness, LoanFact, LoanSolution, LoanSolver};
#[allow(unused_imports)]
pub(crate) use origins::{OriginFact, OriginSolution, OriginSolver, OriginTrace, OriginTraceRule};
#[allow(unused_imports)]
pub(crate) use report::{BoracleReport, BoracleSolver, ReactiveObservation};
#[allow(unused_imports)]
pub(crate) use service::{
    BoracleDump, BoracleExperiment, BoracleFunctionReport, BoracleModuleReport,
    BoracleServiceOptions, run_hir_module, solve_hir_module,
};

#[cfg(test)]
mod tests;
