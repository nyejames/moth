//! Shared render-local "did you mean?" name suggestion helper.
//!
//! WHAT: exposes a single edit-distance-based closest-name finder used by
//! named-argument, field-access, and choice-variant renderers.
//! WHY: renderers with known candidates should share one deterministic policy.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};

/// Find the closest candidate name for a misspelled identifier.
///
/// WHAT: returns the best candidate from `candidates` when it is "close enough"
/// to `name` by edit distance, otherwise `None`.
/// WHY: the candidate list is already carried by each diagnostic payload.
///
/// The threshold mirrors the original call-argument policy: at most half the
/// longer name length, with a floor of two so short transpositions still match.
/// Comparison is case-insensitive.
pub(crate) fn closest_name_suggestion(
    name: &str,
    candidates: &[StringId],
    string_table: &dyn StringTableResolver,
) -> Option<String> {
    let lower_name = name.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for candidate_id in candidates {
        let candidate = string_table.resolve(*candidate_id);
        let distance = levenshtein(&lower_name, &candidate.to_lowercase());
        let max_len = name.len().max(candidate.len());
        let threshold = (max_len / 2).max(2);

        if distance <= threshold && best.is_none_or(|(best_dist, _)| distance < best_dist) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, name)| name.to_owned())
}

/// Simple Levenshtein distance for short identifiers.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut previous = (0..=b_len).collect::<Vec<_>>();
    let mut current = vec![0; b_len + 1];

    for i in 1..=a_len {
        current[0] = i;
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b_len]
}
