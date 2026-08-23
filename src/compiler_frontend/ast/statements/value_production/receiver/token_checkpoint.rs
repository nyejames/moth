//! Token stream rollback for uncommitted single-predicate speculation.
//!
//! WHAT: snapshots `token_stream.index` before parsing a possible scrutinee.
//! WHY: `classify_if_header` can mark a header as a potential single predicate
//! while the scrutinee is not option/choice-eligible. Authored diagnostics from
//! that uncommitted parse restore the stream so Bool parsing can proceed.
//! Infrastructure errors never restore and never fall back.
//!
//! Rollback is valid only before `is` is consumed and the pattern parser starts.

use crate::compiler_frontend::tokenizer::tokens::FileTokens;

/// A lightweight snapshot of the token stream index.
///
/// WHAT: records `token_stream.index` so a speculative parse can be rolled back.
/// WHY: this is cheaper and clearer than manual `let start = stream.index;`
/// followed by `stream.index = start;` at every speculative site.
pub(super) struct TokenCheckpoint {
    index: usize,
}

impl TokenCheckpoint {
    /// Capture the current token index.
    pub(super) fn capture(token_stream: &FileTokens) -> Self {
        Self {
            index: token_stream.index,
        }
    }

    /// Restore the token stream to the captured index.
    pub(super) fn restore(self, token_stream: &mut FileTokens) {
        token_stream.index = self.index;
    }

    /// Consume the checkpoint without restoring, marking speculation as successful.
    pub(super) fn commit(self) {}
}
