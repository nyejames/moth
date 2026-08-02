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

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Portable validated relative-path components.
///
/// WHAT: holds the normal components of a validated project-relative path.
/// WHY: the identity key is derived directly from these components, so `dev` and `DEV` compare
/// equal and spelling with either `/` or `\` normalises to the same value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PortableRelativePath {
    components: Vec<String>,
}

impl PortableRelativePath {
    fn identity_string(&self) -> String {
        self.components
            .iter()
            .map(|component| component.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn to_path_buf(&self) -> PathBuf {
        let mut path = PathBuf::new();
        for component in &self.components {
            path.push(component);
        }
        path
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DanglingSymlink;

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
        if has_invalid_portable_component_spelling(segment) {
            return Err(InvalidOutputFolderReason::InvalidPathComponent);
        }
        components.push(segment.to_owned());
    }

    Ok(PortableRelativePath { components })
}

/// Reject names that Windows normalises, reserves or cannot represent safely.
///
/// WHAT: applies the strictest common component rules on every host.
/// WHY: a manifest or output batch prepared on Unix must remain unambiguous when consumed on
/// Windows, where trailing dots/spaces, device basenames and reserved characters have aliases.
fn has_invalid_portable_component_spelling(component: &str) -> bool {
    component.ends_with('.')
        || component.ends_with(' ')
        || component.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
        })
        || is_reserved_windows_device_basename(component)
}

fn is_reserved_windows_device_basename(component: &str) -> bool {
    let basename = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' ']);
    let uppercase = basename.to_ascii_uppercase();
    if matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }

    let is_numbered_device = (uppercase.starts_with("COM") || uppercase.starts_with("LPT"))
        && uppercase.len() == 4
        && uppercase.as_bytes()[3].is_ascii_digit()
        && uppercase.as_bytes()[3] != b'0';
    is_numbered_device
        || matches!(
            uppercase.as_str(),
            "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
        )
}

/// Build the deterministic identity for a project-relative path.
///
/// Returns an error when the path is not a valid portable relative path, so invalid components
/// such as roots, prefixes or parent segments are never silently ignored.
pub(crate) fn output_path_identity(
    relative: &Path,
) -> Result<OutputPathIdentity, InvalidOutputFolderReason> {
    let raw = relative
        .to_str()
        .ok_or(InvalidOutputFolderReason::NonUtf8)?;
    parse_relative_path(raw).map(|portable| OutputPathIdentity(portable.identity_string()))
}

/// Build the deterministic identity of every component in a project-relative path.
pub(crate) fn output_path_component_identities(
    relative: &Path,
) -> Result<Vec<String>, InvalidOutputFolderReason> {
    let raw = relative
        .to_str()
        .ok_or(InvalidOutputFolderReason::NonUtf8)?;
    parse_relative_path(raw).map(|portable| {
        portable
            .components
            .into_iter()
            .map(|component| component.to_ascii_lowercase())
            .collect()
    })
}

/// Check whether a normalized relative path begins with a case-folded reserved component.
pub(crate) fn path_starts_with_component_identity(
    relative: &Path,
    reserved_component: &Path,
) -> bool {
    let Ok(relative_components) = output_path_component_identities(relative) else {
        return false;
    };
    let Ok(reserved_components) = output_path_component_identities(reserved_component) else {
        return false;
    };
    let Some(first_component) = relative_components.first() else {
        return false;
    };
    let Some(reserved_component) = reserved_components.first() else {
        return false;
    };

    first_component == reserved_component
}

/// Normalize a portable relative path into the host path representation used for filesystem IO.
pub(crate) fn normalize_relative_path(
    relative: &Path,
) -> Result<PathBuf, InvalidOutputFolderReason> {
    let raw = relative
        .to_str()
        .ok_or(InvalidOutputFolderReason::NonUtf8)?;
    parse_relative_path(raw).map(|portable| portable.to_path_buf())
}

/// Check whether a filesystem-relative path can be serialized and parsed without changing it.
pub(crate) fn is_lossless_portable_relative_path(relative: &Path) -> bool {
    let Some(spelling) = relative.to_str() else {
        return false;
    };
    if spelling.contains(['\r', '\n']) {
        return false;
    }

    normalize_relative_path(relative)
        .map(|normalized| normalized == relative)
        .unwrap_or(false)
}

/// Canonicalize a path when it exists, or canonicalize its nearest existing ancestor and retain
/// the missing suffix.
fn canonicalize_or_nearest_ancestor(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let mut ancestor = path.to_path_buf();
    while let Some(parent) = ancestor.parent() {
        if let Ok(canonical) = fs::canonicalize(parent) {
            let suffix = path.strip_prefix(parent).unwrap_or(Path::new(""));
            return canonical.join(suffix);
        }
        ancestor = parent.to_path_buf();
    }

    path.to_path_buf()
}

/// Canonicalize an output path after rejecting dangling symlink components.
///
/// WHAT: follows every existing symlink component before applying nearest-ancestor fallback.
/// WHY: a dangling alias can become resolvable after an earlier output is emitted, so treating it
/// as an ordinary missing suffix would make preflight disagree with emission.
pub(crate) fn canonicalize_output_path(path: &Path) -> Result<PathBuf, DanglingSymlink> {
    inspect_existing_path_components(path)?;
    Ok(canonicalize_or_nearest_ancestor(path))
}

/// Check whether an output path contains an existing symlink component.
pub(crate) fn relative_path_contains_symlink_component(
    output_root: &Path,
    relative_path: &Path,
) -> Result<bool, DanglingSymlink> {
    inspect_existing_path_components_from(output_root, relative_path)
}

fn inspect_existing_path_components(path: &Path) -> Result<bool, DanglingSymlink> {
    inspect_existing_path_components_from(Path::new(""), path)
}

fn inspect_existing_path_components_from(
    initial_path: &Path,
    path_suffix: &Path,
) -> Result<bool, DanglingSymlink> {
    let mut component_path = initial_path.to_path_buf();
    let mut contains_symlink = false;
    for component in path_suffix.components() {
        inspect_existing_path_component(
            &mut component_path,
            component.as_os_str(),
            &mut contains_symlink,
        )?;
    }

    Ok(contains_symlink)
}

fn inspect_existing_path_component(
    component_path: &mut PathBuf,
    component: &OsStr,
    contains_symlink: &mut bool,
) -> Result<(), DanglingSymlink> {
    component_path.push(component);
    let metadata = match fs::symlink_metadata(&component_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(DanglingSymlink),
    };

    if metadata.file_type().is_symlink() {
        *contains_symlink = true;
        if fs::canonicalize(&component_path).is_err() {
            return Err(DanglingSymlink);
        }
    }

    Ok(())
}
