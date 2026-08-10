//! Stage 0 durable module identity and structural topology.
//!
//! WHAT: owns the dense `ModuleId`, `ModuleIdentityRecord`, deterministic assignment table,
//! structural ancestry and the filename-to-root-role classifier produced by the one Stage 0
//! source-tree traversal, and derives frontend module-root lookup tables from those identities.
//! The cross-stage portable value types — `StablePackageIdentity`,
//! `StableModuleOriginIdentity` and `ModuleRootRole` — are compiler-semantic identity owned by
//! [`crate::compiler_frontend::semantic_identity`]; this module imports them so Stage 0 stays
//! the assignment and table owner while identity values remain build-independent.
//! WHY: durable identity and topology are build-system-owned data. The directory frontend
//! resolver consumes a derived normal-and-support root lookup table, while synthetic single-file
//! compilation can request the narrower normal-root view. Later graph-construction phases consume
//! the identity and ancestry directly from this table. The dense `ModuleId` is the build-local
//! handle for one build boundary; the owned `StableModuleOriginIdentity` is the cross-build
//! semantic identity that later exported declaration identities embed.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_normal_module_root_file, file_name_is_support_root_file,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Dense deterministic handle for one canonical module inside one build boundary.
///
/// `ModuleId` is the build-local index assigned by sorting canonical logical module paths, so
/// it is independent of traversal completion order and cosmetic root filename suffixes
/// (`@mod.moth` and `@page.moth` in the same directory collapse to one root whose identity
/// comes from the directory, not the filename).
///
/// It is deliberately not the persistent semantic identity: its numeric value is a build-local
/// table slot that may cross stages inside that build boundary but must not identify a module
/// across builds or reach persistent artefacts. The cross-build semantic identity is the owned
/// [`StableModuleOriginIdentity`](crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ModuleId(usize);

impl ModuleId {
    /// The build-local table slot index for this module.
    ///
    /// WHAT: lets Stage 0 owned-source classification, the project module graph and other
    /// co-owners index parallel arrays that are kept in `ModuleId` order. It is a build-local
    /// handle, not a persistent semantic identity, so it must not cross build boundaries.
    pub(crate) fn index(self) -> usize {
        self.0
    }

    /// Construct a `ModuleId` from a raw table slot index.
    ///
    /// WHAT: lets synthetic single-module boundaries and focused tests build dense identities.
    /// Production callers must use only validated ranges; defensive validation still rejects
    /// out-of-range values at every boundary that consumes the handle.
    pub(crate) fn from_index(index: usize) -> ModuleId {
        ModuleId(index)
    }
}

/// The root role implied by a canonical root filename, or `None` for non-root files.
///
/// WHAT: maps the filename marker to a root role. A `+*.moth` filename maps to `Support`; Stage 0
///      discovery upgrades a project-root `+*.moth` beside `config.moth` to
///      `ProjectPackageFacade` from its location, which this filename-only classifier cannot
///      infer.
pub(crate) fn module_root_role_for_file_name(file_name: &str) -> Option<ModuleRootRole> {
    if file_name_is_normal_module_root_file(file_name) {
        Some(ModuleRootRole::Normal)
    } else if file_name_is_support_root_file(file_name) {
        Some(ModuleRootRole::Support)
    } else {
        None
    }
}

/// One canonical module root with its durable structural identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleIdentityRecord {
    root_directory: PathBuf,
    root_file: PathBuf,
    role: ModuleRootRole,
    logical_module_path: PathBuf,
    stable_origin: StableModuleOriginIdentity,
}

impl ModuleIdentityRecord {
    /// Construct one canonical module record and derive its cross-build origin identity.
    ///
    /// `package` is the shared stable package identity for the project graph (or, later, a
    /// dependency package graph). The portable stable origin identity is derived from
    /// `logical_module_path` so the dense `PathBuf` remains the single source of truth for the
    /// build-local relative path while the stable identity stores only the portable spelling.
    /// Returns an internal `CompilerError` if `logical_module_path` contains an invalid or
    /// non-UTF-8 component; the caller surfaces it at the Stage 0 boundary.
    pub(crate) fn new(
        root_directory: PathBuf,
        root_file: PathBuf,
        role: ModuleRootRole,
        logical_module_path: PathBuf,
        package: &StablePackageIdentity,
    ) -> Result<Self, CompilerError> {
        let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
            package.clone(),
            &logical_module_path,
            role,
        )?;
        Ok(Self {
            root_directory,
            root_file,
            role,
            logical_module_path,
            stable_origin,
        })
    }

    // Identity accessors consumed by the project module graph (built in `project_roots`), by
    // owned-source classification and by focused tests.
    pub(crate) fn root_directory(&self) -> &Path {
        &self.root_directory
    }

    pub(crate) fn root_file(&self) -> &Path {
        &self.root_file
    }

    pub(crate) fn role(&self) -> ModuleRootRole {
        self.role
    }

    #[allow(dead_code)]
    pub(crate) fn logical_module_path(&self) -> &Path {
        &self.logical_module_path
    }

    /// The owned cross-build origin identity for this module.
    pub(crate) fn stable_origin(&self) -> &StableModuleOriginIdentity {
        &self.stable_origin
    }
}

/// Canonical module identities, root roles and structural ancestry for one build.
///
/// `ModuleId` assignment sorts records by canonical logical module path so identities are
/// deterministic. Nearest ancestry is computed over non-facade roots by directory containment;
/// the project package facade carries identity but no ancestry because it sits outside the
/// entry-root containment tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ModuleIdentityTable {
    records: Vec<ModuleIdentityRecord>,
    by_root_directory: HashMap<PathBuf, ModuleId>,
    nearest_ancestor: Vec<Option<ModuleId>>,
    direct_children: Vec<Vec<ModuleId>>,
}

impl ModuleIdentityTable {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    /// Build the identity table from Stage 0 root records.
    ///
    /// Records are sorted by canonical logical module path (then root directory and role as
    /// tie-breakers) so `ModuleId` assignment is deterministic. Ancestry is computed over
    /// non-facade roots by nearest-module directory containment. The facade shares the project
    /// root directory when the project root equals the entry root, so it is registered for
    /// directory identity only when no non-facade root already claims that directory.
    pub(crate) fn from_records(mut records: Vec<ModuleIdentityRecord>) -> Self {
        records.sort_by(|left, right| {
            left.logical_module_path
                .cmp(&right.logical_module_path)
                .then_with(|| left.root_directory.cmp(&right.root_directory))
                .then_with(|| left.role.cmp(&right.role))
        });

        let record_count = records.len();
        let mut by_root_directory: HashMap<PathBuf, ModuleId> = HashMap::new();
        let mut non_facade_by_directory: HashMap<PathBuf, ModuleId> = HashMap::new();

        for (index, record) in records.iter().enumerate() {
            let module_id = ModuleId(index);
            if record.role != ModuleRootRole::ProjectPackageFacade {
                non_facade_by_directory.insert(record.root_directory.clone(), module_id);
                by_root_directory.insert(record.root_directory.clone(), module_id);
            }
        }

        // The facade may share the project root directory when the project root equals the entry
        // root. Register it for directory identity only when no non-facade root already claims
        // that directory, so the shared directory keeps resolving to its module root.
        for (index, record) in records.iter().enumerate() {
            if record.role == ModuleRootRole::ProjectPackageFacade {
                by_root_directory
                    .entry(record.root_directory.clone())
                    .or_insert(ModuleId(index));
            }
        }

        let nearest_ancestor = compute_nearest_ancestors(&records, &non_facade_by_directory);
        let direct_children = compute_direct_children(record_count, &nearest_ancestor);

        Self {
            records,
            by_root_directory,
            nearest_ancestor,
            direct_children,
        }
    }

    /// Derive the narrow frontend module-root lookup table from the normal roots.
    ///
    /// Only `Normal` roots enter the frontend table so support and facade records stay out of
    /// import-resolution and header-role lookup in this slice. The frontend table preserves its
    /// pre-slice shape and responsibility.
    pub(crate) fn derive_module_root_table(&self) -> ModuleRootTable {
        let normal_records = self
            .records
            .iter()
            .filter(|record| record.role == ModuleRootRole::Normal)
            .map(|record| {
                ModuleRootRecord::new(record.root_directory.clone(), record.root_file.clone())
            })
            .collect();

        ModuleRootTable::from_records(normal_records)
    }

    /// Derive the complete root table for semantic compilation inside one package boundary.
    ///
    /// Package scheduling compiles normal and support modules directly. Its resolver therefore
    /// needs every indexed non-facade root for membership and same-directory content preparation.
    /// The project-root facade remains a retained Stage 0 identity, but it is not a filesystem
    /// module root for resolver membership and must not replace a normal root when both share the
    /// project directory in the compatibility layout.
    pub(crate) fn derive_compilation_root_table(&self) -> ModuleRootTable {
        let records = self
            .records
            .iter()
            .filter(|record| record.role != ModuleRootRole::ProjectPackageFacade)
            .map(|record| {
                ModuleRootRecord::new(record.root_directory.clone(), record.root_file.clone())
            })
            .collect();

        ModuleRootTable::from_records(records)
    }

    /// All module identities in deterministic canonical logical path order.
    pub(crate) fn module_ids(&self) -> impl Iterator<Item = ModuleId> + '_ {
        (0..self.records.len()).map(ModuleId)
    }

    /// The canonical record for one module identity.
    pub(crate) fn record(&self, module_id: ModuleId) -> &ModuleIdentityRecord {
        &self.records[module_id.0]
    }

    /// The stable identity for a canonical root directory, across all roles.
    #[allow(dead_code)]
    pub(crate) fn module_id_for_directory(&self, directory: &Path) -> Option<ModuleId> {
        self.by_root_directory.get(directory).copied()
    }

    /// The nearest enclosing non-facade module for an arbitrary directory, by walking parent
    /// directories.
    ///
    /// WHAT: classifies a directory (typically a source file's parent) under the nearest normal
    /// or support root that contains it. The project package facade is excluded because it sits
    /// outside entry-root containment and owns only its explicitly discovered root file, so a
    /// file under the entry root can never be owned by the facade even though the project root
    /// is its filesystem ancestor.
    /// WHY: Stage 0 owned-source classification needs one nearest-module lookup shared with the
    /// existing non-facade ancestry map. Returns `None` for a directory with no enclosing
    /// non-facade module root, which makes unrooted recognized candidates explicit.
    pub(crate) fn nearest_module_for_directory(&self, directory: &Path) -> Option<ModuleId> {
        let mut current = Some(directory);
        while let Some(candidate) = current {
            if let Some(&module_id) = self.by_root_directory.get(candidate) {
                // The facade shares the project root directory only when no non-facade root
                // claims it, so it never owns contained source files: a file under the entry
                // root always finds its real module first, and a file with no enclosing
                // non-facade root returns `None` rather than being silently owned by the
                // facade.
                if self.records[module_id.0].role != ModuleRootRole::ProjectPackageFacade {
                    return Some(module_id);
                }
            }
            current = candidate.parent();
        }
        None
    }

    /// The nearest enclosing non-facade module by directory containment, or `None` for the entry
    /// root or the project package facade.
    ///
    /// Consumed by the project module graph (built in `project_roots`) and by focused tests.
    pub(crate) fn nearest_ancestor_module(&self, module_id: ModuleId) -> Option<ModuleId> {
        self.nearest_ancestor[module_id.0]
    }

    /// Direct child modules whose nearest ancestor is `module_id`, in deterministic order.
    ///
    /// Consumed by the project module graph (built in `project_roots`) and by focused tests.
    pub(crate) fn direct_child_modules(&self, module_id: ModuleId) -> &[ModuleId] {
        &self.direct_children[module_id.0]
    }
}

/// Compute the nearest enclosing non-facade module for each record by walking parent directories.
///
/// The walk uses the non-facade directory map so the project package facade never becomes an
/// ancestor of entry-root modules. Facade records have no ancestor.
fn compute_nearest_ancestors(
    records: &[ModuleIdentityRecord],
    non_facade_by_directory: &HashMap<PathBuf, ModuleId>,
) -> Vec<Option<ModuleId>> {
    records
        .iter()
        .map(|record| {
            if record.role == ModuleRootRole::ProjectPackageFacade {
                return None;
            }

            let mut current = record.root_directory.parent();
            while let Some(directory) = current {
                if let Some(ancestor_id) = non_facade_by_directory.get(directory) {
                    return Some(*ancestor_id);
                }
                current = directory.parent();
            }

            None
        })
        .collect()
}

/// Collect direct children for each module from the nearest-ancestor relationships.
///
/// Children are gathered in deterministic `ModuleId` order so the resulting vectors are stable.
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
