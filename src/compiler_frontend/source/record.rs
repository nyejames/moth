//! Retained identity metadata for one frontend source record.

use crate::compiler_frontend::symbols::interned_path::InternedPath;
use std::path::PathBuf;

/// Identity and path metadata for one retained source file.
#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub id: super::SourceId,
    pub canonical_os_path: PathBuf,
    pub logical_path: InternedPath,
}
