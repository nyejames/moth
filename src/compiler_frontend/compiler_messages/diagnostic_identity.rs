//! Stable identity facts for compiler diagnostics.
//!
//! WHAT: exposes the diagnostic code, actual severity, and optional typed reason key needed by
//! compiler tooling and tests.
//! WHY: reason keys are a compiler-owned contract independent of rendered wording, titles, and
//! `Debug` output, so renderers can evolve without changing structured identity.

#[cfg(test)]
use super::diagnostic_payload::DiagnosticPayload;
#[cfg(test)]
use super::diagnostic_severity::DiagnosticSeverity;

/// Returns whether `reason` uses the compiler's qualified lower-snake-case identity format.
///
/// A reason key has at least two dot-separated segments. Each segment starts with an ASCII
/// lowercase letter, then contains only lowercase letters, digits, and underscores. Segments
/// cannot end with or contain consecutive underscores.
pub(crate) fn is_well_formed_reason_key(reason: &str) -> bool {
    let mut segment_count = 0;

    for segment in reason.split('.') {
        segment_count += 1;
        if !is_well_formed_reason_key_segment(segment) {
            return false;
        }
    }

    segment_count >= 2
}

fn is_well_formed_reason_key_segment(segment: &str) -> bool {
    let mut characters = segment.chars();
    let Some(first_character) = characters.next() else {
        return false;
    };

    if !first_character.is_ascii_lowercase() {
        return false;
    }

    let mut previous_was_underscore = false;
    for character in characters {
        if character == '_' {
            if previous_was_underscore {
                return false;
            }
            previous_was_underscore = true;
        } else if character.is_ascii_lowercase() || character.is_ascii_digit() {
            previous_was_underscore = false;
        } else {
            return false;
        }
    }

    !previous_was_underscore
}

// The identity record is the compiler-facing boundary value consumed by tests and tooling.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticIdentity {
    pub(crate) code: &'static str,
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) reason_key: Option<&'static str>,
}

#[cfg(test)]
impl DiagnosticIdentity {
    // Keep construction beside the identity record so callers cannot rebuild the reason bridge.
    pub(super) fn new(
        code: &'static str,
        severity: DiagnosticSeverity,
        payload: &DiagnosticPayload,
    ) -> Self {
        Self {
            code,
            severity,
            reason_key: payload.stable_reason_key(),
        }
    }
}
