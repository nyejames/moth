//! State-safe prepared source input for discovered project compilation.
//!
//! WHAT: one build-system-private owned enum variant per source kind. Directory Moth inputs carry
//!       their final `SourceId` and retained tokens for the one header-preparation pass; synthetic
//!       Moth and Moth-template inputs carry the complete file output produced during discovery.
//!       PlainMarkdown carries only its final `SourceId`; paths and snapshots come from the
//!       authoritative `SourceDatabase`.
//! WHY: the variant makes source-kind ownership explicit. A directory Moth source cannot reach
//!      header preparation without its retained `FileTokens`, while synthetic inputs cannot be
//!      prepared again after their complete outputs have been retained.
//!
//! This type is the build-system-owned transient handoff between Stage 0 source selection and
//! frontend file/header preparation. Each input carries its final source identity plus the one
//! work product that cannot be recomputed without repeating Stage 0 (tokens or complete prepared
//! output); paths and retained snapshots are resolved from the shared source database.

use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareOutput;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

/// Owned prepared source input keyed by its final build-lifetime source identity.
///
/// Construct this only from Stage 0 source preparation. Directory Moth files have already been
/// tokenized once; their retained `FileTokens` are carried here so header preparation never lexes
/// the same source again. Synthetic Moth and Moth-template files carry the corresponding prepared
/// variants because their complete header output was already produced while discovering the
/// source closure.
///
/// Source paths and snapshots belong to the final `SourceDatabase`. The source ID is the only
/// identity carried by this transient handoff, so consumers resolve paths and retained text from
/// one authoritative database.
pub(crate) enum PreparedSourceInput {
    /// A Moth module source with the token stream from its single lexical pass.
    Moth {
        source_id: SourceId,
        tokens: Box<FileTokens>,
        span_builder: ExtendedSpanBuilder,
    },
    /// A Moth file whose complete header output was retained during synthetic discovery.
    ///
    /// The output owns its header token substreams, clause shell and selection table. It is
    /// consumed directly by module aggregation; no raw token stream or second file preparation
    /// is available on this variant.
    MothPrepared {
        source_id: SourceId,
        output: Box<FileFrontendPrepareOutput>,
    },
    /// A Moth-template file whose complete header output was retained during synthetic discovery.
    MothTemplatePrepared {
        source_id: SourceId,
        output: Box<FileFrontendPrepareOutput>,
    },
    /// A Moth template body, tokenized once by the template-body preparation path.
    MothTemplate { source_id: SourceId },
    /// Plain Markdown content, never tokenized.
    PlainMarkdown { source_id: SourceId },
}

impl PreparedSourceInput {
    pub(crate) fn source_id(&self) -> SourceId {
        match self {
            Self::Moth { source_id, .. }
            | Self::MothPrepared { source_id, .. }
            | Self::MothTemplatePrepared { source_id, .. }
            | Self::MothTemplate { source_id }
            | Self::PlainMarkdown { source_id } => *source_id,
        }
    }

    /// Whether this selected source is a Moth template body.
    pub(crate) fn is_moth_template(&self) -> bool {
        matches!(
            self,
            Self::MothTemplate { .. } | Self::MothTemplatePrepared { .. }
        )
    }
}
