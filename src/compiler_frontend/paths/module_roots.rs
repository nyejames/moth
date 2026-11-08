//! Prepared module-root lookup data for project-aware path resolution.
//!
//! WHAT: stores the canonical module-root records prepared by Stage 0 and provides
//! nearest-root lookups for the frontend resolver.
//! WHY: filesystem discovery and durable module identity belong to Stage 0. The frontend
//! consumes this lookup table without discovering project structure or owning module identity,
//! roles or ancestry. Directory compilation includes normal, support and entry roots; synthetic
//! single-file compilation includes every present root in its bounded source tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::compiler_frontend::semantic_identity::{ModuleRootFiles, ModuleRootRole};

/// Internal index for one prepared module-root record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ModuleRootId(usize);

/// One canonical module root directory with its fixed role-keyed root inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleRootRecord {
    root_directory: PathBuf,
    root_files: ModuleRootFiles<PathBuf>,
}

impl ModuleRootRecord {
    /// Construct a normal-only record for focused tests.
    #[cfg(test)]
    pub(crate) fn new(root_directory: PathBuf, root_file: PathBuf) -> Self {
        Self::from_roots(
            root_directory,
            ModuleRootFiles::new(Some(root_file), None, None),
        )
    }

    /// Construct a structural record with any present roots.
    pub(crate) fn from_roots(
        root_directory: PathBuf,
        root_files: ModuleRootFiles<PathBuf>,
    ) -> Self {
        debug_assert!(
            !root_files.is_empty(),
            "a module root record has at least one surface"
        );
        Self {
            root_directory,
            root_files,
        }
    }

    fn preparation_anchor_file(&self) -> &Path {
        self.root_files
            .preparation_anchor()
            .map(PathBuf::as_path)
            .expect("a module root record has at least one surface")
    }

    fn root_file_for_surface(&self, role: ModuleRootRole) -> Option<&Path> {
        self.root_files.get(role).map(PathBuf::as_path)
    }
}

/// Prepared module-root records and indexes used by path resolution.
///
/// Stage 0 builds this table from its durable module identity table. Directory compilation keeps
/// every present root available for membership, header-role lookup and same-directory
/// preparation; synthetic single-file callers build the same table from their bounded source tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ModuleRootTable {
    records: Vec<ModuleRootRecord>,
    by_directory: HashMap<PathBuf, ModuleRootId>,
}

impl ModuleRootTable {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn from_records(mut records: Vec<ModuleRootRecord>) -> Self {
        records.sort_by(|left, right| {
            left.root_directory
                .cmp(&right.root_directory)
                .then_with(|| {
                    left.preparation_anchor_file()
                        .cmp(right.preparation_anchor_file())
                })
        });

        let mut table = Self {
            records,
            ..Self::default()
        };

        for (index, record) in table.records.iter().enumerate() {
            let id = ModuleRootId(index);
            table.by_directory.insert(record.root_directory.clone(), id);
        }

        table
    }

    pub(crate) fn root_directories(&self) -> impl Iterator<Item = &PathBuf> {
        self.records.iter().map(|record| &record.root_directory)
    }

    /// The directory's API facade root file: normal when present, otherwise support.
    ///
    /// Same-directory template queuing uses it because a `.mtf` source may only read
    /// export-capable roots. An entry root is never an API facade, so an entry-only directory
    /// has no such root. Dependency resolution requests its explicit surface instead.
    pub(crate) fn api_facade_root_file_for_directory(&self, directory: &Path) -> Option<&Path> {
        let record = &self.records[self.by_directory.get(directory)?.0];
        record
            .root_file_for_surface(ModuleRootRole::Normal)
            .or_else(|| record.root_file_for_surface(ModuleRootRole::Support))
    }

    /// The root file for one explicit surface, when that surface exists in the directory.
    pub(crate) fn root_file_for_directory_surface(
        &self,
        directory: &Path,
        role: ModuleRootRole,
    ) -> Option<&Path> {
        let module_root_id = self.by_directory.get(directory)?;
        self.records[module_root_id.0].root_file_for_surface(role)
    }

    /// Every present root file with its authored role, in deterministic record order.
    ///
    /// WHAT: exposes the bounded inventory's role facts to synthetic single-file preparation.
    /// WHY: a flattened synthetic unit prepares the foreign module roots its dependency clauses
    ///      reach in one source set, and their header roles come from this Stage 0 classification
    ///      rather than a second filename scan.
    pub(crate) fn root_file_roles(&self) -> impl Iterator<Item = (ModuleRootRole, &Path)> {
        self.records.iter().flat_map(|record| {
            record
                .root_files
                .present()
                .map(|(role, path)| (role, path.as_path()))
        })
    }

    /// The authored role of one root file in this inventory, or `None` for any other file.
    ///
    /// WHAT: joins a canonical file to its nearest module root directory and then to the present
    ///       root surface with the same path.
    /// WHY: synthetic single-file discovery prepares reached Moth files before the final module
    ///      preparation pass runs, so it needs the Stage 0 root-role fact keyed by canonical path
    ///      rather than by the traversal-local source identity.
    pub(crate) fn root_role_for_file(&self, file: &Path) -> Option<ModuleRootRole> {
        let root_directory = self.module_root_for_file(file)?;
        let module_root_id = self.by_directory.get(&root_directory)?;
        self.records[module_root_id.0]
            .root_files
            .present()
            .find_map(|(role, root_path)| (root_path.as_path() == file).then_some(role))
    }

    pub(crate) fn module_root_for_file(&self, file: &Path) -> Option<PathBuf> {
        let mut current = file.parent();

        while let Some(directory) = current {
            if let Some(module_root_id) = self.by_directory.get(directory) {
                return Some(self.records[module_root_id.0].root_directory.clone());
            }

            current = directory.parent();
        }

        None
    }
}
