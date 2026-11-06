//! Pure AST control-flow exit analysis.
//!
//! WHAT: summarises every reachable way a statement sequence can leave its current position:
//! falling through to the next statement, producing values for an enclosing value block,
//! returning to the caller, diverging, or targeting the nearest loop with `break`/`continue`.
//! WHY: function terminality, value-production completeness and loop scanning all depend on the
//! same control-flow facts. One owner keeps branch union, statement sequencing and nearest-loop
//! exit consumption consistent instead of duplicating conservative walkers per consumer.
//!
//! The walk is deliberately structural. It evaluates only folded `Bool` conditions from the
//! shared constant store; every dynamic condition stays a possible exit. Consumers own their
//! diagnostic policy and ask named questions such as `may_complete_normally` or
//! `is_value_producing_branch_complete`.
//!
//! Loop accounting follows Moth's unlabelled nearest-loop rule: `break` and `continue` set facts
//! on the body summary, and the owning loop consumes them at its boundary. A nested loop
//! therefore cannot leak its exits into an outer loop. Range and collection loops stay
//! conservative because their iteration count can be zero.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionStore,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;

/// Independent facts about every reachable exit from a statement sequence.
///
/// WHAT: keeps fallthrough, value production, caller returns, divergence and nearest-loop exits
/// separate. A branch may both produce on one path and return on another, so no single
/// classification can represent the summary.
/// WHY: function terminality needs fallthrough, value completeness accepts produce-or-exit,
/// declared Never bodies need all-path divergence, and loop boundaries need to consume only the
/// break/continue facts that target them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControlFlowExitSummary {
    pub(crate) can_fall_through: bool,
    pub(crate) produces_value: bool,
    pub(crate) returns_to_caller: bool,
    pub(crate) diverges: bool,
    pub(crate) breaks_current_loop: bool,
    pub(crate) continues_current_loop: bool,
}

impl ControlFlowExitSummary {
    /// An empty or ordinary statement sequence: control reaches the next statement.
    pub(crate) const FALLS_THROUGH: Self = Self {
        can_fall_through: true,
        produces_value: false,
        returns_to_caller: false,
        diverges: false,
        breaks_current_loop: false,
        continues_current_loop: false,
    };

    /// A `then` statement produces values and does not continue in the same sequence.
    pub(crate) const PRODUCES_VALUE: Self = Self {
        can_fall_through: false,
        produces_value: true,
        returns_to_caller: false,
        diverges: false,
        breaks_current_loop: false,
        continues_current_loop: false,
    };

    /// `return` or `return!` hands control back to the caller.
    pub(crate) const RETURNS_TO_CALLER: Self = Self {
        can_fall_through: false,
        produces_value: false,
        returns_to_caller: true,
        diverges: false,
        breaks_current_loop: false,
        continues_current_loop: false,
    };

    /// A folded-false assertion or a proven non-exiting loop stops the path.
    pub(crate) const DIVERGES: Self = Self {
        can_fall_through: false,
        produces_value: false,
        returns_to_caller: false,
        diverges: true,
        breaks_current_loop: false,
        continues_current_loop: false,
    };

    /// `break` resumes after the nearest loop.
    pub(crate) const BREAKS_CURRENT_LOOP: Self = Self {
        can_fall_through: false,
        produces_value: false,
        returns_to_caller: false,
        diverges: false,
        breaks_current_loop: true,
        continues_current_loop: false,
    };

    /// `continue` starts the next iteration of the nearest loop.
    pub(crate) const CONTINUES_CURRENT_LOOP: Self = Self {
        can_fall_through: false,
        produces_value: false,
        returns_to_caller: false,
        diverges: false,
        breaks_current_loop: false,
        continues_current_loop: true,
    };

    /// Unions alternative branches such as `if`/`else` or match arms.
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            can_fall_through: self.can_fall_through || other.can_fall_through,
            produces_value: self.produces_value || other.produces_value,
            returns_to_caller: self.returns_to_caller || other.returns_to_caller,
            diverges: self.diverges || other.diverges,
            breaks_current_loop: self.breaks_current_loop || other.breaks_current_loop,
            continues_current_loop: self.continues_current_loop || other.continues_current_loop,
        }
    }

    /// Sequences the next statement only onto paths that still fall through.
    ///
    /// WHY: once every path leaves, later statements are unreachable and must not contribute
    /// exits, values or diagnostic facts to the sequence summary.
    pub(crate) fn then_sequence(self, next: Self) -> Self {
        if !self.can_fall_through {
            return self;
        }

        Self {
            can_fall_through: next.can_fall_through,
            produces_value: self.produces_value || next.produces_value,
            returns_to_caller: self.returns_to_caller || next.returns_to_caller,
            diverges: self.diverges || next.diverges,
            breaks_current_loop: self.breaks_current_loop || next.breaks_current_loop,
            continues_current_loop: self.continues_current_loop || next.continues_current_loop,
        }
    }

    /// An ordinary function body may reach its end without an explicit exit.
    pub(crate) fn may_complete_normally(self) -> bool {
        self.can_fall_through
    }

    /// Every path leaves the value-producing branch without falling through.
    ///
    /// A branch may produce values, return to the caller, diverge or leave through a `break` or
    /// `continue` targeting a loop outside this block. A loop nested inside the branch consumes
    /// its own exits at that loop boundary, so its `break` or `continue` cannot satisfy this
    /// branch's completeness. Loop exits remain separate facts and never count as divergence.
    pub(crate) fn is_value_producing_branch_complete(self) -> bool {
        !self.can_fall_through
    }

    /// Folds exits raised by statement bodies nested inside this statement's expressions.
    ///
    /// WHAT: keeps this statement's fallthrough and value-production facts while adding caller
    /// returns, divergence and loop exits that can occur while evaluating its expressions.
    /// WHY: a value producer or catch statement still has a producing or success path, so it
    /// falls through, but a hidden `return` or loop exit must stay visible to the enclosing
    /// function and loop.
    fn with_nested_expression_exits(self, nested: Self) -> Self {
        Self {
            can_fall_through: self.can_fall_through,
            produces_value: self.produces_value,
            returns_to_caller: self.returns_to_caller || nested.returns_to_caller,
            diverges: self.diverges || nested.diverges,
            breaks_current_loop: self.breaks_current_loop || nested.breaks_current_loop,
            continues_current_loop: self.continues_current_loop || nested.continues_current_loop,
        }
    }

    /// At least one reachable path produces values.
    pub(crate) fn has_producing_path(self) -> bool {
        self.produces_value
    }

    /// Every path leaves without observing a value at the enclosing receiver.
    ///
    /// WHY: static control-flow selection drops a value receiver when its selected body never
    /// gives the receiver a value, so HIR must not invent an unreachable merge or result local.
    pub(crate) fn leaves_without_value(self) -> bool {
        self.is_value_producing_branch_complete() && !self.produces_value
    }

    /// The normalised-true loop's body exits to the loop boundary.
    ///
    /// WHAT: clears the nearest-loop facts because the owning loop consumes them.
    fn with_loop_control_consumed(self) -> Self {
        Self {
            breaks_current_loop: false,
            continues_current_loop: false,
            ..self
        }
    }
}

/// Reusable expression worklist for one exit-analysis traversal.
///
/// WHAT: holds pending expression IDs while statement bodies recurse.
/// WHY: expression graphs can be deep, so descending through them iteratively avoids stack
/// growth proportional to expression depth. Statement-body recursion stays bounded by source
/// nesting, matching the other AST walkers.
#[derive(Default)]
pub(crate) struct ControlFlowExitScratch {
    pending_expressions: Vec<ExprId>,
}

/// Analyses a statement sequence into independent exit facts.
///
/// WHAT: walks statements in order and applies each next statement only to paths that still
/// fall through. Stops once no path can reach the next statement.
/// WHY: nested `if`/`match` can produce on one path and terminate on another; later statements
/// must not run on those exited paths.
pub(crate) fn analyze_control_flow_exits(
    body: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
) -> ControlFlowExitSummary {
    analyze_control_flow_exits_in_scratch(
        body,
        expressions,
        constants,
        &mut ControlFlowExitScratch::default(),
    )
}

/// Analyses a statement sequence while reusing an expression worklist.
///
/// WHY: callers that analyse every statement in a body can keep one worklist instead of
/// allocating a new vector for each statement.
pub(crate) fn analyze_control_flow_exits_in_scratch(
    body: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    let mut summary = ControlFlowExitSummary::FALLS_THROUGH;

    for statement in body {
        summary =
            summary.then_sequence(statement_exits(statement, expressions, constants, scratch));
        if !summary.can_fall_through {
            break;
        }
    }

    summary
}

fn statement_exits(
    statement: &AstNode,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    let summary = match &statement.kind {
        NodeKind::ThenValue(_) => ControlFlowExitSummary::PRODUCES_VALUE,

        NodeKind::Return(_) | NodeKind::ReturnError(_) => ControlFlowExitSummary::RETURNS_TO_CALLER,

        NodeKind::If(_, then_body, Some(else_body), _) => {
            analyze_control_flow_exits_in_scratch(then_body, expressions, constants, scratch).union(
                analyze_control_flow_exits_in_scratch(else_body, expressions, constants, scratch),
            )
        }

        NodeKind::If(_, then_body, None, _) => {
            analyze_control_flow_exits_in_scratch(then_body, expressions, constants, scratch)
                .union(ControlFlowExitSummary::FALLS_THROUGH)
        }

        NodeKind::Match {
            arms,
            default,
            exhaustiveness,
            ..
        } => match_exits(
            arms,
            default.as_deref(),
            *exhaustiveness,
            expressions,
            constants,
            scratch,
        ),

        NodeKind::LexicalScope { body } => {
            analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch)
        }

        NodeKind::WhileLoop(condition, body) => {
            while_loop_exits(*condition, body, expressions, constants, scratch)
        }

        NodeKind::RangeLoop { body, .. } | NodeKind::CollectionLoop { body, .. } => {
            finite_loop_exits(body, expressions, constants, scratch)
        }

        NodeKind::Break => ControlFlowExitSummary::BREAKS_CURRENT_LOOP,

        NodeKind::Continue => ControlFlowExitSummary::CONTINUES_CURRENT_LOOP,

        NodeKind::Assert { condition, .. } => {
            if folded_bool_condition(*condition, expressions, constants) == Some(false) {
                ControlFlowExitSummary::DIVERGES
            } else {
                ControlFlowExitSummary::FALLS_THROUGH
            }
        }

        // Declarations, assignments, expression statements and runtime fragment pushes fall
        // through; their expression-owned exits are folded below.
        _ => ControlFlowExitSummary::FALLS_THROUGH,
    };

    summary.with_nested_expression_exits(nested_expression_exits(
        statement,
        expressions,
        constants,
        scratch,
    ))
}

/// Collects exits from statement bodies owned by expressions directly on this statement.
///
/// WHAT: walks each immediate expression root iteratively and unions the exits of value-producing
/// `if`, `match`, lexical-scope and catch bodies.
/// WHY: a declaration, assignment or catch statement can hide a `return` or an escaping loop
/// exit inside a value block. The statement itself still has a producing or success path, so
/// only the non-fallthrough facts are folded in. Expression depth uses the shared worklist rather
/// than the call stack.
fn nested_expression_exits(
    statement: &AstNode,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    let mut nested = ControlFlowExitSummary::FALLS_THROUGH;
    let base = scratch.pending_expressions.len();
    statement.visit_immediate_expression_ids(&mut |expression| {
        scratch.pending_expressions.push(expression);
    });

    while scratch.pending_expressions.len() > base {
        let Some(expression) = scratch.pending_expressions.pop() else {
            break;
        };
        let Ok(view) = expressions.view(expression) else {
            continue;
        };

        if let ExpressionOperationView::ValueBlock(block) = view.operation() {
            nested = nested.union(value_block_owned_exits(
                block,
                expressions,
                constants,
                scratch,
            ));
        } else {
            view.visit_children(|child| scratch.pending_expressions.push(child.id()));
        }
    }

    debug_assert_eq!(scratch.pending_expressions.len(), base);
    scratch.pending_expressions.truncate(base);
    nested
}

fn value_block_owned_exits(
    block: &ValueBlock,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    let mut exits = ControlFlowExitSummary::FALLS_THROUGH;

    match block {
        ValueBlock::If(value_if) => {
            exits = exits
                .union(analyze_control_flow_exits_in_scratch(
                    &value_if.then_body,
                    expressions,
                    constants,
                    scratch,
                ))
                .union(analyze_control_flow_exits_in_scratch(
                    &value_if.else_body,
                    expressions,
                    constants,
                    scratch,
                ));
        }

        ValueBlock::LexicalScope(scope) => {
            exits = exits.union(analyze_control_flow_exits_in_scratch(
                &scope.body,
                expressions,
                constants,
                scratch,
            ));
        }

        ValueBlock::Match(value_match) => {
            for arm in &value_match.arms {
                exits = exits.union(analyze_control_flow_exits_in_scratch(
                    &arm.body,
                    expressions,
                    constants,
                    scratch,
                ));
            }
            if let Some(default) = &value_match.default {
                exits = exits.union(analyze_control_flow_exits_in_scratch(
                    default,
                    expressions,
                    constants,
                    scratch,
                ));
            }
        }

        ValueBlock::Catch(value_catch) => {
            if let FallibleHandling::Handler { body, .. } = &value_catch.handler {
                exits = exits.union(analyze_control_flow_exits_in_scratch(
                    body,
                    expressions,
                    constants,
                    scratch,
                ));
            }
        }
    }

    exits
}

fn match_exits(
    arms: &[MatchArm],
    default_body: Option<&[AstNode]>,
    exhaustiveness: MatchExhaustiveness,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    let mut combined: Option<ControlFlowExitSummary> = None;

    for arm in arms {
        let arm_exits =
            analyze_control_flow_exits_in_scratch(&arm.body, expressions, constants, scratch);
        combined = Some(match combined {
            Some(existing) => existing.union(arm_exits),
            None => arm_exits,
        });
    }

    match exhaustiveness {
        MatchExhaustiveness::HasDefault => {
            let Some(default_body) = default_body else {
                // A default-marked match without a default body has an uncharted path.
                return combined
                    .unwrap_or(ControlFlowExitSummary::FALLS_THROUGH)
                    .union(ControlFlowExitSummary::FALLS_THROUGH);
            };
            let default_exits = analyze_control_flow_exits_in_scratch(
                default_body,
                expressions,
                constants,
                scratch,
            );
            combined.unwrap_or(default_exits).union(default_exits)
        }

        // An exhaustive choice match is closed only when it has at least one arm. A match with
        // no arms cannot witness an exit, so it stays a fallthrough path.
        MatchExhaustiveness::ExhaustiveChoice => {
            combined.unwrap_or(ControlFlowExitSummary::FALLS_THROUGH)
        }
    }
}

/// A conditional loop whose condition is a folded `true` and whose body has no reachable break
/// never reaches its post-loop continuation.
fn while_loop_exits(
    condition: ExprId,
    body: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    match folded_bool_condition(condition, expressions, constants) {
        // A folded-false loop never runs its body, so body exits are unreachable.
        Some(false) => ControlFlowExitSummary::FALLS_THROUGH,

        Some(true) => {
            let body_exits =
                analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch);
            let mut summary = body_exits.with_loop_control_consumed();

            // Body fallthrough and `continue` both start another iteration. The loop's own
            // condition never becomes false, so those paths diverge at this boundary.
            if body_exits.can_fall_through || body_exits.continues_current_loop {
                summary.diverges = true;
            }

            // A reachable break targeting this loop makes the continuation reachable.
            summary.can_fall_through = body_exits.breaks_current_loop;
            summary.produces_value = false;
            summary
        }

        None => {
            // A dynamic condition may be false on the first evaluation, so the loop body's
            // exits are possible but cannot prove all-path non-fallthrough.
            let body_exits =
                analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch);
            let mut summary = body_exits.with_loop_control_consumed();
            summary.can_fall_through = true;
            summary.produces_value = false;
            summary
        }
    }
}

/// Range and collection loops may run zero times, so they always keep a fallthrough path while
/// still reporting the inner caller returns and divergence that can happen during an iteration.
fn finite_loop_exits(
    body: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> ControlFlowExitSummary {
    let body_exits = analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch);
    let mut summary = body_exits.with_loop_control_consumed();
    summary.can_fall_through = true;
    summary.produces_value = false;
    summary
}

fn folded_bool_condition(
    condition: ExprId,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
) -> Option<bool> {
    let view = expressions.view(condition).ok()?;
    constants.folded_bool_value(view)
}

#[cfg(test)]
#[path = "tests/control_flow_exits_tests.rs"]
mod control_flow_exits_tests;
