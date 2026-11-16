//! The sole host-path-to-readable-text formatter for user-facing display.
//!
//! WHAT: converts a native host path into lossy, deterministic, platform-independent text.
//! WHY: diagnostics, rendered prose and cross-platform assertions need stable path text, while
//!      filesystem reads, containment checks and path identity must keep native `Path`/`PathBuf`
//!      values.
//!
//! The result is human-readable text only. It is never a path key, security check, URL encoder,
//! filesystem argument or portable on-disk identity. `compiler_frontend::paths` and source path
//! semantics keep their distinct owners, and source diagnostics retain their actual provenance
//! and typed path facts.

use std::path::Path;

#[cfg(windows)]
use crate::compiler_frontend::utilities::basic::normalize_windows_verbatim_path;

/// Render a host filesystem path as deterministic, platform-independent display text.
///
/// WHAT: strips Windows verbatim disk/UNC prefixes, then renders separators as `/`.
/// WHY: diagnostics and cross-platform assertions need stable text that does not expose host
///      spelling. Filesystem operations must continue to use the native path.
pub(crate) fn portable_path_text(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();

    // Verbatim prefixes are a Windows spelling detail, so only Windows pays to strip them.
    #[cfg(windows)]
    let display_path = normalize_windows_verbatim_path(path);
    #[cfg(not(windows))]
    let display_path = path;

    // Synthetic backslash-separated Unix names are rewritten too; display text is lossy by contract.
    display_path.to_string_lossy().replace('\\', "/")
}
