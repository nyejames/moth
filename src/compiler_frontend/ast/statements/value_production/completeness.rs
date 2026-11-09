//! Value-production completeness and reachable `then` traversal.
//!
//! WHAT: validates that value-producing `if`, match and catch bodies leave every reachable path
//! through an accepted exit, and traverses only the `then` groups that remain reachable.
//! WHY: value-producing blocks share one completeness contract. Producing values, returning,
//! diverging and leaving through `break` or `continue` targeting an enclosing loop are complete;
//! a nested loop consumes its own exits, so it cannot complete the branch on its own. Any real
//! fallthrough is rejected. Dead values must not constrain an inferred type, so inference keeps
//! reachable-only traversal.
//!
//! Exit facts belong to
//! [`crate::compiler_frontend::ast::statements::control_flow_exits`]. This module owns the
//! value-production policy: which exits are acceptable at a value branch and the required
//! at-least-one-producing-path rule. It does not own missing-`else` syntax checks.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::statements::control_flow_exits::{
    ControlFlowExitScratch, ControlFlowExitSummary, analyze_control_flow_exits_in_scratch,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::types::ProducedValues;
use crate::compiler_frontend::compiler_messages::InvalidControlFlowStatementReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::source::SourceSpan;

/// Validates every value-match arm and optional default against the shared contract.
pub fn validate_value_match_completeness(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
    span: Option<SourceSpan>,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> Result<(), ExpressionParseError> {
    let mut combined: Option<ControlFlowExitSummary> = None;
    let mut scratch = ControlFlowExitScratch::default();

    for arm in arms {
        let arm_exits = analyze_control_flow_exits_in_scratch(
            &arm.body,
            expressions.store(),
            constants,
            &mut scratch,
        );
        reject_fallthrough(arm_exits, span)?;
        combined = Some(match combined {
            Some(existing) => existing.union(arm_exits),
            None => arm_exits,
        });
    }

    if let Some(default_body) = default {
        let default_exits = analyze_control_flow_exits_in_scratch(
            default_body,
            expressions.store(),
            constants,
            &mut scratch,
        );
        reject_fallthrough(default_exits, span)?;
        combined = Some(match combined {
            Some(existing) => existing.union(default_exits),
            None => default_exits,
        });
    }

    let Some(combined) = combined else {
        return Err(no_producing_path(span));
    };

    if combined.has_producing_path() {
        Ok(())
    } else {
        Err(no_producing_path(span))
    }
}

/// Visits every reachable `ThenValue` group without entering unreachable tails.
pub fn visit_reachable_then_values<E>(
    body: &[AstNode],
    visitor: &mut impl FnMut(&ProducedValues) -> Result<(), E>,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> Result<(), E> {
    visit_reachable_then_values_in_scratch(
        body,
        visitor,
        expressions,
        constants,
        &mut ControlFlowExitScratch::default(),
    )
}

fn visit_reachable_then_values_in_scratch<E>(
    body: &[AstNode],
    visitor: &mut impl FnMut(&ProducedValues) -> Result<(), E>,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> Result<(), E> {
    let mut reachable = true;

    for statement in body {
        if !reachable {
            break;
        }

        visit_statement_then_values(statement, visitor, expressions, constants, scratch)?;
        reachable = analyze_control_flow_exits_in_scratch(
            std::slice::from_ref(statement),
            expressions.store(),
            constants,
            scratch,
        )
        .can_fall_through;
    }

    Ok(())
}

/// Mutably visits every authored `ThenValue` group, including groups after a terminal statement.
///
/// WHAT: walks each statement body in authored order and recurses through `if`, `match` and
///       lexical-scope bodies. Loop bodies are not searched: `then` is rejected inside a loop body
///       because loop contexts never inherit a value target.
/// WHY: a statically inactive branch or a tail after a terminal statement keeps its authored group
///      until module finalization erases it, so every group still receives receiving validation.
///      Reachability still governs inference contributions, which use the reachable visitors.
pub fn visit_authored_then_values_mut<E>(
    body: &mut [AstNode],
    visitor: &mut impl FnMut(&mut ProducedValues) -> Result<(), E>,
) -> Result<(), E> {
    for statement in body {
        match &mut statement.kind {
            NodeKind::ThenValue(produced_values) => visitor(produced_values)?,

            NodeKind::If(_, then_body, Some(else_body), _) => {
                visit_authored_then_values_mut(then_body, visitor)?;
                visit_authored_then_values_mut(else_body, visitor)?;
            }

            NodeKind::If(_, then_body, None, _) => {
                visit_authored_then_values_mut(then_body, visitor)?;
            }

            NodeKind::Match {
                arms,
                default: maybe_default_body,
                ..
            } => {
                for arm in arms {
                    visit_authored_then_values_mut(&mut arm.body, visitor)?;
                }
                if let Some(default_body) = maybe_default_body {
                    visit_authored_then_values_mut(default_body, visitor)?;
                }
            }

            NodeKind::LexicalScope { body } => visit_authored_then_values_mut(body, visitor)?,

            _ => {}
        }
    }

    Ok(())
}

fn visit_statement_then_values<E>(
    statement: &AstNode,
    visitor: &mut impl FnMut(&ProducedValues) -> Result<(), E>,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    scratch: &mut ControlFlowExitScratch,
) -> Result<(), E> {
    match &statement.kind {
        NodeKind::ThenValue(produced_values) => visitor(produced_values),

        NodeKind::If(_, then_body, Some(else_body), _) => {
            visit_reachable_then_values_in_scratch(
                then_body,
                visitor,
                expressions,
                constants,
                scratch,
            )?;
            visit_reachable_then_values_in_scratch(
                else_body,
                visitor,
                expressions,
                constants,
                scratch,
            )
        }

        NodeKind::If(_, then_body, None, _) => visit_reachable_then_values_in_scratch(
            then_body,
            visitor,
            expressions,
            constants,
            scratch,
        ),

        NodeKind::Match {
            arms,
            default: maybe_default_body,
            ..
        } => {
            for arm in arms {
                visit_reachable_then_values_in_scratch(
                    &arm.body,
                    visitor,
                    expressions,
                    constants,
                    scratch,
                )?;
            }
            if let Some(default_body) = maybe_default_body {
                visit_reachable_then_values_in_scratch(
                    default_body,
                    visitor,
                    expressions,
                    constants,
                    scratch,
                )?;
            }
            Ok(())
        }

        NodeKind::LexicalScope { body } => {
            visit_reachable_then_values_in_scratch(body, visitor, expressions, constants, scratch)
        }

        _ => Ok(()),
    }
}

pub(crate) fn validate_closed_branch_pair(
    then_exits: ControlFlowExitSummary,
    else_exits: ControlFlowExitSummary,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    reject_fallthrough(then_exits, span)?;
    reject_fallthrough(else_exits, span)?;

    if then_exits.has_producing_path() || else_exits.has_producing_path() {
        Ok(())
    } else {
        Err(no_producing_path(span))
    }
}

fn reject_fallthrough(
    exits: ControlFlowExitSummary,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    if exits.is_value_producing_branch_complete() {
        Ok(())
    } else {
        Err(choices_control_flow::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfBranchFallsThrough,
            span,
        )
        .into())
    }
}

fn no_producing_path(span: Option<SourceSpan>) -> ExpressionParseError {
    choices_control_flow::invalid_control_flow_statement(
        InvalidControlFlowStatementReason::ValueIfNoProducingPath,
        span,
    )
    .into()
}
