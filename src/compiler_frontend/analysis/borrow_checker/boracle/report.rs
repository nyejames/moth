//! Structured Boracle result assembly.
//!
//! WHAT: joins independent provenance, last-use and loan analyses into one deterministic report.
//! WHY: interactive dumps and future differential checks need typed facts rather than diagnostic
//! prose or a second semantic renderer.

// Some report queries remain research-facing rather than appearing in every current dump. Keep
// the typed result boundary warning-free as those queries evolve.
#![allow(dead_code)]

use super::super::last_use::{
    FutureUseStatus, LastUseAnalysis, LastUseLocation, LastUseResult, LastUseSubject,
};
use super::super::problem::{BorrowProblem, EventId, EventKind, PlaceId, PointId};
use super::{LoanSolution, OriginSolution};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// One reactive observation retained as metadata, never as an active loan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReactiveObservation {
    pub(crate) event: EventId,
    pub(crate) place: PlaceId,
}

/// One complete reference-solver report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoracleReport {
    pub(crate) rule_set: &'static str,
    pub(crate) origin: OriginSolution,
    pub(crate) last_use: Box<[LastUseResult]>,
    pub(crate) loans: LoanSolution,
    pub(crate) reactive_observations: Box<[ReactiveObservation]>,
    pub(crate) blocked_optional_transfer_places: Box<[PlaceId]>,
}

impl BoracleReport {
    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }

    pub(crate) fn last_use_debug_dump(&self) -> String {
        format!("{:#?}", self.last_use)
    }

    pub(crate) fn conflicts_debug_dump(&self) -> String {
        format!("{:#?}", self.loans.conflicts())
    }

    pub(crate) fn witnesses_debug_dump(&self) -> String {
        format!(
            "last-use witnesses:\n{:#?}\nconflict witnesses:\n{:#?}",
            self.last_use,
            self.loans.conflicts()
        )
    }

    pub(crate) fn has_conflicts(&self) -> bool {
        !self.loans.conflicts().is_empty()
    }

    pub(crate) fn optional_transfer_allowed(&self, place: PlaceId) -> bool {
        !self.blocked_optional_transfer_places.contains(&place)
    }

    /// Return the conservative optional-transfer decision at one normalized point.
    ///
    /// A transfer is only advised when the point is a proven final observation and the place is
    /// not a stable reactive root. Any missing or path-dependent future-use row falls back to
    /// borrowing, matching the reference rule without assigning a lifetime owner.
    pub(crate) fn optional_transfer_allowed_at(&self, place: PlaceId, point: PointId) -> bool {
        if !self.optional_transfer_allowed(place) {
            return false;
        }
        self.last_use.iter().any(|result| {
            result.subject == LastUseSubject::Place(place)
                && result.location.point == point
                && result.status == FutureUseStatus::NoFutureUse
        })
    }
}

/// One compiler-owned entry point for the reference analyses.
pub(crate) struct BoracleSolver;

impl BoracleSolver {
    pub(crate) fn solve(problem: &BorrowProblem) -> Result<BoracleReport, CompilerError> {
        let origin = super::OriginSolver::solve(problem)?;
        let last_use_analysis = LastUseAnalysis::from_problem(problem)?;
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

        let loans = super::LoanSolver::solve(problem)?;
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
        let mut blocked_optional_transfer_places = reactive_observations
            .iter()
            .map(|observation| observation.place)
            .collect::<Vec<_>>();
        blocked_optional_transfer_places.sort_by_key(|place| place.raw());
        blocked_optional_transfer_places.dedup();

        Ok(BoracleReport {
            rule_set: "boracle-reference-v1",
            origin,
            last_use: last_use.into_boxed_slice(),
            loans,
            reactive_observations: reactive_observations.into_boxed_slice(),
            blocked_optional_transfer_places: blocked_optional_transfer_places.into_boxed_slice(),
        })
    }
}
