//! Source-backed package discovery and boundary indexing for Stage 0.
//!
//! WHAT: indexes independently registered source-backed package roots and builds one independent
//! `SourceTreeIndex` per selected package boundary.
//! WHY: source-backed package discovery and filesystem indexing belong to Stage 0. Keeping their
//! one traversal here prevents path resolution and frontend semantics from rediscovering package
//! roots, public surfaces or sibling collisions. Canonical project-local packages are structural
//! support roots owned by `SourceTreeIndex`; this module does not interpret legacy config folders.

use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{ProvidedSourceRoot, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::source_tree_index::SourceTreeIndex;

use std::fs;

/// One source-package boundary index paired with its package prefix, in deterministic order.
///
/// WHAT: owned by [`SourcePackageBoundaryIndexes`] and iterated in package-prefix order so the
/// Stage 0 package-boundary owner preserves one canonical package order at every surface.
#[derive(Debug)]
pub(crate) struct SourcePackageBoundaryIndex {
    package_prefix: String,
    index: SourceTreeIndex,
}

/// The Stage 0 owner of one independent [`SourceTreeIndex`] per selected source-package boundary.
///
/// WHAT: stores one boundary index per registered source-backed package in deterministic
/// package-prefix order. Each index owns its own stable package identity, dense `SourceId`s,
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
    /// Iterate the boundary indexes in deterministic package-prefix order.
    ///
    /// Production code derives the resolver view from this iteration; focused tests inspect the
    /// boundary-local indexes through the same single surface.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &SourceTreeIndex)> {
        self.indexes
            .iter()
            .map(|entry| (entry.package_prefix.as_str(), &entry.index))
    }

    /// Derive the resolver's narrow package-root view from indexed facts.
    ///
    /// WHAT: projects each boundary index to its canonical root directory and unique normal-root
    /// file, producing the immutable [`PreparedSourcePackageRoots`] contract that path
    /// resolution consumes. No `read_dir`, canonicalize or root-file discovery pass runs here;
    /// missing and multiple roots were already rejected as structured diagnostics during index
    /// construction, so every successful index contributes one validated public surface.
    pub(crate) fn prepared_source_package_roots(&self) -> PreparedSourcePackageRoots {
        let mut entries = Vec::with_capacity(self.indexes.len());

        for (package_prefix, index) in self.iter() {
            let root = index.entry_root().to_path_buf();
            let root_file = index
                .root_file_for_entry_root()
                .expect("a successful package boundary index has an entry-root module")
                .to_path_buf();
            entries.push((package_prefix.to_owned(), root, root_file));
        }

        PreparedSourcePackageRoots::from_entries(entries)
    }
}

/// Build one source-package boundary index per registered source-backed package.
///
/// WHAT: canonicalizes each registered filesystem root, traverses it as an independent
/// `SourceTreeIndex` with its own stable package identity, and stores the indexes in
/// deterministic package-prefix order. The traversal owns direct root discovery and sibling
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
            StablePackageIdentity::source_package(package.metadata.origin, &package.package_prefix);

        let index = SourceTreeIndex::discover_package(
            canonical_root,
            package_identity,
            &package.package_prefix,
            source_file_kinds,
            external_import_providers,
            string_table,
        )?;

        indexes.push(SourcePackageBoundaryIndex {
            package_prefix: package.package_prefix.clone(),
            index,
        });
    }

    // `source_packages.iter()` already yields package-prefix order from the registry's `BTreeMap`,
    // so the owner preserves one canonical package order without an explicit re-sort.
    Ok(SourcePackageBoundaryIndexes { indexes })
}
