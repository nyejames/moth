//! Generic module-root and project-config filename identity.
//!
//! WHAT: classifies canonical root/config filenames and their extensionless import components.
//! WHY: Stage 0, header import validation, and diagnostic rendering must share one filename
//!      policy for generic module roots and the canonical project config.

use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{CONFIG_FILE_NAME, LANGUAGE_SOURCE_SUFFIX};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// The direct-child hash-root state for one source-backed package directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HashRootFileDiscovery {
    Missing,
    Unique(PathBuf),
    Multiple(Vec<PathBuf>),
    Unreadable(String),
}

/// A typed failure from direct-child hash-root discovery.
///
/// WHAT: distinguishes filesystem read failures from unrepresentable filenames so the owning
///     Stage 0 boundary can map each kind to its correct error channel.
/// WHY: a non-UTF-8 direct-child name cannot be classified as a hash-root candidate and must not
///      be silently skipped. The offending `PathBuf` is preserved independently of
///      `CompilerMessages` and `CompilerError` so the calling stage owns the diagnostic lane.
#[derive(Debug)]
pub(crate) enum HashRootDiscoveryError {
    /// Filesystem `read_dir` or entry iteration failure.
    Io(io::Error),
    /// A direct-child filename that cannot be represented as UTF-8.
    InvalidFileName(PathBuf),
}

/// Immutable source-backed package roots and their prepared public-surface states.
///
/// WHAT: carries the canonical filesystem roots and the typed direct-child hash-root discovery
///     result from Stage 0 into path resolution and header preparation.
///
/// Both maps use `BTreeMap` so that every public iteration surface preserves one canonical
/// import-prefix order. Callers never observe `HashMap` iteration order from roots or root-file
/// discoveries.
/// WHY: resolver construction must consume filesystem preparation rather than rediscovering
///     source-backed package roots or public surfaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedSourcePackageRoots {
    roots: BTreeMap<String, PathBuf>,
    root_files: BTreeMap<String, HashRootFileDiscovery>,
}

impl PreparedSourcePackageRoots {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Build one prepared contract from Stage 0's canonical roots and discoveries.
    pub(crate) fn from_entries(
        entries: impl IntoIterator<Item = (String, PathBuf, HashRootFileDiscovery)>,
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

    pub(crate) fn root_files(&self) -> &BTreeMap<String, HashRootFileDiscovery> {
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

/// Discover direct-child Moth hash roots without assigning a semantic filename role.
///
/// WHAT: applies the generic `#*.moth` filename policy to one source-backed package directory.
/// WHY: source-backed package preflight and path resolution must inspect the same filesystem candidates
///      while keeping missing or ambiguous roots available for typed Stage 0 diagnostics.
pub(crate) fn discover_hash_root_file(
    directory: &Path,
) -> Result<HashRootFileDiscovery, HashRootDiscoveryError> {
    let mut root_files = Vec::new();

    for entry in std::fs::read_dir(directory).map_err(HashRootDiscoveryError::Io)? {
        let entry = entry.map_err(HashRootDiscoveryError::Io)?;
        let path = entry.path();

        // A direct-child filename that is not valid UTF-8 cannot be classified as a hash-root
        // candidate. Fail immediately with the offending path instead of silently skipping it,
        // which could misreport a valid root as missing.
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| HashRootDiscoveryError::InvalidFileName(path.clone()))?;

        if file_name_is_hash_root_file(file_name) && path.is_file() {
            root_files.push(path);
        }
    }

    root_files.sort();

    Ok(match root_files.as_slice() {
        [] => HashRootFileDiscovery::Missing,
        [root_file] => HashRootFileDiscovery::Unique(root_file.clone()),
        _ => HashRootFileDiscovery::Multiple(root_files),
    })
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
