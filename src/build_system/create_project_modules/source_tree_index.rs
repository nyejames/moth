//! Stage 0 source-tree indexing for project and source-package boundaries.
//!
//! WHAT: performs one deterministic traversal per project or source-package boundary, preparing
//! canonical module identities, source ownership and sibling import-name collision facts. Project
//! boundaries also own entry/package-prefix collisions and optional project-facade discovery;
//! package boundaries own their required public normal-root validation. Each discovered root gets
//! a deterministic `ModuleId`, explicit `ModuleRootRole` and boundary-relative logical module path.
//! WHY: filesystem discovery belongs to Stage 0. Keeping it here prevents the frontend resolver,
//! module inventory, and collision validators from repeating the same expensive walk.
//!
//! The traversal also inventories every compiler-recognized or provider-owned source file once
//! into a central `SourceRecord` table addressed by dense `SourceId` values. Owned and unrooted collections
//! store only `SourceId`s, so the index is the sole source inventory/ownership owner and later
//! consumers resolve source data through it rather than through duplicated per-module records.

use super::module_identity::{
    ModuleId, ModuleIdentityRecord, ModuleIdentityTable, module_root_role_for_file_name,
};
use crate::builder_surface::external_import_providers::provider::ExternalFileExtension;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StableOwnedSourceIdentity, StablePackageIdentity,
    portable_relative_logical_path_from,
};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_legacy_hash_root_file, file_name_is_module_root_file,
    file_name_is_normal_module_root_file, file_name_is_support_root_file,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

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
    pub(crate) normal_root_files_seen: usize,
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

/// One recognized source candidate discovered by the Stage 0 traversal, before ownership
/// classification.
///
/// WHAT: pairs the canonical physical path of a compiler-recognized or provider-owned source file
/// with its source classification and its entry-root-relative portable logical candidate path.
/// The canonical path is the physical handle; the logical candidate path is the portable
/// entry-root-relative spelling kept separately so physical lookup and portable identity never
/// share a field.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredSourceCandidate {
    canonical_path: PathBuf,
    classification: SourceClassification,
    supported: bool,
    logical_candidate_path: String,
}

/// Dense build-local handle addressing one slot in the contiguous [`SourceRecord`] table.
///
/// `SourceId` is assigned deterministically from portable logical identity (see
/// [`SourceLogicalIdentity`]) and is independent of traversal, file-creation order and checkout
/// root. Absolute physical paths never become semantic identity. `SourceId` is build-local and
/// never leaks across module boundaries or into persistent artefacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SourceId(usize);

impl SourceId {
    /// The contiguous table index this handle addresses.
    pub(crate) fn index(self) -> usize {
        self.0
    }

    fn from_index(index: usize) -> Self {
        SourceId(index)
    }
}

/// The portable logical identity of one source record, used as the deterministic `SourceId` sort
/// key.
///
/// `Owned` carries the cross-build [`StableOwnedSourceIdentity`] rooted in the owning module
/// origin plus the module-relative source file path. `Unrooted` carries the entry-root-relative
/// portable logical path for files outside any module root. Both variants are portable forward
/// slash spellings with no absolute path component, so ordering is stable across checkout roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceLogicalIdentity {
    Owned(StableOwnedSourceIdentity),
    Unrooted(UnrootedSourceLogicalPath),
}

impl SourceLogicalIdentity {
    /// The owning module origin for an owned source, or `None` for an unrooted source.
    pub(crate) fn module_origin(&self) -> Option<&StableModuleOriginIdentity> {
        match self {
            SourceLogicalIdentity::Owned(identity) => Some(identity.module_origin()),
            SourceLogicalIdentity::Unrooted(_) => None,
        }
    }
}

impl PartialOrd for SourceLogicalIdentity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SourceLogicalIdentity {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SourceLogicalIdentity::Owned(left), SourceLogicalIdentity::Owned(right)) => left
                .module_origin()
                .cmp(right.module_origin())
                .then_with(|| {
                    left.relative_source_path()
                        .cmp(right.relative_source_path())
                }),
            (SourceLogicalIdentity::Unrooted(left), SourceLogicalIdentity::Unrooted(right)) => {
                left.as_str().cmp(right.as_str())
            }
            (SourceLogicalIdentity::Owned(_), SourceLogicalIdentity::Unrooted(_)) => Ordering::Less,
            (SourceLogicalIdentity::Unrooted(_), SourceLogicalIdentity::Owned(_)) => {
                Ordering::Greater
            }
        }
    }
}

/// The entry-root-relative portable logical path for one unrooted source file.
///
/// A newtype around the portable forward-slash spelling so it is never confused with an owned
/// source's module-relative path or an absolute physical path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct UnrootedSourceLogicalPath(String);

impl UnrootedSourceLogicalPath {
    fn from_portable(path: String) -> Self {
        Self(path)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The ownership state of one source record.
///
/// `Owned(module_id)` records the canonical module that owns the source; `Unrooted` records a
/// file with no enclosing module root. The owning `ModuleId` is the build-local handle carried
/// alongside the portable logical identity so source ownership stays single-owned by the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceOwnership {
    Owned(ModuleId),
    Unrooted,
}

/// The closed classification of one source record: compiler semantic input or explicit
/// provider-owned input.
///
/// WHAT: distinguishes files that feed tokenization, header preparation and semantic
/// compilation from files whose extension is registered with an external import provider but is
/// not a compiler `SourceFileKind`. Provider-owned records never pretend to be `.moth`, `.mtf`
/// or `.md`; the provider extension is retained so the directory provider import path validates
/// the target from indexed facts rather than re-probing the filesystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceClassification {
    /// A compiler semantic input (`.moth`, `.mtf`, `.md`) that feeds the frontend pipeline.
    CompilerSemantic(SourceFileKind),
    /// An explicit provider-owned input whose extension is registered with an external import
    /// provider but is not a compiler `SourceFileKind`.
    ProviderOwned(ExternalFileExtension),
}

/// One recognized compiler source or provider-owned file stored exactly once in the central
/// [`SourceTreeIndex`] table.
///
/// WHAT: owns the dense `SourceId`, canonical physical path (the IO handle), source
/// classification, portable logical identity (the deterministic sort key) and explicit owned
/// `ModuleId` or unrooted state. The canonical path is never semantic identity; the logical
/// identity is.
/// WHY: the index is the sole source inventory/ownership owner. Later Stage 0 consumers resolve
/// source data through `SourceId` rather than through duplicated per-module records, so identity,
/// ownership and physical lookup each have one owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceRecord {
    id: SourceId,
    canonical_path: PathBuf,
    classification: SourceClassification,
    supported: bool,
    logical_identity: SourceLogicalIdentity,
    ownership: SourceOwnership,
}

impl SourceRecord {
    pub(crate) fn id(&self) -> SourceId {
        self.id
    }

    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    /// The closed classification of this source record.
    ///
    /// Consumed by the directory provider import path to validate that an authored provider
    /// target is an explicit provider-owned input, not a compiler semantic file.
    pub(crate) fn classification(&self) -> &SourceClassification {
        &self.classification
    }

    /// Whether this source's extension is registered in the project's `SourceFileKindRegistry`.
    ///
    /// `Moth` is always supported. Other recognized kinds (`MothTemplate`, `PlainMarkdown`) are
    /// supported only when the builder registered the extension. The namespace checks this flag
    /// to produce `UnsupportedSourceFileKind` diagnostics for recognized-but-unsupported imports
    /// without a filesystem probe.
    pub(crate) fn supported(&self) -> bool {
        self.supported
    }

    pub(crate) fn logical_identity(&self) -> &SourceLogicalIdentity {
        &self.logical_identity
    }

    /// The ownership state of this source record.
    ///
    /// Consumed by the directory provider import path to validate same-module ownership from
    /// indexed facts rather than reconstructing the module boundary from the import path.
    pub(crate) fn ownership(&self) -> SourceOwnership {
        self.ownership
    }
}

/// One classified source awaiting deterministic `SourceId` assignment.
///
/// Built during the single post-traversal classification pass. The logical identity is the
/// portable sort key; ownership records the owning module or unrooted state. `SourceId` is
/// assigned after sorting, so this struct deliberately carries no dense handle.
struct ClassifiedSource {
    canonical_path: PathBuf,
    classification: SourceClassification,
    supported: bool,
    logical_identity: SourceLogicalIdentity,
    ownership: SourceOwnership,
    entry_root_relative_logical_path: Option<String>,
}

/// Completed central source inventory produced after classification and deterministic ID
/// assignment.
///
/// WHAT: groups the one record table with its ID-only owned and unrooted projections plus the
/// entry-root-relative logical path and canonical path lookup maps that let the directory
/// provider import path resolve an authored target to a `SourceId` without filesystem probing.
/// WHY: the collections form one construction result and must enter `SourceTreeIndex` together
/// without an opaque tuple or a second durable owner.
struct SourceInventory {
    sources: Vec<SourceRecord>,
    owned_source_ids: Vec<Vec<SourceId>>,
    unrooted_source_ids: Vec<SourceId>,
    logical_path_to_source_id: FxHashMap<String, SourceId>,
    canonical_path_to_source_id: FxHashMap<PathBuf, SourceId>,
}

/// Canonical module identities and traversal evidence for one directory build.
///
/// `module_identities` is the Stage 0 durable identity and topology table; the project module
/// graph built from it owns normal entry classification and compile-wave scheduling. `module_roots`
/// is the narrow frontend normal-root lookup table derived from it for current resolver consumers.
///
/// `sources` is the central contiguous `SourceRecord` table addressed by dense `SourceId`s; it is
/// the sole source inventory/ownership owner. `owned_source_ids` and `unrooted_source_ids` store
/// only `SourceId`s so no consumer duplicates source records.
///
/// `logical_path_to_source_id` and `canonical_path_to_source_id` are the two non-probing lookup
/// maps for the directory provider import path: the logical path map resolves an authored
/// provider target by its entry-root-relative portable spelling, and the canonical path map
/// resolves an importer file to its owning record. Canonical paths remain IO handles; the maps
/// never make them semantic identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceTreeIndex {
    entry_root: PathBuf,
    module_identities: ModuleIdentityTable,
    module_roots: ModuleRootTable,
    sources: Vec<SourceRecord>,
    owned_source_ids: Vec<Vec<SourceId>>,
    unrooted_source_ids: Vec<SourceId>,
    logical_path_to_source_id: FxHashMap<String, SourceId>,
    canonical_path_to_source_id: FxHashMap<PathBuf, SourceId>,
    stats: SourceTreeDiscoveryStats,
}

/// One Stage 0 compilation boundary's traversal inputs.
///
/// WHAT: carries the entry root, stable package identity, skip policy and
/// project-only options that distinguish a directory-project traversal from a source-package
/// boundary traversal.
/// WHY: project and package boundaries share one filesystem traversal owner but own distinct
/// stable package identities, source IDs and module IDs. The descriptor keeps the shared
/// traversal's project/package branches explicit, so no parallel package traversal or
/// compatibility scanner is needed.
struct SourceTreeBoundary<'a> {
    entry_root: PathBuf,
    package_identity: StablePackageIdentity,
    skip_policy: SourceTreeSkipPolicy,
    kind: SourceTreeBoundaryKind<'a>,
}

/// Boundary-specific policy for the shared source-tree traversal.
#[derive(Clone, Copy)]
enum SourceTreeBoundaryKind<'a> {
    Project {
        project_root: &'a Path,
        source_packages: &'a SourcePackageRegistry,
    },
    Package {
        import_prefix: &'a str,
    },
}

impl SourceTreeIndex {
    /// Build the index with one deterministic traversal of the configured entry root.
    ///
    /// The traversal also owns entry-root sibling `.moth` file/folder import-name collisions and
    /// entry-root folder/source-backed package-prefix collisions, using the same sorted directory
    /// entries it already reads. Skipped directories neither contribute collision facts nor get
    /// recursively scanned.
    ///
    /// Project and package boundaries share one traversal implementation through
    /// [`SourceTreeIndex::discover_for_boundary`]; this entry point supplies the project
    /// boundary descriptor and delegates to it. Package-boundary sibling collisions and root
    /// discovery are owned by the same traversal via [`SourceTreeIndex::discover_package`].
    ///
    /// Each source directory may contain one `@*.moth` normal root or one `+*.moth` support root.
    /// Multiple or mixed roots in one directory are rejected through the existing structured config
    /// diagnostic lane. Legacy `#*.moth` root-like filenames are rejected with a structured
    /// diagnostic that tells the author to rename to `@*.moth`. Normal roots carry the `Normal`
    /// role used by the project module graph to classify entry modules; the graph, not this index,
    /// owns entry selection. The optional project-root `+*.moth` facade beside `config.moth` is
    /// discovered as a separate `ProjectPackageFacade` node outside the entry-root containment
    /// tree and is never an entry.
    ///
    /// The same traversal inventories every compiler-recognized source candidate (`.moth`,
    /// `.mtf` and `.md`) and every explicit
    /// provider-owned file whose extension is registered with the external provider registry but is
    /// not a compiler `SourceFileKind`. Unknown extensions never enter owned source sets. After
    /// deterministic `ModuleIdentityTable` construction, each recognized candidate is classified
    /// under its nearest containing normal or support root into
    /// the central [`SourceRecord`] table; the optional project facade owns its root file even
    /// though it sits outside entry-root containment. Recognized candidates with no enclosing
    /// module root remain explicit unrooted [`SourceRecord`]s rather than being silently
    /// discarded. Each record receives a dense [`SourceId`] assigned in portable logical identity
    /// order, and per-module owned and unrooted collections store only `SourceId`s.
    pub(super) fn discover(
        entry_root: PathBuf,
        project_root: &Path,
        config: &Config,
        source_packages: &SourcePackageRegistry,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, CompilerMessages> {
        let boundary = SourceTreeBoundary {
            entry_root: entry_root.clone(),
            package_identity: StablePackageIdentity::project_local(&config.project_name),
            skip_policy: SourceTreeSkipPolicy::from_config(project_root, &entry_root, config),
            kind: SourceTreeBoundaryKind::Project {
                project_root,
                source_packages,
            },
        };
        Self::discover_for_boundary(
            boundary,
            source_file_kinds,
            external_import_providers,
            string_table,
        )
    }

    /// Build one source-package boundary index from its already-canonical root directory.
    ///
    /// WHAT: traverses one source-backed package root with its own stable package identity,
    /// dense `SourceId`s, module IDs and ownership tables. The package index becomes the
    /// filesystem owner for the package root's direct-child root discovery and for sibling
    /// `.moth` file/folder collisions throughout the package tree, using the same traversal
    /// implementation as the project index.
    /// WHY: each project or package compilation boundary owns a separate `SourceTreeIndex` so
    /// raw `SourceId`/`ModuleId` values never cross boundaries. Reusing the project traversal
    /// avoids a parallel package traversal while preserving package-specific missing-root and
    /// multiple-root diagnostics at the package root directory.
    pub(crate) fn discover_package(
        canonical_root: PathBuf,
        package_identity: StablePackageIdentity,
        import_prefix: &str,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, CompilerMessages> {
        let boundary = SourceTreeBoundary {
            entry_root: canonical_root,
            package_identity,
            skip_policy: SourceTreeSkipPolicy::default(),
            kind: SourceTreeBoundaryKind::Package { import_prefix },
        };
        Self::discover_for_boundary(
            boundary,
            source_file_kinds,
            external_import_providers,
            string_table,
        )
    }

    /// Run the shared boundary-parameterized source-tree traversal.
    ///
    /// This is the single filesystem inventory implementation. The boundary descriptor supplies
    /// the stable package identity, skip policy, project-only facade and entry-root prefix
    /// collision inputs, and selects package-specific root-directory classification. Every other
    /// step — per-directory collision checks, candidate inventory, nearest-module ownership,
    /// deterministic `SourceId` assignment — is shared between project and package boundaries.
    fn discover_for_boundary(
        boundary: SourceTreeBoundary,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, CompilerMessages> {
        let discovery_start = crate::timing::start_pipeline_timing();

        let SourceTreeBoundary {
            entry_root,
            package_identity: boundary_package,
            skip_policy,
            kind,
        } = boundary;
        // One stable package identity shared by every node in this boundary's graph: normal
        // roots, support roots and, for project boundaries, the optional package facade. It is
        // derived from the configured project name (project boundary) or the registry's
        // `PackageOrigin` plus import prefix (package boundary), never from an absolute path.
        let mut stats = SourceTreeDiscoveryStats::default();
        let mut queue = VecDeque::from([entry_root.clone()]);
        let mut records = Vec::new();
        let mut recognized_candidates: Vec<DiscoveredSourceCandidate> = Vec::new();

        // The optional project package facade is discovered only for project boundaries; package
        // boundaries have no facade and their root directory is the package public surface.
        let facade_root_file = match kind {
            SourceTreeBoundaryKind::Project { project_root, .. } => {
                discover_project_package_facade(project_root, &mut stats, string_table)?
            }
            SourceTreeBoundaryKind::Package { .. } => None,
        };
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
            let mut source_files_by_stem: BTreeMap<String, String> = BTreeMap::new();
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
                    source_files_by_stem.insert(stem.to_owned(), file_name.to_owned());
                }

                // Legacy `#*.moth` root-like filenames are invalid after the `@` migration.
                // Reject them with a structured diagnostic before any other classification so
                // they are never treated as ordinary source files or silently ignored.
                if file_name_is_legacy_hash_root_file(file_name) {
                    return Err(project_structure_messages(
                        &path,
                        InvalidConfigReason::LegacyModuleRootFileName {
                            file_name: string_table.intern(file_name),
                            directory: path_id(&directory, string_table),
                        },
                        string_table,
                    ));
                }

                let is_module_root = file_name_is_module_root_file(file_name);
                let source_kind = recognized_source_kind(file_name);
                let source_supported = source_kind.is_some_and(|kind| {
                    source_file_kinds.supports_recognized_extension(kind.extension())
                });
                let provider_extension = source_kind
                    .is_none()
                    .then(|| {
                        provider_owned_extension_for_file(file_name, external_import_providers)
                    })
                    .flatten();

                // A file that is neither a module root, a compiler-recognized source, nor an
                // explicit provider-owned file needs no canonical path and contributes no
                // inventory fact.
                if !is_module_root && source_kind.is_none() && provider_extension.is_none() {
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

                if let Some(kind) = source_kind
                    // When the project root equals the entry root, the facade root file is
                    // reached by the traversal. It is owned only
                    // by the facade module through the direct facade assignment below, so it must
                    // not also enter the ordinary supported-candidate list. The accepted future
                    // strict-entry-root design never reaches the facade during traversal.
                    && {
                        let is_facade_file = Some(&canonical_path) == facade_root_file.as_ref();
                        !is_facade_file
                    }
                {
                    let logical_candidate_path =
                        entry_root_relative_logical_path(&path, &entry_root, string_table)?;
                    recognized_candidates.push(DiscoveredSourceCandidate {
                        canonical_path: canonical_path.clone(),
                        classification: SourceClassification::CompilerSemantic(kind),
                        supported: source_supported,
                        logical_candidate_path,
                    });
                } else if let Some(extension) = provider_extension {
                    // Provider-owned files are never module roots, so they need no facade
                    // exclusion. They enter the inventory as explicit provider-owned candidates
                    // classified under their nearest enclosing module root, exactly like
                    // compiler semantic candidates.
                    let logical_candidate_path =
                        entry_root_relative_logical_path(&path, &entry_root, string_table)?;
                    recognized_candidates.push(DiscoveredSourceCandidate {
                        canonical_path: canonical_path.clone(),
                        classification: SourceClassification::ProviderOwned(extension),
                        supported: true,
                        logical_candidate_path,
                    });
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
                    stats.normal_root_files_seen += 1;
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
            for (stem, file_name) in &source_files_by_stem {
                if let Some(folder_name) = importable_folder_names
                    .iter()
                    .find(|folder_name| folder_name.eq_ignore_ascii_case(stem))
                {
                    return Err(project_structure_messages(
                        &directory,
                        InvalidConfigReason::SourceFileFolderCollision {
                            file_name: string_table.intern(file_name),
                            folder_name: string_table.intern(folder_name),
                            directory: path_id(&directory, string_table),
                        },
                        string_table,
                    ));
                }
            }

            // On the project root pass, reject entry-root folders whose names collide with
            // registered source-backed package import prefixes. Package boundaries carry no
            // prefix-collision packages, so this check is skipped for them.
            if let SourceTreeBoundaryKind::Project {
                source_packages, ..
            } = kind
                && directory == entry_root
            {
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

            // Package boundaries require exactly one root at the package root directory and
            // report package-specific missing/multiple-root diagnostics there. Every other
            // directory (and all project-boundary directories) uses the shared one-root-per-
            // directory classification with its mixed/multiple-root diagnostic.
            let directory_root = match kind {
                SourceTreeBoundaryKind::Package { import_prefix } if directory == entry_root => {
                    classify_package_root_directory(
                        &directory,
                        &mut directory_roots,
                        import_prefix,
                        string_table,
                    )?
                }
                SourceTreeBoundaryKind::Project { .. } | SourceTreeBoundaryKind::Package { .. } => {
                    classify_directory_root(&directory, &mut directory_roots, string_table)?
                }
            };
            if let Some(root) = directory_root {
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
                        &boundary_package,
                    )
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
                );
            }

            subdirectories.sort();
            queue.extend(subdirectories);
        }

        if let Some(facade_file) = facade_root_file {
            let SourceTreeBoundaryKind::Project { project_root, .. } = kind else {
                unreachable!("only project boundaries discover a project package facade");
            };
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
                    &boundary_package,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
            );
        }

        record_discovery_metrics(&stats, discovery_start);

        let module_identities = ModuleIdentityTable::from_records(records);
        let module_roots = module_identities.derive_module_root_table();
        let module_count = module_identities.module_ids().count();

        let classified = classify_owned_sources(
            &module_identities,
            recognized_candidates,
            facade_file_for_inventory,
            &entry_root,
            string_table,
        )?;

        let SourceInventory {
            sources,
            owned_source_ids,
            unrooted_source_ids,
            logical_path_to_source_id,
            canonical_path_to_source_id,
        } = build_source_inventory(classified, module_count)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

        Ok(Self {
            entry_root,
            module_identities,
            module_roots,
            sources,
            owned_source_ids,
            unrooted_source_ids,
            logical_path_to_source_id,
            canonical_path_to_source_id,
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
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<ModuleRootTable, CompilerMessages> {
        let file_name = entry_file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                non_utf8_filesystem_name_error(entry_file, "single-file entry name", string_table)
            })?;
        if !file_name_is_normal_module_root_file(file_name) {
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
            external_import_providers,
            string_table,
        )
        .map(|index| index.module_roots)
    }

    pub(crate) fn entry_root(&self) -> &Path {
        &self.entry_root
    }

    /// The canonical root file of the module rooted at this boundary's entry root.
    ///
    /// WHAT: for a package boundary index, the package public-surface normal root file. Returns
    /// `None` when no module is rooted at the entry root.
    /// WHY: the package-boundary index owner derives the resolver's narrow package-root view
    /// from this indexed fact instead of a separate `read_dir` or root-file discovery pass.
    pub(crate) fn root_file_for_entry_root(&self) -> Option<&Path> {
        let module_id = self
            .module_identities
            .module_id_for_directory(&self.entry_root)?;
        Some(self.module_identities.record(module_id).root_file())
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

    /// The central contiguous source record table, addressed by dense `SourceId`s.
    ///
    /// WHAT: every recognized compiler source and provider-owned file discovered by the single
    /// Stage 0 traversal, stored
    /// exactly once with its canonical physical path, source classification, portable logical
    /// identity and owned `ModuleId` or unrooted state. `SourceId`s are assigned in deterministic
    /// portable logical identity order so the table order is independent of traversal,
    /// file-creation order and checkout root.
    /// WHY: the index is the sole source inventory/ownership owner. Later Stage 0 consumers
    /// resolve source data through `SourceId` rather than through duplicated per-module records.
    #[allow(dead_code)]
    pub(crate) fn sources(&self) -> &[SourceRecord] {
        &self.sources
    }

    /// The number of source records in this index.
    ///
    /// Each `SourceId` from `0` to `source_count() - 1` addresses a valid `SourceRecord`.
    pub(crate) fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// One source record addressed by its dense `SourceId`.
    ///
    /// `source_id` must be a valid `SourceId` produced by this index. A valid `SourceId` always
    /// addresses a record because `SourceId`s are assigned contiguously from `0`.
    pub(crate) fn source(&self, source_id: SourceId) -> &SourceRecord {
        let record = &self.sources[source_id.index()];
        debug_assert_eq!(record.id(), source_id);
        record
    }

    /// The `SourceId`s owned by one canonical module, indexed by `ModuleId`.
    ///
    /// WHAT: every recognized compiler source or provider-owned file whose nearest containing
    /// normal or support root is this
    /// module, plus the optional project package facade's root file. The IDs are in portable
    /// module-relative source path order so ordering is independent of traversal and checkout
    /// root.
    /// WHY: later Phase 3 slices consume this as the ownership authority for semantic source
    /// sets, check-only orphan units and source attribution.
    pub(crate) fn owned_source_ids(&self, module_id: ModuleId) -> &[SourceId] {
        &self.owned_source_ids[module_id.index()]
    }

    /// The `SourceId`s for recognized or provider-owned files with no enclosing module root.
    ///
    /// WHAT: explicit deterministic Stage 0 facts for files that sit outside any normal or
    /// support module root. They are not silently discarded; later phases decide whether they
    /// become check-only orphan units or are rejected. This slice invents no orphan diagnostic.
    /// The IDs are in portable entry-root-relative logical path order.
    #[allow(dead_code)]
    pub(crate) fn unrooted_source_ids(&self) -> &[SourceId] {
        &self.unrooted_source_ids
    }

    /// Resolve one `SourceId` by its entry-root-relative portable logical path.
    ///
    /// Focused index-invariant tests use this to verify deterministic logical lookup. Production
    /// import resolution consumes the prebuilt module namespace instead.
    #[cfg(test)]
    pub(crate) fn source_id_for_entry_root_relative_logical_path(
        &self,
        logical_path: &str,
    ) -> Option<SourceId> {
        self.logical_path_to_source_id.get(logical_path).copied()
    }

    /// Resolve one `SourceId` by its canonical physical path.
    ///
    /// The directory namespace uses this to select the importing file's owning boundary-local
    /// module. Returns `None` when no indexed record carries that canonical path.
    pub(crate) fn source_id_for_canonical_path(&self, canonical_path: &Path) -> Option<SourceId> {
        self.canonical_path_to_source_id
            .get(canonical_path)
            .copied()
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

/// Classify the single required root at one source-package root directory.
///
/// WHAT: a package boundary requires exactly one direct-child normal root at its root directory
/// and reports package-specific missing-root and multiple-root diagnostics, preserving the typed
/// `SourcePackageMissingRoot`/`SourcePackageMultipleRoots` payloads and deterministic candidate
/// order that the separate preflight validators previously produced. The shared
/// one-module-root-per-directory rule still rejects a support root beside that normal root.
/// WHY: the package index traversal owns root discovery for the package boundary, so the
/// root-directory classification must produce the same structured diagnostics rather than the
/// generic `MultipleModuleRootFiles` rejection or a silent empty result.
fn classify_package_root_directory(
    directory: &Path,
    directory_roots: &mut Vec<DiscoveredDirectoryRoot>,
    import_prefix: &str,
    string_table: &mut StringTable,
) -> Result<Option<DiscoveredDirectoryRoot>, CompilerMessages> {
    let normal_roots = directory_roots
        .iter()
        .filter(|root| root.role == ModuleRootRole::Normal)
        .collect::<Vec<_>>();

    if normal_roots.is_empty() {
        return Err(project_structure_messages(
            directory,
            InvalidConfigReason::SourcePackageMissingRoot {
                prefix: string_table.intern(import_prefix),
                root: path_id(directory, string_table),
            },
            string_table,
        ));
    }

    if normal_roots.len() > 1 {
        let mut candidate_paths = normal_roots
            .iter()
            .map(|root| &root.root_file)
            .collect::<Vec<_>>();
        candidate_paths.sort();
        let candidates = candidate_paths
            .into_iter()
            .map(|path| path_id(path, string_table))
            .collect();
        return Err(project_structure_messages(
            directory,
            InvalidConfigReason::SourcePackageMultipleRoots {
                prefix: string_table.intern(import_prefix),
                root: path_id(directory, string_table),
                candidates,
            },
            string_table,
        ));
    }

    // A package root still follows the global one-module-root-per-directory rule. A support root
    // beside the required normal root is rejected by the shared classifier rather than being
    // silently ignored as the old direct-child root scan did.
    classify_directory_root(directory, directory_roots, string_table)
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

/// Extract the import-name stem from a compiler-recognized source file name.
///
/// The caller must have already validated `file_name` as UTF-8 so that extension and stem
/// extraction can never silently skip a non-UTF-8 component.
fn source_stem_from_file_name(file_name: &str) -> Option<&str> {
    let path = Path::new(file_name);
    let extension = path.extension().and_then(|extension| extension.to_str())?;
    SourceFileKind::from_extension(extension)?;
    path.file_stem().and_then(|stem| stem.to_str())
}

/// Resolve the compiler-recognized source kind for one validated UTF-8 file name.
///
/// Returns `Some(kind)` for every recognized extension (`moth`, `mtf`, `md`) regardless of
/// whether the project's `SourceFileKindRegistry` supports it. The discovery pass computes
/// `supported` separately from `supports_recognized_extension` so the index can surface
/// recognized-but-unsupported files for structured import diagnostics without a filesystem
/// probe during import resolution.
fn recognized_source_kind(file_name: &str) -> Option<SourceFileKind> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())?;
    SourceFileKind::from_extension(extension)
}

/// Resolve the provider-owned extension for one validated UTF-8 file name.
///
/// Returns the registered `ExternalFileExtension` only when the extension is not a compiler
/// `SourceFileKind` and some registered external import provider supports it. Compiler semantic
/// extensions are never classified as provider-owned, so a provider that happens to register a
/// compiler-kind extension cannot displace the compiler's ownership.
fn provider_owned_extension_for_file(
    file_name: &str,
    external_import_providers: &ExternalImportProviderRegistry,
) -> Option<ExternalFileExtension> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())?;
    if SourceFileKind::from_extension(extension).is_some() {
        return None;
    }
    external_import_providers
        .supports_extension(extension)
        .then(|| ExternalFileExtension::from(extension))
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

/// Compute the entry-root-relative portable logical path for the optional facade file.
///
/// The facade lives beside `config.moth` at the project root. When the project root equals the
/// entry root, the facade is under the entry root and receives a
/// logical path entry. The accepted future strict-entry-root design places the facade outside
/// entry-root containment, so the strip then fails and `None` is returned rather than erroring;
/// the facade still enters the canonical-path lookup map because its canonical path is always
/// available.
fn entry_root_relative_logical_path_for_facade(
    facade_file: &Path,
    entry_root: &Path,
) -> Option<String> {
    let relative_path = facade_file.strip_prefix(entry_root).ok()?;
    portable_relative_logical_path_from(relative_path).ok()
}

/// Classify every recognized candidate discovered during traversal and assign each a portable
/// logical identity and explicit ownership, ready for deterministic `SourceId` assignment.
///
/// WHAT: classifies every recognized candidate under its nearest containing normal or support
/// root by walking parent directories through the identity table. A nested module root and all
/// files beneath it transfer to the nested module because the nearest-module walk finds it
/// first. Unrooted internal subdirectories stay owned by their nearest ancestor module. The
/// optional project facade owns its root file even though it sits outside entry-root
/// containment, so it is added directly as a classified owned source. Recognized candidates
/// with no enclosing module root become explicit deterministic unrooted classified sources.
/// WHY: one authoritative classification feeds later Phase 3 semantic-source-set and
/// check-only slices. Each classified source carries its portable logical identity and explicit
/// ownership; `SourceId`s are assigned deterministically afterwards in [`build_source_inventory`],
/// so ordering is independent of traversal and checkout root.
fn classify_owned_sources(
    module_identities: &ModuleIdentityTable,
    recognized_candidates: Vec<DiscoveredSourceCandidate>,
    facade_file_for_inventory: Option<PathBuf>,
    entry_root: &Path,
    string_table: &mut StringTable,
) -> Result<Vec<ClassifiedSource>, CompilerMessages> {
    let mut classified = Vec::new();

    for candidate in recognized_candidates {
        let Some(parent_directory) = candidate.canonical_path.parent() else {
            classified.push(unrooted_classified(candidate));
            continue;
        };

        let Some(module_id) = module_identities.nearest_module_for_directory(parent_directory)
        else {
            classified.push(unrooted_classified(candidate));
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

        classified.push(ClassifiedSource {
            canonical_path: candidate.canonical_path,
            classification: candidate.classification,
            supported: candidate.supported,
            logical_identity: SourceLogicalIdentity::Owned(stable_identity),
            ownership: SourceOwnership::Owned(module_id),
            entry_root_relative_logical_path: Some(candidate.logical_candidate_path),
        });
    }

    // The optional project package facade root file lives beside config.moth, outside entry-root
    // containment. When the project root equals the entry root (the current compatibility case)
    // the facade file is reached by the traversal but excluded from the supported-candidate list
    // so it appears exactly once, owned only by the facade module. Assign it directly to the
    // facade module as a classified owned source.
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

        let entry_root_relative_logical_path =
            entry_root_relative_logical_path_for_facade(&facade_file, entry_root);
        classified.push(ClassifiedSource {
            canonical_path: facade_file,
            classification: SourceClassification::CompilerSemantic(SourceFileKind::Moth),
            supported: true,
            logical_identity: SourceLogicalIdentity::Owned(stable_identity),
            ownership: SourceOwnership::Owned(facade_module_id),
            entry_root_relative_logical_path,
        });
    }

    Ok(classified)
}

/// Build one classified unrooted source from a traversal candidate that has no enclosing module
/// root.
fn unrooted_classified(candidate: DiscoveredSourceCandidate) -> ClassifiedSource {
    let logical_candidate_path = candidate.logical_candidate_path;
    ClassifiedSource {
        canonical_path: candidate.canonical_path,
        classification: candidate.classification,
        supported: candidate.supported,
        logical_identity: SourceLogicalIdentity::Unrooted(
            UnrootedSourceLogicalPath::from_portable(logical_candidate_path.clone()),
        ),
        ownership: SourceOwnership::Unrooted,
        entry_root_relative_logical_path: Some(logical_candidate_path),
    }
}

/// Assign dense `SourceId`s and build the central [`SourceRecord`] table plus per-module owned
/// and unrooted `SourceId` collections from the classified sources.
///
/// WHAT: sorts every classified source by its portable logical identity and rejects duplicate
/// logical identities as an internal invariant violation. Each source receives a contiguous
/// `SourceId` equal to its table index. Absolute paths remain diagnostic and IO context only; they
/// never participate in identity or ordering.
/// Per-module owned IDs and the unrooted ID list are projected from the sorted table, so each
/// module's owned IDs are in portable module-relative source path order and the unrooted IDs are
/// in portable entry-root-relative logical path order.
/// WHY: one deterministic inventory projection feeds later Phase 3 semantic-source-set and
/// check-only slices. Ordering is independent of traversal, file-creation order and checkout
/// root because the sort key is the portable logical identity, not the absolute path.
fn build_source_inventory(
    classified: Vec<ClassifiedSource>,
    module_count: usize,
) -> Result<SourceInventory, CompilerError> {
    let mut classified = classified;
    classified.sort_by(|left, right| left.logical_identity.cmp(&right.logical_identity));

    for sources in classified.windows(2) {
        let [left, right] = sources else {
            unreachable!("windows(2) always yields pairs");
        };
        if left.logical_identity == right.logical_identity {
            return Err(CompilerError::compiler_error(format!(
                "Source tree index classified two physical sources with the same portable logical identity {:?}: {} and {}; source identity must be unique before SourceId assignment",
                left.logical_identity,
                left.canonical_path.display(),
                right.canonical_path.display(),
            )));
        }
    }

    let mut sources = Vec::with_capacity(classified.len());
    let mut owned_source_ids: Vec<Vec<SourceId>> = (0..module_count).map(|_| Vec::new()).collect();
    let mut unrooted_source_ids = Vec::new();
    let mut logical_path_to_source_id: FxHashMap<String, SourceId> = FxHashMap::default();
    let mut canonical_path_to_source_id: FxHashMap<PathBuf, SourceId> = FxHashMap::default();

    for (index, source) in classified.into_iter().enumerate() {
        let source_id = SourceId::from_index(index);
        if let Some(logical_path) = &source.entry_root_relative_logical_path
            && logical_path_to_source_id
                .insert(logical_path.clone(), source_id)
                .is_some()
        {
            return Err(CompilerError::compiler_error(format!(
                "Source tree index assigned entry-root-relative logical path {logical_path:?} to \
                 multiple source records; provider lookup paths must be unique",
            )));
        }
        if canonical_path_to_source_id
            .insert(source.canonical_path.clone(), source_id)
            .is_some()
        {
            return Err(CompilerError::compiler_error(format!(
                "Source tree index classified physical source {} more than once; each canonical \
                 source path must have exactly one SourceRecord",
                source.canonical_path.display(),
            )));
        }
        match source.ownership {
            SourceOwnership::Owned(module_id) => {
                owned_source_ids[module_id.index()].push(source_id);
            }
            SourceOwnership::Unrooted => {
                unrooted_source_ids.push(source_id);
            }
        }
        sources.push(SourceRecord {
            id: source_id,
            canonical_path: source.canonical_path,
            classification: source.classification,
            supported: source.supported,
            logical_identity: source.logical_identity,
            ownership: source.ownership,
        });
    }

    Ok(SourceInventory {
        sources,
        owned_source_ids,
        unrooted_source_ids,
        logical_path_to_source_id,
        canonical_path_to_source_id,
    })
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
        "source_tree_index.normal_root_files_seen",
        stats.normal_root_files_seen as f64,
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
