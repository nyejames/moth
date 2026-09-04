//! Retained identity metadata for one frontend source record.

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

/// Identity and path metadata for one retained source record.
#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub id: super::SourceId,
    pub canonical_os_path: Option<PathBuf>,
    pub logical_path: InternedPath,
    pub provenance: SourceProvenance,
}
