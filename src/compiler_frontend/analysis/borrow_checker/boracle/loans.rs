//! Loan liveness and access-conflict solving for Boracle.
//!
//! WHAT: issues explicit and call-argument loans, tracks CFG-aware liveness and records typed
//! overlap conflicts.
//! WHY: legality depends on the access capability and normalized place overlap, not on binding
//! names or lexical visibility alone.

// Some research-facing rows are not printed by every current dump. Keep the complete typed
// result surface warning-free as future investigation queries are added.
#![allow(dead_code)]

use super::super::problem::{
    AccessKind, BorrowProblem, Event, EventId, EventKind, Loan, LoanId, PlaceId, PlaceOverlap,
    PointId, UseId, UseKind,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One solver-owned loan, including loans inferred from one call argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoanFact {
    pub(crate) id: LoanId,
    pub(crate) kind: AccessKind,
    pub(crate) issued_at: PointId,
    pub(crate) issue_event: Option<EventId>,
    pub(crate) place: PlaceId,
    pub(crate) holders: Box<[PlaceId]>,
    pub(crate) uses: Box<[UseId]>,
    pub(crate) kills: Box<[PointId]>,
    pub(crate) live_points: Box<[PointId]>,
}

/// The overlap evidence for one rejected access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConflictWitness {
    pub(crate) access_event: EventId,
    pub(crate) access_use: Option<UseId>,
    pub(crate) access_kind: AccessKind,
    pub(crate) access_place: PlaceId,
    pub(crate) conflicting_loan: LoanId,
    pub(crate) loan_issue_point: PointId,
    pub(crate) keeping_use: Option<UseId>,
    pub(crate) overlap: PlaceOverlap,
}

/// One access decision retained even when it is legal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccessDecision {
    pub(crate) event: EventId,
    pub(crate) use_id: Option<UseId>,
    pub(crate) place: PlaceId,
    pub(crate) kind: AccessKind,
    pub(crate) allowed: bool,
}

/// Complete loan and conflict result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoanSolution {
    loans: Box<[LoanFact]>,
    decisions: Box<[AccessDecision]>,
    conflicts: Box<[ConflictWitness]>,
}

impl LoanSolution {
    pub(crate) fn loans(&self) -> &[LoanFact] {
        &self.loans
    }

    pub(crate) fn decisions(&self) -> &[AccessDecision] {
        &self.decisions
    }

    pub(crate) fn conflicts(&self) -> &[ConflictWitness] {
        &self.conflicts
    }

    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }
}

/// Reference loan solver.
pub(crate) struct LoanSolver;

impl LoanSolver {
    pub(crate) fn solve(problem: &BorrowProblem) -> Result<LoanSolution, CompilerError> {
        problem.validate()?;
        let graph = EventGraph::new(problem)?;
        let mut loans = problem
            .loans()
            .iter()
            .map(|loan| graph.explicit_loan(problem, loan))
            .collect::<Result<Vec<_>, _>>()?;

        let explicit_count = loans.len();
        let mut call_argument_loans = Vec::new();
        for event in problem.events() {
            let EventKind::CallEffect(effect) = &event.kind else {
                continue;
            };
            for argument in &effect.arguments {
                let id = LoanId::new(
                    u32::try_from(explicit_count + call_argument_loans.len()).map_err(|_| {
                        CompilerError::compiler_error("Boracle loan table is larger than u32::MAX")
                    })?,
                );
                call_argument_loans.push(LoanFact {
                    id,
                    kind: argument.access,
                    issued_at: event.point,
                    issue_event: Some(event.id),
                    place: argument.place,
                    holders: vec![argument.place].into_boxed_slice(),
                    uses: vec![argument.use_id].into_boxed_slice(),
                    kills: vec![event.point].into_boxed_slice(),
                    live_points: vec![event.point].into_boxed_slice(),
                });
            }
        }
        loans.extend(call_argument_loans);

        let mut decisions = Vec::new();
        let mut conflicts = Vec::new();
        for event in problem.events() {
            let accesses = event_accesses(problem, event)?;
            for access in accesses {
                let mut access_conflicts = Vec::new();
                for loan in &loans {
                    if access
                        .use_id
                        .is_some_and(|use_id| loan.uses.contains(&use_id))
                    {
                        continue;
                    }
                    if !graph.loan_live_at_event(problem, loan, event.id) {
                        continue;
                    }
                    if !access_conflicts_with_loan(problem, &access, loan) {
                        continue;
                    }
                    let overlap = problem.places()[access.place.index()]
                        .overlap(&problem.places()[loan.place.index()]);
                    let witness = ConflictWitness {
                        access_event: event.id,
                        access_use: access.use_id,
                        access_kind: access.kind,
                        access_place: access.place,
                        conflicting_loan: loan.id,
                        loan_issue_point: loan.issued_at,
                        keeping_use: loan.uses.first().copied(),
                        overlap,
                    };
                    access_conflicts.push(witness.clone());
                    conflicts.push(witness);
                }

                let decision = AccessDecision {
                    event: event.id,
                    use_id: access.use_id,
                    place: access.place,
                    kind: access.kind,
                    allowed: access_conflicts.is_empty(),
                };
                decisions.push(decision);
            }

            if let EventKind::CallEffect(effect) = &event.kind {
                for (index, left) in effect.arguments.iter().enumerate() {
                    for right in effect.arguments.iter().skip(index + 1) {
                        if !access_kinds_conflict(left.access, right.access) {
                            continue;
                        }
                        let overlap = problem.places()[left.place.index()]
                            .overlap(&problem.places()[right.place.index()]);
                        if overlap == PlaceOverlap::Disjoint {
                            continue;
                        }
                        let left_loan = loans.iter().find(|loan| {
                            loan.issue_event == Some(event.id)
                                && loan.place == left.place
                                && loan.uses.contains(&left.use_id)
                        });
                        let Some(left_loan) = left_loan else {
                            continue;
                        };
                        conflicts.push(ConflictWitness {
                            access_event: event.id,
                            access_use: Some(right.use_id),
                            access_kind: right.access,
                            access_place: right.place,
                            conflicting_loan: left_loan.id,
                            loan_issue_point: left_loan.issued_at,
                            keeping_use: Some(left.use_id),
                            overlap,
                        });
                    }
                }
            }
        }

        decisions.sort_by_key(|decision| (decision.event.raw(), decision.use_id.map(UseId::raw)));
        conflicts.sort_by_key(|conflict| {
            (
                conflict.access_event.raw(),
                conflict.access_use.map(UseId::raw),
                conflict.conflicting_loan.raw(),
            )
        });
        loans.sort_by_key(|loan| loan.id.raw());
        Ok(LoanSolution {
            loans: loans.into_boxed_slice(),
            decisions: decisions.into_boxed_slice(),
            conflicts: conflicts.into_boxed_slice(),
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct AccessFact {
    use_id: Option<UseId>,
    place: PlaceId,
    kind: AccessKind,
}

fn event_accesses(
    problem: &BorrowProblem,
    event: &Event,
) -> Result<Vec<AccessFact>, CompilerError> {
    match &event.kind {
        EventKind::Access { use_id } => {
            let use_row = problem.uses().get(use_id.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Boracle loan solver cannot locate access use {:?}",
                    use_id
                ))
            })?;
            Ok(vec![AccessFact {
                use_id: Some(*use_id),
                place: use_row.place,
                kind: match use_row.kind {
                    UseKind::Read | UseKind::LoanObservation => AccessKind::Shared,
                    UseKind::Write => AccessKind::Exclusive,
                },
            }])
        }
        EventKind::CallEffect(effect) => Ok(effect
            .arguments
            .iter()
            .map(|argument| AccessFact {
                use_id: Some(argument.use_id),
                place: argument.place,
                kind: argument.access,
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

fn access_conflicts_with_loan(
    problem: &BorrowProblem,
    access: &AccessFact,
    loan: &LoanFact,
) -> bool {
    if access.kind == AccessKind::Shared && loan.kind == AccessKind::Shared {
        return false;
    }
    let overlap =
        problem.places()[access.place.index()].overlap(&problem.places()[loan.place.index()]);
    overlap != PlaceOverlap::Disjoint
}

fn access_kinds_conflict(left: AccessKind, right: AccessKind) -> bool {
    left == AccessKind::Exclusive || right == AccessKind::Exclusive
}

struct EventGraph {
    events_by_block: BTreeMap<super::super::problem::BlockId, Vec<EventId>>,
    successors: BTreeMap<super::super::problem::BlockId, Vec<super::super::problem::BlockId>>,
    event_location: BTreeMap<EventId, (super::super::problem::BlockId, usize)>,
}

impl EventGraph {
    fn new(problem: &BorrowProblem) -> Result<Self, CompilerError> {
        let mut events_by_block = BTreeMap::new();
        let mut event_location = BTreeMap::new();
        for block in &problem.control_flow().blocks {
            events_by_block.insert(block.id, block.events.to_vec());
            for (index, event) in block.events.iter().enumerate() {
                event_location.insert(*event, (block.id, index));
            }
        }
        let mut successors = BTreeMap::new();
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
        if event_location.len() != problem.events().len() {
            return Err(CompilerError::compiler_error(
                "Boracle loan solver found an event without CFG ownership",
            ));
        }
        Ok(Self {
            events_by_block,
            successors,
            event_location,
        })
    }

    fn explicit_loan(
        &self,
        problem: &BorrowProblem,
        loan: &Loan,
    ) -> Result<LoanFact, CompilerError> {
        let issue_event = problem.events().iter().find_map(|event| {
            matches!(event.kind, EventKind::LoanIssue { loan: id } if id == loan.id)
                .then_some(event.id)
        });
        if issue_event.is_none() {
            return Err(CompilerError::compiler_error(format!(
                "Boracle loan solver cannot locate issue event for loan {:?}",
                loan.id
            )));
        }
        let mut live_points = problem
            .points()
            .iter()
            .filter_map(|point| {
                let live = self
                    .events_by_block
                    .get(&point.block)
                    .into_iter()
                    .flatten()
                    .any(|event| {
                        problem
                            .events()
                            .get(event.index())
                            .is_some_and(|row| row.point == point.id)
                            && self.loan_live_at_event(
                                problem,
                                &self.stub_loan(loan, issue_event),
                                *event,
                            )
                    });
                live.then_some(point.id)
            })
            .collect::<Vec<_>>();
        live_points.sort_by_key(|point| point.raw());
        live_points.dedup();
        Ok(LoanFact {
            id: loan.id,
            kind: loan.kind,
            issued_at: loan.issued_at,
            issue_event,
            place: loan.place,
            holders: loan.holders.clone(),
            uses: loan.uses.clone(),
            kills: loan.kills.clone(),
            live_points: live_points.into_boxed_slice(),
        })
    }

    fn stub_loan(&self, loan: &Loan, issue_event: Option<EventId>) -> LoanFact {
        LoanFact {
            id: loan.id,
            kind: loan.kind,
            issued_at: loan.issued_at,
            issue_event,
            place: loan.place,
            holders: loan.holders.clone(),
            uses: loan.uses.clone(),
            kills: loan.kills.clone(),
            live_points: Box::new([]),
        }
    }

    fn loan_live_at_event(
        &self,
        problem: &BorrowProblem,
        loan: &LoanFact,
        target_event: EventId,
    ) -> bool {
        let Some(issue_event) = loan.issue_event else {
            return false;
        };
        if issue_event == target_event {
            return true;
        }
        self.reaches_without_kill(problem, issue_event, target_event, &loan.kills)
    }

    fn reaches_without_kill(
        &self,
        problem: &BorrowProblem,
        issue_event: EventId,
        target_event: EventId,
        kills: &[PointId],
    ) -> bool {
        let Some(&(issue_block, issue_index)) = self.event_location.get(&issue_event) else {
            return false;
        };
        let mut queue = VecDeque::from([(issue_block, issue_index + 1)]);
        let mut visited = BTreeSet::new();
        while let Some((block, index)) = queue.pop_front() {
            if !visited.insert((block, index)) {
                continue;
            }
            let Some(events) = self.events_by_block.get(&block) else {
                continue;
            };
            if index >= events.len() {
                queue.extend(
                    self.successors
                        .get(&block)
                        .into_iter()
                        .flatten()
                        .map(|successor| (*successor, 0)),
                );
                continue;
            }
            let event_id = events[index];
            if event_id == target_event {
                return true;
            }
            let Some(event) = problem.events().get(event_id.index()) else {
                continue;
            };
            if kills.contains(&event.point) {
                continue;
            }
            queue.push_back((block, index + 1));
        }
        false
    }
}
