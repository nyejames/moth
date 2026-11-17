//! Renderer for source-constraint verification diagnostics.
//!
//! WHAT: turns the typed reason and optional unavailable-callee fact into the user-facing message
//!       and guidance.
//! WHY: verification emits typed facts; only this renderer owns British-English prose. Stable
//!       reasons keep test assertions independent of wording changes.

use super::payload::RenderedPayload;
use crate::compiler_frontend::compiler_messages::InvalidSourceConstraintReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::constraints::InvalidSourceConstraintView;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;

/// Renders a failed `$safe_math` or `$infallible` boundary.
pub(crate) fn render_invalid_source_constraint(
    diagnostic: &DiagnosticView<'_>,
    view: &InvalidSourceConstraintView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let callee = view.callee.map(|name| context.string_table.resolve(name));
    let message = source_constraint_message(view.reason, callee);
    let mut guidance = Vec::with_capacity(2);

    guidance.push(source_constraint_guidance(view.reason, callee));

    let hop_count = diagnostic
        .durable_labels()
        .filter(|label| matches!(label.message, DurableLabelMessage::ConstraintCallHop))
        .count();
    if hop_count > 0 {
        guidance.push(format!(
            "The witness follows {hop_count} call {} from this boundary; each marked call site is one hop.",
            if hop_count == 1 { "hop" } else { "hops" }
        ));
    }

    RenderedPayload { message, guidance }
}

fn source_constraint_message(
    reason: InvalidSourceConstraintReason,
    callee: Option<&str>,
) -> String {
    match reason {
        InvalidSourceConstraintReason::FastOperation => {
            "`$safe_math` forbids fast numeric semantics, but this boundary selects a fast arithmetic operation."
                .to_owned()
        }
        InvalidSourceConstraintReason::FastRangeGuard => {
            "`$safe_math` forbids fast numeric semantics, but this boundary's range guard uses fast semantics."
                .to_owned()
        }
        InvalidSourceConstraintReason::FastWriteBack => {
            "`$safe_math` forbids fast numeric semantics, but this boundary writes back a fast compound assignment."
                .to_owned()
        }
        InvalidSourceConstraintReason::UnknownFastEvidence => format!(
            "`$safe_math` cannot be proved because `{}` is an unknown external with no trusted numeric contract.",
            callee.unwrap_or("the called function")
        ),
        InvalidSourceConstraintReason::UnavailableCalleeFastEvidence => format!(
            "`$safe_math` cannot be proved because `{}` reports fast semantics and its body is not available for a local witness.",
            callee.unwrap_or("the called function")
        ),
        InvalidSourceConstraintReason::FastEvidenceBeyondWitnessBound => format!(
            "`$safe_math` cannot be proved: fast evidence lies beyond the three-call-hop witness bound through `{}`.",
            callee.unwrap_or("a called function")
        ),
        InvalidSourceConstraintReason::FastEvidenceRecursiveCycle => format!(
            "`$safe_math` cannot be proved: the witness search returned to an already visited function through `{}` without reaching fast evidence.",
            callee.unwrap_or("a called function")
        ),
        InvalidSourceConstraintReason::TrapOverflow => {
            "`$infallible` forbids reachable traps, but this boundary's arithmetic may overflow."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapDivision => {
            "`$infallible` forbids reachable traps, but this boundary's integer division may divide by zero or overflow."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapDivideByZero => {
            "`$infallible` forbids reachable traps, but this boundary's integer division or remainder may divide by zero."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapExponent => {
            "`$infallible` forbids reachable traps, but this boundary's integer power may overflow or reject a negative exponent."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapFloatNonFinite => {
            "`$infallible` forbids reachable traps, but this boundary's floating-point arithmetic may produce a non-finite result."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapFloatDivision => {
            "`$infallible` forbids reachable traps, but this boundary's floating-point division may divide by zero or produce a non-finite result."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapRangeGuard => {
            "`$infallible` forbids reachable traps, but this boundary's range-loop guard may trap on an invalid step."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapWriteBack => {
            "`$infallible` forbids reachable traps, but this boundary's fast compound write-back may not fit its target."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapAssertion => {
            "`$infallible` forbids reachable traps, but this boundary's assertion may fail."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapFloatValidation => {
            "`$infallible` forbids reachable traps, but validation of an external floating-point result may trap."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapConversion => {
            "`$infallible` forbids reachable traps, but this boundary's checked conversion may trap."
                .to_owned()
        }
        InvalidSourceConstraintReason::UnknownTrapEvidence => format!(
            "`$infallible` cannot be proved because `{}` is an unknown external with no trusted numeric contract.",
            callee.unwrap_or("the called function")
        ),
        InvalidSourceConstraintReason::UnavailableCalleeTrapEvidence => format!(
            "`$infallible` cannot be proved because `{}` reports a possible trap and its body is not available for a local witness.",
            callee.unwrap_or("the called function")
        ),
        InvalidSourceConstraintReason::TrapEvidenceBeyondWitnessBound => format!(
            "`$infallible` cannot be proved: trap evidence lies beyond the three-call-hop witness bound through `{}`.",
            callee.unwrap_or("a called function")
        ),
        InvalidSourceConstraintReason::TrapEvidenceRecursiveCycle => format!(
            "`$infallible` cannot be proved: the witness search returned to an already visited function through `{}` without reaching trap evidence.",
            callee.unwrap_or("a called function")
        ),
        InvalidSourceConstraintReason::EscapingError => {
            "`$infallible` forbids escaping recoverable errors, but this boundary can return an `Error`."
                .to_owned()
        }
    }
}

fn source_constraint_guidance(
    reason: InvalidSourceConstraintReason,
    callee: Option<&str>,
) -> String {
    match reason {
        InvalidSourceConstraintReason::FastOperation
        | InvalidSourceConstraintReason::FastRangeGuard
        | InvalidSourceConstraintReason::FastWriteBack => {
            "Remove `$safe_math`, or move the fast operation outside this boundary.".to_owned()
        }
        InvalidSourceConstraintReason::UnknownFastEvidence => format!(
            "Register a trusted closed numeric contract for `{}`, or keep this boundary from calling it.",
            callee.unwrap_or("the callable")
        ),
        InvalidSourceConstraintReason::UnavailableCalleeFastEvidence => format!(
            "Remove `$safe_math`, or replace `{}` with a callable whose contract proves ordinary semantics.",
            callee.unwrap_or("the callable")
        ),
        InvalidSourceConstraintReason::FastEvidenceBeyondWitnessBound => format!(
            "Inspect `{}` directly; only the first three call hops are rendered.",
            callee.unwrap_or("the named callee")
        ),
        InvalidSourceConstraintReason::FastEvidenceRecursiveCycle => format!(
            "Inspect `{}` directly. The recursive cycle hides the fast evidence that rejects this boundary.",
            callee.unwrap_or("the named callee")
        ),
        InvalidSourceConstraintReason::TrapOverflow
        | InvalidSourceConstraintReason::TrapDivision
        | InvalidSourceConstraintReason::TrapDivideByZero
        | InvalidSourceConstraintReason::TrapExponent
        | InvalidSourceConstraintReason::TrapFloatNonFinite
        | InvalidSourceConstraintReason::TrapFloatDivision => {
            "Prove the operation total with exact operand facts, or remove `$infallible` from this boundary."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapRangeGuard => {
            "Use a literal step proved non-zero and progressing, or remove `$infallible` from this boundary."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapWriteBack => {
            "Use a write-back whose converted value always fits, or remove `$infallible` from this boundary."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapAssertion => {
            "Remove the assertion, prove it cannot fail, or remove `$infallible` from this boundary."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapFloatValidation => {
            "Handle the external float validation explicitly, or remove `$infallible` from this boundary."
                .to_owned()
        }
        InvalidSourceConstraintReason::TrapConversion => {
            "Make the checked operation total, or remove `$infallible` from this boundary.".to_owned()
        }
        InvalidSourceConstraintReason::UnknownTrapEvidence => format!(
            "Register a trusted closed numeric contract for `{}`, or keep this boundary from calling it.",
            callee.unwrap_or("the callable")
        ),
        InvalidSourceConstraintReason::UnavailableCalleeTrapEvidence => format!(
            "Remove `$infallible`, or replace `{}` with a callable whose contract proves trap freedom.",
            callee.unwrap_or("the callable")
        ),
        InvalidSourceConstraintReason::TrapEvidenceBeyondWitnessBound => format!(
            "Inspect `{}` directly; only the first three call hops are rendered.",
            callee.unwrap_or("the named callee")
        ),
        InvalidSourceConstraintReason::TrapEvidenceRecursiveCycle => format!(
            "Inspect `{}` directly. The recursive cycle hides the trap evidence that rejects this boundary.",
            callee.unwrap_or("the named callee")
        ),
        InvalidSourceConstraintReason::EscapingError => {
            "Catch and handle the error inside the boundary, or remove `$infallible`.".to_owned()
        }
    }
}
