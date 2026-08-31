//! Compile-time path base classification for dependency resolution.
//!
//! Stage 0 owns physical file resolution and structural file-value construction. This module
//! retains only the semantic base used while resolving source dependencies and validating their
//! project boundaries.

/// How a dependency path was resolved relative to the project layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileTimePathBase {
    /// Resolved relative to the declaring file (`./` or `../`).
    RelativeToFile,
    /// First segment matched a source-backed package prefix.
    SourcePackageRoot,
    /// Fell through to the configured `entry_root`.
    EntryRoot,
}
