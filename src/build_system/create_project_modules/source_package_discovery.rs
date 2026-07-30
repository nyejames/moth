//! Source-backed package discovery and boundary indexing for Stage 0.
//!
//! WHAT: scans configured legacy package folders, merges project-local packages with
//! builder-provided packages, rejects ambiguous import-prefix ownership and builds one independent
//! `SourceTreeIndex` per selected package boundary.
//! WHY: source-backed package discovery and filesystem indexing belong to Stage 0. Keeping their
//! one traversal here prevents path resolution and frontend semantics from rediscovering package
//! roots, public surfaces or sibling collisions.

use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{ProvidedSourceRoot, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use super::source_tree_index::SourceTreeIndex;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::project_structure_diagnostics::{
    config_diagnostic_messages, non_utf8_filesystem_name_error, path_id,
};

/// One source-package boundary index paired with its import prefix, in deterministic order.
///
/// WHAT: owned by [`SourcePackageBoundaryIndexes`] and iterated in import-prefix order so the
/// Stage 0 package-boundary owner preserves one canonical package order at every surface.
#[derive(Debug)]
pub(crate) struct SourcePackageBoundaryIndex {
    import_prefix: String,
    index: SourceTreeIndex,
}

/// The Stage 0 owner of one independent [`SourceTreeIndex`] per selected source-package boundary.
///
/// WHAT: stores one boundary index per registered source-backed package in deterministic
/// import-prefix order. Each index owns its own stable package identity, dense `SourceId`s,
/// `ModuleId`s and ownership tables; raw IDs never cross boundaries.
/// WHY: the build-system authority requires Core, Builder and dependency source packages to
/// compile as separate graphs with their own source indexes. This owner is the single Stage 0
/// home for those indexes, and the resolver's narrow package-root view is derived from it
/// without another filesystem scan.
#[derive(Debug)]
pub(crate) struct SourcePackageBoundaryIndexes {
    indexes: Vec<SourcePackageBoundaryIndex>,
}

impl SourcePackageBoundaryIndexes {
    /// Iterate the boundary indexes in deterministic import-prefix order.
    ///
    /// Production code derives the resolver view from this iteration; focused tests inspect the
    /// boundary-local indexes through the same single surface.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &SourceTreeIndex)> {
        self.indexes
            .iter()
            .map(|entry| (entry.import_prefix.as_str(), &entry.index))
    }

    /// Derive the resolver's narrow package-root view from indexed facts.
    ///
    /// WHAT: projects each boundary index to its canonical root directory and unique hash-root
    /// file, producing the immutable [`PreparedSourcePackageRoots`] contract that path
    /// resolution consumes. No `read_dir`, canonicalize or root-file discovery pass runs here;
    /// missing and multiple roots were already rejected as structured diagnostics during index
    /// construction, so every successful index contributes one validated public surface.
    pub(crate) fn prepared_source_package_roots(&self) -> PreparedSourcePackageRoots {
        let mut entries = Vec::with_capacity(self.indexes.len());

        for (import_prefix, index) in self.iter() {
            let root = index.entry_root().to_path_buf();
            let root_file = index
                .root_file_for_entry_root()
                .expect("a successful package boundary index has an entry-root module")
                .to_path_buf();
            entries.push((import_prefix.to_owned(), root, root_file));
        }

        PreparedSourcePackageRoots::from_entries(entries)
    }
}

/// Build one source-package boundary index per registered source-backed package.
///
/// WHAT: canonicalizes each registered filesystem root, traverses it as an independent
/// `SourceTreeIndex` with its own stable package identity, and stores the indexes in
/// deterministic import-prefix order. The traversal owns direct root discovery and sibling
/// `.moth` file/folder collisions for each package tree, so no separate package-root or
/// package-tree collision scan remains.
/// WHY: Stage 0 owns filesystem preparation. Each package boundary owns a separate index so
/// raw `SourceId`/`ModuleId` values never cross boundaries, and the resolver view is derived
/// from indexed facts rather than rediscovered.
pub(crate) fn build_source_package_boundary_indexes(
    source_packages: &SourcePackageRegistry,
    source_file_kinds: &SourceFileKindRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
    string_table: &mut StringTable,
) -> Result<SourcePackageBoundaryIndexes, CompilerMessages> {
    let mut indexes = Vec::new();

    for package in source_packages.iter() {
        let ProvidedSourceRoot::Filesystem(path) = &package.root;

        // Canonicalize each registered filesystem root before traversal so the package index
        // never proceeds against a path whose canonicalization failed. This preserves the
        // existing file-error diagnostic for unresolvable package roots.
        let canonical_root = fs::canonicalize(path).map_err(|error| {
            CompilerMessages::from_error_ref(
                CompilerError::file_error(
                    path,
                    format!("Failed to canonicalize source-backed package root: {error}"),
                    string_table,
                ),
                string_table,
            )
        })?;

        let package_identity =
            StablePackageIdentity::source_package(package.metadata.origin, &package.import_prefix);

        let index = SourceTreeIndex::discover_package(
            canonical_root,
            package_identity,
            &package.import_prefix,
            source_file_kinds,
            external_import_providers,
            string_table,
        )?;

        indexes.push(SourcePackageBoundaryIndex {
            import_prefix: package.import_prefix.clone(),
            index,
        });
    }

    // `source_packages.iter()` already yields import-prefix order from the registry's `BTreeMap`,
    // so the owner preserves one canonical package order without an explicit re-sort.
    Ok(SourcePackageBoundaryIndexes { indexes })
}

/// Discover project-local source-backed packages from configured `package_folders`.
///
/// WHAT: scans each configured top-level folder under the project root and registers one source
/// package root per direct child directory.
/// WHY: project-local package discovery must follow config rather than hardcoding `/lib`.
pub(super) fn discover_project_local_source_packages(
    config: &Config,
    project_root: &Path,
    string_table: &mut StringTable,
) -> Result<SourcePackageRegistry, CompilerMessages> {
    let mut discovered_packages = SourcePackageRegistry::new();
    let mut discovered_prefixes: BTreeMap<String, PathBuf> = BTreeMap::new();

    for configured_folder in &config.package_folders {
        let folder_path = project_root.join(configured_folder);

        // Validate configured package roots before scanning children so config mistakes stay as
        // typed diagnostics instead of becoming later import-resolution failures.
        if !folder_path.exists() {
            if config.has_explicit_package_folders {
                return Err(config_diagnostic_messages(
                    config,
                    "package_folders",
                    InvalidConfigReason::ConfiguredPackageFolderMissing {
                        folder: path_id(configured_folder, string_table),
                    },
                    string_table,
                ));
            }

            continue;
        }

        if !folder_path.is_dir() {
            return Err(config_diagnostic_messages(
                config,
                "package_folders",
                InvalidConfigReason::ConfiguredPackageFolderNotDirectory {
                    folder: path_id(configured_folder, string_table),
                },
                string_table,
            ));
        }

        scan_project_package_folder(
            config,
            &folder_path,
            &mut discovered_packages,
            &mut discovered_prefixes,
            string_table,
        )?;
    }

    Ok(discovered_packages)
}

/// Merge builder-provided and project-local packages, preserving the builder/project collision
/// diagnostic that config validation expects.
pub(super) fn merge_source_packages(
    config: &Config,
    builder_source_packages: &SourcePackageRegistry,
    project_local_packages: &SourcePackageRegistry,
    string_table: &mut StringTable,
) -> Result<SourcePackageRegistry, CompilerMessages> {
    let mut merged_packages = builder_source_packages.clone();

    if let Err(collisions) = merged_packages.merge(project_local_packages) {
        let collision_list = collisions.join(", ");

        return Err(config_diagnostic_messages(
            config,
            "package_folders",
            InvalidConfigReason::SourcePackageBuilderPrefixCollision {
                prefixes: string_table.get_or_intern(collision_list),
                package_folders: string_table
                    .get_or_intern(format_package_folder_list(&config.package_folders)),
            },
            string_table,
        ));
    }

    Ok(merged_packages)
}

fn scan_project_package_folder(
    config: &Config,
    folder_path: &Path,
    discovered_packages: &mut SourcePackageRegistry,
    discovered_prefixes: &mut BTreeMap<String, PathBuf>,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    let entries = fs::read_dir(folder_path).map_err(|error| {
        CompilerMessages::from_error_ref(
            CompilerError::file_error(
                folder_path,
                format!("Failed to read configured package folder: {error}"),
                string_table,
            ),
            string_table,
        )
    })?;

    // Collect directory entries before registration so prefix collision diagnostics and
    // registration order are deterministic regardless of filesystem iteration order.
    let mut package_entries: Vec<(String, PathBuf)> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| {
            CompilerMessages::from_error_ref(
                CompilerError::file_error(
                    folder_path,
                    format!("Failed to read package folder entry: {error}"),
                    string_table,
                ),
                string_table,
            )
        })?;

        let package_root = entry.path();
        if !package_root.is_dir() {
            continue;
        }

        let prefix = package_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                non_utf8_filesystem_name_error(
                    &package_root,
                    "project-local package prefix",
                    string_table,
                )
            })?;
        package_entries.push((prefix.to_owned(), package_root));
    }

    package_entries.sort_by(|(prefix_a, _), (prefix_b, _)| prefix_a.cmp(prefix_b));

    for (prefix, package_root) in package_entries {
        // Prevent duplicate @prefixes across different project-local package roots.
        if let Some(previous_root) = discovered_prefixes.get(&prefix) {
            return Err(config_diagnostic_messages(
                config,
                "package_folders",
                InvalidConfigReason::SourcePackagePrefixCollision {
                    prefix: string_table.intern(&prefix),
                    first_root: path_id(previous_root, string_table),
                    second_root: path_id(&package_root, string_table),
                },
                string_table,
            ));
        }

        discovered_prefixes.insert(prefix.clone(), package_root.clone());
        discovered_packages.register_filesystem_root(
            prefix,
            package_root,
            crate::builder_surface::PackageOrigin::ProjectLocal,
        );
    }

    Ok(())
}

fn format_package_folder_list(package_folders: &[PathBuf]) -> String {
    let mut folders = package_folders
        .iter()
        .map(|folder| folder.display().to_string())
        .collect::<Vec<_>>();
    folders.sort();
    folders.join(", ")
}
