//! Stage 0 source-tree indexing for directory projects.
//!
//! WHAT: performs the one deterministic entry-root traversal that prepares canonical module
//! identities, root entry candidates, sibling import-name collision facts, and entry-root
//! source-backed package prefix collision facts for the rest of the build. Each discovered root
//! receives a deterministic `ModuleId`, an explicit `ModuleRootRole` and a source-relative
//! logical module path owned by `module_identity`.
//! WHY: filesystem discovery belongs to Stage 0. Keeping it here prevents the frontend resolver,
//! module inventory, and collision validators from repeating the same expensive walk.

use super::module_identity::{
    ModuleId, ModuleIdentityRecord, ModuleIdentityTable, module_root_role_for_file_name,
};
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableOwnedSourceIdentity, StablePackageIdentity,
    portable_relative_logical_path_from,
};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_hash_root_file, file_name_is_module_root_file, file_name_is_support_root_file,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use super::project_structure_diagnostics::{
    non_utf8_filesystem_name_error, path_id, project_structure_messages,
};

const FIXED_SKIPPED_DIRECTORY_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "release",
    "dev",
    "dist",
    "build",
    ".cache",
];

/// Counts work performed by the Stage 0 source-tree traversal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceTreeDiscoveryStats {
    pub(crate) dirs_visited: usize,
    pub(crate) dirs_skipped: usize,
    pub(crate) files_seen: usize,
    pub(crate) hash_root_files_seen: usize,
    pub(crate) support_root_files_seen: usize,
    pub(crate) module_roots_found: usize,
    pub(crate) project_package_facade_found: bool,
}

/// Directory names and configured output boundaries excluded from entry-root traversal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceTreeSkipPolicy {
    configured_directories: Vec<PathBuf>,
}

impl SourceTreeSkipPolicy {
    fn from_config(project_root: &Path, entry_root: &Path, config: &Config) -> Self {
        let mut configured_directories = Vec::new();
        for configured_folder in [&config.dev_folder, &config.release_folder] {
            let configured_path = if configured_folder.is_absolute() {
                configured_folder.clone()
            } else {
                project_root.join(configured_folder)
            };

            if let Ok(canonical_path) = fs::canonicalize(configured_path)
                && canonical_path != entry_root
            {
                configured_directories.push(canonical_path);
            }
        }

        configured_directories.sort();
        configured_directories.dedup();

        Self {
            configured_directories,
        }
    }

    fn should_skip(&self, directory: &Path) -> bool {
        let fixed_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| FIXED_SKIPPED_DIRECTORY_NAMES.contains(&name));

        fixed_name
            || self
                .configured_directories
                .binary_search(&directory.to_path_buf())
                .is_ok()
    }
}

/// One canonical root file discovered inside a traversed directory, with its structural role.
struct DiscoveredDirectoryRoot {
    root_file: PathBuf,
    role: ModuleRootRole,
}

/// One supported source file candidate discovered by the Stage 0 traversal, before ownership
/// classification.
///
/// WHAT: pairs the canonical physical path of a builder-supported source file with its source
/// kind and its entry-root-relative portable logical candidate path. The canonical path is the
/// physical handle; the logical candidate path is the portable entry-root-relative spelling kept
/// separately so physical lookup and portable identity never share a field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveredSourceCandidate {
    canonical_path: PathBuf,
    kind: SourceFileKind,
    logical_candidate_path: String,
}

impl DiscoveredSourceCandidate {
    #[allow(dead_code)]
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    #[allow(dead_code)]
    pub(crate) fn kind(&self) -> SourceFileKind {
        self.kind
    }

    /// The entry-root-relative portable logical candidate path (forward-slash spelling).
    #[allow(dead_code)]
    pub(crate) fn logical_candidate_path(&self) -> &str {
        &self.logical_candidate_path
    }
}

/// One owned supported source file with its canonical physical path, source kind and portable
/// stable identity.
///
/// WHAT: the owned-source record stored inside a module's [`OwnedSourceSet`]. The canonical path
/// is the physical handle; `stable_identity` is the cross-build logical identity rooted in the
/// owning module origin plus the module-relative source file path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedSourceEntry {
    canonical_path: PathBuf,
    kind: SourceFileKind,
    stable_identity: StableOwnedSourceIdentity,
}

impl OwnedSourceEntry {
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    #[allow(dead_code)]
    pub(crate) fn kind(&self) -> SourceFileKind {
        self.kind
    }

    pub(crate) fn stable_identity(&self) -> &StableOwnedSourceIdentity {
        &self.stable_identity
    }
}

/// Deterministic set of owned supported sources for one canonical module.
///
/// WHAT: every supported source file whose nearest containing normal or support root is this
/// module, plus the project package facade's root file. Entries are sorted by their portable
/// module-relative source path so ordering is independent of traversal and checkout root.
/// WHY: ownership determines legal filesystem boundaries, collision scope, diagnostic
/// attribution and deterministic inventory identity for later Phase 3 semantic-source-set and
/// check-only slices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedSourceSet {
    module_id: ModuleId,
    entries: Vec<OwnedSourceEntry>,
}

impl OwnedSourceSet {
    #[allow(dead_code)]
    pub(crate) fn module_id(&self) -> ModuleId {
        self.module_id
    }

    pub(crate) fn entries(&self) -> &[OwnedSourceEntry] {
        &self.entries
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Canonical module identities and traversal evidence for one directory build.
///
/// `module_identities` is the Stage 0 durable identity and topology table; the project module
/// graph built from it owns normal entry classification and compile-wave scheduling. `module_roots`
/// is the narrow frontend normal-root lookup table derived from it for current resolver consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceTreeIndex {
    entry_root: PathBuf,
    module_identities: ModuleIdentityTable,
    module_roots: ModuleRootTable,
    owned_source_sets: Vec<OwnedSourceSet>,
    unrooted_candidates: Vec<DiscoveredSourceCandidate>,
    stats: SourceTreeDiscoveryStats,
}

impl SourceTreeIndex {
    /// Build the index with one deterministic traversal of the configured entry root.
    ///
    /// The traversal also owns entry-root sibling `.moth` file/folder import-name collisions and
    /// entry-root folder/source-backed package-prefix collisions, using the same sorted directory
    /// entries it already reads. Skipped directories neither contribute collision facts nor get
    /// recursively scanned. Source-backed package-tree collision validation remains separate because
    /// registered source-backed package traversal lives outside entry-root indexing.
    ///
    /// Each source directory may contain one `#*.moth` normal root or one `+*.moth` support root.
    /// Multiple or mixed roots in one directory are rejected through the existing structured config
    /// diagnostic lane. Normal roots carry the `Normal` role used by the project module graph to
    /// classify entry modules; the graph, not this index, owns entry selection. The optional
    /// project-root `+*.moth` facade beside `config.moth` is discovered as a separate
    /// `ProjectPackageFacade` node outside the entry-root containment tree and is never an entry.
    ///
    /// The same traversal inventories every builder-supported source candidate (`.moth` always,
    /// plus the `.mtf`/`.md` kinds the selected builder registered). Unknown extensions and
    /// known-but-unselected kinds never enter owned source sets. After deterministic
    /// `ModuleIdentityTable` construction, each supported candidate is classified under its
    /// nearest containing normal or support root into one [`OwnedSourceSet`] per module; the
    /// optional project facade owns its root file even though it sits outside entry-root
    /// containment. Supported candidates with no enclosing module root remain explicit
    /// `unrooted_candidates` facts rather than being silently discarded.
    pub(super) fn discover(
        entry_root: PathBuf,
        project_root: &Path,
        config: &Config,
        source_packages: &SourcePackageRegistry,
        source_file_kinds: &SourceFileKindRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, CompilerMessages> {
        let discovery_start = crate::timing::start_pipeline_timing();
        let skip_policy = SourceTreeSkipPolicy::from_config(project_root, &entry_root, config);

        // One stable project package identity shared by every node in the project graph: normal
        // roots, support roots and the optional project package facade. It is derived from the
        // configured project name, never from the checkout directory or an absolute path.
        let project_package = StablePackageIdentity::project_local(&config.project_name);

        let mut stats = SourceTreeDiscoveryStats::default();
        let mut queue = VecDeque::from([entry_root.clone()]);
        let mut records = Vec::new();
        let mut supported_candidates: Vec<DiscoveredSourceCandidate> = Vec::new();

        let facade_root_file =
            discover_project_package_facade(project_root, &mut stats, string_table)?;
        let facade_file_for_inventory = facade_root_file.clone();

        while let Some(directory) = queue.pop_front() {
            stats.dirs_visited += 1;

            let mut entries = fs::read_dir(&directory)
                .map_err(|error| Self::directory_read_error(&directory, error, string_table))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    CompilerError::file_error(
                        &directory,
                        format!(
                            "Failed to read directory entry while indexing source tree: {error}"
                        ),
                        string_table,
                    )
                })
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            entries.sort_by_key(|entry| entry.path());

            let mut subdirectories = Vec::new();
            let mut source_file_stems: BTreeSet<String> = BTreeSet::new();
            let mut importable_folder_names: BTreeSet<String> = BTreeSet::new();
            let mut directory_roots: Vec<DiscoveredDirectoryRoot> = Vec::new();

            for entry in entries {
                let path = entry.path();

                if path.is_dir() {
                    if skip_policy.should_skip(&path) {
                        stats.dirs_skipped += 1;
                    } else {
                        let folder_name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| {
                                non_utf8_filesystem_name_error(
                                    &path,
                                    "source tree folder name",
                                    string_table,
                                )
                            })?;
                        importable_folder_names.insert(folder_name.to_owned());
                        subdirectories.push(path);
                    }
                    continue;
                }

                if !path.is_file() {
                    continue;
                }

                stats.files_seen += 1;

                let file_name =
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| {
                            non_utf8_filesystem_name_error(
                                &path,
                                "source tree file name",
                                string_table,
                            )
                        })?;

                if let Some(stem) = source_stem_from_file_name(file_name) {
                    source_file_stems.insert(stem.to_owned());
                }

                let is_module_root = file_name_is_module_root_file(file_name);
                let source_kind = source_kind_for_file(file_name, source_file_kinds);

                // A file that is neither a module root nor a builder-supported source needs no
                // canonical path and contributes no inventory fact.
                if !is_module_root && source_kind.is_none() {
                    continue;
                }

                let canonical_path = fs::canonicalize(&path)
                    .map_err(|error| {
                        CompilerError::file_error(
                            &path,
                            format!("Failed to canonicalize source path: {error}"),
                            string_table,
                        )
                    })
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

                if let Some(kind) = source_kind {
                    // When the project root equals the entry root (the current compatibility
                    // case), the facade root file is reached by the traversal. It is owned only
                    // by the facade module through the direct facade assignment below, so it must
                    // not also enter the ordinary supported-candidate list. The accepted future
                    // strict-entry-root design never reaches the facade during traversal.
                    let is_facade_file = Some(&canonical_path) == facade_root_file.as_ref();
                    if !is_facade_file {
                        let logical_candidate_path =
                            entry_root_relative_logical_path(&path, &entry_root, string_table)?;
                        supported_candidates.push(DiscoveredSourceCandidate {
                            canonical_path: canonical_path.clone(),
                            kind,
                            logical_candidate_path,
                        });
                    }
                }

                if !is_module_root {
                    continue;
                }

                // The project package facade is discovered beside config.moth at the project root
                // and classified as a separate node. Skip it here so a directory shared with the
                // facade does not also classify it as a support root or trigger mixed-root
                // rejection. This also prevents the facade file from entering directory root
                // classification when it lies inside the traversal.
                let is_facade_file = Some(&canonical_path) == facade_root_file.as_ref();
                if is_facade_file {
                    continue;
                }

                let role = module_root_role_for_file_name(file_name)
                    .expect("a module root file name has a role after is_module_root_file");

                if role == ModuleRootRole::Normal {
                    stats.hash_root_files_seen += 1;
                } else if role == ModuleRootRole::Support {
                    stats.support_root_files_seen += 1;
                }

                directory_roots.push(DiscoveredDirectoryRoot {
                    root_file: canonical_path,
                    role,
                });
            }

            // Check sibling source file/folder import-name collisions from the same sorted
            // entries. Skipped folders are absent from importable_folder_names so they cannot
            // create false collisions.
            for stem in &source_file_stems {
                if importable_folder_names.contains(stem) {
                    return Err(project_structure_messages(
                        &directory,
                        InvalidConfigReason::BstFileFolderCollision {
                            file_name: string_table.intern(&format!("{stem}.moth")),
                            folder_name: string_table.intern(stem),
                            directory: path_id(&directory, string_table),
                        },
                        string_table,
                    ));
                }
            }

            // On the root pass, reject entry-root folders whose names collide with
            // source-backed package import prefixes.
            if directory == entry_root {
                for folder_name in &importable_folder_names {
                    if source_packages.has_prefix(folder_name) {
                        let colliding_folder = directory.join(folder_name);
                        return Err(project_structure_messages(
                            &colliding_folder,
                            InvalidConfigReason::EntryRootPackagePrefixCollision {
                                prefix: string_table.intern(folder_name),
                                entry_folder: path_id(&colliding_folder, string_table),
                            },
                            string_table,
                        ));
                    }
                }
            }

            if let Some(root) =
                classify_directory_root(&directory, &mut directory_roots, string_table)?
            {
                let canonical_root_directory = fs::canonicalize(&directory)
                    .map_err(|error| {
                        CompilerError::file_error(
                            &directory,
                            format!("Failed to canonicalize module root directory: {error}"),
                            string_table,
                        )
                    })
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

                let logical_module_path =
                    logical_module_path_from(&canonical_root_directory, &entry_root, string_table)?;

                stats.module_roots_found += 1;
                records.push(
                    ModuleIdentityRecord::new(
                        canonical_root_directory,
                        root.root_file,
                        root.role,
                        logical_module_path,
                        &project_package,
                    )
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
                );
            }

            subdirectories.sort();
            queue.extend(subdirectories);
        }

        if let Some(facade_file) = facade_root_file {
            let facade_directory = fs::canonicalize(project_root)
                .map_err(|error| {
                    CompilerError::file_error(
                        project_root,
                        format!("Failed to canonicalize project root directory: {error}"),
                        string_table,
                    )
                })
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

            records.push(
                ModuleIdentityRecord::new(
                    facade_directory.clone(),
                    facade_file,
                    ModuleRootRole::ProjectPackageFacade,
                    logical_module_path_from(&facade_directory, &facade_directory, string_table)?,
                    &project_package,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
            );
        }

        record_discovery_metrics(&stats, discovery_start);

        let module_identities = ModuleIdentityTable::from_records(records);
        let module_roots = module_identities.derive_module_root_table();
        let module_count = module_identities.module_ids().count();

        let (owned_source_sets, unrooted_candidates) = classify_owned_sources(
            &module_identities,
            module_count,
            supported_candidates,
            facade_file_for_inventory,
            string_table,
        )?;

        Ok(Self {
            entry_root,
            module_identities,
            module_roots,
            owned_source_sets,
            unrooted_candidates,
            stats,
        })
    }

    /// Prepare bounded root data for a directly compiled special entry file.
    ///
    /// The source tree uses the same index owner as directory compilation. The caller consumes
    /// only the prepared root table because its entry file is already explicit.
    pub(super) fn bounded_module_roots_for_single_file(
        entry_file: &Path,
        config: &Config,
        source_packages: &SourcePackageRegistry,
        source_file_kinds: &SourceFileKindRegistry,
        string_table: &mut StringTable,
    ) -> Result<ModuleRootTable, CompilerMessages> {
        let file_name = entry_file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                non_utf8_filesystem_name_error(entry_file, "single-file entry name", string_table)
            })?;
        if !file_name_is_hash_root_file(file_name) {
            return Ok(ModuleRootTable::empty());
        }

        let Some(root_directory) = entry_file.parent() else {
            return Ok(ModuleRootTable::empty());
        };
        let canonical_root = fs::canonicalize(root_directory)
            .map_err(|error| {
                CompilerError::file_error(
                    root_directory,
                    format!("Failed to canonicalize single-file source root: {error}"),
                    string_table,
                )
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

        Self::discover(
            canonical_root.clone(),
            &canonical_root,
            config,
            source_packages,
            source_file_kinds,
            string_table,
        )
        .map(|index| index.module_roots)
    }

    #[cfg(test)]
    pub(crate) fn entry_root(&self) -> &Path {
        &self.entry_root
    }

    pub(crate) fn module_roots(&self) -> &ModuleRootTable {
        &self.module_roots
    }

    /// The Stage 0 durable module identity and topology table.
    ///
    /// Consumed by the project module graph (built in `project_roots`) and by focused tests; the
    /// narrow frontend lookup table is available through [`SourceTreeIndex::module_roots`].
    pub(crate) fn module_identities(&self) -> &ModuleIdentityTable {
        &self.module_identities
    }

    /// One deterministic [`OwnedSourceSet`] per canonical module, indexed by `ModuleId`.
    ///
    /// WHAT: the Stage 0 owned supported-source inventory for every module in the project graph,
    /// including the optional project package facade. Each set is sorted by portable
    /// module-relative source path so ordering is independent of traversal and checkout root.
    /// WHY: later Phase 3 slices consume this as the ownership authority for semantic source
    /// sets, check-only orphan units and source attribution.
    #[allow(dead_code)]
    pub(crate) fn owned_source_sets(&self) -> &[OwnedSourceSet] {
        &self.owned_source_sets
    }

    /// The owned supported-source set for one module.
    ///
    /// Consumed by the project module graph (built in `project_roots`) and by focused tests.
    pub(crate) fn owned_source_set(&self, module_id: ModuleId) -> &OwnedSourceSet {
        &self.owned_source_sets[module_id.index()]
    }

    /// Supported source candidates with no enclosing module root.
    ///
    /// WHAT: explicit deterministic Stage 0 facts for files that sit outside any normal or
    /// support module root. They are not silently discarded; later phases decide whether they
    /// become check-only orphan units or are rejected. This slice invents no orphan diagnostic.
    #[allow(dead_code)]
    pub(crate) fn unrooted_candidates(&self) -> &[DiscoveredSourceCandidate] {
        &self.unrooted_candidates
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> &SourceTreeDiscoveryStats {
        &self.stats
    }

    fn directory_read_error(
        directory: &Path,
        error: std::io::Error,
        string_table: &mut StringTable,
    ) -> CompilerMessages {
        CompilerMessages::from_error_ref(
            CompilerError::file_error(
                directory,
                format!("Failed to read directory while indexing source tree: {error}"),
                string_table,
            ),
            string_table,
        )
    }
}

/// Discover the optional project package facade beside `config.moth` at the project root.
///
/// WHAT: scans the project root for one direct-child `+*.moth` file.
/// WHY: the facade is a node outside the entry-root containment tree. Discovering it here keeps it
///      out of the per-directory root classification so a shared directory does not classify it as
///      a support root or trigger mixed-root rejection.
fn discover_project_package_facade(
    project_root: &Path,
    stats: &mut SourceTreeDiscoveryStats,
    string_table: &mut StringTable,
) -> Result<Option<PathBuf>, CompilerMessages> {
    // A project-root read failure is an infrastructure error, not the absence of a facade.
    // Preserve it through the file-error lane with the project-root path so the build boundary
    // can render it instead of silently treating the facade as missing.
    let entries = fs::read_dir(project_root).map_err(|error| {
        CompilerMessages::from_error_ref(
            CompilerError::file_error(
                project_root,
                format!("Failed to read project root while discovering package facade: {error}"),
                string_table,
            ),
            string_table,
        )
    })?;

    let mut support_roots = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| {
                CompilerError::file_error(
                    project_root,
                    format!(
                        "Failed to read project root entry while discovering package facade: {error}"
                    ),
                    string_table,
                )
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
            .path();

        if !path.is_file() {
            continue;
        }

        // A non-UTF-8 direct-child filename cannot be classified as a support-root candidate and
        // must not be silently skipped. Use the same typed filesystem-name error owner as the
        // source-tree traversal so the offending path is preserved for rendering.
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                non_utf8_filesystem_name_error(
                    &path,
                    "project package facade candidate name",
                    string_table,
                )
            })?;

        if file_name_is_support_root_file(file_name) {
            let canonical = fs::canonicalize(&path)
                .map_err(|error| {
                    CompilerError::file_error(
                        &path,
                        format!("Failed to canonicalize project package facade path: {error}"),
                        string_table,
                    )
                })
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            support_roots.push(canonical);
        }
    }

    support_roots.sort();

    if support_roots.len() > 1 {
        let candidates = support_roots
            .iter()
            .map(|path| path_id(path, string_table))
            .collect();
        return Err(project_structure_messages(
            project_root,
            InvalidConfigReason::MultipleModuleRootFiles {
                directory: path_id(project_root, string_table),
                candidates,
            },
            string_table,
        ));
    }

    if support_roots.len() == 1 {
        stats.project_package_facade_found = true;
        Ok(support_roots.pop())
    } else {
        Ok(None)
    }
}

/// Reject multiple or mixed roots in one directory and return the single allowed root.
fn classify_directory_root(
    directory: &Path,
    directory_roots: &mut Vec<DiscoveredDirectoryRoot>,
    string_table: &mut StringTable,
) -> Result<Option<DiscoveredDirectoryRoot>, CompilerMessages> {
    if directory_roots.is_empty() {
        return Ok(None);
    }

    if directory_roots.len() > 1 {
        directory_roots.sort_by(|left, right| left.root_file.cmp(&right.root_file));
        let candidates = directory_roots
            .iter()
            .map(|root| path_id(&root.root_file, string_table))
            .collect();
        return Err(project_structure_messages(
            directory,
            InvalidConfigReason::MultipleModuleRootFiles {
                directory: path_id(directory, string_table),
                candidates,
            },
            string_table,
        ));
    }

    Ok(Some(directory_roots.pop().expect(
        "non-empty directory root list has one root after validation",
    )))
}

/// Compute the source-relative logical module path for a canonical root directory.
///
/// `base` is the canonical entry root (or, for the facade, the project root). A canonical root
/// directory discovered under the entry root always shares that prefix, so a `strip_prefix`
/// failure is a proven internal invariant: it means the entry root was not canonicalized before
/// indexing or the directory escaped the entry-root tree. Rather than silently falling back to an
/// absolute machine-local path (which would make `ModuleId` non-deterministic across machines),
/// surface it as an internal compiler error so the failure is never hidden.
fn logical_module_path_from(
    root_directory: &Path,
    base: &Path,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerMessages> {
    root_directory
        .strip_prefix(base)
        .map(PathBuf::from)
        .map_err(|_| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "Module root directory {root_directory:?} is not under the canonical base \
                 {base:?}; logical module path cannot fall back to an absolute path"
                )),
                string_table,
            )
        })
}

/// Extract the import-name stem from a validated source file name, or `None` for other extensions.
///
/// The caller must have already validated `file_name` as UTF-8 so that extension and stem
/// extraction can never silently skip a non-UTF-8 component.
fn source_stem_from_file_name(file_name: &str) -> Option<&str> {
    let path = Path::new(file_name);
    let extension = path.extension().and_then(|extension| extension.to_str())?;
    if extension != LANGUAGE_SOURCE_EXTENSION {
        return None;
    }
    path.file_stem().and_then(|stem| stem.to_str())
}

/// Resolve the builder-supported source kind for one validated UTF-8 file name.
///
/// `.moth` is always the compiler-owned `Moth` kind. Other extensions enter the inventory
/// only when they are compiler-recognized (via `SourceFileKind::from_extension`) and the selected
/// builder registered them with the correct extension-to-kind mapping (via
/// `supports_recognized_extension`). An arbitrary registered unknown extension (for example
/// `txt -> Moth template`) and a mismatched known mapping (for example `bd -> PlainMarkdown`) both
/// return `None` and stay out of owned source sets, so Stage 0 never duplicates the registry's
/// recognition policy.
fn source_kind_for_file(
    file_name: &str,
    source_file_kinds: &SourceFileKindRegistry,
) -> Option<SourceFileKind> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())?;
    let kind = SourceFileKind::from_extension(extension)?;
    if source_file_kinds.supports_recognized_extension(extension) {
        Some(kind)
    } else {
        None
    }
}

/// Compute the module-relative logical source path for one owned source file.
///
/// `file_path` is the canonical physical source path and `module_root_directory` is its owning
/// module's canonical root directory, so a `strip_prefix` failure is a proven internal
/// invariant: it means ownership classification assigned a file to a module that does not
/// contain it. Rather than silently falling back to an absolute path, surface it as an internal
/// compiler error so the failure is never hidden.
fn relative_source_path_from(
    file_path: &Path,
    module_root_directory: &Path,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerMessages> {
    file_path
        .strip_prefix(module_root_directory)
        .map(PathBuf::from)
        .map_err(|_| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "Owned source {file_path:?} is not under its nearest module root \
                     {module_root_directory:?}; module-relative source path cannot fall back to \
                     an absolute path"
                )),
                string_table,
            )
        })
}

/// Compute the entry-root-relative portable logical candidate path for one traversal source.
///
/// `traversal_path` is the non-canonicalized path built by joining entry-root descendants during
/// the walk, so stripping `entry_root` yields the entry-root-relative path without an
/// absolute-path fallback. Components are validated through the shared portable-path helper so
/// non-UTF-8 or invalid components surface through the existing error lanes.
fn entry_root_relative_logical_path(
    traversal_path: &Path,
    entry_root: &Path,
    string_table: &mut StringTable,
) -> Result<String, CompilerMessages> {
    let relative_path = traversal_path
        .strip_prefix(entry_root)
        .map(PathBuf::from)
        .map_err(|_| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "Discovered source candidate {traversal_path:?} is not under the entry root \
                     {entry_root:?}; logical candidate path cannot fall back to an absolute path"
                )),
                string_table,
            )
        })?;
    portable_relative_logical_path_from(&relative_path)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

/// Build one deterministic [`OwnedSourceSet`] per module and the explicit unrooted candidate
/// list from the supported candidates discovered during traversal.
///
/// WHAT: classifies every supported candidate under its nearest containing normal or support
/// root by walking parent directories through the identity table. A nested module root and all
/// files beneath it transfer to the nested module because the nearest-module walk finds it
/// first. Unrooted internal subdirectories stay owned by their nearest ancestor module. The
/// optional project facade owns its root file even though it sits outside entry-root
/// containment, so it is added directly to the facade module's owned set. Supported candidates
/// with no enclosing module root become explicit deterministic `unrooted_candidates` facts.
/// WHY: one authoritative classification feeds later Phase 3 semantic-source-set and
/// check-only slices. Owned entries are sorted by their portable module-relative source path so
/// ordering is independent of traversal and checkout root; unrooted candidates are sorted by
/// their portable entry-root-relative logical candidate path so the fact list is stable across
/// checkout roots and creation order.
fn classify_owned_sources(
    module_identities: &ModuleIdentityTable,
    module_count: usize,
    supported_candidates: Vec<DiscoveredSourceCandidate>,
    facade_file_for_inventory: Option<PathBuf>,
    string_table: &mut StringTable,
) -> Result<(Vec<OwnedSourceSet>, Vec<DiscoveredSourceCandidate>), CompilerMessages> {
    let mut owned_buckets: Vec<Vec<OwnedSourceEntry>> =
        (0..module_count).map(|_| Vec::new()).collect();
    let mut unrooted_candidates = Vec::new();

    for candidate in supported_candidates {
        let Some(parent_directory) = candidate.canonical_path.parent() else {
            unrooted_candidates.push(candidate);
            continue;
        };

        let Some(module_id) = module_identities.nearest_module_for_directory(parent_directory)
        else {
            unrooted_candidates.push(candidate);
            continue;
        };

        let record = module_identities.record(module_id);
        let relative_path = relative_source_path_from(
            &candidate.canonical_path,
            record.root_directory(),
            string_table,
        )?;
        let stable_identity = StableOwnedSourceIdentity::from_relative_source_path(
            record.stable_origin().clone(),
            &relative_path,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

        owned_buckets[module_id.index()].push(OwnedSourceEntry {
            canonical_path: candidate.canonical_path,
            kind: candidate.kind,
            stable_identity,
        });
    }

    // The optional project package facade root file lives beside config.moth, outside entry-root
    // containment. When the project root equals the entry root (the current compatibility case)
    // the facade file is reached by the traversal but excluded from the supported-candidate list
    // so it appears exactly once, owned only by the facade module. Assign it directly to the
    // facade module's owned set.
    if let Some(facade_file) = facade_file_for_inventory {
        let facade_module_id = module_identities.module_ids().find(|module_id| {
            module_identities.record(*module_id).role() == ModuleRootRole::ProjectPackageFacade
        });

        let facade_module_id = facade_module_id.ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "A project package facade file {facade_file:?} was discovered but no matching \
                     facade module record exists; the facade source must not be silently skipped"
                )),
                string_table,
            )
        })?;
        let record = module_identities.record(facade_module_id);
        let relative_path =
            relative_source_path_from(&facade_file, record.root_directory(), string_table)?;
        let stable_identity = StableOwnedSourceIdentity::from_relative_source_path(
            record.stable_origin().clone(),
            &relative_path,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

        owned_buckets[facade_module_id.index()].push(OwnedSourceEntry {
            canonical_path: facade_file,
            kind: SourceFileKind::Moth,
            stable_identity,
        });
    }

    let mut owned_source_sets = Vec::with_capacity(module_count);
    for (module_id, bucket) in module_identities.module_ids().zip(owned_buckets) {
        owned_source_sets.push(build_owned_source_set(module_id, bucket));
    }

    // Sort unrooted candidates by their portable entry-root-relative logical candidate path so
    // the fact list is stable across checkout roots and creation order. The canonical physical
    // path is a narrow tie-breaker for safety, though distinct files cannot share one
    // entry-root-relative path.
    unrooted_candidates.sort_by(|left, right| {
        left.logical_candidate_path
            .cmp(&right.logical_candidate_path)
            .then_with(|| left.canonical_path.cmp(&right.canonical_path))
    });

    Ok((owned_source_sets, unrooted_candidates))
}

/// Sort one module's owned entries by their portable module-relative source path and wrap them
/// in an [`OwnedSourceSet`].
fn build_owned_source_set(
    module_id: ModuleId,
    mut entries: Vec<OwnedSourceEntry>,
) -> OwnedSourceSet {
    entries.sort_by(|left, right| {
        left.stable_identity
            .relative_source_path()
            .cmp(right.stable_identity.relative_source_path())
    });
    OwnedSourceSet { module_id, entries }
}

fn record_discovery_metrics(
    stats: &SourceTreeDiscoveryStats,
    discovery_start: crate::timing::PipelineTimingStart,
) {
    crate::timing::record_started_pipeline_timing(
        "stage0.source_tree_index.discovery",
        discovery_start,
    );
    crate::timing::record_counter("source_tree_index.discovery_runs", 1.0);
    crate::timing::record_counter("source_tree_index.dirs_visited", stats.dirs_visited as f64);
    crate::timing::record_counter("source_tree_index.dirs_skipped", stats.dirs_skipped as f64);
    crate::timing::record_counter("source_tree_index.files_seen", stats.files_seen as f64);
    crate::timing::record_counter(
        "source_tree_index.hash_root_files_seen",
        stats.hash_root_files_seen as f64,
    );
    crate::timing::record_counter(
        "source_tree_index.support_root_files_seen",
        stats.support_root_files_seen as f64,
    );
    crate::timing::record_counter(
        "source_tree_index.module_roots_found",
        stats.module_roots_found as f64,
    );
    crate::timing::record_counter(
        "source_tree_index.project_package_facade_found",
        if stats.project_package_facade_found {
            1.0
        } else {
            0.0
        },
    );
}
