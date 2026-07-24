//! State-safe prepared source input for discovered project compilation.
//!
//! WHAT: one build-system-private owned enum variant per source kind. The Moth variant
//!       carries the retained `FileTokens` from the single Stage 0 lexical pass; the Moth template and
//!       PlainMarkdown variants carry only raw source text.
//! WHY: the variant makes the source-kind/token relationship unrepresentable as an invalid
//!      state. A discovered Moth source always carries its `FileTokens` by type, so frontend
//!      header preparation receives tokens directly and cannot panic on absent tokens, while
//!      Moth template and PlainMarkdown cannot accidentally carry Moth tokens.
//!
//! This type is the build-system-owned storage threaded through `ReachableSourceInventory`
//! assembly, `DiscoveredModule`, single-file compilation and `FrontendModuleBuildContext`.

use crate::compiler_frontend::tokenizer::tokens::FileTokens;

use std::path::{Path, PathBuf};

/// Owned prepared source input carrying the strict source-kind/token relationship.
///
/// Construct this only from Stage 0 reachable-file discovery. Moth files must already have
/// been tokenized once; the retained `FileTokens` are carried here so header preparation never
/// lexes the same source again.
///
/// The Moth `tokens` are boxed so the enum is not sized by `FileTokens` (which is large);
/// the Moth template and PlainMarkdown variants stay small and moving a `PreparedSourceInput` only
/// copies a pointer for the retained token stream.
pub(crate) enum PreparedSourceInput {
    /// A Moth module source with the retained token stream from its single lexical pass.
    Moth {
        source_code: String,
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
    /// Raw source text for byte-counting and diagnostics.
    pub(crate) fn source_code(&self) -> &str {
        match self {
            PreparedSourceInput::Moth { source_code, .. }
            | PreparedSourceInput::MothTemplate { source_code, .. }
            | PreparedSourceInput::PlainMarkdown { source_code, .. } => source_code,
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
}
