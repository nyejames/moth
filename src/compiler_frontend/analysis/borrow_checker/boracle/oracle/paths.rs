//! Deterministic bounded path enumeration for the operational oracle.
//!
//! WHAT: owns the explicit depth-first frontier, per-path entry checks, the retained first
//!       complete safe trace and bounds spanning executions.
//! WHY: each concrete successor gets an independent dynamic state; no predecessor states are ever
//!      merged or widened.

use super::conflicts;
use super::execute::{
    EventCursor, EventExecutionResult, OracleExecutionContext, execute_block, oracle_error,
    require_block, validate_branch_targets,
};
use super::state::{OracleState, PlaceIndex};
use super::traces::{ExecutionTrace, TraceBuilder};
use super::{OracleBounds, OracleLimitReason, OracleOutcome};
use crate::compiler_frontend::analysis::borrow_checker::problem::{BlockId, BorrowProblem};
use crate::compiler_frontend::compiler_errors::CompilerError;

struct PathFrame {
    cursor: EventCursor,
    state: OracleState,
    trace: TraceBuilder,
}

impl PathFrame {
    fn new(block: BlockId, state: OracleState, trace: TraceBuilder) -> Self {
        Self {
            cursor: EventCursor {
                block,
                event_index: 0,
            },
            state,
            trace,
        }
    }
}

pub(crate) fn execute_bounded(
    problem: &BorrowProblem,
    bounds: OracleBounds,
) -> Result<OracleOutcome, CompilerError> {
    problem.validate()?;

    validate_branch_targets(problem)?;

    let place_index = PlaceIndex::new(problem);
    let state = OracleState::new(problem);
    let trace = TraceBuilder::new();
    let entry = problem.control_flow().entry;
    require_block(problem, entry)?;

    let mut frontier = vec![PathFrame::new(entry, state, trace)];
    let mut enumerated_executions = 0;
    let mut completed_executions = 0;
    let mut explored_events = 0;
    let mut first_truncation = None;
    let mut first_safe_trace: Option<ExecutionTrace> = None;

    while let Some(mut frame) = frontier.pop() {
        require_block(problem, frame.cursor.block)?;
        if frame.state.is_repeated_block_entry(frame.cursor.block) {
            enumerated_executions += 1;
            remember_truncation(
                &mut first_truncation,
                OracleLimitReason::NonTerminatingCycle {
                    block: frame.cursor.block,
                },
            );
            continue;
        }
        if enumerated_executions >= bounds.max_executions {
            remember_truncation(
                &mut first_truncation,
                OracleLimitReason::ExecutionBound {
                    limit: bounds.max_executions,
                },
            );
            break;
        }

        if let Some(reason) = frame
            .state
            .enter_block(frame.cursor.block, bounds.max_block_entries)
        {
            enumerated_executions += 1;
            remember_truncation(&mut first_truncation, reason);
            continue;
        }

        let result = {
            let mut context = OracleExecutionContext {
                problem,
                place_index: &place_index,
                state: &mut frame.state,
                trace: &mut frame.trace,
                explored_events: &mut explored_events,
                trace_index: 0,
                bounds,
            };
            execute_block(&mut context, frame.cursor)?
        };

        match result {
            EventExecutionResult::NextBlock(target) => {
                frame.cursor = EventCursor {
                    block: target,
                    event_index: 0,
                };
                frontier.push(frame);
            }
            EventExecutionResult::NextBlocks(targets) => {
                let Some((&first, rest)) = targets.split_first() else {
                    return Err(oracle_error(format!(
                        "block {:?} produced no successor blocks",
                        frame.cursor.block
                    )));
                };
                // Deeper pushes are popped later, so the lowest target is explored first. The
                // frame itself carries the lowest target, which keeps a single successor free of
                // any state or trace copy.
                for target in rest.iter().rev().copied() {
                    frontier.push(PathFrame::new(
                        target,
                        frame.state.clone(),
                        frame.trace.clone(),
                    ));
                }
                frame.cursor = EventCursor {
                    block: first,
                    event_index: 0,
                };
                frontier.push(frame);
            }
            EventExecutionResult::Complete => {
                enumerated_executions += 1;
                match conflicts::find_interval_conflict(
                    frame.trace.entries(),
                    &frame.state.capabilities,
                ) {
                    Ok(Some(witness)) => {
                        let trace = frame.trace.finish(
                            &frame.state.capabilities,
                            frame.state.block_entry_counts(),
                            Some(witness),
                        );
                        return Ok(OracleOutcome::RuntimeConflict { trace });
                    }
                    Ok(None) => {
                        completed_executions += 1;
                        // Capture the first complete conflict-free trace before the frame drops.
                        // If a later sibling truncates, only the observed count is published with
                        // the inconclusive outcome, and no truncated run is ever classified safe.
                        if first_safe_trace.is_none() {
                            first_safe_trace = Some(frame.trace.finish(
                                &frame.state.capabilities,
                                frame.state.block_entry_counts(),
                                None,
                            ));
                        }
                    }
                    Err(reason) => remember_truncation(&mut first_truncation, reason),
                }
            }
            EventExecutionResult::RuntimeConflict(witness) => {
                let trace = frame.trace.finish(
                    &frame.state.capabilities,
                    frame.state.block_entry_counts(),
                    Some(witness),
                );
                return Ok(OracleOutcome::RuntimeConflict { trace });
            }
            EventExecutionResult::Inconclusive(reason) => {
                enumerated_executions += 1;
                remember_truncation(&mut first_truncation, reason);
            }
            EventExecutionResult::Continue => {
                return Err(oracle_error(format!(
                    "block {:?} execution stopped without a terminator",
                    frame.cursor.block
                )));
            }
        }
    }

    if let Some(reason) = first_truncation {
        return Ok(OracleOutcome::Inconclusive {
            reason,
            explored: explored_events,
            completed_executions,
        });
    }

    // Without any truncation the enumeration always completed at least one conflict-free path,
    // so the representative trace exists; a missing one is an internal invariant break rather
    // than a silent empty report.
    let Some(trace) = first_safe_trace else {
        return Err(oracle_error(
            "bounded path enumeration finished without truncation or a complete conflict-free \
             execution",
        ));
    };

    Ok(OracleOutcome::CompleteSafe {
        executions: completed_executions,
        trace,
    })
}

fn remember_truncation(first: &mut Option<OracleLimitReason>, reason: OracleLimitReason) {
    if first.is_none() {
        *first = Some(reason);
    }
}
