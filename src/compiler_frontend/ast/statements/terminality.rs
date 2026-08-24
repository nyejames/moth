//! Function-body terminality validation.
//!
//! WHAT: checks that a function body is guaranteed to terminate on every reachable path before
//! control can fall off the end of the function.
//! WHY: AST owns this user-facing rule diagnostic so HIR lowering only needs to handle
//! infrastructure invariants. Keeping the check at the AST stage lets body parsing emit a
//! source-level diagnostic tied to the function declaration.
//!
//! ## Rules
//!
//! This is a conservative, control-flow-only analysis. It does not evaluate runtime conditions
//! beyond structurally-folded `assert(false)`.
//!
//! - `Return(_)` and `ReturnError(_)` terminate.
//! - `Assert { condition: false, .. }` terminates only when the condition is the literal `false`.
//! - `ScopedBlock { body }` terminates when its body terminates.
//! - `If(_, then_body, Some(else_body))` terminates only when both bodies terminate.
//! - `If(_, _, None)` does not terminate.
//! - `Match` with a default arm terminates only when every arm body and the default body terminate.
//! - `ExhaustiveChoice` matches without a default arm terminate only when every arm body terminates.
//! - Loops, `Break`, `Continue`, declarations, assignments, expression statements, and runtime
//!   fragment pushes do not terminate.
//!
//! A body is terminal when scanning in source order finds any statement that guarantees
//! all-path terminality. Earlier fall-through statements may be followed by a later terminal
//! statement.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, MatchExhaustiveness, NodeKind, SourceLocation,
};
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidReturnShapeReason};

/// Policy that decides whether a function body must be terminal on all paths.
///
/// WHAT: distinguishes functions that may implicitly fall through from functions that must
/// explicitly return, plus the special entry `start()` shape.
/// WHY: the policy is separate from the checker so callers can decide the shape based on the
/// function signature and origin without duplicating the control-flow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionTerminalityPolicy {
    /// The function has no required success-channel return values; falling off the end is allowed.
    AllowImplicitUnit,

    /// The function has one or more required success-channel return values; every path must
    /// reach a terminator before the body ends.
    RequireExplicitReturn,

    /// Entry `start()` is implicitly completed by returning the accumulated fragment vector.
    ///
    /// WHAT: preserves the existing entry-start contract where the compiler synthesizes the
    /// return value from top-level runtime fragments.
    /// WHY: users must not be required to author an explicit return of the fragment vector.
    EntryStartImplicitReturn,
}

/// Chooses the terminality policy for a function from its resolved signature and origin.
///
/// WHAT: maps the function signature to the appropriate policy.
/// WHY: this keeps policy selection in one place and makes the call sites read as a named step.
pub(crate) fn terminality_policy_for_signature(
    signature: &FunctionSignature,
    is_entry_start: bool,
) -> FunctionTerminalityPolicy {
    if is_entry_start {
        return FunctionTerminalityPolicy::EntryStartImplicitReturn;
    }

    if signature.success_returns().is_empty() {
        FunctionTerminalityPolicy::AllowImplicitUnit
    } else {
        FunctionTerminalityPolicy::RequireExplicitReturn
    }
}

/// Validates that a function body satisfies the requested terminality policy.
///
/// WHAT: scans the body and returns a `FunctionMayFallThrough` diagnostic when the policy
/// requires explicit terminality and the body can fall through.
/// WHY: body parsing already owns user-facing diagnostics, so this is the natural place to
/// reject missing returns before HIR lowering is invoked.
pub(crate) fn validate_function_body_terminality(
    body: &[AstNode],
    policy: FunctionTerminalityPolicy,
    location: SourceLocation,
) -> Option<CompilerDiagnostic> {
    match policy {
        FunctionTerminalityPolicy::AllowImplicitUnit
        | FunctionTerminalityPolicy::EntryStartImplicitReturn => None,

        FunctionTerminalityPolicy::RequireExplicitReturn => {
            if body_is_all_paths_terminal(body) {
                None
            } else {
                Some(CompilerDiagnostic::invalid_return_shape(
                    InvalidReturnShapeReason::FunctionMayFallThrough,
                    location,
                ))
            }
        }
    }
}

/// Returns true if the body is guaranteed to terminate on every reachable path.
///
/// WHAT: scans statements in order and returns true once any statement guarantees all-path
/// terminality. Earlier fall-through statements may be followed by a later terminal statement.
/// WHY: this matches the conservative contract documented at the module level.
fn body_is_all_paths_terminal(body: &[AstNode]) -> bool {
    for statement in body {
        if statement_is_all_paths_terminal(statement) {
            return true;
        }
    }

    false
}

/// Returns true when a single statement guarantees that all paths through it terminate.
///
/// WHAT: inspects the AST node kind and delegates to recursive analysis for compound
/// statements (`if`, `match`, `ScopedBlock`).
/// WHY: simple statements have fixed behavior, but nested control flow requires checking
/// every branch.
fn statement_is_all_paths_terminal(statement: &AstNode) -> bool {
    match &statement.kind {
        NodeKind::Return(_) | NodeKind::ReturnError(_) => true,

        NodeKind::Assert { condition, .. } => {
            matches!(condition.kind, ExpressionKind::Bool(false))
        }

        NodeKind::ScopedBlock { body } => body_is_all_paths_terminal(body),

        NodeKind::If(_, then_body, Some(else_body), _) => {
            body_is_all_paths_terminal(then_body) && body_is_all_paths_terminal(else_body)
        }

        NodeKind::If(_, _, None, _) => false,

        NodeKind::Match {
            arms,
            default: maybe_default_body,
            exhaustiveness,
            ..
        } => {
            let arms_terminal = arms.iter().all(|arm| body_is_all_paths_terminal(&arm.body));

            match exhaustiveness {
                MatchExhaustiveness::HasDefault => {
                    let Some(default_body) = maybe_default_body else {
                        return false;
                    };

                    arms_terminal && body_is_all_paths_terminal(default_body)
                }

                MatchExhaustiveness::ExhaustiveChoice => arms_terminal,
            }
        }

        _ => false,
    }
}

#[cfg(test)]
#[path = "tests/terminality_tests.rs"]
mod terminality_tests;
