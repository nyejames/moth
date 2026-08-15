//! State-safe prepared source input for discovered project compilation.
//!
//! WHAT: one build-system-private owned enum variant per source kind. Directory Moth inputs carry
//!       retained tokens for the one header-preparation pass; synthetic Moth inputs carry the
//!       complete file output produced during discovery. Moth template and PlainMarkdown variants
//!       carry raw source text.
//! WHY: the variant makes source-kind ownership explicit. A directory Moth source cannot reach
//!      header preparation without its retained tokens, while a synthetic Moth source cannot be
//!      prepared again after its complete output has been retained. Templates and Markdown cannot
//!      accidentally carry Moth tokens.
//!
//! This type is the build-system-owned transient handoff between Stage 0 source selection and
//! frontend file/header preparation. It is consumed before `PreparedModule` reaches semantic
//! compilation; directory module jobs retain only that completed module payload.

use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareOutput;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

use std::path::{Path, PathBuf};

/// Owned prepared source input carrying the strict source-kind/token relationship.
///
/// Construct this only from Stage 0 source preparation. Directory Moth files have already been
/// tokenized once; their retained `FileTokens` are carried here so header preparation never lexes
/// the same source again. Synthetic Moth files use [`PreparedSourceInput::MothPrepared`] instead,
/// because their complete header output was already produced while discovering the source
/// closure.
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
    /// A Moth file whose complete header output was retained during synthetic discovery.
    ///
    /// The output owns its header token substreams, clause shell and selection table. It is
    /// consumed directly by module aggregation; no raw token stream or second file preparation
    /// is available on this variant.
    MothPrepared {
        source_byte_len: usize,
        source_path: PathBuf,
        output: Box<FileFrontendPrepareOutput>,
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
            }
            | PreparedSourceInput::MothPrepared {
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
            | PreparedSourceInput::MothPrepared { source_path, .. }
            | PreparedSourceInput::MothTemplate { source_path, .. }
            | PreparedSourceInput::PlainMarkdown { source_path, .. } => source_path,
        }
    }

    /// Whether this selected source is a Moth template body.
    pub(crate) fn is_moth_template(&self) -> bool {
        matches!(self, Self::MothTemplate { .. })
    }
}
