//! State-safe prepared source input for discovered project compilation.
//!
//! WHAT: one canonical-owner variant, one retained-output variant and one deferred-text variant,
//!       plus one move-owned `PreparedSource` slot value per selected `SourceId` and the
//!       `PreparedSourceSlots` store that retains those values in deterministic input order.
//!       Directory Moth inputs carry their final `SourceId`, one `SourceTokenOwner` and its
//!       preparing path table; synthetic Moth and Moth-template inputs carry one complete
//!       prepared output produced during discovery. Deferred Moth-template and Markdown inputs
//!       carry only their final `SourceId`; source kind, paths and snapshots come from the
//!       authoritative `SourceDatabase`.
//! WHY: the variant makes canonical-token ownership explicit without duplicating source kind. A
//!      directory Moth source cannot reach header preparation without its retained canonical
//!      owner and preparing path table, while synthetic inputs cannot be prepared again after
//!      their complete outputs have been retained. The slot owner makes the exactly-once result
//!      ownership explicit: each selected source contributes one `FileFrontendPrepareOutput`
//!      whose stamped `file_id` must match its slot's `SourceId`, without per-source token copies
//!      or cloned outputs.
//!
//! This type is the build-system-owned transient handoff between Stage 0 source selection and
//! frontend file/header preparation. Each input carries its final source identity plus the one
//! work product that cannot be recomputed without repeating Stage 0 (the canonical token owner
//! and preparing path table, a complete prepared output, or nothing for deferred text); source
//! kind, paths and retained snapshots are resolved from the shared source database.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::headers::SourceTokenOwner;
use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareOutput;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::SourceId;

use std::sync::Arc;

/// Owned prepared source input keyed by its final build-lifetime source identity.
///
/// Construct this only from Stage 0 source preparation. Directory Moth files have already been
/// tokenized once; their canonical `SourceTokenOwner` and preparing path table are carried here.
/// Synthetic Moth and Moth-template files carry one complete prepared output because their
/// complete header output was already produced while discovering the source closure. Deferred
/// Moth-template and Markdown files carry only their `SourceId` because their one preparation pass
/// borrows retained text directly.
///
/// Source kind, paths and snapshots belong to the final `SourceDatabase`. The source ID is the
/// only identity carried by this transient handoff besides the canonical token owner metadata.
pub(crate) struct PreparedSourceInput {
    pub(crate) source_id: SourceId,
    pub(crate) source: PreparedSourceKind,
}

pub(crate) enum PreparedSourceKind {
    /// A Moth module source with one canonical token owner and its preparing path table.
    Moth {
        owner: SourceTokenOwner,
        path_syntax: Arc<PathSyntaxTable>,
    },
    /// Complete Moth or Moth-template syntax retained by private synthetic discovery.
    MothPrepared {
        output: Box<FileFrontendPrepareOutput>,
    },
    /// A Moth-template or Markdown body awaiting its one preparation pass. The source kind is
    /// resolved from the authoritative `SourceDatabase` at conversion time.
    Deferred,
}
impl PreparedSourceInput {
    pub(crate) fn source_id(&self) -> SourceId {
        self.source_id
    }
}

/// One move-owned preparation result for a selected source identity.
///
/// WHAT: owns the one `FileFrontendPrepareOutput` whose stamped identity was validated against
///       the selected source ID at construction.
/// WHY: the slot makes source identity explicit at the exactly-once ownership boundary. A
///      stamped `file_id` that disagrees with the slot owner is rejected before the output can be
///      retained; an occupied or unfilled slot means the same source was prepared twice or not at
///      all.
pub(crate) struct PreparedSource {
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

        Ok(Self { output })
    }

    pub(crate) fn into_output(self) -> FileFrontendPrepareOutput {
        self.output
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

    /// Verify that every slot is occupied before a caller that requires a complete module
    /// preparation consumes the store.
    pub(crate) fn ensure_filled(&self) -> Result<(), CompilerError> {
        for (file_index, slot) in self.slots.iter().enumerate() {
            if slot.is_none() {
                return Err(CompilerError::compiler_error(format!(
                    "file preparation left file index {file_index} unfilled; every \
                     selected source must be prepared exactly once",
                )));
            }
        }
        Ok(())
    }

    /// Consume occupied prepared sources in their deterministic input order.
    ///
    /// Directory reachability may leave candidate slots unfilled because candidates include files
    /// that were not reached. Callers with a complete fixed-size preparation set must invoke
    /// `ensure_filled` before consuming this handoff.
    pub(crate) fn into_ordered_outputs(self) -> Vec<PreparedSource> {
        self.slots.into_iter().flatten().collect()
    }
}
