//! Automatic `$md` heading IDs.
//!
//! WHAT: normalises a heading's static visible label into an HTML fragment ID while the
//! inline renderer walks that label, so no separate label string is collected first.
//! WHY: Moth-aware links such as `@#section (Section)` address `$md` sections through these
//! IDs. The mapping is pure and independent of document order, so equal labels produce
//! equal IDs and adding an earlier heading never renames a later fragment. The inline
//! renderer owns label scope and discards the builder when an opaque anchor makes the
//! label unknown.

/// Streaming normaliser for one heading label.
///
/// The generated alphabet is lowercase Unicode letters and digits plus ASCII `-`, so the
/// ID can be written into an HTML attribute without escaping.
#[derive(Debug, Default)]
pub(super) struct HeadingIdBuilder {
    id: String,
    pending_separator: bool,
}

impl HeadingIdBuilder {
    /// Feeds one visible label character.
    ///
    /// Whitespace, `_` and `-` runs collapse into one separator that is only written once
    /// a later letter or digit arrives, which trims leading and trailing separators.
    /// Other punctuation is discarded without ending a separator run.
    pub(super) fn push_label_char(&mut self, label_char: char) {
        if label_char.is_whitespace() || matches!(label_char, '_' | '-') {
            self.pending_separator = !self.id.is_empty();
            return;
        }

        for lowered_char in label_char.to_lowercase() {
            if !lowered_char.is_alphanumeric() {
                continue;
            }

            if self.pending_separator {
                self.id.push('-');
                self.pending_separator = false;
            }
            self.id.push(lowered_char);
        }
    }

    /// Returns the ID, or `None` when the label held no letters or digits.
    pub(super) fn finish(self) -> Option<String> {
        if self.id.is_empty() {
            return None;
        }

        Some(self.id)
    }
}
