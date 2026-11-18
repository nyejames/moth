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

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, LoopBindings, MatchExhaustiveness, NodeKind,
};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionStore,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::symbols::path_interner::PathId;
use rustc_hash::FxHashSet;

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
    ///
    /// A standalone call whose published contract is `-> !` also diverges: it has no fallthrough
    /// path and no value, error or optional result for a continuation to consume.
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

    /// Every reachable path diverges instead of leaving the function body.
    ///
    /// WHAT: requires divergence and rejects every other exit from the body: falling through,
    /// producing a value for an enclosing receiver, returning to the caller, and loop control
    /// that would resume outside the current body.
    /// WHY: a declared `-> !` contract is a promise that no caller-visible continuation exists,
    /// so any reachable caller return or fallthrough makes the body invalid. A value produced by
    /// a hidden block is not a function return but still continues an enclosing receiver, so it
    /// cannot satisfy the contract either. Loop exits are already consumed by their owning loop
    /// inside the analysis; a surviving fact here means control resumed outside this body.
    pub(crate) fn diverges_on_every_path(self) -> bool {
        self.diverges
            && !self.can_fall_through
            && !self.produces_value
            && !self.returns_to_caller
            && !self.breaks_current_loop
            && !self.continues_current_loop
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
/// WHAT: holds pending expression IDs while statement bodies recurse, and optionally collects the
/// `-> !` call targets that a declared Never body's divergence proof can rely on.
/// WHY: expression graphs can be deep, so descending through them iteratively avoids stack
/// growth proportional to expression depth. Statement-body recursion stays bounded by source
/// nesting, matching the other AST walkers. Proof targets are only collected when the caller asks:
/// ordinary terminality and value completeness pay nothing for them.
#[derive(Default)]
pub(crate) struct ControlFlowExitScratch {
    pending_expressions: Vec<ExprId>,
    never_call_targets: Option<Vec<PathId>>,
    /// Whether statically infallible catch handlers contribute no exits.
    ///
    /// WHY: HIR lowers such a handler only as skipped evidence, never into the CFG, so a consumer
    /// projecting emitted work must not count a `continue`, `return` or `break` inside it.
    /// Authored analyses keep every retained handler reachable.
    skip_statically_infallible_handlers: bool,
}

impl ControlFlowExitScratch {
    /// Creates a worklist for a consumer that must project lowered emitted work.
    ///
    /// WHAT: statically infallible catch handlers contribute no `break`, `continue` or `return`
    ///       edges, matching HIR, which lowers them only as skipped evidence.
    /// WHY: failure classification and release speculation decide whether a statement is emitted;
    ///      counting a pruned handler's control transfer would keep a `loop true` fallthrough
    ///      alive and fold or classify a tail HIR never lowers.
    pub(crate) fn for_emitted_work() -> Self {
        Self {
            skip_statically_infallible_handlers: true,
            ..Self::default()
        }
    }

    /// Runs one reachable-body walk and returns the Never call targets it saw.
    fn take_never_call_targets(&mut self) -> Vec<PathId> {
        self.never_call_targets.take().unwrap_or_default()
    }

    /// Records one reachable standalone `-> !` call as a divergence-proof dependency.
    fn record_never_call(&mut self, target: PathId) {
        if let Some(targets) = self.never_call_targets.as_mut() {
            targets.push(target);
        }
    }

    /// Stops collecting proof targets for a nested body the caller consumes differently.
    ///
    /// WHAT: parks the target sink while `walk` runs, then restores the same sink.
    /// WHY: loop bodies and expression-owned statement bodies never decide whether the enclosing
    /// function body can fall through, so a Never call inside them cannot be part of its proof.
    fn without_never_call_targets<R>(&mut self, walk: impl FnOnce(&mut Self) -> R) -> R {
        let targets = self.never_call_targets.take();
        let result = walk(self);
        self.never_call_targets = targets;
        result
    }
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

/// Whether a statement range loop's generated ordinal update reaches lowered HIR.
///
/// WHAT: true only when the header binds the optional ordinal and the body can reach the step
///       block by falling through or executing `continue`; statically infallible catch handlers
///       contribute nothing because lowering emits them as skipped evidence, not as CFG.
/// WHY: the ordinal update lives in the step block, which lowering discards when the body cannot
///       reach it. Failure classification, skipped-source constraint evidence and the pre-fold
///       fast-operation ledger must project the same emitted work, so all call this one owner
///       instead of re-deriving reachability.
pub(crate) fn range_ordinal_update_is_emitted(
    bindings: &LoopBindings,
    body: &[AstNode],
    expressions: &ExpressionStore,
    constants: Option<&ConstValueStore>,
) -> bool {
    if bindings.index.is_none() {
        return false;
    }
    // Provisional parse-time projections can run before their owner has a constant store. An
    // empty store votes no folded conditions, so every dynamic branch stays possible and the
    // approximation can only keep the update, never drop a trap the live program would emit.
    let empty_constants;
    let constants = match constants {
        Some(constants) => constants,
        None => {
            empty_constants = ConstValueStore::default();
            &empty_constants
        }
    };
    let mut scratch = ControlFlowExitScratch::for_emitted_work();
    let exits = analyze_control_flow_exits_in_scratch(body, expressions, constants, &mut scratch);
    exits.can_fall_through || exits.continues_current_loop
}

/// Analyses a statement sequence for both exit facts and `-> !` proof dependencies.
///
/// WHAT: returns the ordinary exit summary together with every standalone Never call target
/// reached on a path that can still fall through the body.
/// WHY: recursive proof safety must know which Never contracts a declared Never body uses as its
/// own divergence proof. Callers decide how to key the targets: the declaring module's template
/// graph folds concrete generic instances into their templates, while a requester's exact-target
/// union keeps the original call paths. Calls after an independent divergence are unreachable and
/// are not collected, and calls inside loop bodies or expression-owned statement bodies cannot
/// decide whether the enclosing body falls through, so they are excluded too.
pub(crate) fn analyze_control_flow_exits_with_never_targets(
    body: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
) -> (ControlFlowExitSummary, Vec<PathId>) {
    let mut scratch = ControlFlowExitScratch {
        never_call_targets: Some(Vec::new()),
        ..ControlFlowExitScratch::default()
    };
    let summary = analyze_control_flow_exits_in_scratch(body, expressions, constants, &mut scratch);
    (summary, scratch.take_never_call_targets())
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

/// Summarises the authored exits of one statement.
///
/// WHY: sequence walks, loop boundaries and the failure classifier's emitted-work reachability
///      query must all apply the same per-statement rule. One owner prevents a consumer from
///      re-deriving reachability with a subtly different approximation.
pub(crate) fn statement_exits(
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

        // A call whose published contract is `-> !` has no fallthrough, no value and no error or
        // optional result. Statement sequencing therefore makes everything after it unreachable,
        // and a declared Never body may rely on the call for its own divergence proof.
        NodeKind::NeverCall(call) => {
            if let crate::compiler_frontend::ast::statements::never_call::NeverCallTarget::Source(
                target,
            ) = call.target
            {
                scratch.record_never_call(target);
            }
            ControlFlowExitSummary::DIVERGES
        }

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
    // A statement whose expressions own statement bodies always keeps its own fallthrough fact,
    // so a Never call inside those bodies cannot decide whether this sequence leaves. Only the
    // statement-position walk contributes divergence-proof dependencies.
    let proof_targets = scratch.never_call_targets.take();
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
    scratch.never_call_targets = proof_targets;
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
                // HIR lowers a statically infallible handler as skipped evidence only, so it
                // contributes no `continue`, `return` or `break` edge to emitted control flow.
                let lowered_as_skipped = scratch.skip_statically_infallible_handlers
                    && expressions
                        .view(value_catch.handled_value)
                        .ok()
                        .and_then(|view| view.failure_facts())
                        .is_some_and(|facts| facts.is_statically_infallible_catch());
                if !lowered_as_skipped {
                    exits = exits.union(analyze_control_flow_exits_in_scratch(
                        body,
                        expressions,
                        constants,
                        scratch,
                    ));
                }
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
            // A folded-true loop with no reachable break never resumes after the loop, whatever
            // its body does, so a Never call inside the body is not part of the enclosing body's
            // divergence proof.
            let body_exits = scratch.without_never_call_targets(|scratch| {
                analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch)
            });
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
            // exits are possible but cannot prove all-path non-fallthrough. A Never call in the
            // body is likewise not needed for the enclosing body's divergence proof.
            let body_exits = scratch.without_never_call_targets(|scratch| {
                analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch)
            });
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
    // A finite loop can run zero times, so its body never decides whether the enclosing function
    // diverges; a Never call inside it is not part of that proof.
    let body_exits = scratch.without_never_call_targets(|scratch| {
        analyze_control_flow_exits_in_scratch(body, expressions, constants, scratch)
    });
    let mut summary = body_exits.with_loop_control_consumed();
    summary.can_fall_through = true;
    summary.produces_value = false;
    summary
}

/// Collects the expression roots of statements that authored control flow cannot reach.
///
/// WHAT: walks every statement sequence in `nodes` in authored order through the same exit
///       summary that owns terminality, and records each expression root directly owned by a
///       statement in an unreachable tail, including every statement of a folded-false `while`
///       body and the dead inner tails of reachable value blocks.
/// WHY: release speculation must fold only authored-live calls, and HIR lowering already omits
///      the same dead statements while preserving their selected evidence. Deciding the skip per
///      statement occurrence keeps a const template's shared definition spans from making a live
///      use look dead, which a module-wide span set cannot do.
///
/// A dead statement's folder entry skips its whole expression graph at that root, so only the
/// statement's directly owned roots are needed. One scratch owns the exit analysis and a
/// depth-indexed pool of expression walks; the pool keeps allocations across statements and lets
/// a nested sequence or value block run while its ancestor's walk still owns its stack.
pub(crate) fn collect_authored_dead_expression_roots(
    nodes: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    dead_roots: &mut FxHashSet<ExprId>,
) {
    let mut scratch = DeadRootScratch {
        exit: ControlFlowExitScratch::for_emitted_work(),
        ..DeadRootScratch::default()
    };
    collect_sequence_dead_roots(nodes, expressions, constants, dead_roots, &mut scratch, 0);
}

#[derive(Default)]
struct DeadRootScratch {
    exit: ControlFlowExitScratch,
    walks: Vec<ExpressionWalkScratch>,
}

/// One reusable expression worklist. Every entry is cleared before reuse; the pool index is the
/// structural depth, so an active ancestor walk is never overwritten by its nested sequence.
#[derive(Default)]
struct ExpressionWalkScratch {
    pending: Vec<ExprId>,
    visited: FxHashSet<ExprId>,
}

impl DeadRootScratch {
    fn take_expression_walk(&mut self, depth: usize) -> ExpressionWalkScratch {
        while self.walks.len() <= depth {
            self.walks.push(ExpressionWalkScratch::default());
        }
        let mut walk = std::mem::take(&mut self.walks[depth]);
        walk.pending.clear();
        walk.visited.clear();
        walk
    }

    fn put_expression_walk(&mut self, depth: usize, walk: ExpressionWalkScratch) {
        self.walks[depth] = walk;
    }
}

fn collect_sequence_dead_roots(
    nodes: &[AstNode],
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    dead_roots: &mut FxHashSet<ExprId>,
    scratch: &mut DeadRootScratch,
    depth: usize,
) {
    let mut summary = ControlFlowExitSummary::FALLS_THROUGH;

    for (index, node) in nodes.iter().enumerate() {
        if !summary.can_fall_through {
            for dead_node in &nodes[index..] {
                record_dead_node_roots(dead_node, dead_roots);
            }
            return;
        }

        collect_live_node_dead_tails(node, expressions, constants, dead_roots, scratch, depth);
        summary = summary.then_sequence(statement_exits(
            node,
            expressions,
            constants,
            &mut scratch.exit,
        ));
    }
}

/// Finds dead tails nested inside a reachable statement's bodies and value blocks.
fn collect_live_node_dead_tails(
    node: &AstNode,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    dead_roots: &mut FxHashSet<ExprId>,
    scratch: &mut DeadRootScratch,
    depth: usize,
) {
    match &node.kind {
        NodeKind::Function(_, _, body)
        | NodeKind::EntryStart(_, body)
        | NodeKind::LexicalScope { body } => {
            collect_sequence_dead_roots(
                body,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth + 1,
            );
        }
        NodeKind::If(_, then_body, else_body, _) => {
            collect_sequence_dead_roots(
                then_body,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth + 1,
            );
            if let Some(else_body) = else_body {
                collect_sequence_dead_roots(
                    else_body,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth + 1,
                );
            }
        }
        NodeKind::Match { arms, default, .. } => {
            for arm in arms {
                collect_sequence_dead_roots(
                    &arm.body,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth + 1,
                );
            }
            if let Some(default) = default {
                collect_sequence_dead_roots(
                    default,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth + 1,
                );
            }
        }
        NodeKind::RangeLoop { body, .. } | NodeKind::CollectionLoop { body, .. } => {
            collect_sequence_dead_roots(
                body,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth + 1,
            );
        }
        NodeKind::WhileLoop(condition, body) => {
            // A pre-fold folded-false condition never runs the body. The body is authored-dead in
            // every profile, consistently with `while_loop_exits`; a condition that only release
            // folding makes false is not folded by this constant store and stays reachable here.
            if folded_bool_condition(*condition, expressions, constants) == Some(false) {
                for dead_node in body {
                    record_dead_node_roots(dead_node, dead_roots);
                }
            } else {
                collect_sequence_dead_roots(
                    body,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth + 1,
                );
            }
        }
        _ => {}
    }

    let mut walk = scratch.take_expression_walk(depth);
    node.visit_immediate_expression_ids(&mut |expression| walk.pending.push(expression));
    collect_live_expression_dead_tails(
        &mut walk,
        expressions,
        constants,
        dead_roots,
        scratch,
        depth,
    );
    scratch.put_expression_walk(depth, walk);
}

/// Walks reachable expressions once, looking only for value blocks with dead inner tails.
///
/// WHAT: `walk.pending` carries the statement's immediate expression roots. A value block pushes
///       only its header expressions; its bodies are analysed by the statement walk that owns them.
/// WHY: following a value block's children as well would walk the same body again at every
///      nesting level, which is exponential in value-block depth. The pool slot keeps the walk
///      linear in expression size without allocating per statement.
fn collect_live_expression_dead_tails(
    walk: &mut ExpressionWalkScratch,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    dead_roots: &mut FxHashSet<ExprId>,
    scratch: &mut DeadRootScratch,
    depth: usize,
) {
    while let Some(expression) = walk.pending.pop() {
        if !walk.visited.insert(expression) {
            continue;
        }
        let Ok(view) = expressions.view(expression) else {
            continue;
        };
        if let ExpressionOperationView::ValueBlock(block) = view.operation() {
            collect_value_block_dead_roots(
                block,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth + 1,
            );
            push_value_block_headers(block, &mut walk.pending);
        } else {
            let _ = view.visit_child_ids(&mut |child| walk.pending.push(child));
        }
    }
}

/// Queues the expressions a value block evaluates outside its statement bodies.
///
/// WHY: expression graphs flatten a value block's statement bodies into its child ids, so a
///      consumer that also walks those bodies structurally must not descend through them here.
///      Both the dead-root walk and the pre-fold fast-operation walk share this rule.
pub(crate) fn push_value_block_headers(block: &ValueBlock, pending: &mut Vec<ExprId>) {
    match block {
        ValueBlock::If(value_if) => pending.push(value_if.condition),
        ValueBlock::LexicalScope(_) => {}
        ValueBlock::Match(value_match) => {
            pending.push(value_match.scrutinee);
            for arm in &value_match.arms {
                if let Some(guard) = arm.guard {
                    pending.push(guard);
                }
                arm.pattern
                    .visit_expression_ids(&mut |expression| pending.push(expression));
            }
        }
        ValueBlock::Catch(value_catch) => pending.push(value_catch.handled_value),
    }
}

fn collect_value_block_dead_roots(
    block: &ValueBlock,
    expressions: &ExpressionStore,
    constants: &ConstValueStore,
    dead_roots: &mut FxHashSet<ExprId>,
    scratch: &mut DeadRootScratch,
    depth: usize,
) {
    match block {
        ValueBlock::If(value_if) => {
            collect_sequence_dead_roots(
                &value_if.then_body,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth,
            );
            collect_sequence_dead_roots(
                &value_if.else_body,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth,
            );
        }
        ValueBlock::LexicalScope(scope) => {
            collect_sequence_dead_roots(
                &scope.body,
                expressions,
                constants,
                dead_roots,
                scratch,
                depth,
            );
        }
        ValueBlock::Match(value_match) => {
            for arm in &value_match.arms {
                collect_sequence_dead_roots(
                    &arm.body,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth,
                );
            }
            if let Some(default) = &value_match.default {
                collect_sequence_dead_roots(
                    default,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth,
                );
            }
        }
        ValueBlock::Catch(value_catch) => {
            if let FallibleHandling::Handler { body, .. } = &value_catch.handler {
                collect_sequence_dead_roots(
                    body,
                    expressions,
                    constants,
                    dead_roots,
                    scratch,
                    depth,
                );
            }
        }
    }
}

/// Records the directly owned expression roots of a statement the sequence cannot reach.
fn record_dead_node_roots(node: &AstNode, dead_roots: &mut FxHashSet<ExprId>) {
    // A nested declaration is compiled as its own callable; the enclosing dead tail does not
    // make its body unreachable.
    if matches!(
        &node.kind,
        NodeKind::Function(_, _, _) | NodeKind::EntryStart(_, _)
    ) {
        return;
    }

    node.visit_immediate_expression_ids(&mut |expression| {
        dead_roots.insert(expression);
    });

    match &node.kind {
        NodeKind::If(_, then_body, else_body, _) => {
            for dead_node in then_body {
                record_dead_node_roots(dead_node, dead_roots);
            }
            if let Some(else_body) = else_body {
                for dead_node in else_body {
                    record_dead_node_roots(dead_node, dead_roots);
                }
            }
        }
        NodeKind::Match { arms, default, .. } => {
            for arm in arms {
                for dead_node in &arm.body {
                    record_dead_node_roots(dead_node, dead_roots);
                }
            }
            if let Some(default) = default {
                for dead_node in default {
                    record_dead_node_roots(dead_node, dead_roots);
                }
            }
        }
        NodeKind::LexicalScope { body }
        | NodeKind::RangeLoop { body, .. }
        | NodeKind::CollectionLoop { body, .. }
        | NodeKind::WhileLoop(_, body) => {
            for dead_node in body {
                record_dead_node_roots(dead_node, dead_roots);
            }
        }
        _ => {}
    }
}

/// Reads a condition's pre-fold boolean value.
pub(crate) fn folded_bool_condition(
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
