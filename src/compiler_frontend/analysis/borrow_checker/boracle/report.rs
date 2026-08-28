//! Structured Boracle result assembly.
//!
//! WHAT: joins independent provenance, last-use and loan analyses into one deterministic report.
//! WHY: interactive dumps and future differential checks need typed facts rather than diagnostic
//! prose or a second semantic renderer.

// Some report queries remain research-facing rather than appearing in every current dump. Keep
// the typed result boundary warning-free as those queries evolve.
#![allow(dead_code)]

use super::super::last_use::{
    FutureUseStatus, LastUseAnalysis, LastUseLocation, LastUseObservation, LastUseResult,
    LastUseSubject, event_for_use,
};
use super::super::problem::{BorrowProblem, EventId, EventKind, PlaceId, PointId, ValueOriginId};
use super::service::BoracleRuleSelection;
use super::{LoanSolution, OriginSolution};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// One reactive observation retained as metadata, never as an active loan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReactiveObservation {
    pub(crate) event: EventId,
    pub(crate) place: PlaceId,
}

/// One complete Boracle solver report, including the typed rule selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoracleReport {
    pub(crate) rule_selection: BoracleRuleSelection,
    pub(crate) origin: OriginSolution,
    pub(crate) last_use: Box<[LastUseResult]>,
    pub(crate) origin_last_use: Box<[LastUseResult]>,
    pub(crate) loan_last_use: Box<[LastUseResult]>,
    pub(crate) origin_last_use_after_event: Box<[LastUseResult]>,
    pub(crate) loan_last_use_after_event: Box<[LastUseResult]>,
    pub(crate) loans: LoanSolution,
    pub(crate) reactive_observations: Box<[ReactiveObservation]>,
    pub(crate) reactive_transfer_blocked_places: Box<[PlaceId]>,
}

impl BoracleReport {
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
                &self.origin_last_use_after_event,
                &self.loan_last_use_after_event,
            ),
            self.loans.conflicts()
        )
    }

    pub(crate) fn has_conflicts(&self) -> bool {
        !self.loans.conflicts().is_empty()
    }

    pub(crate) fn final_use_candidate_for_place(&self, place: PlaceId) -> bool {
        !self.reactive_transfer_blocked_places.contains(&place)
    }

    /// Return whether a place has a proven final-use candidate at one normalized point.
    ///
    /// This is a last-use proof, not the complete optional-transfer contract. A later caller must
    /// also combine it with call-boundary transfer metadata, provenance and ownership rules.
    pub(crate) fn final_use_candidate_at(&self, place: PlaceId, point: PointId) -> bool {
        if !self.final_use_candidate_for_place(place) {
            return false;
        }
        self.last_use.iter().any(|result| {
            result.subject == LastUseSubject::Place(place)
                && result.location.point == point
                && result.status == FutureUseStatus::NoFutureUse
        })
    }

    /// Query a final-use candidate at the exact event that consumes an origin.
    pub(crate) fn final_use_candidate_for_origin_after_event(
        &self,
        origin: ValueOriginId,
        event: EventId,
        point: PointId,
    ) -> bool {
        if self.reactive_observations.iter().any(|observation| {
            self.origin
                .origins_after_event(observation.event, observation.place)
                .is_some_and(|origins| origins.contains(&origin))
        }) {
            return false;
        }
        self.origin_last_use_after_event.iter().any(|result| {
            result.subject == LastUseSubject::Origin(origin)
                && result.location == LastUseLocation::after_event(event, point)
                && result.status == FutureUseStatus::NoFutureUse
        })
    }
}

/// One compiler-owned entry point for the reference analyses.
pub(crate) struct BoracleSolver;

impl BoracleSolver {
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

        let mut origin_last_use_after_event = Vec::new();
        for event in problem.events() {
            for origin_id in problem.origins().iter().map(|origin| origin.id) {
                origin_last_use_after_event.push(last_use_analysis.query(
                    LastUseSubject::Origin(origin_id),
                    LastUseLocation::after_event(event.id, event.point),
                )?);
            }
        }
        origin_last_use_after_event.sort_by_key(|result| {
            (
                result.location.after_event.map(EventId::raw),
                match result.subject {
                    LastUseSubject::Origin(origin) => origin.raw(),
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
        let mut reactive_observations = Vec::new();
        for event in problem.events() {
            if let EventKind::ReactiveObserve { place } = &event.kind {
                reactive_observations.push(ReactiveObservation {
                    event: event.id,
                    place: *place,
                });
            }
        }
        reactive_observations
            .sort_by_key(|observation| (observation.event.raw(), observation.place.raw()));
        let mut reactive_transfer_blocked_places = reactive_observations
            .iter()
            .map(|observation| observation.place)
            .collect::<Vec<_>>();
        reactive_transfer_blocked_places.sort_by_key(|place| place.raw());
        reactive_transfer_blocked_places.dedup();

        Ok(BoracleReport {
            rule_selection,
            origin,
            last_use: last_use.into_boxed_slice(),
            origin_last_use: origin_last_use.into_boxed_slice(),
            loan_last_use: loan_last_use.into_boxed_slice(),
            origin_last_use_after_event: origin_last_use_after_event.into_boxed_slice(),
            loan_last_use_after_event: loan_last_use_after_event.into_boxed_slice(),
            loans,
            reactive_observations: reactive_observations.into_boxed_slice(),
            reactive_transfer_blocked_places: reactive_transfer_blocked_places.into_boxed_slice(),
        })
    }
}
