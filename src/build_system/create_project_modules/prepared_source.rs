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
use crate::compiler_frontend::source::SourceId;
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
pub(crate) struct PreparedSourceInput {
    pub(crate) source_id: SourceId,
    pub(crate) source: PreparedSourceKind,
}

pub(crate) enum PreparedSourceKind {
    /// A Moth module source with tokens from its single lexical pass.
    Moth { tokens: Box<FileTokens> },
    /// Complete Moth syntax retained by private synthetic discovery.
    MothPrepared {
        output: Box<FileFrontendPrepareOutput>,
    },
    /// Complete Moth-template syntax retained by private synthetic discovery.
    MothTemplatePrepared {
        output: Box<FileFrontendPrepareOutput>,
    },
    /// A Moth-template body awaiting its one template-body preparation pass.
    MothTemplate,
    /// Plain Markdown content, never tokenized.
    PlainMarkdown,
}

impl PreparedSourceInput {
    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }

    /// Whether this selected source is a Moth template body.
    pub(crate) fn is_moth_template(&self) -> bool {
        matches!(
            &self.source,
            PreparedSourceKind::MothTemplate | PreparedSourceKind::MothTemplatePrepared { .. }
        )
    }
}
