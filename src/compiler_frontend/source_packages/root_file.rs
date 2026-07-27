//! Generic module-root and project-config filename identity.
//!
//! WHAT: classifies canonical root/config filenames and their extensionless import components.
//! WHY: Stage 0, header import validation, and diagnostic rendering must share one filename
//!      policy for generic module roots and the canonical project config.

use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{CONFIG_FILE_NAME, LANGUAGE_SOURCE_SUFFIX};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Immutable source-backed package roots and their prepared public-surface files.
///
/// WHAT: carries canonical filesystem roots and their validated direct-child hash-root files from
/// Stage 0 into path resolution and header preparation. Missing, multiple and unreadable roots
/// never enter this successful view.
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

/// Whether a filesystem filename is a non-config Moth module root.
pub(crate) fn file_name_is_hash_root_file(file_name: &str) -> bool {
    let Some(root_name) = file_name.strip_prefix('#') else {
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

/// Whether a filesystem filename is any canonical Moth module root (`#*.moth` or `+*.moth`).
pub(crate) fn file_name_is_module_root_file(file_name: &str) -> bool {
    file_name_is_hash_root_file(file_name) || file_name_is_support_root_file(file_name)
}

/// Whether a filesystem filename is the canonical project configuration file.
pub(crate) fn file_name_is_config_file(file_name: &str) -> bool {
    file_name == CONFIG_FILE_NAME
}

/// Whether an extensionless import component identifies a hash-root file.
pub(crate) fn import_component_is_hash_root_file(component: &str) -> bool {
    file_name_is_hash_root_file(component)
        || (component.starts_with('#') && !component.contains('.') && component.len() > 1)
}

/// Return the canonical root filename represented by an import component.
pub(crate) fn hash_root_file_name_from_import_component(component: &str) -> Option<String> {
    if file_name_is_hash_root_file(component) {
        return Some(component.to_owned());
    }

    import_component_is_hash_root_file(component)
        .then(|| format!("{component}{LANGUAGE_SOURCE_SUFFIX}"))
}

/// Whether an import component identifies the canonical project config file.
pub(crate) fn import_component_is_config_file(component: &str) -> bool {
    component == "config" || file_name_is_config_file(component)
}

/// Whether a direct import's source component is a hash-root file.
pub(crate) fn import_path_references_hash_root_file(
    path: &InternedPath,
    from_grouped_import: bool,
    string_table: &StringTable,
) -> bool {
    import_source_component(path, from_grouped_import, string_table)
        .is_some_and(import_component_is_hash_root_file)
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
