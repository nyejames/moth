//! Prepared module-root lookup data for project-aware path resolution.
//!
//! WHAT: stores the canonical module-root records prepared by Stage 0 and provides
//! nearest-root lookups for the frontend resolver.
//! WHY: filesystem discovery and durable module identity belong to Stage 0. The frontend
//! consumes this lookup table without discovering project structure or owning module identity,
//! roles or ancestry. Directory compilation includes normal and support roots; synthetic
//! single-file compilation may provide only its normal roots.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::compiler_frontend::semantic_identity::ModuleRootRole;

/// Internal index for one prepared module-root record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ModuleRootId(usize);

/// One canonical module root directory with its optional normal and support root files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleRootRecord {
    root_directory: PathBuf,
    normal_root_file: Option<PathBuf>,
    support_root_file: Option<PathBuf>,
}

impl ModuleRootRecord {
    /// Construct a normal-only record for synthetic single-file callers and focused tests.
    #[cfg(test)]
    pub(crate) fn new(root_directory: PathBuf, root_file: PathBuf) -> Self {
        Self {
            root_directory,
            normal_root_file: Some(root_file),
            support_root_file: None,
        }
    }

    /// Construct a structural record with either or both root surfaces.
    pub(crate) fn from_surfaces(
        root_directory: PathBuf,
        normal_root_file: Option<PathBuf>,
        support_root_file: Option<PathBuf>,
    ) -> Self {
        debug_assert!(
            normal_root_file.is_some() || support_root_file.is_some(),
            "a module root record has at least one surface"
        );
        Self {
            root_directory,
            normal_root_file,
            support_root_file,
        }
    }

    fn primary_root_file(&self) -> &Path {
        self.normal_root_file
            .as_deref()
            .or(self.support_root_file.as_deref())
            .expect("a module root record has at least one surface")
    }

    fn root_file_for_surface(&self, role: ModuleRootRole) -> Option<&Path> {
        match role {
            ModuleRootRole::Normal => self.normal_root_file.as_deref(),
            ModuleRootRole::Support => self.support_root_file.as_deref(),
            ModuleRootRole::Shared | ModuleRootRole::SyntheticProjectContext => None,
        }
    }
}

/// Prepared module-root records and indexes used by path resolution.
///
/// Stage 0 builds this table from its durable module identity table. Directory compilation keeps
/// normal and support roots available for membership, header-role lookup and same-directory
/// preparation; synthetic single-file callers can still provide a normal-only table.
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
                .then_with(|| left.primary_root_file().cmp(right.primary_root_file()))
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

    /// The primary root file for a directory: normal when present, otherwise support.
    pub(crate) fn root_file_for_directory(&self, directory: &Path) -> Option<&Path> {
        let module_root_id = self.by_directory.get(directory)?;
        Some(self.records[module_root_id.0].primary_root_file())
    }

    /// The root file for one explicit surface, when that surface exists in the directory.
    #[allow(dead_code)]
    pub(crate) fn root_file_for_directory_surface(
        &self,
        directory: &Path,
        role: ModuleRootRole,
    ) -> Option<&Path> {
        let module_root_id = self.by_directory.get(directory)?;
        self.records[module_root_id.0].root_file_for_surface(role)
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
