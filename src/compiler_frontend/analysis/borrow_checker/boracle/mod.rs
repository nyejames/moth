//! Boracle feature lane entry for future borrow-checker work.
//!
//! This module is intentionally isolated behind the `boracle` cargo feature and does
//! not participate in the shipped alpha checker path.

use super::problem::BorrowProblem;
use crate::compiler_frontend::compiler_errors::CompilerError;

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
pub(crate) use service::{BoracleDump, BoracleExperiment, BoracleServiceOptions, run_hir_module};

/// Validate and format a normalized problem at the Boracle orchestration boundary.
///
/// The real solver and source command will replace this consumer in later phases. Returning
/// the dump keeps this seam side-effect free while giving those callers one deterministic output.
#[allow(dead_code)]
pub(crate) fn dump_validated_problem(problem: &BorrowProblem) -> Result<String, CompilerError> {
    problem.validate()?;
    Ok(problem.debug_dump())
}

#[cfg(test)]
const BORACLE_FEATURE_MARKER: &str = "boracle";

#[cfg(test)]
mod tests;
