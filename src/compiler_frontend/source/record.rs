//! Registered source slots and loaded source snapshots for one frontend compilation.
//!
//! A [`SourceSlot`] is the compact registration row that exists for every candidate, including
//! candidates that never load. A [`SourceRecord`] is only created after a snapshot loads, so it
//! unconditionally owns the exact text and line-start table used by compilation.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use std::path::{Path, PathBuf};

/// Describes how a source record entered the compiler's identity context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceProvenance {
    /// The deterministic synthetic record that anchors project-wide diagnostics.
    CompilationRoot,
    /// Source authored as a physical file.
    AuthoredPhysical,
}

/// Lexical classification of a physical source record.
///
/// WHAT: distinguishes compiler-recognized source from provider-owned identity records.
/// WHY: Stage 0 registers the whole sorted canonical inventory, including provider-owned
///      physical files that exist to hold an identity rather than to be compiled. The
///      provider-owned extension stays on `canonical_os_path`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// The source kind its producer classified from the authored spelling.
    Compiler(SourceFileKind),
    /// A provider-owned physical file that is not compiled.
    ProviderOwned,
}

/// Dense-array index of a loaded [`SourceRecord`].
///
/// The field is private and the type reaches no further than the source module, so only the
/// database can mint one or read it back. No consumer can name another source's loaded row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadedSourceIndex(u32);

impl LoadedSourceIndex {
    pub(super) fn from_index(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("source database cannot contain more than u32::MAX loaded records"),
        )
    }

    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Dense-array index of a cold load failure.
///
/// Carries the same guarantee as [`LoadedSourceIndex`] for the failure array: the database is the
/// only code that can mint one, so no consumer can point a slot at another candidate's failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadFailureIndex(u32);

impl LoadFailureIndex {
    pub(super) fn from_index(index: usize) -> Self {
        Self(
            u32::try_from(index)
                .expect("source database cannot contain more than u32::MAX load failures"),
        )
    }

    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Load status stored by the registration slot rather than by a loaded record.
///
/// WHAT: keeps pending and failed candidates out of the loaded array, while a loaded slot points
///       at exactly one [`SourceRecord`].
/// WHY: a read failure is rare and 192 bytes wide, so it lives in a cold database-owned failure
///      store while the dense registration row keeps only which candidate failed.
#[derive(Debug)]
pub(super) enum SourceLoadStatus {
    /// Identity has been assigned, but loading has not been attempted.
    Pending,
    /// The exact UTF-8 snapshot used for compilation, indexed in the database's loaded array.
    Loaded(LoadedSourceIndex),
    /// The structured read failure for this candidate; no loaded record exists for this slot.
    Failed(LoadFailureIndex),
}

/// One dense row per registered candidate: identity, cold metadata and load status.
///
/// WHAT: owns the identity and registration metadata that every candidate needs, whether or not
///       its source snapshot ever loads.
/// WHY: keeping only load status here leaves the dense row compact; provider-owned or pending
///      candidates carry neither snapshot nor failure payload they never use.
#[derive(Debug)]
pub struct SourceSlot {
    pub id: super::SourceId,
    pub canonical_os_path: Option<PathBuf>,
    pub logical_path: InternedPath,
    pub kind: Option<SourceKind>,
    pub provenance: SourceProvenance,
    pub(super) load: SourceLoadStatus,
}

/// A source that actually loaded.
///
/// A loaded record unconditionally owns the exact UTF-8 snapshot and its line-start table. It has
/// no identity or failure state of its own: the registration [`SourceSlot`] owns that metadata
/// and points here when loading succeeds.
#[derive(Debug)]
pub struct SourceRecord {
    pub(super) text: Box<str>,
    pub(super) line_starts: Box<[u32]>,
}

/// Reject a snapshot that `u32` byte offsets cannot address.
///
/// WHY: a physical source too large to address is the user's file, so it fails in the file lane
/// carrying that source's own identity, exactly as a source-read failure does. Every other
/// provenance is compiler-produced, so its own oversized snapshot is a compiler bug.
pub(super) fn ensure_source_snapshot_fits(
    byte_length: usize,
    provenance: SourceProvenance,
    logical_path: &InternedPath,
    canonical_os_path: Option<&Path>,
) -> Result<(), CompilerError> {
    let limit = u32::MAX as usize;
    if byte_length < limit {
        return Ok(());
    }

    match provenance {
        SourceProvenance::AuthoredPhysical => {
            let mut error = CompilerError::compiler_error(format!(
                "source file {} is {byte_length} bytes; a source must be shorter than {limit} bytes",
                canonical_os_path.unwrap_or(Path::new("<unknown>")).display(),
            ))
            .with_error_type(ErrorType::File);
            // The record's own interned identity is the location, so no path is reinterned here.
            error.location = SourceLocation::new(
                logical_path.clone(),
                CharPosition::default(),
                CharPosition::default(),
            );
            Err(error)
        }
        SourceProvenance::CompilationRoot => Err(CompilerError::compiler_error(format!(
            "compiler-produced source snapshot is {byte_length} bytes; \
             a source must be shorter than {limit} bytes",
        ))),
    }
}
