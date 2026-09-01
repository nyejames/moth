//! Deterministic reduction of normalized Boracle disagreement inputs.
//!
//! WHAT: rewrites one validated problem and its execution bounds while retaining the complete
//!       differential classification vector and bounded oracle outcome identity.
//! WHY: a reduced disagreement is useful only when every surviving candidate is validated and
//!      still describes the same reference and experiment classes and outcome identity. Runtime
//!      conflict traces and outcome counts are deliberately not compared because reductions
//!      densely renumber rows and may change how many executions are explored.
// Reduction has no production caller. This test-time developer facility has a reachable workflow
// deferred in the roadmap without an owning plan, so keep its complete surface warning-free.
#![allow(dead_code)]

use super::differential::{OracleComparisonClass, OracleComparisonSet, compare_problem_parts};
use super::oracle::{OracleBounds, OracleLimitReason, OracleOutcome, execute_bounded};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallId, CallResult, CallResultProvenance, CfgBlock, CfgEdge,
    Event, EventId, EventKind, KillReason, Loan, LoanId, OriginKind, Place, PlaceId, PointId,
    ProgramPoint, ProjectionElem, RebindValue, TerminatorEventKind, Use, UseId, UseKind,
    ValueOrigin, ValueOriginId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// The ordered transformations used by [`reduce_problem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReductionPass {
    RemoveUnreachableBlocks,
    RemoveEvents,
    RemoveUsesAndLoans,
    RemoveEdges,
    SimplifyProjections,
    ReduceOrigins,
    ReduceBindings,
    ReplaceCallsWithSimplerEffects,
    LowerLoopBounds,
}

impl ReductionPass {
    /// The reduction order is part of the reproducible output contract.
    pub(crate) const ALL: [Self; 9] = [
        Self::RemoveUnreachableBlocks,
        Self::RemoveEvents,
        Self::RemoveUsesAndLoans,
        Self::RemoveEdges,
        Self::SimplifyProjections,
        Self::ReduceOrigins,
        Self::ReduceBindings,
        Self::ReplaceCallsWithSimplerEffects,
        Self::LowerLoopBounds,
    ];
}

/// The discriminant is preserved for every outcome, and an [`OracleLimitReason`] is preserved
/// exactly for [`Self::Inconclusive`]. Runtime-conflict traces and outcome counts are not
/// compared because reduction renumbers rows and can change path counts.
///
/// Exact reason identity is deliberately conservative: reasons can embed row identifiers and
/// limits, so a candidate that shifts a referenced row is rejected even when it retains the same
/// disagreement. In particular, a [`OracleLimitReason::BlockEntryBound`] cannot lower
/// `max_block_entries` because doing so changes the embedded limit; a bound that remains stuck is
/// therefore an expected consequence of this false-safe-resistant trade-off, not a reduction bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OracleOutcomeIdentity {
    CompleteSafe,
    RuntimeConflict,
    Inconclusive { reason: OracleLimitReason },
}

impl OracleOutcomeIdentity {
    pub(crate) fn from_outcome(outcome: &OracleOutcome) -> Self {
        match outcome {
            OracleOutcome::CompleteSafe { .. } => Self::CompleteSafe,
            OracleOutcome::RuntimeConflict { .. } => Self::RuntimeConflict,
            OracleOutcome::Inconclusive { reason, .. } => Self::Inconclusive {
                reason: reason.clone(),
            },
        }
    }
}

/// A lexicographic natural-number measure for reduction termination.
///
/// Rows dominate nested data, nested data dominates semantic detail, and all problem data
/// dominates the four execution bounds. Every accepted candidate must be strictly smaller in this
/// order. Because every component is a natural number, the outer fixpoint loop is well-founded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ReductionSize {
    pub(crate) rows: usize,
    pub(crate) nested: usize,
    pub(crate) detail: usize,
    pub(crate) bounds: usize,
}

/// The minimal validated problem, bounds and rendering retained by a reduction run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReducedProblem {
    pub(crate) problem: BorrowProblem,
    pub(crate) bounds: OracleBounds,
    pub(crate) comparison_classes: Box<[OracleComparisonClass]>,
    pub(crate) static_accepts: Box<[bool]>,
    pub(crate) oracle_outcome: OracleOutcomeIdentity,
    pub(crate) fixture_skeleton: String,
    pub(crate) applied_passes: Box<[ReductionPass]>,
    pub(crate) size_history: Box<[ReductionSize]>,
    pub(crate) size: ReductionSize,
}

impl ReducedProblem {
    pub(crate) fn static_accepts(&self) -> &[bool] {
        &self.static_accepts
    }
    pub(crate) fn comparison_classes(&self) -> &[OracleComparisonClass] {
        &self.comparison_classes
    }

    pub(crate) fn oracle_outcome(&self) -> &OracleOutcomeIdentity {
        &self.oracle_outcome
    }

    pub(crate) fn fixture_skeleton(&self) -> &str {
        &self.fixture_skeleton
    }

    pub(crate) fn applied_passes(&self) -> &[ReductionPass] {
        &self.applied_passes
    }

    pub(crate) fn size_history(&self) -> &[ReductionSize] {
        &self.size_history
    }

    pub(crate) const fn size_measure(&self) -> ReductionSize {
        self.size
    }
}

/// Return the deterministic structural measure for one normalized problem and its bounds.
pub(crate) fn reduction_size(problem: &BorrowProblem, bounds: OracleBounds) -> ReductionSize {
    measure_parts(&problem_parts(problem), bounds)
}

fn validate_bounds(bounds: OracleBounds) -> Result<(), CompilerError> {
    for (name, value) in [
        ("max_executions", bounds.max_executions),
        ("max_executed_events", bounds.max_executed_events),
        ("max_block_entries", bounds.max_block_entries),
        ("max_dynamic_generations", bounds.max_dynamic_generations),
    ] {
        if value == 0 {
            return Err(CompilerError::compiler_error(format!(
                "Boracle reducer requires {name} to be greater than zero"
            )));
        }
    }
    Ok(())
}

/// Reduce a normalized problem to a fixpoint while preserving every differential class, static
/// verdict and the bounded oracle outcome identity.
pub(crate) fn reduce_problem(
    problem: BorrowProblem,
    bounds: OracleBounds,
) -> Result<ReducedProblem, CompilerError> {
    validate_bounds(bounds)?;
    let initial_set = compare_problem_parts(problem_parts(&problem), bounds)?;
    let comparison_classes = classes_from_set(&initial_set);
    let static_accepts = static_accepts_from_set(&initial_set)
        .ok_or_else(|| CompilerError::compiler_error("cannot reduce a malformed borrow problem"))?;
    let oracle_outcome = initial_set
        .oracle_outcome
        .as_ref()
        .map(OracleOutcomeIdentity::from_outcome)
        .ok_or_else(|| CompilerError::compiler_error("cannot reduce a malformed borrow problem"))?;
    let normalized_problem = initial_set.problem.ok_or_else(|| {
        CompilerError::compiler_error("cannot reduce a malformed normalized borrow problem")
    })?;
    let mut current = ReductionState {
        measure: reduction_size(&normalized_problem, bounds),
        problem: normalized_problem,
        bounds,
        comparison_classes,
        static_accepts,
        oracle_outcome,
    };
    let mut applied_passes = Vec::new();
    let mut size_history = vec![current.measure];
    loop {
        let mut made_progress = false;

        for pass in ReductionPass::ALL {
            while let Some(candidate) = attempt_pass(&current, pass)? {
                // `attempt_pass` applies this same guard before returning. Keeping the check here
                // makes the termination boundary explicit if a future pass is added incorrectly.
                if candidate.measure >= current.measure {
                    break;
                }
                current = candidate;
                size_history.push(current.measure);
                applied_passes.push(pass);
                made_progress = true;
            }
        }

        if !made_progress {
            break;
        }
    }

    let fixture_skeleton = render_fixture_skeleton(&current.problem, current.bounds);
    Ok(ReducedProblem {
        problem: current.problem,
        bounds: current.bounds,
        comparison_classes: current.comparison_classes,
        static_accepts: current.static_accepts,
        oracle_outcome: current.oracle_outcome,
        fixture_skeleton,
        applied_passes: applied_passes.into_boxed_slice(),
        size_history: size_history.into_boxed_slice(),
        size: current.measure,
    })
}

struct ReductionState {
    problem: BorrowProblem,
    bounds: OracleBounds,
    comparison_classes: Box<[OracleComparisonClass]>,
    static_accepts: Box<[bool]>,
    oracle_outcome: OracleOutcomeIdentity,
    measure: ReductionSize,
}

fn attempt_pass(
    current: &ReductionState,
    pass: ReductionPass,
) -> Result<Option<ReductionState>, CompilerError> {
    match pass {
        ReductionPass::RemoveUnreachableBlocks => attempt_remove_unreachable_blocks(current),
        ReductionPass::RemoveEvents => attempt_remove_events(current),
        ReductionPass::RemoveUsesAndLoans => attempt_remove_uses_and_loans(current),
        ReductionPass::RemoveEdges => attempt_remove_edges(current),
        ReductionPass::SimplifyProjections => attempt_simplify_projections(current),
        ReductionPass::ReduceOrigins => attempt_reduce_origins(current),
        ReductionPass::ReduceBindings => attempt_reduce_bindings(current),
        ReductionPass::ReplaceCallsWithSimplerEffects => {
            attempt_replace_calls_with_simpler_effects(current)
        }
        ReductionPass::LowerLoopBounds => attempt_lower_loop_bounds(current),
    }
}

fn try_candidate(
    current: &ReductionState,
    parts: BorrowProblemParts,
    bounds: OracleBounds,
) -> Result<Option<ReductionState>, CompilerError> {
    let measure = measure_parts(&parts, bounds);
    if measure >= current.measure
        || bounds.max_executions == 0
        || bounds.max_executed_events == 0
        || bounds.max_block_entries == 0
        || bounds.max_dynamic_generations == 0
    {
        return Ok(None);
    }

    let Ok(problem) = BorrowProblem::new(parts.clone()) else {
        return Ok(None);
    };
    let Ok(_) = execute_bounded(&problem, bounds) else {
        return Ok(None);
    };

    // Validation and operational-oracle errors belong to the candidate and reject it. Once that
    // pre-check succeeds, an error from comparison belongs to the static solver and must
    // propagate rather than hiding an invariant defect. This static-solver branch is defensive
    // today: every known input-dependent static failure also fails operational execution, and no
    // existing test covers a static-only failure.
    let comparison_set = compare_problem_parts(parts, bounds)?;
    let Some(oracle_outcome) = comparison_set
        .oracle_outcome
        .as_ref()
        .map(OracleOutcomeIdentity::from_outcome)
    else {
        return Ok(None);
    };
    if oracle_outcome != current.oracle_outcome {
        return Ok(None);
    }
    let comparison_classes = classes_from_set(&comparison_set);
    if comparison_classes.as_ref() != current.comparison_classes.as_ref() {
        return Ok(None);
    }
    let Some(static_accepts) = static_accepts_from_set(&comparison_set) else {
        return Ok(None);
    };
    if static_accepts.as_ref() != current.static_accepts.as_ref() {
        return Ok(None);
    }
    let Some(problem) = comparison_set.problem else {
        return Ok(None);
    };

    Ok(Some(ReductionState {
        problem,
        bounds,
        comparison_classes,
        static_accepts,
        oracle_outcome,
        measure,
    }))
}

fn classes_from_set(comparison_set: &OracleComparisonSet) -> Box<[OracleComparisonClass]> {
    comparison_set
        .comparisons()
        .map(|comparison| comparison.class)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn static_accepts_from_set(comparison_set: &OracleComparisonSet) -> Option<Box<[bool]>> {
    comparison_set
        .comparisons()
        .map(|comparison| {
            comparison
                .static_report
                .as_ref()
                .map(|report| !report.has_conflicts())
        })
        .collect::<Option<Vec<_>>>()
        .map(|verdicts| verdicts.into_boxed_slice())
}

fn problem_parts(problem: &BorrowProblem) -> BorrowProblemParts {
    let flow = problem.control_flow();
    BorrowProblemParts {
        bindings: problem.bindings().to_vec(),
        points: problem.points().to_vec(),
        blocks: flow.blocks.to_vec(),
        edges: flow.edges.to_vec(),
        entry: flow.entry,
        exits: flow.exits.to_vec(),
        places: problem.places().to_vec(),
        origins: problem.origins().to_vec(),
        loans: problem.loans().to_vec(),
        uses: problem.uses().to_vec(),
        calls: problem.calls().to_vec(),
        events: problem.events().to_vec(),
    }
}

fn measure_parts(parts: &BorrowProblemParts, bounds: OracleBounds) -> ReductionSize {
    let rows = parts.bindings.len()
        + parts.points.len()
        + parts.blocks.len()
        + parts.edges.len()
        + parts.places.len()
        + parts.origins.len()
        + parts.loans.len()
        + parts.uses.len()
        + parts.calls.len()
        + parts.events.len();

    let mut nested = 0;
    nested += parts
        .blocks
        .iter()
        .map(|block| block.events.len())
        .sum::<usize>();
    nested += parts
        .loans
        .iter()
        .map(|loan| loan.origins.len() + loan.holders.len() + loan.uses.len() + loan.kills.len())
        .sum::<usize>();
    nested += parts.uses.len();
    nested += parts
        .places
        .iter()
        .map(|place| place.projections.len())
        .sum::<usize>();
    nested += parts.origins.iter().map(origin_nested_size).sum::<usize>();
    nested += parts.events.iter().map(event_nested_size).sum::<usize>();

    let detail = parts
        .bindings
        .iter()
        .map(|binding| usize::from(binding.mutable) + usize::from(binding.compiler_temporary))
        .sum::<usize>()
        + parts
            .calls
            .iter()
            .map(|call| call.label.len())
            .sum::<usize>()
        + parts
            .places
            .iter()
            .flat_map(|place| place.projections.iter())
            .map(projection_detail)
            .sum::<usize>()
        + parts.origins.iter().map(origin_detail).sum::<usize>()
        + parts.events.iter().map(event_detail).sum::<usize>();

    ReductionSize {
        rows,
        nested,
        detail,
        bounds: bounds.max_executions
            + bounds.max_executed_events
            + bounds.max_block_entries
            + bounds.max_dynamic_generations,
    }
}

fn origin_nested_size(origin: &ValueOrigin) -> usize {
    match &origin.kind {
        OriginKind::Alias(origins)
        | OriginKind::ExclusiveAlias(origins)
        | OriginKind::Copy(origins)
        | OriginKind::Join(origins) => origins.len(),
        OriginKind::Projection { .. } => 1,
        OriginKind::CallResult { provenance, .. } => match provenance {
            CallResultProvenance::Alias(origins) => origins.len(),
            CallResultProvenance::AliasParams(indices) => indices.len(),
            CallResultProvenance::Fresh | CallResultProvenance::Unknown(_) => 0,
        },
        OriginKind::Unknown | OriginKind::Parameter { .. } | OriginKind::Fresh => 0,
    }
}

fn event_nested_size(event: &Event) -> usize {
    match &event.kind {
        EventKind::Alias { origins, .. } | EventKind::ExclusiveAlias { origins, .. } => {
            origins.len()
        }
        EventKind::Aggregate { fields, .. } => fields.len(),
        EventKind::CallArgument { .. } => 1,
        EventKind::CallEffect(effect) => {
            effect.arguments.len() + usize::from(effect.result.is_some())
        }
        EventKind::Terminator {
            kind: TerminatorEventKind::Branch { targets },
        } => targets.len(),
        EventKind::Fresh { .. }
        | EventKind::AliasFromPlace { .. }
        | EventKind::ExclusiveAliasFromPlace { .. }
        | EventKind::Copy { .. }
        | EventKind::Projection { .. }
        | EventKind::Rebind { .. }
        | EventKind::ScopeExit { .. }
        | EventKind::ReactiveObserve { .. }
        | EventKind::Terminator { .. }
        | EventKind::Access { .. }
        | EventKind::LoanIssue { .. }
        | EventKind::LoanKill { .. } => 0,
    }
}

fn projection_detail(projection: &ProjectionElem) -> usize {
    match projection {
        ProjectionElem::Field(index) => 1 + *index as usize,
        ProjectionElem::FixedIndex(index) => 2 + *index as usize,
        ProjectionElem::DynamicIndex => 4,
        ProjectionElem::CollectionElement => 5,
        ProjectionElem::MapEntry => 6,
    }
}

fn origin_detail(origin: &ValueOrigin) -> usize {
    match &origin.kind {
        OriginKind::Unknown => 0,
        OriginKind::Fresh => 1,
        OriginKind::Parameter { index } => 2 + *index as usize,
        OriginKind::Copy(origins) => 3 + origins.len(),
        OriginKind::Projection { projection, .. } => 4 + projection_detail(projection),
        OriginKind::Alias(origins) => 5 + origins.len(),
        OriginKind::ExclusiveAlias(origins) => 6 + origins.len(),
        OriginKind::Join(origins) => 7 + origins.len(),
        OriginKind::CallResult { provenance, .. } => {
            8 + match provenance {
                CallResultProvenance::Fresh => 0,
                CallResultProvenance::Alias(origins) => origins.len(),
                CallResultProvenance::AliasParams(indices) => indices.len(),
                CallResultProvenance::Unknown(_) => 1,
            }
        }
    }
}

fn event_detail(event: &Event) -> usize {
    match &event.kind {
        EventKind::Aggregate { fields, .. } => fields
            .iter()
            .map(|field| projection_detail(&field.projection))
            .sum(),
        EventKind::CallEffect(effect) => effect
            .arguments
            .iter()
            .map(|argument| match argument.access {
                AccessKind::Shared => 1,
                AccessKind::Exclusive => 2,
            })
            .sum::<usize>(),
        EventKind::CallArgument { argument, .. } => match argument.access {
            AccessKind::Shared => 1,
            AccessKind::Exclusive => 2,
        },
        EventKind::Terminator {
            kind: TerminatorEventKind::Branch { targets },
        } => targets.len(),
        _ => 0,
    }
}

#[derive(Clone)]
struct KeepRows {
    bindings: Vec<bool>,
    blocks: Vec<bool>,
    points: Vec<bool>,
    places: Vec<bool>,
    origins: Vec<bool>,
    loans: Vec<bool>,
    uses: Vec<bool>,
    calls: Vec<bool>,
    events: Vec<bool>,
}

impl KeepRows {
    fn all(parts: &BorrowProblemParts) -> Self {
        Self {
            bindings: vec![true; parts.bindings.len()],
            blocks: vec![true; parts.blocks.len()],
            points: vec![true; parts.points.len()],
            places: vec![true; parts.places.len()],
            origins: vec![true; parts.origins.len()],
            loans: vec![true; parts.loans.len()],
            uses: vec![true; parts.uses.len()],
            calls: vec![true; parts.calls.len()],
            events: vec![true; parts.events.len()],
        }
    }
}

fn dense_map<T: Copy>(keep: &[bool], make: impl Fn(u32) -> T) -> Vec<Option<T>> {
    let mut next = 0_u32;
    keep.iter()
        .map(|is_kept| {
            if *is_kept {
                let id = make(next);
                next += 1;
                Some(id)
            } else {
                None
            }
        })
        .collect()
}

fn mapped<T: Copy>(map: &[Option<T>], index: usize) -> Option<T> {
    map.get(index).copied().flatten()
}

fn remap_values<T: Copy, U>(
    values: &[T],
    mut remap: impl FnMut(T) -> Option<U>,
) -> Option<Box<[U]>> {
    let mut output = Vec::with_capacity(values.len());
    for value in values.iter().copied() {
        output.push(remap(value)?);
    }
    Some(output.into_boxed_slice())
}

fn remap_parts(parts: BorrowProblemParts, keep: &KeepRows) -> Option<BorrowProblemParts> {
    let binding_map = dense_map(&keep.bindings, BindingId::new);
    let block_map = dense_map(&keep.blocks, BlockId::new);
    let point_map = dense_map(&keep.points, PointId::new);
    let place_map = dense_map(&keep.places, PlaceId::new);
    let origin_map = dense_map(&keep.origins, ValueOriginId::new);
    let loan_map = dense_map(&keep.loans, LoanId::new);
    let use_map = dense_map(&keep.uses, UseId::new);
    let call_map = dense_map(&keep.calls, CallId::new);
    let event_map = dense_map(&keep.events, EventId::new);

    let bindings = parts
        .bindings
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.bindings[*index])
        .map(|(index, binding)| {
            let mut binding = binding.clone();
            binding.id = mapped(&binding_map, index)?;
            Some(binding)
        })
        .collect::<Option<Vec<_>>>()?;

    let points = parts
        .points
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.points[*index])
        .map(|(index, point)| {
            let mut point = point.clone();
            point.id = mapped(&point_map, index)?;
            point.block = mapped(&block_map, point.block.index())?;
            Some(point)
        })
        .collect::<Option<Vec<_>>>()?;
    let blocks = parts
        .blocks
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.blocks[*index])
        .map(|(index, block)| {
            let mut block = block.clone();
            block.id = mapped(&block_map, index)?;
            block.entry = mapped(&point_map, block.entry.index())?;
            block.exit = mapped(&point_map, block.exit.index())?;
            block.events = block
                .events
                .iter()
                .filter_map(|event_id| mapped(&event_map, event_id.index()))
                .collect::<Vec<_>>()
                .into_boxed_slice();
            Some(block)
        })
        .collect::<Option<Vec<_>>>()?;

    let places = parts
        .places
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.places[*index])
        .map(|(index, place)| {
            let mut place = place.clone();
            place.id = mapped(&place_map, index)?;
            place.root = mapped(&binding_map, place.root.index())?;
            Some(place)
        })
        .collect::<Option<Vec<_>>>()?;

    let origins = parts
        .origins
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.origins[*index])
        .map(|(index, origin)| {
            Some(ValueOrigin {
                id: mapped(&origin_map, index)?,
                kind: remap_origin_kind(&origin.kind, &origin_map, &call_map)?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let loans = parts
        .loans
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.loans[*index])
        .map(|(index, loan)| {
            Some(Loan {
                id: mapped(&loan_map, index)?,
                kind: loan.kind,
                issued_at: mapped(&point_map, loan.issued_at.index())?,
                place: mapped(&place_map, loan.place.index())?,
                origins: remap_values(&loan.origins, |origin| mapped(&origin_map, origin.index()))?,
                holders: remap_values(&loan.holders, |place| mapped(&place_map, place.index()))?,
                uses: remap_values(&loan.uses, |use_id| mapped(&use_map, use_id.index()))?,
                kills: remap_values(&loan.kills, |point| mapped(&point_map, point.index()))?,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let uses = parts
        .uses
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.uses[*index])
        .map(|(index, use_row)| {
            Some(Use {
                id: mapped(&use_map, index)?,
                point: mapped(&point_map, use_row.point.index())?,
                place: mapped(&place_map, use_row.place.index())?,
                kind: use_row.kind,
                definition: use_row.definition,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let calls = parts
        .calls
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.calls[*index])
        .map(|(index, call)| {
            Some(Call {
                id: mapped(&call_map, index)?,
                label: call.label.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let events = parts
        .events
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.events[*index])
        .map(|(index, event)| {
            Some(Event {
                id: mapped(&event_map, index)?,
                point: mapped(&point_map, event.point.index())?,
                source: event.source.clone(),
                kind: remap_event_kind(
                    &event.kind,
                    &binding_map,
                    &block_map,
                    &place_map,
                    &origin_map,
                    &loan_map,
                    &use_map,
                    &call_map,
                )?,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let edges = parts
        .edges
        .iter()
        .filter_map(|edge| {
            Some(CfgEdge {
                from: mapped(&block_map, edge.from.index())?,
                to: mapped(&block_map, edge.to.index())?,
            })
        })
        .collect();
    let entry = mapped(&block_map, parts.entry.index())?;
    let exits = parts
        .exits
        .iter()
        .filter_map(|exit| mapped(&block_map, exit.index()))
        .collect();

    Some(BorrowProblemParts {
        bindings,
        points,
        blocks,
        edges,
        entry,
        exits,
        places,
        origins,
        loans,
        uses,
        calls,
        events,
    })
}

fn remap_origin_kind(
    kind: &OriginKind,
    origin_map: &[Option<ValueOriginId>],
    call_map: &[Option<CallId>],
) -> Option<OriginKind> {
    match kind {
        OriginKind::Unknown => Some(OriginKind::Unknown),
        OriginKind::Parameter { index } => Some(OriginKind::Parameter { index: *index }),
        OriginKind::Fresh => Some(OriginKind::Fresh),
        OriginKind::Alias(origins) => Some(OriginKind::Alias(remap_values(origins, |origin| {
            mapped(origin_map, origin.index())
        })?)),
        OriginKind::ExclusiveAlias(origins) => Some(OriginKind::ExclusiveAlias(remap_values(
            origins,
            |origin| mapped(origin_map, origin.index()),
        )?)),
        OriginKind::Copy(origins) => Some(OriginKind::Copy(remap_values(origins, |origin| {
            mapped(origin_map, origin.index())
        })?)),
        OriginKind::Projection { source, projection } => Some(OriginKind::Projection {
            source: mapped(origin_map, source.index())?,
            projection: *projection,
        }),
        OriginKind::Join(origins) => Some(OriginKind::Join(remap_values(origins, |origin| {
            mapped(origin_map, origin.index())
        })?)),
        OriginKind::CallResult { call, provenance } => Some(OriginKind::CallResult {
            call: mapped(call_map, call.index())?,
            provenance: remap_call_result_provenance(provenance, origin_map)?,
        }),
    }
}

fn remap_call_result_provenance(
    provenance: &CallResultProvenance,
    origin_map: &[Option<ValueOriginId>],
) -> Option<CallResultProvenance> {
    match provenance {
        CallResultProvenance::Fresh => Some(CallResultProvenance::Fresh),
        CallResultProvenance::Alias(origins) => Some(CallResultProvenance::Alias(remap_values(
            origins,
            |origin| mapped(origin_map, origin.index()),
        )?)),
        CallResultProvenance::AliasParams(indices) => {
            Some(CallResultProvenance::AliasParams(indices.clone()))
        }
        CallResultProvenance::Unknown(reason) => Some(CallResultProvenance::Unknown(*reason)),
    }
}

#[allow(clippy::too_many_arguments)]
fn remap_event_kind(
    kind: &EventKind,
    binding_map: &[Option<BindingId>],
    block_map: &[Option<BlockId>],
    place_map: &[Option<PlaceId>],
    origin_map: &[Option<ValueOriginId>],
    loan_map: &[Option<LoanId>],
    use_map: &[Option<UseId>],
    call_map: &[Option<CallId>],
) -> Option<EventKind> {
    let place = |id: PlaceId| mapped(place_map, id.index());
    let origin = |id: ValueOriginId| mapped(origin_map, id.index());
    let call = |id: CallId| mapped(call_map, id.index());
    match kind {
        EventKind::Fresh {
            destination,
            origin: origin_id,
        } => Some(EventKind::Fresh {
            destination: place(*destination)?,
            origin: origin(*origin_id)?,
        }),
        EventKind::Alias {
            source,
            destination,
            origins,
        } => Some(EventKind::Alias {
            source: place(*source)?,
            destination: place(*destination)?,
            origins: remap_values(origins, origin)?,
        }),
        EventKind::AliasFromPlace {
            source,
            destination,
        } => Some(EventKind::AliasFromPlace {
            source: place(*source)?,
            destination: place(*destination)?,
        }),
        EventKind::ExclusiveAlias {
            source,
            destination,
            origins,
        } => Some(EventKind::ExclusiveAlias {
            source: place(*source)?,
            destination: place(*destination)?,
            origins: remap_values(origins, origin)?,
        }),
        EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => Some(EventKind::ExclusiveAliasFromPlace {
            source: place(*source)?,
            destination: place(*destination)?,
        }),
        EventKind::Copy {
            source,
            destination,
            origin: origin_id,
        } => Some(EventKind::Copy {
            source: place(*source)?,
            destination: place(*destination)?,
            origin: origin(*origin_id)?,
        }),
        EventKind::Projection {
            source,
            destination,
            origin: origin_id,
        } => Some(EventKind::Projection {
            source: place(*source)?,
            destination: place(*destination)?,
            origin: origin(*origin_id)?,
        }),
        EventKind::Rebind { destination, value } => Some(EventKind::Rebind {
            destination: place(*destination)?,
            value: remap_rebind_value(value, place, origin)?,
        }),
        EventKind::Aggregate {
            destination,
            origin: origin_id,
            fields,
        } => Some(EventKind::Aggregate {
            destination: place(*destination)?,
            origin: origin(*origin_id)?,
            fields: fields
                .iter()
                .map(|field| {
                    Some(AggregateField {
                        projection: field.projection,
                        source: place(field.source)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?
                .into_boxed_slice(),
        }),
        EventKind::ScopeExit { bindings } => Some(EventKind::ScopeExit {
            bindings: remap_values(bindings, |binding| mapped(binding_map, binding.index()))?,
        }),
        EventKind::ReactiveObserve { place: place_id } => Some(EventKind::ReactiveObserve {
            place: place(*place_id)?,
        }),
        EventKind::CallArgument {
            call: call_id,
            index,
            argument,
        } => Some(EventKind::CallArgument {
            call: call(*call_id)?,
            index: *index,
            argument: remap_call_argument(argument, place, use_map)?,
        }),
        EventKind::Terminator { kind } => Some(EventKind::Terminator {
            kind: remap_terminator_kind(kind, block_map)?,
        }),
        EventKind::CallEffect(effect) => Some(EventKind::CallEffect(remap_call_effect(
            effect, place, origin, use_map, call,
        )?)),
        EventKind::Access { use_id } => Some(EventKind::Access {
            use_id: mapped(use_map, use_id.index())?,
        }),
        EventKind::LoanIssue { loan } => Some(EventKind::LoanIssue {
            loan: mapped(loan_map, loan.index())?,
        }),
        EventKind::LoanKill { loan, reason } => Some(EventKind::LoanKill {
            loan: mapped(loan_map, loan.index())?,
            reason: *reason,
        }),
    }
}

fn remap_rebind_value(
    value: &RebindValue,
    place: impl Fn(PlaceId) -> Option<PlaceId>,
    origin: impl Fn(ValueOriginId) -> Option<ValueOriginId>,
) -> Option<RebindValue> {
    match value {
        RebindValue::Fresh(origin_id) => Some(RebindValue::Fresh(origin(*origin_id)?)),
        RebindValue::Alias(origins) => Some(RebindValue::Alias(remap_values(origins, origin)?)),
        RebindValue::AliasFromPlace(place_id) => {
            Some(RebindValue::AliasFromPlace(place(*place_id)?))
        }
    }
}

fn remap_call_argument(
    argument: &CallArgument,
    place: impl Fn(PlaceId) -> Option<PlaceId>,
    use_map: &[Option<UseId>],
) -> Option<CallArgument> {
    Some(CallArgument {
        place: place(argument.place)?,
        access: argument.access,
        use_id: mapped(use_map, argument.use_id.index())?,
    })
}

fn remap_call_effect(
    effect: &CallEffect,
    place: impl Fn(PlaceId) -> Option<PlaceId>,
    origin: impl Fn(ValueOriginId) -> Option<ValueOriginId>,
    use_map: &[Option<UseId>],
    call: impl Fn(CallId) -> Option<CallId>,
) -> Option<CallEffect> {
    let result = match effect.result {
        Some(result) => Some(CallResult {
            place: place(result.place)?,
            origin: origin(result.origin)?,
        }),
        None => None,
    };
    Some(CallEffect {
        call: call(effect.call)?,
        arguments: effect
            .arguments
            .iter()
            .map(|argument| remap_call_argument(argument, &place, use_map))
            .collect::<Option<Vec<_>>>()?
            .into_boxed_slice(),
        result,
    })
}

fn remap_terminator_kind(
    kind: &TerminatorEventKind,
    block_map: &[Option<BlockId>],
) -> Option<TerminatorEventKind> {
    match kind {
        TerminatorEventKind::Jump { target } => Some(TerminatorEventKind::Jump {
            target: mapped(block_map, target.index())?,
        }),
        TerminatorEventKind::Branch { targets } => Some(TerminatorEventKind::Branch {
            targets: remap_values(targets, |target| mapped(block_map, target.index()))?,
        }),
        TerminatorEventKind::Return => Some(TerminatorEventKind::Return),
        TerminatorEventKind::ReturnSuccess => Some(TerminatorEventKind::ReturnSuccess),
        TerminatorEventKind::ReturnError => Some(TerminatorEventKind::ReturnError),
        TerminatorEventKind::Break { target } => Some(TerminatorEventKind::Break {
            target: mapped(block_map, target.index())?,
        }),
        TerminatorEventKind::Continue { target } => Some(TerminatorEventKind::Continue {
            target: mapped(block_map, target.index())?,
        }),
        TerminatorEventKind::RuntimeFailure => Some(TerminatorEventKind::RuntimeFailure),
        TerminatorEventKind::AssertFailure => Some(TerminatorEventKind::AssertFailure),
    }
}

fn reachable_blocks(parts: &BorrowProblemParts) -> Vec<bool> {
    let mut reachable = vec![false; parts.blocks.len()];
    let mut frontier = vec![parts.entry];
    while let Some(block) = frontier.pop() {
        if reachable[block.index()] {
            continue;
        }
        reachable[block.index()] = true;
        for edge in parts.edges.iter().filter(|edge| edge.from == block) {
            frontier.push(edge.to);
        }
    }
    reachable
}

fn event_removal_rows(
    parts: &BorrowProblemParts,
    removed_events: &[EventId],
    mut keep: KeepRows,
) -> Option<KeepRows> {
    let mut dropped_loans = Vec::new();
    for event_id in removed_events {
        let event = parts.events.get(event_id.index())?;
        keep.events[event_id.index()] = false;
        match &event.kind {
            EventKind::Access { use_id } => keep.uses[use_id.index()] = false,
            EventKind::LoanIssue { loan } | EventKind::LoanKill { loan, .. } => {
                dropped_loans.push(*loan)
            }
            EventKind::CallArgument { .. } | EventKind::CallEffect(_) => return None,
            _ => {}
        }
    }

    for loan_id in dropped_loans {
        keep.loans[loan_id.index()] = false;
        for (index, event) in parts.events.iter().enumerate() {
            let matches_loan = match event.kind {
                EventKind::LoanIssue { loan } | EventKind::LoanKill { loan, .. } => loan == loan_id,
                _ => false,
            };
            if matches_loan {
                keep.events[index] = false;
            }
        }
    }
    Some(keep)
}

fn attempt_remove_unreachable_blocks(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);
    let reachable = reachable_blocks(&parts);
    for (block_index, is_reachable) in reachable.iter().enumerate() {
        if *is_reachable {
            continue;
        }
        let removed_events = parts.blocks[block_index].events.to_vec();
        let Some(mut keep) = event_removal_rows(&parts, &removed_events, KeepRows::all(&parts))
        else {
            continue;
        };
        keep.blocks[block_index] = false;
        for (point_index, point) in parts.points.iter().enumerate() {
            if point.block == parts.blocks[block_index].id {
                keep.points[point_index] = false;
            }
        }
        if let Some(candidate_parts) = remap_parts(parts.clone(), &keep)
            && let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)?
        {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn attempt_remove_events(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);
    for event in &parts.events {
        if matches!(
            event.kind,
            EventKind::Terminator { .. }
                | EventKind::CallArgument { .. }
                | EventKind::CallEffect(_)
                | EventKind::LoanIssue { .. }
                | EventKind::LoanKill { .. }
                | EventKind::Access { .. }
        ) {
            continue;
        }
        let Some(keep) = event_removal_rows(
            &parts,
            std::slice::from_ref(&event.id),
            KeepRows::all(&parts),
        ) else {
            return Ok(None);
        };
        let Some(candidate_parts) = remap_parts(parts.clone(), &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn attempt_remove_uses_and_loans(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);

    for use_row in &parts.uses {
        let Some(event_index) = parts.events.iter().position(
            |event| matches!(event.kind, EventKind::Access { use_id } if use_id == use_row.id),
        ) else {
            continue;
        };
        let mut keep = KeepRows::all(&parts);
        keep.uses[use_row.id.index()] = false;
        keep.events[event_index] = false;
        let mut candidate_parts = parts.clone();
        for loan in &mut candidate_parts.loans {
            loan.uses = loan
                .uses
                .iter()
                .copied()
                .filter(|use_id| keep.uses[use_id.index()])
                .collect();
        }
        let Some(candidate_parts) = remap_parts(candidate_parts, &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    for loan in &parts.loans {
        let mut keep = KeepRows::all(&parts);
        keep.loans[loan.id.index()] = false;
        for (event_index, event) in parts.events.iter().enumerate() {
            if matches!(
                event.kind,
                EventKind::LoanIssue { loan: event_loan }
                    | EventKind::LoanKill { loan: event_loan, .. }
                    if event_loan == loan.id
            ) {
                keep.events[event_index] = false;
            }
        }
        let Some(candidate_parts) = remap_parts(parts.clone(), &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

fn recompute_exits(parts: &mut BorrowProblemParts) {
    let mut has_outgoing = vec![false; parts.blocks.len()];
    for edge in &parts.edges {
        has_outgoing[edge.from.index()] = true;
    }
    parts.exits = parts
        .blocks
        .iter()
        .filter(|block| !has_outgoing[block.id.index()])
        .map(|block| block.id)
        .collect();
}

fn attempt_remove_edges(current: &ReductionState) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);
    for edge_index in 0..parts.edges.len() {
        let removed_edge = parts.edges[edge_index];
        let mut candidate_parts = parts.clone();
        candidate_parts.edges.remove(edge_index);
        let outgoing_targets = candidate_parts
            .edges
            .iter()
            .filter(|edge| edge.from == removed_edge.from)
            .map(|edge| edge.to)
            .collect::<Vec<_>>();
        let Some(block) = candidate_parts.blocks.get(removed_edge.from.index()) else {
            return Ok(None);
        };
        let Some(terminator_id) = block.events.last().copied() else {
            return Ok(None);
        };
        let Some(event) = candidate_parts.events.get_mut(terminator_id.index()) else {
            return Ok(None);
        };
        let EventKind::Terminator { kind } = &event.kind else {
            continue;
        };
        let Some(kind) = terminator_after_edge_removal(kind, &outgoing_targets) else {
            continue;
        };
        event.kind = EventKind::Terminator { kind };
        recompute_exits(&mut candidate_parts);
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

pub(super) fn terminator_after_edge_removal(
    kind: &TerminatorEventKind,
    outgoing_targets: &[BlockId],
) -> Option<TerminatorEventKind> {
    match kind {
        TerminatorEventKind::Branch { .. } => {
            if outgoing_targets.is_empty() {
                Some(TerminatorEventKind::Return)
            } else {
                let mut targets = outgoing_targets.to_vec();
                targets.sort_unstable();
                Some(TerminatorEventKind::Branch {
                    targets: targets.into_boxed_slice(),
                })
            }
        }
        TerminatorEventKind::Jump { .. } => match outgoing_targets {
            [] => Some(TerminatorEventKind::Return),
            [target] => Some(TerminatorEventKind::Jump { target: *target }),
            _ => None,
        },
        TerminatorEventKind::Break { .. } => match outgoing_targets {
            [] => Some(TerminatorEventKind::Return),
            [target] => Some(TerminatorEventKind::Break { target: *target }),
            _ => None,
        },
        TerminatorEventKind::Continue { .. } => match outgoing_targets {
            [] => Some(TerminatorEventKind::Return),
            [target] => Some(TerminatorEventKind::Continue { target: *target }),
            _ => None,
        },
        TerminatorEventKind::Return
        | TerminatorEventKind::ReturnSuccess
        | TerminatorEventKind::ReturnError
        | TerminatorEventKind::RuntimeFailure
        | TerminatorEventKind::AssertFailure => None,
    }
}

fn simpler_projection(projection: ProjectionElem) -> Option<ProjectionElem> {
    match projection {
        ProjectionElem::Field(0) => None,
        ProjectionElem::Field(_) | ProjectionElem::FixedIndex(_) => Some(ProjectionElem::Field(0)),
        ProjectionElem::DynamicIndex
        | ProjectionElem::CollectionElement
        | ProjectionElem::MapEntry => Some(ProjectionElem::Field(0)),
    }
}

fn attempt_simplify_projections(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);

    for (place_index, place) in parts.places.iter().enumerate() {
        for (projection_index, projection) in place.projections.iter().copied().enumerate() {
            let Some(simple) = simpler_projection(projection) else {
                continue;
            };
            let mut candidate_parts = parts.clone();
            let mut projections = candidate_parts.places[place_index].projections.to_vec();
            projections[projection_index] = simple;
            candidate_parts.places[place_index].projections = projections.into_boxed_slice();
            if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
                return Ok(Some(candidate));
            }
        }
    }

    for (origin_index, origin) in parts.origins.iter().enumerate() {
        let OriginKind::Projection { projection, .. } = origin.kind else {
            continue;
        };
        let Some(simple) = simpler_projection(projection) else {
            continue;
        };
        let mut candidate_parts = parts.clone();
        if let OriginKind::Projection { projection, .. } =
            &mut candidate_parts.origins[origin_index].kind
        {
            *projection = simple;
        }
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    for (event_index, event) in parts.events.iter().enumerate() {
        let EventKind::Aggregate { fields, .. } = &event.kind else {
            continue;
        };
        for (field_index, field) in fields.iter().enumerate() {
            let Some(simple) = simpler_projection(field.projection) else {
                continue;
            };
            let mut candidate_parts = parts.clone();
            if let EventKind::Aggregate {
                fields: candidate_fields,
                ..
            } = &mut candidate_parts.events[event_index].kind
            {
                let mut fields = candidate_fields.to_vec();
                fields[field_index].projection = simple;
                *candidate_fields = fields.into_boxed_slice();
            }
            if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
                return Ok(Some(candidate));
            }
        }
    }

    for (place_index, place) in parts.places.iter().enumerate() {
        for projection_index in 0..place.projections.len() {
            let mut candidate_parts = parts.clone();
            let mut projections = candidate_parts.places[place_index].projections.to_vec();
            projections.remove(projection_index);
            candidate_parts.places[place_index].projections = projections.into_boxed_slice();
            if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
                return Ok(Some(candidate));
            }
        }
    }

    Ok(None)
}

fn simpler_origin_kind(kind: &OriginKind) -> Option<OriginKind> {
    match kind {
        OriginKind::Parameter { index } if *index > 0 => Some(OriginKind::Parameter { index: 0 }),
        OriginKind::Parameter { .. } => None,
        OriginKind::Alias(origins) => {
            if origins.len() > 1 {
                Some(OriginKind::Alias(vec![origins[0]].into_boxed_slice()))
            } else {
                Some(OriginKind::Fresh)
            }
        }
        OriginKind::ExclusiveAlias(origins) => {
            if origins.len() > 1 {
                Some(OriginKind::ExclusiveAlias(
                    vec![origins[0]].into_boxed_slice(),
                ))
            } else {
                Some(OriginKind::Fresh)
            }
        }
        OriginKind::Copy(origins) => {
            if origins.len() > 1 {
                Some(OriginKind::Copy(vec![origins[0]].into_boxed_slice()))
            } else {
                Some(OriginKind::Fresh)
            }
        }
        OriginKind::Join(origins) => {
            if origins.len() > 1 {
                Some(OriginKind::Join(vec![origins[0]].into_boxed_slice()))
            } else {
                Some(OriginKind::Fresh)
            }
        }
        OriginKind::CallResult { call, provenance } => {
            let simpler_provenance = match provenance {
                CallResultProvenance::Alias(origins) if origins.len() > 1 => Some(
                    CallResultProvenance::Alias(vec![origins[0]].into_boxed_slice()),
                ),
                CallResultProvenance::Alias(origins) if origins.len() == 1 => {
                    Some(CallResultProvenance::Fresh)
                }
                CallResultProvenance::AliasParams(indices) if indices.len() > 1 => Some(
                    CallResultProvenance::AliasParams(vec![indices[0]].into_boxed_slice()),
                ),
                CallResultProvenance::AliasParams(indices) if indices.len() == 1 => {
                    Some(CallResultProvenance::Fresh)
                }
                CallResultProvenance::Unknown(_) => Some(CallResultProvenance::Fresh),
                CallResultProvenance::Fresh
                | CallResultProvenance::Alias(_)
                | CallResultProvenance::AliasParams(_) => None,
            }?;
            Some(OriginKind::CallResult {
                call: *call,
                provenance: simpler_provenance,
            })
        }
        OriginKind::Unknown | OriginKind::Fresh | OriginKind::Projection { .. } => None,
    }
}

fn origin_is_referenced(parts: &BorrowProblemParts, origin_id: ValueOriginId) -> bool {
    for origin in &parts.origins {
        if origin.id == origin_id {
            continue;
        }
        if origin_kind_references(&origin.kind, origin_id) {
            return true;
        }
    }
    if parts
        .loans
        .iter()
        .any(|loan| loan.origins.contains(&origin_id))
    {
        return true;
    }
    parts
        .events
        .iter()
        .any(|event| event_kind_references_origin(&event.kind, origin_id))
}

fn origin_kind_references(kind: &OriginKind, origin_id: ValueOriginId) -> bool {
    match kind {
        OriginKind::Alias(origins)
        | OriginKind::ExclusiveAlias(origins)
        | OriginKind::Copy(origins)
        | OriginKind::Join(origins) => origins.contains(&origin_id),
        OriginKind::Projection { source, .. } => *source == origin_id,
        OriginKind::CallResult { provenance, .. } => match provenance {
            CallResultProvenance::Alias(origins) => origins.contains(&origin_id),
            CallResultProvenance::AliasParams(_)
            | CallResultProvenance::Fresh
            | CallResultProvenance::Unknown(_) => false,
        },
        OriginKind::Unknown | OriginKind::Parameter { .. } | OriginKind::Fresh => false,
    }
}

fn event_kind_references_origin(kind: &EventKind, origin_id: ValueOriginId) -> bool {
    match kind {
        EventKind::Fresh { origin, .. }
        | EventKind::Copy { origin, .. }
        | EventKind::Projection { origin, .. }
        | EventKind::Aggregate { origin, .. } => *origin == origin_id,
        EventKind::Alias { origins, .. } | EventKind::ExclusiveAlias { origins, .. } => {
            origins.contains(&origin_id)
        }
        EventKind::Rebind { value, .. } => match value {
            RebindValue::Fresh(origin) => *origin == origin_id,
            RebindValue::Alias(origins) => origins.contains(&origin_id),
            RebindValue::AliasFromPlace(_) => false,
        },
        EventKind::CallEffect(effect) => effect
            .result
            .is_some_and(|result| result.origin == origin_id),
        EventKind::AliasFromPlace { .. }
        | EventKind::ExclusiveAliasFromPlace { .. }
        | EventKind::ScopeExit { .. }
        | EventKind::ReactiveObserve { .. }
        | EventKind::CallArgument { .. }
        | EventKind::Terminator { .. }
        | EventKind::Access { .. }
        | EventKind::LoanIssue { .. }
        | EventKind::LoanKill { .. } => false,
    }
}

fn attempt_reduce_origins(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);
    for (origin_index, origin) in parts.origins.iter().enumerate() {
        let Some(simple_kind) = simpler_origin_kind(&origin.kind) else {
            continue;
        };
        let mut candidate_parts = parts.clone();
        candidate_parts.origins[origin_index].kind = simple_kind;
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    for origin in &parts.origins {
        if origin_is_referenced(&parts, origin.id) {
            continue;
        }
        let mut keep = KeepRows::all(&parts);
        keep.origins[origin.id.index()] = false;
        let Some(candidate_parts) = remap_parts(parts.clone(), &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn place_is_referenced(parts: &BorrowProblemParts, place_id: PlaceId) -> bool {
    if parts.uses.iter().any(|use_row| use_row.place == place_id) {
        return true;
    }
    if parts
        .loans
        .iter()
        .any(|loan| loan.place == place_id || loan.holders.contains(&place_id))
    {
        return true;
    }
    parts
        .events
        .iter()
        .any(|event| event_kind_references_place(&event.kind, place_id))
}

fn event_kind_references_place(kind: &EventKind, place_id: PlaceId) -> bool {
    match kind {
        EventKind::Fresh { destination, .. } => *destination == place_id,
        EventKind::Alias {
            source,
            destination,
            ..
        }
        | EventKind::AliasFromPlace {
            source,
            destination,
        }
        | EventKind::ExclusiveAlias {
            source,
            destination,
            ..
        }
        | EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        }
        | EventKind::Copy {
            source,
            destination,
            ..
        }
        | EventKind::Projection {
            source,
            destination,
            ..
        } => *source == place_id || *destination == place_id,
        EventKind::Rebind { destination, value } => {
            *destination == place_id
                || matches!(value, RebindValue::AliasFromPlace(source) if *source == place_id)
        }
        EventKind::Aggregate {
            destination,
            fields,
            ..
        } => *destination == place_id || fields.iter().any(|field| field.source == place_id),
        EventKind::ReactiveObserve { place } => *place == place_id,
        EventKind::CallArgument { argument, .. } => argument.place == place_id,
        EventKind::CallEffect(effect) => {
            effect
                .arguments
                .iter()
                .any(|argument| argument.place == place_id)
                || effect.result.is_some_and(|result| result.place == place_id)
        }
        EventKind::ScopeExit { .. }
        | EventKind::Terminator { .. }
        | EventKind::Access { .. }
        | EventKind::LoanIssue { .. }
        | EventKind::LoanKill { .. } => false,
    }
}

fn binding_is_referenced(parts: &BorrowProblemParts, binding_id: BindingId) -> bool {
    parts.places.iter().any(|place| place.root == binding_id)
        || parts.events.iter().any(|event| {
            matches!(&event.kind, EventKind::ScopeExit { bindings } if bindings.contains(&binding_id))
        })
}

fn attempt_reduce_bindings(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);

    for (place_index, place) in parts.places.iter().enumerate() {
        if place_is_referenced(&parts, place.id) {
            continue;
        }
        let mut keep = KeepRows::all(&parts);
        keep.places[place_index] = false;
        let Some(candidate_parts) = remap_parts(parts.clone(), &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    for (binding_index, binding) in parts.bindings.iter().enumerate() {
        if !binding.mutable && !binding.compiler_temporary {
            continue;
        }
        let mut candidate_parts = parts.clone();
        candidate_parts.bindings[binding_index].mutable = false;
        candidate_parts.bindings[binding_index].compiler_temporary = false;
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    for binding in &parts.bindings {
        if binding_is_referenced(&parts, binding.id) {
            continue;
        }
        let mut keep = KeepRows::all(&parts);
        keep.bindings[binding.id.index()] = false;
        let Some(candidate_parts) = remap_parts(parts.clone(), &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn call_is_referenced(parts: &BorrowProblemParts, call_id: CallId) -> bool {
    parts.events.iter().any(|event| match &event.kind {
        EventKind::CallArgument { call, .. } => *call == call_id,
        EventKind::CallEffect(effect) => effect.call == call_id,
        _ => false,
    }) || parts.origins.iter().any(
        |origin| matches!(&origin.kind, OriginKind::CallResult { call, .. } if *call == call_id),
    )
}

fn origin_referenced_except_event(
    parts: &BorrowProblemParts,
    origin_id: ValueOriginId,
    excluded_event: EventId,
) -> bool {
    parts.events.iter().any(|event| {
        event.id != excluded_event && event_kind_references_origin(&event.kind, origin_id)
    }) || parts
        .loans
        .iter()
        .any(|loan| loan.origins.contains(&origin_id))
        || parts
            .origins
            .iter()
            .any(|origin| origin.id != origin_id && origin_kind_references(&origin.kind, origin_id))
}

fn attempt_simplify_call_arguments(
    current: &ReductionState,
    parts: &BorrowProblemParts,
) -> Result<Option<ReductionState>, CompilerError> {
    for (event_index, event) in parts.events.iter().enumerate() {
        let EventKind::CallEffect(effect) = &event.kind else {
            continue;
        };
        let Some(last_argument) = effect.arguments.last() else {
            continue;
        };
        let argument_index = effect.arguments.len() - 1;
        let Some(argument_event_index) = parts.events.iter().position(|argument_event| {
            matches!(
                &argument_event.kind,
                EventKind::CallArgument {
                    call,
                    index,
                    argument
                } if *call == effect.call
                    && *index as usize == argument_index
                    && argument == last_argument
            )
        }) else {
            continue;
        };
        let mut candidate_parts = parts.clone();
        if let EventKind::CallEffect(candidate_effect) =
            &mut candidate_parts.events[event_index].kind
        {
            let mut arguments = candidate_effect.arguments.to_vec();
            arguments.pop();
            let argument_count = arguments.len();
            candidate_effect.arguments = arguments.into_boxed_slice();
            if let Some(result) = candidate_effect.result
                && let Some(origin) = candidate_parts.origins.get_mut(result.origin.index())
                && let OriginKind::CallResult { provenance, .. } = &mut origin.kind
                && matches!(
                    provenance,
                    CallResultProvenance::AliasParams(indices)
                        if indices.iter().any(|index| *index >= argument_count)
                )
            {
                *provenance = CallResultProvenance::Fresh;
            }
        }
        let mut keep = KeepRows::all(&candidate_parts);
        keep.events[argument_event_index] = false;
        let removed_use = match &parts.events[argument_event_index].kind {
            EventKind::CallArgument { argument, .. } => argument.use_id,
            _ => continue,
        };
        keep.uses[removed_use.index()] = false;
        for loan in &mut candidate_parts.loans {
            loan.uses = loan
                .uses
                .iter()
                .copied()
                .filter(|use_id| *use_id != removed_use)
                .collect();
        }
        let Some(candidate_parts) = remap_parts(candidate_parts, &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn attempt_replace_calls_with_simpler_effects(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);

    for (call_index, call) in parts.calls.iter().enumerate() {
        if call.label.is_empty() {
            continue;
        }
        let mut candidate_parts = parts.clone();
        candidate_parts.calls[call_index].label.clear();
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    if let Some(candidate) = attempt_simplify_call_arguments(current, &parts)? {
        return Ok(Some(candidate));
    }

    for (event_index, event) in parts.events.iter().enumerate() {
        let EventKind::CallEffect(effect) = &event.kind else {
            continue;
        };
        let Some(result) = effect.result else {
            continue;
        };
        if origin_referenced_except_event(&parts, result.origin, event.id) {
            continue;
        }
        let mut candidate_parts = parts.clone();
        if let EventKind::CallEffect(candidate_effect) =
            &mut candidate_parts.events[event_index].kind
        {
            candidate_effect.result = None;
        }
        let mut keep = KeepRows::all(&parts);
        keep.origins[result.origin.index()] = false;
        let Some(candidate_parts) = remap_parts(candidate_parts, &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }

    for call in &parts.calls {
        if call_is_referenced(&parts, call.id) {
            continue;
        }
        let mut keep = KeepRows::all(&parts);
        keep.calls[call.id.index()] = false;
        let Some(candidate_parts) = remap_parts(parts.clone(), &keep) else {
            return Ok(None);
        };
        if let Some(candidate) = try_candidate(current, candidate_parts, current.bounds)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn lower_bound_candidates(value: usize) -> [Option<usize>; 2] {
    match value {
        0 | 1 => [None, None],
        2 => [Some(1), None],
        value => [Some(value / 2), Some(value - 1)],
    }
}

fn attempt_lower_loop_bounds(
    current: &ReductionState,
) -> Result<Option<ReductionState>, CompilerError> {
    let parts = problem_parts(&current.problem);
    let bound_values = [
        current.bounds.max_executions,
        current.bounds.max_executed_events,
        current.bounds.max_block_entries,
        current.bounds.max_dynamic_generations,
    ];
    for (bound_index, value) in bound_values.into_iter().enumerate() {
        for target in lower_bound_candidates(value).into_iter().flatten() {
            let mut candidate_bounds = current.bounds;
            match bound_index {
                0 => candidate_bounds.max_executions = target,
                1 => candidate_bounds.max_executed_events = target,
                2 => candidate_bounds.max_block_entries = target,
                3 => candidate_bounds.max_dynamic_generations = target,
                _ => unreachable!("bound index is fixed by the four bound fields"),
            }
            if let Some(candidate) = try_candidate(current, parts.clone(), candidate_bounds)? {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

pub(crate) fn render_fixture_skeleton(problem: &BorrowProblem, bounds: OracleBounds) -> String {
    let mut output =
        String::from("fn reduced_boracle_problem() -> (BorrowProblem, OracleBounds) {\n");
    output.push_str(
        "    // Inspection-only HIR locals, regions, and binding/point/event source provenance are omitted.\n",
    );
    output.push_str("    let problem = BorrowProblem::new(BorrowProblemParts {\n");
    render_rows(&mut output, problem);
    output.push_str("    })\n");
    output.push_str("        .expect(\"reduced Boracle problem should validate\");\n");
    output.push_str(&format!(
        "    let bounds = OracleBounds::new({}, {}, {}, {});\n",
        bounds.max_executions,
        bounds.max_executed_events,
        bounds.max_block_entries,
        bounds.max_dynamic_generations,
    ));
    output.push_str("    (problem, bounds)\n");
    output.push_str("}\n");
    output
}

fn render_rows(output: &mut String, problem: &BorrowProblem) {
    let flow = problem.control_flow();
    render_list_field(output, 2, "bindings", problem.bindings(), render_binding);
    render_list_field(output, 2, "points", problem.points(), render_point);
    render_list_field(output, 2, "blocks", &flow.blocks, render_block);
    render_list_field(output, 2, "edges", &flow.edges, render_edge);
    render_line(
        output,
        2,
        &format!("entry: {},", render_block_id(flow.entry)),
    );
    render_list_field(output, 2, "exits", &flow.exits, |id| render_block_id(*id));
    render_list_field(output, 2, "places", problem.places(), render_place);
    render_list_field(output, 2, "origins", problem.origins(), render_origin);
    render_list_field(output, 2, "loans", problem.loans(), render_loan);
    render_list_field(output, 2, "uses", problem.uses(), render_use);
    render_list_field(output, 2, "calls", problem.calls(), render_call);
    render_list_field(output, 2, "events", problem.events(), render_event);
}

fn render_line(output: &mut String, indent: usize, line: &str) {
    output.push_str(&"    ".repeat(indent));
    output.push_str(line);
    output.push('\n');
}

fn render_list_field<T>(
    output: &mut String,
    indent: usize,
    name: &str,
    values: &[T],
    render: impl Fn(&T) -> String,
) {
    render_line(output, indent, &format!("{name}: vec!["));
    for value in values {
        render_line(output, indent + 1, &format!("{},", render(value)));
    }
    render_line(output, indent, "],");
}

fn render_boxed<T>(values: &[T], render: impl Fn(&T) -> String) -> String {
    let values = values.iter().map(render).collect::<Vec<_>>().join(", ");
    format!("vec![{values}].into_boxed_slice()")
}

fn render_binding(binding: &Binding) -> String {
    if binding.hir_local.is_none()
        && binding.region.is_none()
        && !binding.mutable
        && !binding.compiler_temporary
    {
        return format!("Binding::synthetic({})", render_binding_id(binding.id));
    }
    format!(
        "Binding::new({}, None, None, {}, {}, EventSource::none())",
        render_binding_id(binding.id),
        binding.mutable,
        binding.compiler_temporary
    )
}

fn render_point(point: &ProgramPoint) -> String {
    format!(
        "ProgramPoint::new({}, {}, {})",
        render_point_id(point.id),
        render_block_id(point.block),
        point.ordinal
    )
}

fn render_block(block: &CfgBlock) -> String {
    format!(
        "CfgBlock::new({}, {}, {}, {})",
        render_block_id(block.id),
        render_point_id(block.entry),
        render_point_id(block.exit),
        render_boxed(&block.events, |id| render_event_id(*id))
            .replace(".into_boxed_slice()", ".into_iter().collect()")
    )
}

fn render_edge(edge: &CfgEdge) -> String {
    format!(
        "CfgEdge::new({}, {})",
        render_block_id(edge.from),
        render_block_id(edge.to)
    )
}

fn render_place(place: &Place) -> String {
    format!(
        "Place::new({}, {}, {})",
        render_place_id(place.id),
        render_binding_id(place.root),
        render_vec(&place.projections, |projection| render_projection(
            *projection
        ))
    )
}

fn render_origin(origin: &ValueOrigin) -> String {
    format!(
        "ValueOrigin::new({}, {})",
        render_origin_id(origin.id),
        render_origin_kind(&origin.kind)
    )
}

fn render_loan(loan: &Loan) -> String {
    format!(
        "Loan {{ id: {}, kind: {}, issued_at: {}, place: {}, origins: {}, holders: {}, uses: {}, kills: {} }}",
        render_loan_id(loan.id),
        render_access_kind(loan.kind),
        render_point_id(loan.issued_at),
        render_place_id(loan.place),
        render_boxed(&loan.origins, |id| render_origin_id(*id)),
        render_boxed(&loan.holders, |id| render_place_id(*id)),
        render_boxed(&loan.uses, |id| render_use_id(*id)),
        render_boxed(&loan.kills, |id| render_point_id(*id))
    )
}

fn render_use(use_row: &Use) -> String {
    format!(
        "Use {{ id: {}, point: {}, place: {}, kind: {}, definition: {} }}",
        render_use_id(use_row.id),
        render_point_id(use_row.point),
        render_place_id(use_row.place),
        render_use_kind(use_row.kind),
        use_row.definition
    )
}

fn render_call(call: &Call) -> String {
    format!(
        "Call {{ id: {}, label: {:?}.to_string() }}",
        render_call_id(call.id),
        call.label
    )
}

fn render_event(event: &Event) -> String {
    format!(
        "Event::new({}, {}, {}, EventSource::none())",
        render_event_id(event.id),
        render_point_id(event.point),
        render_event_kind(&event.kind)
    )
}

fn render_event_kind(kind: &EventKind) -> String {
    match kind {
        EventKind::Fresh {
            destination,
            origin,
        } => format!(
            "EventKind::Fresh {{ destination: {}, origin: {} }}",
            render_place_id(*destination),
            render_origin_id(*origin)
        ),
        EventKind::Alias {
            source,
            destination,
            origins,
        } => format!(
            "EventKind::Alias {{ source: {}, destination: {}, origins: {} }}",
            render_place_id(*source),
            render_place_id(*destination),
            render_boxed(origins, |id| render_origin_id(*id))
        ),
        EventKind::AliasFromPlace {
            source,
            destination,
        } => format!(
            "EventKind::AliasFromPlace {{ source: {}, destination: {} }}",
            render_place_id(*source),
            render_place_id(*destination)
        ),
        EventKind::ExclusiveAlias {
            source,
            destination,
            origins,
        } => format!(
            "EventKind::ExclusiveAlias {{ source: {}, destination: {}, origins: {} }}",
            render_place_id(*source),
            render_place_id(*destination),
            render_boxed(origins, |id| render_origin_id(*id))
        ),
        EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => format!(
            "EventKind::ExclusiveAliasFromPlace {{ source: {}, destination: {} }}",
            render_place_id(*source),
            render_place_id(*destination)
        ),
        EventKind::Copy {
            source,
            destination,
            origin,
        } => format!(
            "EventKind::Copy {{ source: {}, destination: {}, origin: {} }}",
            render_place_id(*source),
            render_place_id(*destination),
            render_origin_id(*origin)
        ),
        EventKind::Projection {
            source,
            destination,
            origin,
        } => format!(
            "EventKind::Projection {{ source: {}, destination: {}, origin: {} }}",
            render_place_id(*source),
            render_place_id(*destination),
            render_origin_id(*origin)
        ),
        EventKind::Rebind { destination, value } => format!(
            "EventKind::Rebind {{ destination: {}, value: {} }}",
            render_place_id(*destination),
            render_rebind_value(value)
        ),
        EventKind::Aggregate {
            destination,
            origin,
            fields,
        } => format!(
            "EventKind::Aggregate {{ destination: {}, origin: {}, fields: {} }}",
            render_place_id(*destination),
            render_origin_id(*origin),
            render_boxed(fields, render_aggregate_field)
        ),
        EventKind::ScopeExit { bindings } => format!(
            "EventKind::ScopeExit {{ bindings: {} }}",
            render_boxed(bindings, |id| render_binding_id(*id))
        ),
        EventKind::ReactiveObserve { place } => format!(
            "EventKind::ReactiveObserve {{ place: {} }}",
            render_place_id(*place)
        ),
        EventKind::CallArgument {
            call,
            index,
            argument,
        } => format!(
            "EventKind::CallArgument {{ call: {}, index: {}, argument: {} }}",
            render_call_id(*call),
            index,
            render_call_argument(argument)
        ),
        EventKind::Terminator { kind } => format!(
            "EventKind::Terminator {{ kind: {} }}",
            render_terminator(kind)
        ),
        EventKind::CallEffect(effect) => {
            format!("EventKind::CallEffect({})", render_call_effect(effect))
        }
        EventKind::Access { use_id } => {
            format!("EventKind::Access {{ use_id: {} }}", render_use_id(*use_id))
        }
        EventKind::LoanIssue { loan } => {
            format!("EventKind::LoanIssue {{ loan: {} }}", render_loan_id(*loan))
        }
        EventKind::LoanKill { loan, reason } => format!(
            "EventKind::LoanKill {{ loan: {}, reason: {} }}",
            render_loan_id(*loan),
            render_kill_reason(*reason)
        ),
    }
}

fn render_aggregate_field(field: &AggregateField) -> String {
    format!(
        "AggregateField {{ projection: {}, source: {} }}",
        render_projection(field.projection),
        render_place_id(field.source)
    )
}

fn render_call_argument(argument: &CallArgument) -> String {
    format!(
        "CallArgument {{ place: {}, access: {}, use_id: {} }}",
        render_place_id(argument.place),
        render_access_kind(argument.access),
        render_use_id(argument.use_id)
    )
}

fn render_call_result(result: CallResult) -> String {
    format!(
        "CallResult {{ place: {}, origin: {} }}",
        render_place_id(result.place),
        render_origin_id(result.origin)
    )
}

fn render_call_effect(effect: &CallEffect) -> String {
    let result = effect
        .result
        .map(render_call_result)
        .map(|result| format!("Some({result})"))
        .unwrap_or_else(|| "None".to_string());
    format!(
        "CallEffect {{ call: {}, arguments: {}, result: {} }}",
        render_call_id(effect.call),
        render_boxed(&effect.arguments, render_call_argument),
        result
    )
}

fn render_rebind_value(value: &RebindValue) -> String {
    match value {
        RebindValue::Fresh(origin) => format!("RebindValue::Fresh({})", render_origin_id(*origin)),
        RebindValue::Alias(origins) => format!(
            "RebindValue::Alias({})",
            render_boxed(origins, |id| render_origin_id(*id))
        ),
        RebindValue::AliasFromPlace(place) => {
            format!("RebindValue::AliasFromPlace({})", render_place_id(*place))
        }
    }
}

fn render_origin_kind(kind: &OriginKind) -> String {
    match kind {
        OriginKind::Unknown => "OriginKind::Unknown".to_string(),
        OriginKind::Parameter { index } => format!("OriginKind::Parameter {{ index: {index} }}"),
        OriginKind::Fresh => "OriginKind::Fresh".to_string(),
        OriginKind::Alias(origins) => {
            format!(
                "OriginKind::Alias({})",
                render_boxed(origins, |id| render_origin_id(*id))
            )
        }
        OriginKind::ExclusiveAlias(origins) => format!(
            "OriginKind::ExclusiveAlias({})",
            render_boxed(origins, |id| render_origin_id(*id))
        ),
        OriginKind::Copy(origins) => {
            format!(
                "OriginKind::Copy({})",
                render_boxed(origins, |id| render_origin_id(*id))
            )
        }
        OriginKind::Projection { source, projection } => format!(
            "OriginKind::Projection {{ source: {}, projection: {} }}",
            render_origin_id(*source),
            render_projection(*projection)
        ),
        OriginKind::Join(origins) => {
            format!(
                "OriginKind::Join({})",
                render_boxed(origins, |id| render_origin_id(*id))
            )
        }
        OriginKind::CallResult { call, provenance } => format!(
            "OriginKind::CallResult {{ call: {}, provenance: {} }}",
            render_call_id(*call),
            render_call_result_provenance(provenance)
        ),
    }
}

fn render_call_result_provenance(provenance: &CallResultProvenance) -> String {
    match provenance {
        CallResultProvenance::Fresh => "CallResultProvenance::Fresh".to_string(),
        CallResultProvenance::Alias(origins) => format!(
            "CallResultProvenance::Alias({})",
            render_boxed(origins, |id| render_origin_id(*id))
        ),
        CallResultProvenance::AliasParams(indices) => format!(
            "CallResultProvenance::AliasParams({})",
            render_boxed(indices, |index| index.to_string())
        ),
        CallResultProvenance::Unknown(reason) => format!(
            "CallResultProvenance::Unknown({})",
            render_call_result_unknown_reason(*reason)
        ),
    }
}

fn render_terminator(kind: &TerminatorEventKind) -> String {
    match kind {
        TerminatorEventKind::Jump { target } => {
            format!(
                "TerminatorEventKind::Jump {{ target: {} }}",
                render_block_id(*target)
            )
        }
        TerminatorEventKind::Branch { targets } => format!(
            "TerminatorEventKind::Branch {{ targets: {} }}",
            render_boxed(targets, |id| render_block_id(*id))
        ),
        TerminatorEventKind::Return => "TerminatorEventKind::Return".to_string(),
        TerminatorEventKind::ReturnSuccess => "TerminatorEventKind::ReturnSuccess".to_string(),
        TerminatorEventKind::ReturnError => "TerminatorEventKind::ReturnError".to_string(),
        TerminatorEventKind::Break { target } => {
            format!(
                "TerminatorEventKind::Break {{ target: {} }}",
                render_block_id(*target)
            )
        }
        TerminatorEventKind::Continue { target } => format!(
            "TerminatorEventKind::Continue {{ target: {} }}",
            render_block_id(*target)
        ),
        TerminatorEventKind::RuntimeFailure => "TerminatorEventKind::RuntimeFailure".to_string(),
        TerminatorEventKind::AssertFailure => "TerminatorEventKind::AssertFailure".to_string(),
    }
}

fn render_vec<T>(values: &[T], render: impl Fn(&T) -> String) -> String {
    format!(
        "vec![{}]",
        values.iter().map(render).collect::<Vec<_>>().join(", ")
    )
}

fn render_projection(projection: ProjectionElem) -> String {
    match projection {
        ProjectionElem::Field(index) => format!("ProjectionElem::Field({index})"),
        ProjectionElem::FixedIndex(index) => format!("ProjectionElem::FixedIndex({index})"),
        ProjectionElem::DynamicIndex => "ProjectionElem::DynamicIndex".to_string(),
        ProjectionElem::CollectionElement => "ProjectionElem::CollectionElement".to_string(),
        ProjectionElem::MapEntry => "ProjectionElem::MapEntry".to_string(),
    }
}

fn render_use_kind(kind: UseKind) -> String {
    match kind {
        UseKind::Read => "UseKind::Read".to_string(),
        UseKind::Write => "UseKind::Write".to_string(),
        UseKind::LoanObservation => "UseKind::LoanObservation".to_string(),
    }
}

fn render_access_kind(kind: AccessKind) -> String {
    match kind {
        AccessKind::Shared => "AccessKind::Shared".to_string(),
        AccessKind::Exclusive => "AccessKind::Exclusive".to_string(),
    }
}

fn render_kill_reason(reason: KillReason) -> String {
    match reason {
        KillReason::FinalUse => "KillReason::FinalUse".to_string(),
        KillReason::Rebind => "KillReason::Rebind".to_string(),
        KillReason::ScopeExit => "KillReason::ScopeExit".to_string(),
        KillReason::UnreachableContinuation => "KillReason::UnreachableContinuation".to_string(),
        KillReason::Explicit => "KillReason::Explicit".to_string(),
    }
}

fn render_call_result_unknown_reason(
    reason: crate::compiler_frontend::analysis::borrow_checker::problem::CallResultUnknownReason,
) -> String {
    match reason {
        crate::compiler_frontend::analysis::borrow_checker::problem::CallResultUnknownReason::SummaryUnknown => {
            "CallResultUnknownReason::SummaryUnknown".to_string()
        }
        crate::compiler_frontend::analysis::borrow_checker::problem::CallResultUnknownReason::MissingSummary => {
            "CallResultUnknownReason::MissingSummary".to_string()
        }
        crate::compiler_frontend::analysis::borrow_checker::problem::CallResultUnknownReason::OpaqueExternal => {
            "CallResultUnknownReason::OpaqueExternal".to_string()
        }
    }
}

fn render_binding_id(id: BindingId) -> String {
    format!("BindingId::new({})", id.raw())
}

fn render_block_id(id: BlockId) -> String {
    format!("BlockId::new({})", id.raw())
}

fn render_event_id(id: EventId) -> String {
    format!("EventId::new({})", id.raw())
}

fn render_loan_id(id: LoanId) -> String {
    format!("LoanId::new({})", id.raw())
}

fn render_place_id(id: PlaceId) -> String {
    format!("PlaceId::new({})", id.raw())
}

fn render_point_id(id: PointId) -> String {
    format!("PointId::new({})", id.raw())
}

fn render_origin_id(id: ValueOriginId) -> String {
    format!("ValueOriginId::new({})", id.raw())
}

fn render_use_id(id: UseId) -> String {
    format!("UseId::new({})", id.raw())
}

fn render_call_id(id: CallId) -> String {
    format!("CallId::new({})", id.raw())
}
