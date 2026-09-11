//! Structured Boracle result assembly.
//!
//! WHAT: joins independent provenance, last-use and loan analyses into one deterministic report.
//! WHY: interactive dumps and future differential checks need typed facts rather than diagnostic
//! prose or a second semantic renderer.

use super::super::last_use::{
    LastUseAnalysis, LastUseLocation, LastUseObservation, LastUseResult, LastUseSubject,
    event_for_use,
};
use super::super::problem::{BorrowProblem, EventId};
use super::service::BoracleRuleSelection;
use super::{LoanSolution, OriginSolution};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// One complete Boracle solver report, including the typed rule selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoracleReport {
    pub(crate) rule_selection: BoracleRuleSelection,
    pub(crate) origin: OriginSolution,
    pub(crate) last_use: Box<[LastUseResult]>,
    pub(crate) origin_last_use: Box<[LastUseResult]>,
    pub(crate) loan_last_use: Box<[LastUseResult]>,
    pub(crate) loan_last_use_after_event: Box<[LastUseResult]>,
    pub(crate) loans: LoanSolution,
}

impl BoracleReport {
    #[cfg(test)]
    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }

    pub(crate) fn last_use_debug_dump(&self) -> String {
        format!(
            "places:\n{:#?}\norigins:\n{:#?}\nloans:\n{:#?}",
            self.last_use, self.origin_last_use, self.loan_last_use
        )
    }

    pub(crate) fn conflicts_debug_dump(&self) -> String {
        format!("{:#?}", self.loans.conflicts())
    }

    pub(crate) fn witnesses_debug_dump(&self) -> String {
        format!(
            "last-use witnesses:\n{:#?}\nconflict witnesses:\n{:#?}",
            (
                &self.last_use,
                &self.origin_last_use,
                &self.loan_last_use,
                &self.loan_last_use_after_event,
            ),
            self.loans.conflicts()
        )
    }

    pub(crate) fn has_conflicts(&self) -> bool {
        !self.loans.conflicts().is_empty()
    }
}

/// One compiler-owned entry point for the reference analyses.
pub(crate) struct BoracleSolver;

impl BoracleSolver {
    #[cfg(test)]
    pub(crate) fn solve(problem: &BorrowProblem) -> Result<BoracleReport, CompilerError> {
        Self::solve_with_rule_selection(problem, BoracleRuleSelection::default())
    }

    pub(crate) fn solve_with_rule_selection(
        problem: &BorrowProblem,
        rule_selection: BoracleRuleSelection,
    ) -> Result<BoracleReport, CompilerError> {
        rule_selection
            .validate()
            .map_err(CompilerError::compiler_error)?;

        let origin = super::OriginSolver::solve(problem)?;
        let exclusive_liveness = if rule_selection
            .experiments
            .contains(&super::BoracleExperiment::DeadExclusiveLoan)
        {
            super::ExclusiveLoanLiveness::UseDriven
        } else {
            super::ExclusiveLoanLiveness::Conservative
        };
        let loans = super::LoanSolver::solve_with_liveness(problem, &origin, exclusive_liveness)?;

        let mut last_use_observations = Vec::new();
        for use_row in problem.uses() {
            if use_row.definition && !origin.is_write_through_use(use_row.id) {
                continue;
            }
            let event_id = event_for_use(problem, use_row.id)?;
            let event = problem.events().get(event_id.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Boracle report cannot locate use event {:?}",
                    event_id
                ))
            })?;
            let location = LastUseLocation::after_event(event_id, event.point);
            let origins = origin.origins_for_place_after_event(problem, event_id, use_row.place);
            if !origins.is_empty() {
                for origin_id in &origins {
                    last_use_observations.push(LastUseObservation {
                        subject: LastUseSubject::Origin(*origin_id),
                        location,
                        use_id: use_row.id,
                    });
                }
            }
        }
        for loan in loans.loans() {
            for use_id in &loan.uses {
                let event_id = event_for_use(problem, *use_id)?;
                let event = problem.events().get(event_id.index()).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Boracle report cannot locate loan-use event {:?}",
                        event_id
                    ))
                })?;
                last_use_observations.push(LastUseObservation {
                    subject: LastUseSubject::Loan(loan.id),
                    location: LastUseLocation::after_event(event_id, event.point),
                    use_id: *use_id,
                });
            }
        }
        let last_use_analysis = LastUseAnalysis::from_problem(problem)?
            .with_observations(problem, last_use_observations)?;
        let mut last_use = Vec::new();
        for point in problem.points() {
            for place in problem.places() {
                let result = last_use_analysis.query(
                    LastUseSubject::Place(place.id),
                    LastUseLocation::at_point(point.id),
                )?;
                last_use.push(result);
            }
        }
        last_use.sort_by_key(|result| {
            (
                result.location.point.raw(),
                match result.subject {
                    LastUseSubject::Place(place) => place.raw(),
                    _ => u32::MAX,
                },
            )
        });

        let mut origin_last_use = Vec::new();
        for point in problem.points() {
            for origin_id in problem.origins().iter().map(|origin| origin.id) {
                origin_last_use.push(last_use_analysis.query(
                    LastUseSubject::Origin(origin_id),
                    LastUseLocation::at_point(point.id),
                )?);
            }
        }
        origin_last_use.sort_by_key(|result| {
            (
                result.location.point.raw(),
                match result.subject {
                    LastUseSubject::Origin(origin) => origin.raw(),
                    _ => u32::MAX,
                },
            )
        });

        let mut loan_last_use = Vec::new();
        for point in problem.points() {
            for loan in loans.loans() {
                loan_last_use.push(last_use_analysis.query(
                    LastUseSubject::Loan(loan.id),
                    LastUseLocation::at_point(point.id),
                )?);
            }
        }
        loan_last_use.sort_by_key(|result| {
            (
                result.location.point.raw(),
                match result.subject {
                    LastUseSubject::Loan(loan) => loan.raw(),
                    _ => u32::MAX,
                },
            )
        });

        let mut loan_last_use_after_event = Vec::new();
        for event in problem.events() {
            for loan in loans.loans() {
                loan_last_use_after_event.push(last_use_analysis.query(
                    LastUseSubject::Loan(loan.id),
                    LastUseLocation::after_event(event.id, event.point),
                )?);
            }
        }
        loan_last_use_after_event.sort_by_key(|result| {
            (
                result.location.after_event.map(EventId::raw),
                match result.subject {
                    LastUseSubject::Loan(loan) => loan.raw(),
                    _ => u32::MAX,
                },
            )
        });

        Ok(BoracleReport {
            rule_selection,
            origin,
            last_use: last_use.into_boxed_slice(),
            origin_last_use: origin_last_use.into_boxed_slice(),
            loan_last_use: loan_last_use.into_boxed_slice(),
            loan_last_use_after_event: loan_last_use_after_event.into_boxed_slice(),
            loans,
        })
    }
}
