//! Module-root and project-config filename identity.
//!
//! WHAT: classifies canonical root/config filenames so Stage 0 discovery, header dependency
//!      validation and diagnostic rendering share one filename policy.
//! WHY: `@mod.moth`, `@package.moth` and `@entry.moth` are the only authored root filenames and
//!      `config.moth` is the only build-only special source name. Every other `@*.moth`,
//!      `+*.moth` and legacy `#*.moth` marker is an authored structure error diagnosed during
//!      Stage 0 discovery.

use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{CONFIG_FILE_NAME, LANGUAGE_SOURCE_SUFFIX};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Immutable source-backed package roots and their prepared public-surface files.
///
/// WHAT: carries canonical filesystem roots and their validated direct-child public-surface root
/// files from Stage 0 into path resolution and header preparation. Missing, multiple and
/// unreadable roots never enter this successful view.
///
/// Both maps use `BTreeMap` so that every public iteration surface preserves one canonical
/// dependency-prefix order. Callers never observe `HashMap` iteration order from roots or root-file
/// records.
/// WHY: resolver construction must consume filesystem preparation rather than rediscovering
///     source-backed package roots or public surfaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedSourcePackageRoots {
    roots: BTreeMap<String, PathBuf>,
    root_files: BTreeMap<String, PathBuf>,
}

impl PreparedSourcePackageRoots {
    /// Build one prepared contract from Stage 0's validated canonical roots and root files.
    pub(crate) fn from_entries(
        entries: impl IntoIterator<Item = (String, PathBuf, PathBuf)>,
    ) -> Self {
        let mut prepared = Self::default();

        for (prefix, root, root_file) in entries {
            prepared.roots.insert(prefix.clone(), root);
            prepared.root_files.insert(prefix, root_file);
        }

        prepared
    }

    pub(crate) fn roots(&self) -> &BTreeMap<String, PathBuf> {
        &self.roots
    }

    pub(crate) fn root_files(&self) -> &BTreeMap<String, PathBuf> {
        &self.root_files
    }
}

/// The one normal API-facade root filename.
pub(crate) const NORMAL_ROOT_FILE_NAME: &str = "@mod.moth";
/// The one support API-facade root filename.
pub(crate) const SUPPORT_ROOT_FILE_NAME: &str = "@package.moth";
/// The one executable entry root filename.
pub(crate) const ENTRY_ROOT_FILE_NAME: &str = "@entry.moth";

/// The fixed root role of one authored root filename, or `None` for every other file.
///
/// The recognition is exact and case-sensitive: `@Entry.moth` is not an entry root.
pub(crate) fn module_root_role_for_file_name(file_name: &str) -> Option<ModuleRootRole> {
    match file_name {
        ENTRY_ROOT_FILE_NAME => Some(ModuleRootRole::Entry),
        SUPPORT_ROOT_FILE_NAME => Some(ModuleRootRole::Support),
        NORMAL_ROOT_FILE_NAME => Some(ModuleRootRole::Normal),
        _ => None,
    }
}

/// Whether a filesystem filename is one of the three canonical Moth module roots.
pub(crate) fn file_name_is_module_root_file(file_name: &str) -> bool {
    module_root_role_for_file_name(file_name).is_some()
}

/// Whether a filesystem filename is the canonical project configuration file.
pub(crate) fn file_name_is_config_file(file_name: &str) -> bool {
    file_name == CONFIG_FILE_NAME
}

/// Canonical replacement advice for one rejected authored root marker.
///
/// WHAT: names the legal root role an author should use instead of a retired spelling.
/// WHY: the suggestion is derived from the filename rather than stored as rendered text, so the
///      frozen diagnostic retains only filename facts and the renderer owns the prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InvalidRootFilenameSuggestion {
    /// Executable root work belongs in the entry root.
    Entry,
    /// Exported API belongs in the normal facade.
    Normal,
    /// The scoped support facade replaces the retired support marker.
    Support,
    /// The retired stem does not identify a role: runtime work and exported API have different
    /// fixed roots, so both legal replacements are named.
    EntryOrNormal,
}

impl InvalidRootFilenameSuggestion {
    /// The replacement phrase used by the invalid-root-filename diagnostic.
    pub(crate) fn replacement(self) -> &'static str {
        match self {
            Self::Entry => "'@entry.moth'",
            Self::Normal => "'@mod.moth'",
            Self::Support => "'@package.moth'",
            Self::EntryOrNormal => "'@entry.moth' for runtime work or '@mod.moth' for exported API",
        }
    }
}

/// The canonical replacement for an invalid authored root marker, when the filename uses one.
///
/// WHAT: recognises the retired `@*.moth`, `+*.moth` and `#*.moth` marker spellings and returns
///       the fixed root role the author should use instead.
/// WHY: Stage 0 rejects these files instead of treating them as helpers or no-ops. Keeping the
///      recognition and the suggestion in one place prevents the directory traversal, the
///      single-file selection and the diagnostic text from disagreeing about the vocabulary.
pub(crate) fn invalid_module_root_file_name_suggestion(
    file_name: &str,
) -> Option<InvalidRootFilenameSuggestion> {
    if module_root_role_for_file_name(file_name).is_some() {
        return None;
    }

    let stem = file_name.strip_suffix(LANGUAGE_SOURCE_SUFFIX)?;
    let (marker, name) = stem.split_at_checked(1)?;

    match marker {
        "@" => Some(suggestion_for_retired_at_name(name)),
        "+" => Some(InvalidRootFilenameSuggestion::Support),
        "#" => Some(suggestion_for_retired_at_name(name)),
        _ => None,
    }
}

/// Map the stem after a retired root marker to the fixed root role that role should use.
fn suggestion_for_retired_at_name(name: &str) -> InvalidRootFilenameSuggestion {
    if name.eq_ignore_ascii_case("entry")
        || name.eq_ignore_ascii_case("page")
        || name.eq_ignore_ascii_case("site")
    {
        return InvalidRootFilenameSuggestion::Entry;
    }
    if name.eq_ignore_ascii_case("package") {
        return InvalidRootFilenameSuggestion::Support;
    }
    if name.eq_ignore_ascii_case("mod") {
        return InvalidRootFilenameSuggestion::Normal;
    }

    // `config.moth` is build-only, not a module root, and an unknown retired stem has no
    // inferable role. Never advise a `config.moth` rename: it drops source in a nested
    // directory and usually collides with the existing project config at the root.
    InvalidRootFilenameSuggestion::EntryOrNormal
}

/// Whether a dependency component identifies the canonical project config file.
pub(crate) fn dependency_component_is_config_file(component: &str) -> bool {
    component == "config" || file_name_is_config_file(component)
}

/// Whether a dependency component uses the retired `+` support-root marker.
///
/// WHAT: detects `+name` / `+name.moth` dependency components that tried to bind a former
///       support-root file directly.
/// WHY: support facades are authored as `@package.moth` and are bound through their containing
///      module directory, never by a special file filename. The diagnostic names the
///      replacement spelling instead of silently resolving a missing package.
pub(crate) fn dependency_component_is_legacy_support_marker(component: &str) -> bool {
    component.starts_with('+') && component.len() > 1
}

pub(crate) fn dependency_path_references_config_file(
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> bool {
    dependency_source_component(path, path_fork, string_table)
        .is_some_and(dependency_component_is_config_file)
}

/// Whether a dependency provider root's source component uses the retired `+` marker.
pub(crate) fn dependency_path_references_legacy_support_marker(
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> bool {
    dependency_source_component(path, path_fork, string_table)
        .is_some_and(dependency_component_is_legacy_support_marker)
}

fn dependency_source_component<'a>(
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &'a StringTable,
) -> Option<&'a str> {
    path_fork
        .component(path)
        .map(|component| string_table.resolve(component))
}
