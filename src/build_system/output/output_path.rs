//! Deterministic output-path identity for comparing resolved output roots.
//!
//! WHAT: normalises a resolved output path and lowercases it so development and release roots
//! such as `dev` and `DEV` compare equal, keeping results stable across case-sensitive and
//! case-insensitive filesystems.
//! WHY: output-plan construction and Phase 3 destination collision checks need one deterministic
//! comparison key that does not depend on the host filesystem's case semantics.

use std::path::{Component, Path};

/// Case-normalised output-path identity.
///
/// WHAT: wraps the normalised, ASCII-lowercased path spelling used only for equality comparisons.
/// WHY: `dev` and `DEV` must resolve to the same output identity so the build stays stable when a
/// project is moved between case-sensitive and case-insensitive filesystems.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct OutputPathIdentity(String);

/// Build the deterministic identity for a resolved output path.
///
/// WHAT: drops authored `.` segments and lowercases each component so equivalent resolved paths
/// compare equal regardless of spelling or host case semantics.
pub(crate) fn output_path_identity(path: &Path) -> OutputPathIdentity {
    let mut normalised = String::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                normalised.push('/');
                normalised.push_str(&part.to_string_lossy().to_ascii_lowercase());
            }
            _ => {}
        }
    }
    if normalised.is_empty() {
        normalised.push('/');
    }
    OutputPathIdentity(normalised)
}
