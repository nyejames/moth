//! Stable identity helpers for compiler diagnostics.
//!
//! WHAT: validates compiler-owned qualified reason keys.
//! WHY: reason keys are a compiler-owned contract independent of rendered wording, titles, and
//! `Debug` output, so renderers can evolve without changing structured identity.

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
