//! Module-root and project-config filename identity.
//!
//! WHAT: classifies canonical root/config filenames so Stage 0 discovery, header import
//!      validation and diagnostic rendering share one filename policy.
//! WHY: normal `@*.moth` roots, support `+*.moth` roots and the canonical `config.moth` are
//!      the only special filenames the compiler recognises. Legacy `#*.moth` root-like
//!      filenames are rejected during Stage 0 discovery.

use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{CONFIG_FILE_NAME, LANGUAGE_SOURCE_SUFFIX};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Immutable source-backed package roots and their prepared public-surface files.
///
/// WHAT: carries canonical filesystem roots and their validated direct-child normal-root
/// files from Stage 0 into path resolution and header preparation. Missing, multiple and
/// unreadable roots never enter this successful view.
///
/// Both maps use `BTreeMap` so that every public iteration surface preserves one canonical
/// import-prefix order. Callers never observe `HashMap` iteration order from roots or root-file
/// records.
/// WHY: resolver construction must consume filesystem preparation rather than rediscovering
///     source-backed package roots or public surfaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedSourcePackageRoots {
    roots: BTreeMap<String, PathBuf>,
    root_files: BTreeMap<String, PathBuf>,
}

impl PreparedSourcePackageRoots {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

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

/// Whether a filesystem filename is a non-config normal Moth module root (`@*.moth`).
pub(crate) fn file_name_is_normal_module_root_file(file_name: &str) -> bool {
    let Some(root_name) = file_name.strip_prefix('@') else {
        return false;
    };
    let Some(root_name) = root_name.strip_suffix(LANGUAGE_SOURCE_SUFFIX) else {
        return false;
    };

    !root_name.is_empty()
}

/// Whether a filesystem filename is a `+*.moth` support root file.
pub(crate) fn file_name_is_support_root_file(file_name: &str) -> bool {
    let Some(root_name) = file_name.strip_prefix('+') else {
        return false;
    };
    let Some(root_name) = root_name.strip_suffix(LANGUAGE_SOURCE_SUFFIX) else {
        return false;
    };

    !root_name.is_empty()
}

/// Whether a filesystem filename is any canonical Moth module root (`@*.moth` or `+*.moth`).
pub(crate) fn file_name_is_module_root_file(file_name: &str) -> bool {
    file_name_is_normal_module_root_file(file_name) || file_name_is_support_root_file(file_name)
}

/// Whether a filesystem filename is the canonical project configuration file.
pub(crate) fn file_name_is_config_file(file_name: &str) -> bool {
    file_name == CONFIG_FILE_NAME
}

/// Whether a filesystem filename is a legacy `#*.moth` root-like filename.
///
/// WHAT: identifies the old normal-root marker so Stage 0 can reject it with a structured
/// diagnostic that tells the author to rename `#name.moth` to `@name.moth`.
/// WHY: the old marker is invalid after the `@` migration. This predicate is used only for
///      the legacy rejection diagnostic, not for root classification.
pub(crate) fn file_name_is_legacy_hash_root_file(file_name: &str) -> bool {
    let Some(root_name) = file_name.strip_prefix('#') else {
        return false;
    };
    let Some(root_name) = root_name.strip_suffix(LANGUAGE_SOURCE_SUFFIX) else {
        return false;
    };

    !root_name.is_empty()
}

/// Whether an import component identifies the canonical project config file.
pub(crate) fn import_component_is_config_file(component: &str) -> bool {
    component == "config" || file_name_is_config_file(component)
}

/// Whether an extensionless import component looks like a normal module-root file reference.
///
/// WHAT: detects when an author wrote `@name` or `@name.moth` as an import component, which
/// would attempt to import a root file directly rather than the module facade.
/// WHY: root files are imported through their directory, not by filename. A helpful diagnostic
///      guides the author to drop the marker prefix and import the directory path instead.
pub(crate) fn import_component_is_normal_module_root_file(component: &str) -> bool {
    file_name_is_normal_module_root_file(component)
        || (component.starts_with('@') && !component.contains('.') && component.len() > 1)
}

/// Whether an extensionless import component looks like a support root file reference.
///
/// WHAT: detects when an author wrote `+name` or `+name.moth` as an import component, which
/// would attempt to import a support root file directly rather than the support package facade.
/// WHY: support packages are imported by their directory name, not by root filename. A helpful
///      diagnostic guides the author to drop the `+` prefix and import the package path instead.
pub(crate) fn import_component_is_support_root_file(component: &str) -> bool {
    file_name_is_support_root_file(component)
        || (component.starts_with('+') && !component.contains('.') && component.len() > 1)
}

/// Whether an extensionless import component looks like any module-root file reference.
pub(crate) fn import_component_is_module_root_file(component: &str) -> bool {
    import_component_is_normal_module_root_file(component)
        || import_component_is_support_root_file(component)
}

/// Return the root filename represented by an import component, if it references a root file.
///
/// WHAT: reconstructs the `@*.moth` or `+*.moth` filename from an extensionless or full
/// component so the diagnostic can name the specific file the author tried to import.
pub(crate) fn module_root_file_name_from_import_component(component: &str) -> Option<String> {
    if file_name_is_normal_module_root_file(component) || file_name_is_support_root_file(component)
    {
        return Some(component.to_owned());
    }

    if import_component_is_normal_module_root_file(component)
        || import_component_is_support_root_file(component)
    {
        return Some(format!("{component}{LANGUAGE_SOURCE_SUFFIX}"));
    }

    None
}

/// Whether a direct import's source component is the canonical project config file.
pub(crate) fn import_path_references_config_file(
    path: &InternedPath,
    from_grouped_import: bool,
    string_table: &StringTable,
) -> bool {
    import_source_component(path, from_grouped_import, string_table)
        .is_some_and(import_component_is_config_file)
}

/// Whether a direct import's source component is a module-root file.
///
/// WHAT: checks the source component (the directory-leaf for bare imports, or the
/// second-to-last for grouped imports) to detect attempts to import a root file by its
/// filename rather than through the module directory.
pub(crate) fn import_path_references_module_root_file(
    path: &InternedPath,
    from_grouped_import: bool,
    string_table: &StringTable,
) -> bool {
    import_source_component(path, from_grouped_import, string_table)
        .is_some_and(import_component_is_module_root_file)
}

fn import_source_component<'a>(
    path: &'a InternedPath,
    from_grouped_import: bool,
    string_table: &'a StringTable,
) -> Option<&'a str> {
    let source_component_offset = if from_grouped_import { 2 } else { 1 };
    path.len()
        .checked_sub(source_component_offset)
        .and_then(|index| path.as_components().get(index))
        .map(|component| string_table.resolve(*component))
}
