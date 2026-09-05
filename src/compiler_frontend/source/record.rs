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

/// Identity, retained source text and loading status for one source record.
///
/// A physical record owns either the exact UTF-8 snapshot used for compilation or the structured
/// error produced while attempting to load it. The reserved compilation root owns neither.
///
/// The load error is boxed because it is absent for every source in a successful build while
/// `CompilerError` is 192 bytes; storing it inline made the failure lane three quarters of every
/// record in the dense identity array.
#[derive(Debug)]
pub struct SourceRecord {
    pub id: super::SourceId,
    pub canonical_os_path: Option<PathBuf>,
    pub logical_path: InternedPath,
    pub text: Option<Box<str>>,
    pub load_error: Option<Box<CompilerError>>,
    pub provenance: SourceProvenance,
}
