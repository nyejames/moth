//! Reusable future-use vocabulary and CFG reference analysis.
//!
//! WHAT: answers whether a normalized subject can be observed after a program location and keeps
//! one deterministic witness for that answer.
//! WHY: borrow validation, optional transfer and later lifetime work need the same conservative
//! future-use contract without sharing an implementation or rescanning HIR.
//!
//! This module consumes [`BorrowProblem`] only. It does not own alpha-checker state, source
//! syntax, borrow legality or lifetime topology. The alpha checker therefore remains on its
//! existing metadata path while Boracle and future consumers can use this independent owner.

// The alpha checker intentionally does not consume this independent vocabulary yet. The local
// allowance keeps the shared handoff warning-free until Boracle and later lifetime consumers use
// each part of the contract.
#![allow(dead_code)]

use crate::compiler_frontend::compiler_errors::CompilerError;

use super::problem::{
    BindingId, BlockId, BorrowProblem, EventId, EventKind, LoanId, PlaceId, PointId, UseId,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The conservative future-use classification shared by downstream analyses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FutureUseStatus {
    /// No reachable continuation observes the subject before an exit.
    NoFutureUse,
    /// At least one continuation observes the subject and at least one does not.
    MayBeUsed,
    /// Every continuation that can continue from the query observes the subject.
    MustBeUsed,
}

/// A normalized subject whose future observations can be queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LastUseSubject {
    Place(PlaceId),
    Binding(BindingId),
    Origin(super::problem::ValueOriginId),
    Loan(LoanId),
}

/// A precise query boundary. An event boundary distinguishes multiple ordered events sharing one
/// program point, such as call arguments and a call result write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LastUseLocation {
    pub(crate) point: PointId,
    pub(crate) after_event: Option<EventId>,
}

impl LastUseLocation {
    pub(crate) const fn at_point(point: PointId) -> Self {
        Self {
            point,
            after_event: None,
        }
    }

    pub(crate) const fn after_event(event: EventId, point: PointId) -> Self {
        Self {
            point,
            after_event: Some(event),
        }
    }
}

/// One source-semantic observation supplied to the analysis.
///
/// Origin observations are intentionally supplied by the origin solver rather than inferred from
/// binding names. Place, binding and loan observations can be derived directly from a validated
/// problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LastUseObservation {
    pub(crate) subject: LastUseSubject,
    pub(crate) location: LastUseLocation,
    pub(crate) use_id: UseId,
}

/// Structured evidence retained with one future-use answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LastUseWitness {
    NoFutureUse {
        explored_exits: Box<[BlockId]>,
    },
    MayBeUsed {
        later_use: UseId,
        no_use_exit: Option<BlockId>,
    },
    MustBeUsed {
        later_use: UseId,
    },
}

/// One deterministic future-use answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LastUseResult {
    pub(crate) subject: LastUseSubject,
    pub(crate) location: LastUseLocation,
    pub(crate) status: FutureUseStatus,
    pub(crate) witness: LastUseWitness,
}

/// Independent CFG future-use analysis over one immutable normalized problem.
#[derive(Debug, Clone)]
pub(crate) struct LastUseAnalysis {
    point_block: BTreeMap<PointId, BlockId>,
    events_by_block: BTreeMap<BlockId, Vec<EventId>>,
    event_location_by_id: BTreeMap<EventId, LastUseLocation>,
    observations_by_subject: BTreeMap<LastUseSubject, Vec<LastUseObservation>>,
    successors: BTreeMap<BlockId, Vec<BlockId>>,
    exits: BTreeSet<BlockId>,
}

impl LastUseAnalysis {
    /// Derive place, binding and loan observations from a validated problem.
    pub(crate) fn from_problem(problem: &BorrowProblem) -> Result<Self, CompilerError> {
        problem.validate()?;

        let mut analysis = Self::empty(problem);
        let mut event_by_use = BTreeMap::<UseId, EventId>::new();
        for event in problem.events() {
            match &event.kind {
                EventKind::Access { use_id } => {
                    event_by_use.insert(*use_id, event.id);
                }
                EventKind::CallEffect(effect) => {
                    for argument in &effect.arguments {
                        event_by_use.insert(argument.use_id, event.id);
                    }
                }
                _ => {}
            }
        }

        for use_row in problem.uses() {
            let Some(event_id) = event_by_use.get(&use_row.id).copied() else {
                return Err(CompilerError::compiler_error(format!(
                    "last-use analysis cannot locate owner of normalized use {:?}",
                    use_row.id
                )));
            };
            let location = *analysis
                .event_location_by_id
                .get(&event_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "last-use analysis cannot locate normalized event {:?}",
                        event_id
                    ))
                })?;

            analysis.add_observation(LastUseObservation {
                subject: LastUseSubject::Place(use_row.place),
                location,
                use_id: use_row.id,
            });

            let place = problem.places().get(use_row.place.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "last-use analysis cannot locate normalized place {:?}",
                    use_row.place
                ))
            })?;
            analysis.add_observation(LastUseObservation {
                subject: LastUseSubject::Binding(place.root),
                location,
                use_id: use_row.id,
            });
        }

        for loan in problem.loans() {
            for use_id in &loan.uses {
                let Some(event_id) = event_by_use.get(use_id).copied() else {
                    return Err(CompilerError::compiler_error(format!(
                        "last-use analysis cannot locate owner of loan use {:?}",
                        use_id
                    )));
                };
                let location = *analysis
                    .event_location_by_id
                    .get(&event_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "last-use analysis cannot locate loan-use event {:?}",
                            event_id
                        ))
                    })?;
                analysis.add_observation(LastUseObservation {
                    subject: LastUseSubject::Loan(loan.id),
                    location,
                    use_id: *use_id,
                });
            }
        }

        analysis.sort_observations();
        Ok(analysis)
    }

    /// Build an analysis from caller-owned observations, retaining normalized CFG and event
    /// ordering. This is the handoff used by origin solving for origin-specific future use.
    pub(crate) fn from_observations(
        problem: &BorrowProblem,
        observations: impl IntoIterator<Item = LastUseObservation>,
    ) -> Result<Self, CompilerError> {
        problem.validate()?;
        let mut analysis = Self::empty(problem);
        for observation in observations {
            analysis.validate_observation(problem, observation)?;
            analysis.add_observation(observation);
        }
        analysis.sort_observations();
        Ok(analysis)
    }

    /// Query one subject after a point or event boundary.
    pub(crate) fn query(
        &self,
        subject: LastUseSubject,
        location: LastUseLocation,
    ) -> Result<LastUseResult, CompilerError> {
        let block = self.block_for_point(location.point)?;
        let start_index = self.start_event_index(block, location)?;
        let local_observations = self
            .observations_by_subject
            .get(&subject)
            .map(|observations| {
                observations
                    .iter()
                    .filter(|observation| {
                        self.observation_after_in_block(observation, block, start_index)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if let Some(observation) = local_observations.first() {
            return Ok(LastUseResult {
                subject,
                location,
                status: FutureUseStatus::MustBeUsed,
                witness: LastUseWitness::MustBeUsed {
                    later_use: observation.use_id,
                },
            });
        }

        let summaries = self.block_summaries(subject, block, start_index);
        let status = match (summaries.may, summaries.must) {
            (false, _) => FutureUseStatus::NoFutureUse,
            (true, true) => FutureUseStatus::MustBeUsed,
            (true, false) => FutureUseStatus::MayBeUsed,
        };
        let witness = match status {
            FutureUseStatus::NoFutureUse => LastUseWitness::NoFutureUse {
                explored_exits: self.reachable_exits(block),
            },
            FutureUseStatus::MayBeUsed => LastUseWitness::MayBeUsed {
                later_use: summaries.later_use.ok_or_else(|| {
                    CompilerError::compiler_error("last-use MAY result has no later-use witness")
                })?,
                no_use_exit: summaries.no_use_exit,
            },
            FutureUseStatus::MustBeUsed => LastUseWitness::MustBeUsed {
                later_use: summaries.later_use.ok_or_else(|| {
                    CompilerError::compiler_error("last-use MUST result has no later-use witness")
                })?,
            },
        };

        Ok(LastUseResult {
            subject,
            location,
            status,
            witness,
        })
    }

    pub(crate) fn query_place(
        &self,
        place: PlaceId,
        location: LastUseLocation,
    ) -> Result<LastUseResult, CompilerError> {
        self.query(LastUseSubject::Place(place), location)
    }

    pub(crate) fn query_loan(
        &self,
        loan: LoanId,
        location: LastUseLocation,
    ) -> Result<LastUseResult, CompilerError> {
        self.query(LastUseSubject::Loan(loan), location)
    }

    fn empty(problem: &BorrowProblem) -> Self {
        let mut point_block = BTreeMap::new();
        for point in problem.points() {
            point_block.insert(point.id, point.block);
        }

        let mut events_by_block = BTreeMap::<BlockId, Vec<EventId>>::new();
        let mut event_location_by_id = BTreeMap::new();
        for block in &problem.control_flow().blocks {
            events_by_block.insert(block.id, block.events.to_vec());
            for event_id in &block.events {
                if let Some(event) = problem.events().get(event_id.index()) {
                    event_location_by_id.insert(
                        *event_id,
                        LastUseLocation::after_event(*event_id, event.point),
                    );
                }
            }
        }

        let mut successors = BTreeMap::<BlockId, Vec<BlockId>>::new();
        for block in &problem.control_flow().blocks {
            successors.insert(block.id, Vec::new());
        }
        for edge in &problem.control_flow().edges {
            successors.entry(edge.from).or_default().push(edge.to);
        }
        for targets in successors.values_mut() {
            targets.sort_by_key(|block| block.raw());
            targets.dedup();
        }

        Self {
            point_block,
            events_by_block,
            event_location_by_id,
            observations_by_subject: BTreeMap::new(),
            successors,
            exits: problem.control_flow().exits.iter().copied().collect(),
        }
    }

    fn validate_observation(
        &self,
        problem: &BorrowProblem,
        observation: LastUseObservation,
    ) -> Result<(), CompilerError> {
        let point = problem
            .points()
            .get(observation.location.point.index())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "last-use observation references unknown point {:?}",
                    observation.location.point
                ))
            })?;
        if let Some(event) = observation.location.after_event {
            let event_row = problem.events().get(event.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "last-use observation references unknown event {:?}",
                    event
                ))
            })?;
            if event_row.point != point.id {
                return Err(CompilerError::compiler_error(
                    "last-use observation event and point do not match",
                ));
            }
        }
        Ok(())
    }

    fn add_observation(&mut self, observation: LastUseObservation) {
        self.observations_by_subject
            .entry(observation.subject)
            .or_default()
            .push(observation);
    }

    fn sort_observations(&mut self) {
        for observations in self.observations_by_subject.values_mut() {
            observations.sort_by_key(|observation| {
                (
                    observation.location.point.raw(),
                    observation.location.after_event.map(EventId::raw),
                    observation.use_id.raw(),
                )
            });
        }
    }

    fn block_for_point(&self, point: PointId) -> Result<BlockId, CompilerError> {
        self.point_block.get(&point).copied().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "last-use query cannot locate block for point {:?}",
                point
            ))
        })
    }

    fn start_event_index(
        &self,
        block: BlockId,
        location: LastUseLocation,
    ) -> Result<usize, CompilerError> {
        let events = self.events_by_block.get(&block).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "last-use query cannot locate normalized block {:?}",
                block
            ))
        })?;
        if let Some(event) = location.after_event {
            let index = events
                .iter()
                .position(|candidate| *candidate == event)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "last-use query event {:?} is not owned by block {:?}",
                        event, block
                    ))
                })?;
            return Ok(index + 1);
        }

        Ok(events
            .iter()
            .position(|event| {
                self.event_location_by_id
                    .get(event)
                    .is_some_and(|event_location| event_location.point == location.point)
            })
            .unwrap_or(0))
    }

    fn observation_after_in_block(
        &self,
        observation: &LastUseObservation,
        block: BlockId,
        start_index: usize,
    ) -> bool {
        let Some(observation_event) = self.event_id_for_observation(observation) else {
            return false;
        };
        self.events_by_block
            .get(&block)
            .and_then(|events| events.iter().position(|event| *event == observation_event))
            .is_some_and(|index| index >= start_index)
    }

    fn event_id_for_observation(&self, observation: &LastUseObservation) -> Option<EventId> {
        if let Some(event) = observation.location.after_event {
            return Some(event);
        }
        self.event_location_by_id
            .iter()
            .find_map(|(event, location)| {
                (location.point == observation.location.point).then_some(*event)
            })
    }

    fn block_summaries(
        &self,
        subject: LastUseSubject,
        query_block: BlockId,
        query_start_index: usize,
    ) -> BlockSummary {
        let mut may = BTreeMap::<BlockId, bool>::new();
        let mut must = BTreeMap::<BlockId, bool>::new();
        for block in self.events_by_block.keys() {
            may.insert(*block, false);
            must.insert(*block, false);
        }

        let mut local_has_use = BTreeMap::<BlockId, bool>::new();
        let mut local_first_use = BTreeMap::<BlockId, UseId>::new();
        for block in self.events_by_block.keys() {
            if let Some(use_id) = self.first_observation_in_block(subject, *block, 0) {
                local_has_use.insert(*block, true);
                local_first_use.insert(*block, use_id);
            }
        }

        let query_local_use =
            self.first_observation_in_block(subject, query_block, query_start_index);
        local_has_use.insert(query_block, query_local_use.is_some());
        if let Some(use_id) = query_local_use {
            local_first_use.insert(query_block, use_id);
        }

        let mut changed = true;
        while changed {
            changed = false;
            for block in self.events_by_block.keys().rev() {
                let local = local_has_use.get(block).copied().unwrap_or(false);
                let successors = self.successors.get(block).map(Vec::as_slice).unwrap_or(&[]);
                let next_may = local || successors.iter().any(|successor| may[successor]);
                let next_must = if local {
                    true
                } else if successors.is_empty() {
                    false
                } else {
                    successors.iter().all(|successor| must[successor])
                };
                if may[block] != next_may {
                    may.insert(*block, next_may);
                    changed = true;
                }
                if must[block] != next_must {
                    must.insert(*block, next_must);
                    changed = true;
                }
            }
        }

        let later_use = if query_local_use.is_some() {
            local_first_use.get(&query_block).copied()
        } else {
            self.find_summary_use(subject, &may, query_block)
        };
        let no_use_exit = self.find_summary_no_use_exit(subject, &may, query_block);

        BlockSummary {
            may: may.get(&query_block).copied().unwrap_or(false),
            must: must.get(&query_block).copied().unwrap_or(false),
            later_use,
            no_use_exit,
        }
    }

    fn first_observation_in_block(
        &self,
        subject: LastUseSubject,
        block: BlockId,
        start_index: usize,
    ) -> Option<UseId> {
        let events = self.events_by_block.get(&block)?;
        let observations = self.observations_by_subject.get(&subject)?;
        events
            .iter()
            .enumerate()
            .skip(start_index)
            .find_map(|(_, event)| {
                observations.iter().find_map(|observation| {
                    (self.event_id_for_observation(observation) == Some(*event))
                        .then_some(observation.use_id)
                })
            })
    }

    fn find_later_use(
        &self,
        subject: LastUseSubject,
        query_block: BlockId,
        query_start_index: usize,
    ) -> Option<UseId> {
        self.first_observation_in_block(subject, query_block, query_start_index)
            .or_else(|| self.find_reachable_use(subject, self.successors.get(&query_block)?))
    }

    fn find_reachable_use(&self, subject: LastUseSubject, starts: &[BlockId]) -> Option<UseId> {
        let mut queue = VecDeque::from(starts.to_vec());
        let mut visited = BTreeSet::new();
        while let Some(block) = queue.pop_front() {
            if !visited.insert(block) {
                continue;
            }
            if let Some(use_id) = self.first_observation_in_block(subject, block, 0) {
                return Some(use_id);
            }
            queue.extend(self.successors.get(&block).into_iter().flatten().copied());
        }
        None
    }

    fn find_summary_use(
        &self,
        subject: LastUseSubject,
        may: &BTreeMap<BlockId, bool>,
        query_block: BlockId,
    ) -> Option<UseId> {
        self.find_reachable_summary_use(subject, may, self.successors.get(&query_block)?)
    }

    fn find_reachable_summary_use(
        &self,
        subject: LastUseSubject,
        may: &BTreeMap<BlockId, bool>,
        starts: &[BlockId],
    ) -> Option<UseId> {
        let mut queue = VecDeque::from(starts.to_vec());
        let mut visited = BTreeSet::new();
        while let Some(block) = queue.pop_front() {
            if !visited.insert(block) || !may.get(&block).copied().unwrap_or(false) {
                continue;
            }
            if let Some(use_id) = self.first_observation_in_block(subject, block, 0) {
                return Some(use_id);
            }
            queue.extend(self.successors.get(&block).into_iter().flatten().copied());
        }
        None
    }

    fn find_summary_no_use_exit(
        &self,
        subject: LastUseSubject,
        may: &BTreeMap<BlockId, bool>,
        query_block: BlockId,
    ) -> Option<BlockId> {
        let successors = self.successors.get(&query_block)?;
        let mut queue = VecDeque::from(successors.clone());
        let mut visited = BTreeSet::new();
        while let Some(block) = queue.pop_front() {
            if !visited.insert(block) {
                continue;
            }
            if self.first_observation_in_block(subject, block, 0).is_some() {
                continue;
            }
            if !may.get(&block).copied().unwrap_or(false) && self.exits.contains(&block) {
                return Some(block);
            }
            queue.extend(self.successors.get(&block).into_iter().flatten().copied());
        }
        None
    }

    fn reachable_exits(&self, start: BlockId) -> Box<[BlockId]> {
        let mut queue = VecDeque::from([start]);
        let mut visited = BTreeSet::new();
        let mut exits = BTreeSet::new();
        while let Some(block) = queue.pop_front() {
            if !visited.insert(block) {
                continue;
            }
            if self.exits.contains(&block) {
                exits.insert(block);
            }
            queue.extend(self.successors.get(&block).into_iter().flatten().copied());
        }
        exits.into_iter().collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct BlockSummary {
    may: bool,
    must: bool,
    later_use: Option<UseId>,
    no_use_exit: Option<BlockId>,
}

#[cfg(test)]
mod tests;
