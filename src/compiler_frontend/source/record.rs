//! Retained identity metadata for one frontend source record.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use std::ops::Range;
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

/// Identity and loading lifecycle for one source record.
///
/// A physical record is registered before its snapshot is loaded. It then becomes either loaded
/// with the exact UTF-8 snapshot used for compilation or unreadable with the structured error
/// produced while attempting to load it. The reserved compilation root remains registered.
///
/// The load error is boxed because it is absent for every source in a successful build while
/// `CompilerError` is 192 bytes; storing it inline would make the failure lane three quarters of
/// every record in the dense identity array.
#[derive(Debug)]
pub(super) enum SourceRecordState {
    /// Identity has been assigned, but loading has not been attempted.
    Registered,
    /// The exact UTF-8 snapshot used for compilation, plus its line-start table.
    Loaded {
        text: Box<str>,
        line_starts: Box<[u32]>,
    },
    /// The structured error produced while attempting to load the source.
    Unreadable(Box<CompilerError>),
}

impl SourceRecordState {
    pub(super) fn retained_text(&self) -> Option<&str> {
        match self {
            Self::Loaded { text, .. } => Some(text),
            Self::Registered | Self::Unreadable(_) => None,
        }
    }

    /// Report whether this record is still awaiting its snapshot.
    ///
    /// WHY: the source-size policy applies to a snapshot this record is about to accept. A
    /// record that already holds one is a lifecycle violation the caller must report as a
    /// compiler bug, so an oversized second snapshot must not preempt it with a user-facing
    /// source-size failure.
    pub(super) fn is_registered(&self) -> bool {
        matches!(self, Self::Registered)
    }

    pub(super) fn source_load_error(&self) -> Option<&CompilerError> {
        match self {
            Self::Unreadable(error) => Some(error),
            Self::Registered | Self::Loaded { .. } => None,
        }
    }

    /// Move a registered record to its loaded snapshot.
    pub(super) fn retain_text(
        &mut self,
        id: super::SourceId,
        text: String,
    ) -> Result<(), CompilerError> {
        if !matches!(self, Self::Registered) {
            return Err(CompilerError::compiler_error(format!(
                "source text for source identity {} was retained more than once",
                id.index()
            )));
        }

        let line_starts = line_start_offsets(&text);
        *self = Self::Loaded {
            text: text.into_boxed_str(),
            line_starts,
        };
        Ok(())
    }

    /// Move a registered record to its recorded read failure.
    pub(super) fn record_load_error(
        &mut self,
        id: super::SourceId,
        error: CompilerError,
    ) -> Result<(), CompilerError> {
        if !matches!(self, Self::Registered) {
            return Err(CompilerError::compiler_error(format!(
                "source load status for source identity {} was recorded more than once",
                id.index()
            )));
        }

        *self = Self::Unreadable(Box::new(error));
        Ok(())
    }
}

/// Identity, loading state and cold path metadata for one source record.
#[derive(Debug)]
pub struct SourceRecord {
    pub id: super::SourceId,
    pub canonical_os_path: Option<PathBuf>,
    pub logical_path: InternedPath,
    pub(super) state: SourceRecordState,
    /// Kind of this source's authored spelling, absent only for the reserved compilation root.
    ///
    /// WHAT: stores whether this physical record is a compiler-recognized source or
    ///       provider-owned, as classified by the producer that registered it.
    /// WHY: the producer knows the authored name, and canonicalize can resolve a recognized
    ///      spelling onto a target whose extension names another kind. Storing the answer also
    ///      spares every consumer holding a `SourceId` from re-parsing a cold path. `None`
    ///      identifies the reserved compilation root, which is not a file.
    pub kind: Option<SourceKind>,
    pub provenance: SourceProvenance,
}

impl SourceRecord {
    pub(crate) fn retained_text(&self) -> Option<&str> {
        self.state.retained_text()
    }

    /// Number of lines in the retained snapshot, or zero when this record is not loaded.
    ///
    /// An empty snapshot has no lines, so the table is empty and this is zero. Slice 1C4 owns
    /// the final empty-file, final-newline and zero-width-EOF semantics.
    pub(crate) fn line_count(&self) -> u32 {
        match &self.state {
            SourceRecordState::Loaded { line_starts, .. } => line_starts.len() as u32,
            SourceRecordState::Registered | SourceRecordState::Unreadable(_) => 0,
        }
    }

    /// Byte range of one zero-based line: this start through the next start, or EOF.
    ///
    /// The range includes a terminating `\n` when one was authored. Lookup is an index into the
    /// line-start table, not a scan of the snapshot.
    pub(crate) fn line_byte_range(&self, line_number: u32) -> Option<Range<u32>> {
        let SourceRecordState::Loaded { text, line_starts } = &self.state else {
            return None;
        };
        let index = line_number as usize;
        let start = *line_starts.get(index)?;
        let end = line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(text.len() as u32);
        Some(start..end)
    }
}

/// Build the line-start table for a snapshot that has just become owned.
///
/// A `\n` byte cannot occur inside a multi-byte UTF-8 sequence, so a byte scan is exact. The
/// first entry is always `0`. Each later entry is the byte immediately after a `\n`. A trailing
/// newline terminates the last authored line; it does not start another, so a start at
/// `text.len()` is not recorded. An empty snapshot has no lines and so no entries, which keeps
/// the rule in the table rather than in every consumer that would otherwise special-case it.
///
/// WHY two passes: counting is a branchless reduction the compiler vectorises, and it sizes the
/// table exactly, so the fill never reallocates. Growing a `Vec` while scanning would copy the
/// table roughly once per doubling for no benefit.
fn line_start_offsets(text: &str) -> Box<[u32]> {
    let bytes = text.as_bytes();
    let Some((_, leading_bytes)) = bytes.split_last() else {
        return Box::default();
    };

    // A newline in the final byte terminates its line without starting another, so only the
    // earlier bytes can contribute a start. Each of those newlines contributes exactly one,
    // alongside the leading zero.
    let line_count = 1 + leading_bytes.iter().filter(|byte| **byte == b'\n').count();

    let mut line_starts = Vec::with_capacity(line_count);
    line_starts.push(0);

    for (offset, byte) in leading_bytes.iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(offset as u32 + 1);
        }
    }

    debug_assert_eq!(
        line_starts.len(),
        line_count,
        "the counting pass must predict the fill exactly, or the fill reallocates"
    );
    line_starts.into_boxed_slice()
}

/// Reject a snapshot that `u32` byte offsets cannot address.
///
/// WHY: a physical source too large to address is the user's file, so it fails in the file lane
/// carrying that source's own identity, exactly as an unreadable source does. Every other
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
