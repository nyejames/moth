//! Function-body terminality validation.
//!
//! WHAT: checks that a function body is guaranteed to exit on every reachable path before
//! control can fall off the end of the function.
//! WHY: AST owns this user-facing rule diagnostic so HIR lowering only needs to handle
//! infrastructure invariants. Keeping the check at the AST stage lets body parsing emit a
//! source-level diagnostic tied to the function declaration.
//!
//! The control-flow facts come from [`crate::compiler_frontend::ast::statements::control_flow_exits`].
//! This module owns only the policy (which function shapes require an explicit return) and the
//! fallthrough diagnostic. It recognises any exit the shared analysis proves, including a
//! normalised-true loop with no reachable break.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionStore;
use crate::compiler_frontend::ast::statements::control_flow_exits::analyze_control_flow_exits;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::InvalidReturnShapeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::source::SourceSpan;

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

    if signature.returns.success().is_empty() {
        FunctionTerminalityPolicy::AllowImplicitUnit
    } else {
        FunctionTerminalityPolicy::RequireExplicitReturn
    }
}

/// Validates that a function body satisfies the requested terminality policy.
///
/// WHAT: asks the shared exit analysis whether any reachable path can complete normally and
/// returns a `FunctionMayFallThrough` diagnostic when the policy requires an explicit exit.
/// WHY: body parsing already owns user-facing diagnostics, so this is the natural place to
/// reject missing returns before HIR lowering is invoked.
pub(crate) fn validate_function_body_terminality(
    body: &[AstNode],
    policy: FunctionTerminalityPolicy,
    span: Option<SourceSpan>,
    expressions: &ExpressionStore,
    const_values: &ConstValueStore,
) -> Option<DiagnosticDraft> {
    match policy {
        FunctionTerminalityPolicy::AllowImplicitUnit
        | FunctionTerminalityPolicy::EntryStartImplicitReturn => None,

        FunctionTerminalityPolicy::RequireExplicitReturn => {
            let exits = analyze_control_flow_exits(body, expressions, const_values);
            if exits.may_complete_normally() {
                Some(calls_data::invalid_return_shape(
                    InvalidReturnShapeReason::FunctionMayFallThrough,
                    span,
                ))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/terminality_tests.rs"]
mod terminality_tests;
