//! Module-root and project-config filename identity.
//!
//! WHAT: classifies canonical root/config filenames so Stage 0 discovery, header dependency
//!      validation and diagnostic rendering share one filename policy.
//! WHY: `@mod.moth`, `@package.moth` and `@entry.moth` are the only authored root filenames and
//!      `config.moth` is the only build-only special source name. Every other `@*.moth`,
//!      `+*.moth` or `#*.moth` filename is an authored structure error diagnosed during Stage 0
//!      discovery.

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

/// Whether a `.moth` filename uses a special marker outside the accepted root vocabulary.
///
/// WHAT: recognizes a case-sensitive `.moth` stem that starts with `@`, `+` or `#` but is not
///       exactly `@mod.moth`, `@package.moth` or `@entry.moth`.
/// WHY: Stage 0 discovery, synthetic single-file selection and helper-name inventory must agree
///      that these spellings are invalid authored root markers, never ordinary helpers. The
///      recognition is exact: case variants such as `@Entry.moth` are invalid, and `config.moth`
///      has no marker, so it retains its separate build-only config meaning.
pub(crate) fn file_name_is_invalid_module_root_marker(file_name: &str) -> bool {
    if module_root_role_for_file_name(file_name).is_some() {
        return false;
    }

    let Some(stem) = file_name.strip_suffix(LANGUAGE_SOURCE_SUFFIX) else {
        return false;
    };

    stem.starts_with('@') || stem.starts_with('+') || stem.starts_with('#')
}

/// Whether a dependency component identifies the canonical project config file.
pub(crate) fn dependency_component_is_config_file(component: &str) -> bool {
    component == "config" || file_name_is_config_file(component)
}

pub(crate) fn dependency_path_references_config_file(
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> bool {
    dependency_source_component(path, path_fork, string_table)
        .is_some_and(dependency_component_is_config_file)
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
