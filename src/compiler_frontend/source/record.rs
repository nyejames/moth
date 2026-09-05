//! Retained identity metadata for one frontend source record.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use std::path::PathBuf;

/// Describes how a source record entered the compiler's identity context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceProvenance {
    /// The deterministic synthetic record that anchors project-wide diagnostics.
    CompilationRoot,
    /// Source authored as a physical file.
    AuthoredPhysical,
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
    /// The exact UTF-8 snapshot used for compilation.
    Loaded(Box<str>),
    /// The structured error produced while attempting to load the source.
    Unreadable(Box<CompilerError>),
}

impl SourceRecordState {
    pub(super) fn retained_text(&self) -> Option<&str> {
        match self {
            Self::Loaded(text) => Some(text),
            Self::Registered | Self::Unreadable(_) => None,
        }
    }

    pub(super) fn source_load_error(&self) -> Option<&CompilerError> {
        match self {
            Self::Unreadable(error) => Some(error),
            Self::Registered | Self::Loaded(_) => None,
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

        *self = Self::Loaded(text.into_boxed_str());
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
    pub provenance: SourceProvenance,
}
