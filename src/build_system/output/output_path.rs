//! Portable output-path parsing and deterministic identity.
//!
//! WHAT: owns the one portable relative-path parser that interprets both `/` and `\` as
//! separators on every host, and the deterministic ASCII-case-folded identity derived from its
//! validated components.
//! WHY: output-folder classification, development/release root comparison and destination
//! collision checks must all consume the same validated components rather than each owning a
//! separate lexical parser. `std::path` component classification is host-dependent (Windows
//! prefixes are only recognised on Windows), so the parser handles every host explicitly.

use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

use std::path::Path;

/// Portable validated relative-path components.
///
/// WHAT: holds the ASCII-lowercased normal components of a validated project-relative path.
/// WHY: the identity key is derived directly from these components, so `dev` and `DEV` compare
/// equal and spelling with either `/` or `\` normalises to the same value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PortableRelativePath {
    components: Vec<String>,
}

impl PortableRelativePath {
    fn identity_string(&self) -> String {
        self.components.join("/")
    }
}

/// Case-folded output-path identity.
///
/// WHAT: wraps the deterministic ASCII-lowercased spelling of a validated relative path, used
/// only for equality comparisons.
/// WHY: development and release roots must compare through one stable key that does not depend
/// on host case semantics or separator spelling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct OutputPathIdentity(String);

/// Parse a portable relative path on every host.
///
/// Both `/` and `\` are separators. Rejects empty, rooted and platform-prefix spellings as well
/// as authored `.` and `..` segments, without relying on host-dependent `std::path` component
/// classification.
pub(crate) fn parse_relative_path(
    raw: &str,
) -> Result<PortableRelativePath, InvalidOutputFolderReason> {
    if raw.is_empty() {
        return Err(InvalidOutputFolderReason::Empty);
    }
    if raw.starts_with(['/', '\\']) {
        return Err(InvalidOutputFolderReason::AbsolutePath);
    }

    let mut components = Vec::new();
    for segment in raw.split(['/', '\\']) {
        if segment.is_empty() {
            return Err(InvalidOutputFolderReason::Empty);
        }
        if segment.contains(':') {
            return Err(InvalidOutputFolderReason::RootOrPrefix);
        }
        if segment == "." {
            return Err(InvalidOutputFolderReason::CurrentDirectory);
        }
        if segment == ".." {
            return Err(InvalidOutputFolderReason::ParentDirectorySegment);
        }
        components.push(segment.to_ascii_lowercase());
    }

    Ok(PortableRelativePath { components })
}

/// Build the deterministic identity for a project-relative path.
///
/// Returns an error when the path is not a valid portable relative path, so invalid components
/// such as roots, prefixes or parent segments are never silently ignored.
pub(crate) fn output_path_identity(
    relative: &Path,
) -> Result<OutputPathIdentity, InvalidOutputFolderReason> {
    parse_relative_path(&relative.to_string_lossy())
        .map(|portable| OutputPathIdentity(portable.identity_string()))
}
