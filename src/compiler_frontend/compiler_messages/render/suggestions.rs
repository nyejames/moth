//! Shared render-local "did you mean?" name suggestion helper.
//!
//! WHAT: exposes a single edit-distance-based closest-name finder used by
//! named-argument, field-access, choice-variant, and unknown-type renderers.
//! WHY: renderers with known candidates should share one deterministic policy.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};

/// Find the closest candidate name for a misspelled identifier.
///
/// WHAT: returns the best candidate from `candidates` when it is "close enough"
/// to `name` by edit distance, otherwise `None`.
/// WHY: the candidate list is already carried by each diagnostic payload.
///
/// The threshold lives only here: at most half the longer name length, with a
/// floor of two so short transpositions still match. Distance must also be
/// strictly less than the longer name length, so names that share no character
/// are never suggested. Comparison is case-insensitive. Equal distances choose
/// the lexically smaller spelling, so candidate order never matters.
pub(crate) fn closest_name_suggestion(
    name: &str,
    candidates: &[StringId],
    string_table: &dyn StringTableResolver,
) -> Option<String> {
    closest_name_suggestion_with_extra(name, candidates, std::iter::empty(), string_table)
}

/// Same policy as [`closest_name_suggestion`], with extra already-resolved spellings.
///
/// WHAT: compares interned candidates and extra static names under one threshold.
/// WHY: builtin type names are scope-independent and must not be stored on every
/// unknown-type diagnostic; the renderer supplies their spellings from the
/// builtin-name owners instead of interned ids.
pub(crate) fn closest_name_suggestion_with_extra<'a>(
    name: &str,
    candidates: &[StringId],
    extra: impl IntoIterator<Item = &'a str>,
    string_table: &dyn StringTableResolver,
) -> Option<String> {
    let mut finder = ClosestNameFinder::new(name);
    for candidate_id in candidates {
        finder.consider(string_table.resolve(*candidate_id));
    }
    for extra_name in extra {
        finder.consider(extra_name);
    }
    finder.finish()
}

struct ClosestNameFinder {
    name_len: usize,
    lower_name: String,
    best: Option<(usize, String)>,
}

impl ClosestNameFinder {
    fn new(name: &str) -> Self {
        Self {
            name_len: name.len(),
            lower_name: name.to_lowercase(),
            best: None,
        }
    }

    fn consider(&mut self, candidate: &str) {
        let distance = levenshtein(&self.lower_name, &candidate.to_lowercase());
        let max_len = self.name_len.max(candidate.len());
        let threshold = (max_len / 2).max(2);

        let improves = self.best.as_ref().is_none_or(|(best_dist, best_name)| {
            (distance, candidate) < (*best_dist, best_name.as_str())
        });
        // Distance equal to `max_len` means the names share no character.
        if distance < max_len && distance <= threshold && improves {
            self.best = Some((distance, candidate.to_owned()));
        }
    }

    fn finish(self) -> Option<String> {
        self.best.map(|(_, name)| name)
    }
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
