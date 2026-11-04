//! Stage 0 durable module identity and structural topology.
//!
//! WHAT: owns the dense `ModuleId`, `ModuleIdentityRecord`, deterministic assignment table,
//! structural ancestry and the filename-to-root-role classifier produced by the one Stage 0
//! source-tree traversal, and derives frontend module-root lookup tables from those identities.
//! A structural module is one rooted canonical directory. It may carry one normal `@*.moth` root,
//! one support `+*.moth` root, or both. Helper files receive one role-neutral shared origin; each
//! present root receives its own role-qualified surface origin.
//! The cross-stage portable value types — `StablePackageIdentity`,
//! `StableModuleOriginIdentity` and `ModuleRootRole` — are compiler-semantic identity owned by
//! [`crate::compiler_frontend::semantic_identity`]; this module imports them so Stage 0 stays
//! the assignment and table owner while identity values remain build-independent.
//! WHY: durable identity and topology are build-system-owned data. A structural module is the
//! scheduling and ownership unit, not one root file. Keep the dense `ModuleId` per rooted
//! directory and derive the role-qualified origins without inventing parallel module records.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_normal_module_root_file, file_name_is_support_root_file,
};

use rustc_hash::FxHashMap;

use std::path::{Path, PathBuf};

/// Dense deterministic handle for one structural module inside one build boundary.
///
/// `ModuleId` is the build-local index assigned by sorting canonical logical module paths, so
/// it is independent of traversal completion order and cosmetic root filename suffixes
/// (`@mod.moth` and `@page.moth` in the same directory collapse to one structural module whose
/// identity comes from the directory, not the filename).
///
/// It is deliberately not the persistent semantic identity: its numeric value is a build-local
/// table slot that may cross stages inside that build boundary but must not identify a module
/// across builds or reach persistent artefacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModuleId(usize);

impl ModuleId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index)
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }
}

/// The root surface implied by a canonical root filename, or `None` for non-root files.
pub(crate) fn module_root_role_for_file_name(file_name: &str) -> Option<ModuleRootRole> {
    if file_name_is_normal_module_root_file(file_name) {
        Some(ModuleRootRole::Normal)
    } else if file_name_is_support_root_file(file_name) {
        Some(ModuleRootRole::Support)
    } else {
        None
    }
}

/// One structural module root directory with its durable identity and optional root surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleIdentityRecord {
    root_directory: PathBuf,
    logical_module_path: PathBuf,
    normal_root_file: Option<PathBuf>,
    support_root_file: Option<PathBuf>,
    shared_origin: StableModuleOriginIdentity,
    normal_origin: Option<StableModuleOriginIdentity>,
    support_origin: Option<StableModuleOriginIdentity>,
}

impl ModuleIdentityRecord {
    /// Construct one structural module record and derive its cross-build origins.
    ///
    /// Helpers always derive from a role-neutral `Shared` origin. Each present root derives a
    /// role-qualified origin with the same package and logical path. Returns an internal
    /// `InfrastructureFailure` if the logical path has an invalid component.
    pub(crate) fn new(
        root_directory: PathBuf,
        normal_root_file: Option<PathBuf>,
        support_root_file: Option<PathBuf>,
        logical_module_path: PathBuf,
        package: &StablePackageIdentity,
    ) -> Result<Self, InfrastructureFailure> {
        let shared_origin = StableModuleOriginIdentity::from_relative_logical_path(
            package.clone(),
            &logical_module_path,
            ModuleRootRole::Shared,
        )?;
        let normal_origin = normal_root_file.as_ref().map(|_| {
            StableModuleOriginIdentity::from_relative_logical_path(
                package.clone(),
                &logical_module_path,
                ModuleRootRole::Normal,
            )
        });
        let support_origin = support_root_file.as_ref().map(|_| {
            StableModuleOriginIdentity::from_relative_logical_path(
                package.clone(),
                &logical_module_path,
                ModuleRootRole::Support,
            )
        });
        let normal_origin = normal_origin.transpose()?;
        let support_origin = support_origin.transpose()?;
        Ok(Self {
            root_directory,
            logical_module_path,
            normal_root_file,
            support_root_file,
            shared_origin,
            normal_origin,
            support_origin,
        })
    }

    pub(crate) fn root_directory(&self) -> &Path {
        &self.root_directory
    }

    pub(crate) fn normal_root_file(&self) -> Option<&Path> {
        self.normal_root_file.as_deref()
    }

    pub(crate) fn support_root_file(&self) -> Option<&Path> {
        self.support_root_file.as_deref()
    }

    pub(crate) fn has_normal_root(&self) -> bool {
        self.normal_root_file.is_some()
    }

    /// The primary root file for legacy single-surface consumers.
    ///
    /// A paired structural module prefers its normal root. Paired consumers must select a
    /// surface explicitly instead of relying on this compatibility accessor.
    #[allow(dead_code)]
    pub(crate) fn root_file(&self) -> &Path {
        self.normal_root_file
            .as_deref()
            .or(self.support_root_file.as_deref())
            .expect("a structural module has at least one root file")
    }

    /// The primary surface role for legacy single-surface consumers.
    pub(crate) fn role(&self) -> ModuleRootRole {
        if self.has_normal_root() {
            ModuleRootRole::Normal
        } else {
            ModuleRootRole::Support
        }
    }

    /// The role-qualified origin for one present surface.
    pub(crate) fn surface_origin(
        &self,
        role: ModuleRootRole,
    ) -> Option<&StableModuleOriginIdentity> {
        match role {
            ModuleRootRole::Normal => self.normal_origin.as_ref(),
            ModuleRootRole::Support => self.support_origin.as_ref(),
            ModuleRootRole::Shared | ModuleRootRole::SyntheticProjectContext => None,
        }
    }

    /// The role-neutral origin used by helper files and as the structural module identity.
    pub(crate) fn shared_origin(&self) -> &StableModuleOriginIdentity {
        &self.shared_origin
    }

    #[allow(dead_code)]
    pub(crate) fn logical_module_path(&self) -> &Path {
        &self.logical_module_path
    }

    /// The primary role-qualified origin for legacy consumers.
    pub(crate) fn stable_origin(&self) -> &StableModuleOriginIdentity {
        self.normal_origin
            .as_ref()
            .or(self.support_origin.as_ref())
            .expect("a structural module has at least one surface origin")
    }
}

/// Canonical structural module identities and ancestry for one build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ModuleIdentityTable {
    records: Vec<ModuleIdentityRecord>,
    by_root_directory: FxHashMap<PathBuf, ModuleId>,
    nearest_ancestor: Vec<Option<ModuleId>>,
    direct_children: Vec<Vec<ModuleId>>,
}

impl ModuleIdentityTable {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Build the identity table from Stage 0 structural records.
    pub(crate) fn from_records(mut records: Vec<ModuleIdentityRecord>) -> Self {
        records.sort_by(|left, right| {
            left.logical_module_path
                .cmp(&right.logical_module_path)
                .then_with(|| left.root_directory.cmp(&right.root_directory))
        });

        let record_count = records.len();
        let mut by_root_directory: FxHashMap<PathBuf, ModuleId> = FxHashMap::default();
        for (index, record) in records.iter().enumerate() {
            debug_assert!(
                !by_root_directory.contains_key(&record.root_directory),
                "one structural module per rooted directory"
            );
            by_root_directory.insert(record.root_directory.clone(), ModuleId(index));
        }

        let nearest_ancestor = compute_nearest_ancestors(&records, &by_root_directory);
        let direct_children = compute_direct_children(record_count, &nearest_ancestor);

        Self {
            records,
            by_root_directory,
            nearest_ancestor,
            direct_children,
        }
    }

    /// Derive the normal-root lookup table used by synthetic and normal-only resolution.
    pub(crate) fn derive_module_root_table(&self) -> ModuleRootTable {
        let normal_records = self
            .records
            .iter()
            .filter_map(|record| {
                record.normal_root_file.as_ref().map(|root_file| {
                    ModuleRootRecord::from_surfaces(
                        record.root_directory.clone(),
                        Some(root_file.clone()),
                        None,
                    )
                })
            })
            .collect();
        ModuleRootTable::from_records(normal_records)
    }

    /// Derive the complete root table for semantic compilation inside one package boundary.
    ///
    /// A paired directory contributes its normal root here. The support root is selected through
    /// structural surface metadata, not through a second same-directory lookup entry.
    pub(crate) fn derive_compilation_root_table(&self) -> ModuleRootTable {
        let records = self
            .records
            .iter()
            .map(|record| {
                ModuleRootRecord::from_surfaces(
                    record.root_directory.clone(),
                    record.normal_root_file.clone(),
                    record.support_root_file.clone(),
                )
            })
            .collect();
        ModuleRootTable::from_records(records)
    }

    /// All structural module identities in deterministic canonical logical path order.
    pub(crate) fn module_ids(&self) -> impl Iterator<Item = ModuleId> + '_ {
        (0..self.records.len()).map(ModuleId)
    }

    /// The canonical record for one module identity.
    pub(crate) fn record(&self, module_id: ModuleId) -> &ModuleIdentityRecord {
        &self.records[module_id.0]
    }

    /// The structural identity for a canonical root directory.
    pub(crate) fn module_id_for_directory(&self, directory: &Path) -> Option<ModuleId> {
        self.by_root_directory.get(directory).copied()
    }

    /// The nearest enclosing structural module for an arbitrary directory.
    pub(crate) fn nearest_module_for_directory(&self, directory: &Path) -> Option<ModuleId> {
        let mut current = Some(directory);
        while let Some(candidate) = current {
            if let Some(&module_id) = self.by_root_directory.get(candidate) {
                return Some(module_id);
            }
            current = candidate.parent();
        }
        None
    }

    /// The nearest enclosing structural parent by directory containment.
    pub(crate) fn nearest_ancestor_module(&self, module_id: ModuleId) -> Option<ModuleId> {
        self.nearest_ancestor[module_id.0]
    }

    /// Direct child structural modules whose nearest ancestor is `module_id`.
    pub(crate) fn direct_child_modules(&self, module_id: ModuleId) -> &[ModuleId] {
        &self.direct_children[module_id.0]
    }
}

/// Compute the nearest enclosing structural module for each record by walking parent directories.
fn compute_nearest_ancestors(
    records: &[ModuleIdentityRecord],
    by_directory: &FxHashMap<PathBuf, ModuleId>,
) -> Vec<Option<ModuleId>> {
    records
        .iter()
        .map(|record| {
            let mut current = record.root_directory.parent();
            while let Some(directory) = current {
                if let Some(ancestor_id) = by_directory.get(directory) {
                    return Some(*ancestor_id);
                }
                current = directory.parent();
            }
            None
        })
        .collect()
}

/// Collect direct children for each module from the nearest-ancestor relationships.
fn compute_direct_children(
    record_count: usize,
    nearest_ancestor: &[Option<ModuleId>],
) -> Vec<Vec<ModuleId>> {
    let mut direct_children = vec![Vec::new(); record_count];
    for (index, ancestor) in nearest_ancestor.iter().enumerate() {
        if let Some(ancestor_id) = ancestor {
            direct_children[ancestor_id.0].push(ModuleId(index));
        }
    }
    direct_children
}
