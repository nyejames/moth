//! State-safe prepared source input for discovered project compilation.
//!
//! WHAT: one build-system-private owned enum variant per source kind. The Moth variant
//!       carries only its byte length and the `FileTokens` produced by the single Stage 0 lexical
//!       pass; the Moth template and PlainMarkdown variants carry raw source text.
//! WHY: the variant makes the source-kind/token relationship unrepresentable as an invalid
//!      state. A discovered Moth source always carries its `FileTokens` by type, so frontend
//!      header preparation receives tokens directly and cannot panic on absent tokens, while
//!      Moth template and PlainMarkdown cannot accidentally carry Moth tokens.
//!
//! This type is the build-system-owned handoff threaded through `ReachableSourceInventory`
//! assembly, `ModuleCompilationJob`, single-file compilation and `FrontendModuleBuildContext`.

use crate::compiler_frontend::tokenizer::tokens::FileTokens;

use std::path::{Path, PathBuf};

/// Owned prepared source input carrying the strict source-kind/token relationship.
///
/// Construct this only from Stage 0 source preparation. Moth files must already have
/// been tokenized once; the retained `FileTokens` are carried here so header preparation never
/// lexes the same source again.
///
/// The Moth `tokens` are boxed so the enum is not sized by `FileTokens` (which is large). Moth
/// source text is consumed for the byte-length fact before tokenization and is not retained after
/// Stage 0; template and Markdown bodies retain their text because their preparation paths consume
/// it.
pub(crate) enum PreparedSourceInput {
    /// A Moth module source with the token stream from its single lexical pass.
    Moth {
        source_byte_len: usize,
        source_path: PathBuf,
        tokens: Box<FileTokens>,
    },
    /// A Moth template body, tokenized once by the template-body preparation path.
    MothTemplate {
        source_code: String,
        source_path: PathBuf,
    },
    /// Plain Markdown content, never tokenized.
    PlainMarkdown {
        source_code: String,
        source_path: PathBuf,
    },
}

impl PreparedSourceInput {
    /// Source byte length retained as a compact fact for capacity and timing accounting.
    pub(crate) fn source_byte_len(&self) -> usize {
        match self {
            PreparedSourceInput::Moth {
                source_byte_len, ..
            } => *source_byte_len,
            PreparedSourceInput::MothTemplate { source_code, .. }
            | PreparedSourceInput::PlainMarkdown { source_code, .. } => source_code.len(),
        }
    }

    /// Canonical source path used for identity and source-table registration.
    pub(crate) fn source_path(&self) -> &Path {
        match self {
            PreparedSourceInput::Moth { source_path, .. }
            | PreparedSourceInput::MothTemplate { source_path, .. }
            | PreparedSourceInput::PlainMarkdown { source_path, .. } => source_path,
        }
    }

    /// Whether this selected source is a Moth template body.
    pub(crate) fn is_moth_template(&self) -> bool {
        matches!(self, Self::MothTemplate { .. })
    }
}
