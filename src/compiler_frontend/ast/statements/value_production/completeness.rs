//! All-path value-production exit analysis and produced-value traversal.
//!
//! WHAT: summarises whether a statement sequence can fall through, produce values
//! or terminate, and visits every reachable `ThenValue` group.
//! WHY: value-producing `if`, match and catch share one completeness contract.
//! Mixed produce/terminate paths are complete; any real fallthrough is not.
//!
//! This module does not own missing-`else` syntax checks.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::ast::statements::value_production::types::{
    BranchExitSummary, ProducedValues,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Analyses a body into independent fallthrough, produce and terminate facts.
///
/// WHAT: walks statements in order and only sequences the next statement onto
/// paths that still fall through.
/// WHY: nested `if`/`match` can produce on one path and terminate on another;
/// later statements must not run on those exited paths.
pub fn analyze_branch_exits(body: &[AstNode]) -> BranchExitSummary {
    let mut summary = BranchExitSummary::FALLS_THROUGH;

    for statement in body {
        summary = summary.then_sequence(statement_exits(statement));
        if !summary.can_fall_through {
            break;
        }
    }

    summary
}

/// Validates every value-match arm and optional default against the shared contract.
pub fn validate_value_match_completeness(
    arms: &[MatchArm],
    default: Option<&[AstNode]>,
    location: &SourceLocation,
) -> Result<(), ExpressionParseError> {
    let mut combined: Option<BranchExitSummary> = None;

    for arm in arms {
        let arm_exits = analyze_branch_exits(&arm.body);
        reject_fallthrough(arm_exits, location)?;
        combined = Some(match combined {
            Some(existing) => existing.union(arm_exits),
            None => arm_exits,
        });
    }

    if let Some(default_body) = default {
        let default_exits = analyze_branch_exits(default_body);
        reject_fallthrough(default_exits, location)?;
        combined = Some(match combined {
            Some(existing) => existing.union(default_exits),
            None => default_exits,
        });
    }

    let Some(combined) = combined else {
        return Err(no_producing_path(location));
    };

    if combined.produces_value {
        Ok(())
    } else {
        Err(no_producing_path(location))
    }
}

/// Visits every reachable `ThenValue` group without entering unreachable tails.
pub fn visit_reachable_then_values<E>(
    body: &[AstNode],
    visitor: &mut impl FnMut(&ProducedValues) -> Result<(), E>,
) -> Result<(), E> {
    let mut reachable = true;

    for statement in body {
        if !reachable {
            break;
        }

        visit_statement_then_values(statement, visitor)?;
        reachable = statement_exits(statement).can_fall_through;
    }

    Ok(())
}

/// Mutably visits every reachable `ThenValue` group for post-inference coercion.
pub fn visit_reachable_then_values_mut<E>(
    body: &mut [AstNode],
    visitor: &mut impl FnMut(&mut ProducedValues) -> Result<(), E>,
) -> Result<(), E> {
    let mut reachable = true;

    for statement in body {
        if !reachable {
            break;
        }

        visit_statement_then_values_mut(statement, visitor)?;
        reachable = statement_exits(statement).can_fall_through;
    }

    Ok(())
}

fn statement_exits(statement: &AstNode) -> BranchExitSummary {
    match &statement.kind {
        NodeKind::ThenValue(_) => BranchExitSummary::PRODUCES,

        NodeKind::Return(_) | NodeKind::ReturnError(_) => BranchExitSummary::TERMINATES,

        NodeKind::If(_, then_body, Some(else_body), _) => {
            analyze_branch_exits(then_body).union(analyze_branch_exits(else_body))
        }

        NodeKind::If(_, then_body, None, _) => {
            analyze_branch_exits(then_body).union(BranchExitSummary::FALLS_THROUGH)
        }

        NodeKind::Match {
            arms,
            default: maybe_default_body,
            ..
        } => match_exits(arms, maybe_default_body.as_deref()),

        NodeKind::ScopedBlock { body } => analyze_branch_exits(body),

        NodeKind::Assert { condition, .. } if assert_condition_is_statically_false(condition) => {
            BranchExitSummary::TERMINATES
        }

        // Loops, break, continue and other compounds stay conservative.
        _ => BranchExitSummary::FALLS_THROUGH,
    }
}

fn match_exits(arms: &[MatchArm], default_body: Option<&[AstNode]>) -> BranchExitSummary {
    let mut combined: Option<BranchExitSummary> = None;

    for arm in arms {
        let arm_exits = analyze_branch_exits(&arm.body);
        combined = Some(match combined {
            Some(existing) => existing.union(arm_exits),
            None => arm_exits,
        });
    }

    if let Some(default_body) = default_body {
        let default_exits = analyze_branch_exits(default_body);
        combined = Some(match combined {
            Some(existing) => existing.union(default_exits),
            None => default_exits,
        });
    }

    combined.unwrap_or(BranchExitSummary::FALLS_THROUGH)
}

fn visit_statement_then_values<E>(
    statement: &AstNode,
    visitor: &mut impl FnMut(&ProducedValues) -> Result<(), E>,
) -> Result<(), E> {
    match &statement.kind {
        NodeKind::ThenValue(produced_values) => visitor(produced_values),

        NodeKind::If(_, then_body, Some(else_body), _) => {
            visit_reachable_then_values(then_body, visitor)?;
            visit_reachable_then_values(else_body, visitor)
        }

        NodeKind::If(_, then_body, None, _) => visit_reachable_then_values(then_body, visitor),

        NodeKind::Match {
            arms,
            default: maybe_default_body,
            ..
        } => {
            for arm in arms {
                visit_reachable_then_values(&arm.body, visitor)?;
            }
            if let Some(default_body) = maybe_default_body {
                visit_reachable_then_values(default_body, visitor)?;
            }
            Ok(())
        }

        NodeKind::ScopedBlock { body } => visit_reachable_then_values(body, visitor),

        _ => Ok(()),
    }
}

fn visit_statement_then_values_mut<E>(
    statement: &mut AstNode,
    visitor: &mut impl FnMut(&mut ProducedValues) -> Result<(), E>,
) -> Result<(), E> {
    match &mut statement.kind {
        NodeKind::ThenValue(produced_values) => visitor(produced_values),

        NodeKind::If(_, then_body, Some(else_body), _) => {
            visit_reachable_then_values_mut(then_body, visitor)?;
            visit_reachable_then_values_mut(else_body, visitor)
        }

        NodeKind::If(_, then_body, None, _) => visit_reachable_then_values_mut(then_body, visitor),

        NodeKind::Match {
            arms,
            default: maybe_default_body,
            ..
        } => {
            for arm in arms {
                visit_reachable_then_values_mut(&mut arm.body, visitor)?;
            }
            if let Some(default_body) = maybe_default_body {
                visit_reachable_then_values_mut(default_body, visitor)?;
            }
            Ok(())
        }

        NodeKind::ScopedBlock { body } => visit_reachable_then_values_mut(body, visitor),

        _ => Ok(()),
    }
}

pub(crate) fn validate_closed_branch_pair(
    then_exits: BranchExitSummary,
    else_exits: BranchExitSummary,
    location: &SourceLocation,
) -> Result<(), ExpressionParseError> {
    reject_fallthrough(then_exits, location)?;
    reject_fallthrough(else_exits, location)?;

    if then_exits.produces_value || else_exits.produces_value {
        Ok(())
    } else {
        Err(no_producing_path(location))
    }
}

fn reject_fallthrough(
    exits: BranchExitSummary,
    location: &SourceLocation,
) -> Result<(), ExpressionParseError> {
    if exits.can_fall_through {
        Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfBranchFallsThrough,
            location.clone(),
        )
        .into())
    } else {
        Ok(())
    }
}

fn no_producing_path(location: &SourceLocation) -> ExpressionParseError {
    CompilerDiagnostic::invalid_control_flow_statement(
        InvalidControlFlowStatementReason::ValueIfNoProducingPath,
        location.clone(),
    )
    .into()
}

/// Detects whether an assert condition is statically known to be `false`.
///
/// WHAT: checks only source-level/lowered literal `false`.
/// WHY: branch completeness deliberately recognizes only literal false assertions
///      and must stay aligned with HIR's direct assertion-failure lowering.
fn assert_condition_is_statically_false(condition: &Expression) -> bool {
    matches!(&condition.kind, ExpressionKind::Bool(false))
}
