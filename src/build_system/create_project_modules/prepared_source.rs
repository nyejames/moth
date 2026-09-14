//! State-safe prepared source input for discovered project compilation.
//!
//! WHAT: one build-system-private owned enum variant per source kind, plus one move-owned
//!       `PreparedSource` slot value per selected `SourceId` and the `PreparedSourceSlots`
//!       store that retains those values in deterministic input order. Directory Moth inputs
//!       carry their final `SourceId` and retained tokens for the one header-preparation pass;
//!       synthetic Moth and Moth-template inputs carry the complete file output produced during
//!       discovery. PlainMarkdown carries only its final `SourceId`; paths and snapshots come
//!       from the authoritative `SourceDatabase`.
//! WHY: the variant makes source-kind ownership explicit. A directory Moth source cannot reach
//!      header preparation without its retained `FileTokens`, while synthetic inputs cannot be
//!      prepared again after their complete outputs have been retained. The slot owner makes the
//!      exactly-once result ownership explicit: each selected source contributes one
//!      `FileFrontendPrepareOutput` whose stamped `file_id` must match its slot's `SourceId`,
//!      without per-source `Arc` or cloned outputs.
//!
//! This type is the build-system-owned transient handoff between Stage 0 source selection and
//! frontend file/header preparation. Each input carries its final source identity plus the one
//! work product that cannot be recomputed without repeating Stage 0 (tokens or complete prepared
//! output); paths and retained snapshots are resolved from the shared source database.

use crate::compiler_frontend::compiler_errors::CompilerError;
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

/// One move-owned preparation result for a selected source identity.
///
/// WHAT: pairs the preparing source's final `SourceId` with the one
///       `FileFrontendPrepareOutput` produced for it.
/// WHY: the slot makes source identity explicit at the exactly-once ownership boundary.
///      A stamped `file_id` that disagrees with the slot owner is a compiler error; an
///      occupied or unfilled slot means the same source was prepared twice or not at all.
pub(crate) struct PreparedSource {
    pub(crate) source_id: SourceId,
    pub(crate) output: FileFrontendPrepareOutput,
}

impl PreparedSource {
    pub(crate) fn new(
        source_id: SourceId,
        output: FileFrontendPrepareOutput,
    ) -> Result<Self, CompilerError> {
        if output.file_id != source_id {
            return Err(CompilerError::compiler_error(format!(
                "prepared source identity {} does not match its slot owner {}",
                output.file_id.index(),
                source_id.index(),
            )));
        }

        Ok(Self { source_id, output })
    }

    fn check_identity(&self) -> Result<(), CompilerError> {
        if self.output.file_id != self.source_id {
            return Err(CompilerError::compiler_error(format!(
                "prepared source identity {} does not match its slot owner {}",
                self.output.file_id.index(),
                self.source_id.index(),
            )));
        }

        Ok(())
    }

    pub(crate) fn into_output(self) -> Result<FileFrontendPrepareOutput, CompilerError> {
        self.check_identity()?;
        Ok(self.output)
    }
}

/// Build-lifetime owner for one module's prepared-source slots.
///
/// WHAT: retains one move-owned `PreparedSource` per selected source in deterministic input
///       order.
/// WHY: slot placement proves each selected file was prepared exactly once. No duplicate
///      preparation can be represented: insertion checks bounds, rejects an occupied slot and
///      rejects an output whose stamped identity disagrees with its slot owner. Consuming the
///      store preserves that input order without per-source `Arc` or cloned outputs.
pub(crate) struct PreparedSourceSlots {
    slots: Vec<Option<PreparedSource>>,
}

impl PreparedSourceSlots {
    pub(crate) fn new(source_count: usize) -> Self {
        let mut slots = Vec::with_capacity(source_count);
        slots.resize_with(source_count, || None);
        Self { slots }
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }


    pub(crate) fn insert(
        &mut self,
        file_index: usize,
        source_id: SourceId,
        output: FileFrontendPrepareOutput,
    ) -> Result<(), CompilerError> {
        let slot_count = self.slots.len();
        let slot = self.slots.get_mut(file_index).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "file preparation record carries file index {file_index} but the module \
                 has only {slot_count} files",
            ))
        })?;

        if slot.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "file preparation record occupies file index {file_index} more than once",
            )));
        }

        *slot = Some(PreparedSource::new(source_id, output)?);
        Ok(())
    }

    pub(crate) fn into_ordered_outputs(self) -> Result<Vec<PreparedSource>, CompilerError> {
        self.slots
            .into_iter()
            .enumerate()
            .map(|(file_index, slot)| {
                slot.ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "file preparation left file index {file_index} unfilled; every \
                         selected source must be prepared exactly once",
                    ))
                })
            })
            .collect()
    }

    pub(crate) fn into_selected_outputs(
        self,
    ) -> Result<Vec<FileFrontendPrepareOutput>, CompilerError> {
        self.slots
            .into_iter()
            .flatten()
            .map(|slot| slot.into_output())
            .collect()
    }
}
