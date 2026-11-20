//! Stage 0 source-tree indexing for project and source-package boundaries.
//!
//! WHAT: performs one deterministic traversal per project or source-package boundary, preparing
//! canonical module identities, source ownership and sibling dependency-name collision facts. Project
//! boundaries also own entry/package-prefix collisions;
//! package boundaries own their required public API-facade validation. Each rooted directory gets
//! one deterministic `ModuleId`, explicit normal/support surface facts and one boundary-relative
//! logical module path; shared helpers belong to that one structural module.
//! WHY: filesystem discovery belongs to Stage 0. Keeping it here prevents the frontend resolver,
//! module inventory, and collision validators from repeating the same expensive walk.
//!
//! The traversal also inventories every compiler-recognized or provider-owned source file once
//! into a central `SourceRecord` table addressed by dense `SourceRecordIndex` handles. Owned and
//! unrooted collections store only those handles, so the index is the sole source
//! inventory/ownership owner and later consumers resolve source data through it rather than
//! through duplicated per-module records.
use super::module_identity::{ModuleId, ModuleIdentityRecord, ModuleIdentityTable};
use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::builder_surface::definition::damaged_package_set_report;
use crate::builder_surface::external_import_providers::provider::ExternalFileExtension;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::package_metadata::{
    PackageSetMembership, is_reserved_package_namespace_root, reserved_package_namespace_claim_root,
};
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticBag, InvalidConfigReason, PreparedFailure,
};
#[cfg(test)]
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_normalization::directory_name_is_hidden;
use crate::compiler_frontend::project_globals::claims_project_globals_name;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootFiles, ModuleRootRole, StableModuleOriginIdentity, StableOwnedSourceIdentity,
    StablePackageIdentity, portable_relative_logical_path_from,
};
use crate::compiler_frontend::source::{
    SourceDatabase, SourceId, SourceKind, SourceRegistrationIndex,
};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_config_file, file_name_is_invalid_module_root_marker,
    file_name_is_module_root_file, module_root_role_for_file_name,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use crate::counter_observation;
use crate::projects::settings::{CONFIG_FILE_NAME, Config};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};

use super::project_structure_diagnostics::{
    entry_root_package_prefix_collision_error, invalid_module_root_file_name_error,
    non_utf8_filesystem_name_error, project_structure_diagnostic,
};

/// Local namespace facts for one synthetic owning root.
///
/// WHAT: the immediate names the synthetic resolver checks for package overlap, one importable
/// claim on a reserved package namespace root when the selection owns no module topology, and the
/// first local directory claiming a registered source-package prefix.
/// WHY: every fact comes from one directory scan, so neither reserved-root ownership nor the
/// exact-prefix collision requires a second traversal or content inference.
pub(super) struct SingleFileNamespaceInventory {
    pub(super) keys: BTreeSet<String>,
    pub(super) reserved_root_claim: Option<(String, PathBuf)>,
    /// The first registered prefix with a matching directory, plus that directory's authored path.
    ///
    /// The pair retains the registered spelling and the authored directory path so the caller can
    /// build `EntryRootPackagePrefixCollision` with the same diagnostic owner as directory mode.
    pub(super) registered_prefix_collision: Option<(String, PathBuf)>,
}

/// Directory names that are never source regardless of configuration.
///
/// `_` is the general source-exclusion rule. These fixed names are intentional skips for external
/// tooling trees, not aliases for the old visible dev/release output locations. Visible `dev`,
/// `release`, `dist` and `build` directories are deliberately absent because they may contain
/// legitimate source; generated output is hidden by its leading `_` component instead.
const FIXED_SKIPPED_DIRECTORY_NAMES: &[&str] = &[".git", "target", "node_modules", ".cache"];

/// Counts work performed by the Stage 0 source-tree traversal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceTreeDiscoveryStats {
    pub(crate) dirs_visited: usize,
    /// Directory entries rejected before traversal, including aliases classified by destination.
    pub(crate) dirs_skipped: usize,
    pub(crate) files_seen: usize,
    pub(crate) normal_root_files_seen: usize,
    pub(crate) support_root_files_seen: usize,
    pub(crate) entry_root_files_seen: usize,
    pub(crate) module_roots_found: usize,
}

/// Directory names and configured output boundaries excluded from entry-root traversal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceTreeSkipPolicy {
    configured_directories: Vec<PathBuf>,
    canonical_output_directories: Vec<PathBuf>,
}

/// Validated project-level inputs shared by source-tree discovery and output-policy exclusion.
pub(super) struct SourceTreeProjectContext<'a> {
    pub(super) validated_output_settings: Option<&'a ValidatedDirectoryOutputSettings>,
    /// Bootstrap-registered config source, when present; discovery excludes its canonical target.
    pub(super) config_source: Option<ProjectConfigSource<'a>>,
}

impl SourceTreeSkipPolicy {
    fn from_validated_output_settings(
        entry_root: &Path,
        validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    ) -> Self {
        let mut configured_directories = Vec::new();
        let mut canonical_output_directories = Vec::new();

        if let Some(settings) = validated_output_settings {
            for configured_path in [&settings.dev.resolved_path, &settings.release.resolved_path] {
                // Keep the validated lexical path so a symlink alias is excluded before Stage 0
                // descends through it, then retain the physical target so another path to the
                // same output tree cannot reintroduce source-looking generated files.
                if configured_path != entry_root {
                    configured_directories.push(configured_path.clone());
                }

                if let Ok(canonical_path) = fs::canonicalize(configured_path)
                    && canonical_path != entry_root
                {
                    configured_directories.push(canonical_path.clone());
                    canonical_output_directories.push(canonical_path);
                }
            }
        }

        configured_directories.sort();
        configured_directories.dedup();
        canonical_output_directories.sort();
        canonical_output_directories.dedup();

        Self {
            configured_directories,
            canonical_output_directories,
        }
    }

    /// Whether an authored directory entry may become a namespace claim and be traversed.
    ///
    /// A normal directory is already canonical because its parent was canonicalized before
    /// listing, so this checks only the authored rules and avoids another filesystem probe.
    fn authored_directory_is_eligible(&self, directory: &Path, entry_root: &Path) -> bool {
        directory.starts_with(entry_root) && !self.lexical_directory_is_skipped(directory)
    }

    /// Whether a resolved directory destination may become a namespace claim and be traversed.
    ///
    /// Directory symlinks are classified by their physical destination before the visible alias
    /// spelling enters dependency-name or package-prefix facts.
    fn resolved_directory_is_eligible(&self, directory: &Path, entry_root: &Path) -> bool {
        directory.starts_with(entry_root)
            && !self.canonical_ancestor_is_skipped(directory, entry_root)
    }

    /// Apply the name and configured-path rules that need no canonicalization.
    fn lexical_directory_is_skipped(&self, directory: &Path) -> bool {
        let file_name = directory.file_name().and_then(|name| name.to_str());
        let fixed_or_hidden = file_name.is_some_and(|name| {
            FIXED_SKIPPED_DIRECTORY_NAMES.contains(&name) || directory_name_is_hidden(name)
        });

        fixed_or_hidden
            || self
                .configured_directories
                .binary_search_by(|candidate| candidate.as_path().cmp(directory))
                .is_ok()
    }

    /// Whether any canonical directory between `path` and the selected root is skipped.
    ///
    /// The selected root is exempt: a root named `_prototype` remains compilable when it is the
    /// explicit source boundary. Callers pass canonical directories, so this repeats the lexical
    /// skip rules without another canonicalization syscall.
    fn canonical_ancestor_is_skipped(&self, path: &Path, entry_root: &Path) -> bool {
        path.ancestors()
            .take_while(|directory| *directory != entry_root)
            .any(|directory| self.canonical_directory_is_skipped(directory))
    }

    /// Apply the same exclusion rules to a directory that is already canonical.
    fn canonical_directory_is_skipped(&self, directory: &Path) -> bool {
        self.lexical_directory_is_skipped(directory)
            || self
                .canonical_output_directories
                .iter()
                .any(|output_root| directory == output_root || directory.starts_with(output_root))
    }
}

/// One root file discovered inside a traversed directory, with its structural role.
///
/// `root_file` is the canonical physical file used for identity; `visible_root_file` retains the
/// authored `@`/`+` spelling so a filesystem-structure diagnostic can name the marker the user
/// actually wrote.
struct DiscoveredDirectoryRoot {
    root_file: PathBuf,
    visible_root_file: PathBuf,
    role: ModuleRootRole,
}

/// The fixed role-keyed root inventory discovered in one rooted directory.
#[derive(Default)]
struct ClassifiedDirectoryRoots {
    roots: ModuleRootFiles<DiscoveredDirectoryRoot>,
}

/// One canonical root-file claim discovered before structural module identities are built.
///
/// Canonical paths decide physical identity, while both authored and canonical spellings remain
/// available for diagnostics. Claims are collected before source IDs exist so a file can be
/// rejected when two modules claim it or when it resolves outside the module that roots it.
struct DiscoveredModuleRootClaim {
    directory: PathBuf,
    root_file: PathBuf,
    visible_root_file: PathBuf,
}

/// One rooted directory discovered before its structural identity record is constructed.
struct DiscoveredModuleRecord {
    directory: PathBuf,
    roots: ModuleRootFiles<PathBuf>,
    logical_module_path: PathBuf,
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
    /// The discovered module-root surface role, retained before canonicalization.
    ///
    /// A root file reached through a symlink has a visible `@`/`+` spelling but a differently
    /// named canonical target. Role inference from the canonical filename loses the surface, so
    /// classification consumes the role discovered from the visible entry instead.
    root_role: Option<ModuleRootRole>,
}

/// Zero-based row handle for a source candidate in one [`SourceTreeIndex`].
///
/// This is a Stage 0 table position, not a compiler source identity. The compiler assigns the
/// boundary's [`crate::compiler_frontend::source::SourceId`] after the rows are registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SourceRecordIndex(usize);

impl SourceRecordIndex {
    pub(crate) fn index(self) -> usize {
        self.0
    }

    fn from_index(index: usize) -> Self {
        Self(index)
    }
}

/// Owned portable entry-root-relative spelling for an unrooted source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct UnrootedSourceLogicalPath(String);

impl UnrootedSourceLogicalPath {
    pub(crate) fn from_portable(spelling: String) -> Self {
        Self(spelling)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The logical identity of one source record.
///
/// `Owned` carries the cross-build [`StableOwnedSourceIdentity`] rooted in the owning module
/// origin plus the module-relative source file path. `Unrooted` carries an owned
/// entry-root-relative portable spelling for files outside any module root. Both variants have no
/// absolute path component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SourceLogicalIdentity {
    Owned(StableOwnedSourceIdentity),
    Unrooted(UnrootedSourceLogicalPath),
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

impl SourceLogicalIdentity {
    /// The owning module origin for an owned source, or `None` for an unrooted source.
    pub(crate) fn module_origin(&self) -> Option<&StableModuleOriginIdentity> {
        match self {
            SourceLogicalIdentity::Owned(identity) => Some(identity.module_origin()),
            SourceLogicalIdentity::Unrooted(_) => None,
        }
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
/// or `.md`; the provider extension is retained so the directory provider path validates the
/// target from indexed facts rather than re-probing the filesystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceClassification {
    /// A compiler semantic input (`.moth`, `.mtf`, `.md`) that feeds the frontend pipeline.
    CompilerSemantic(SourceFileKind),
    /// An explicit provider-owned input whose extension is registered with an external import
    /// provider but is not a compiler `SourceFileKind`.
    ProviderOwned(ExternalFileExtension),
}

/// Map Stage 0's authored classification onto the compiler source-record kind.
///
/// The lexical file name is classified before canonicalize, so this mapping preserves that
/// authority rather than re-deriving kind from the canonical extension.
fn source_kind_from_classification(classification: &SourceClassification) -> SourceKind {
    match classification {
        SourceClassification::CompilerSemantic(kind) => SourceKind::Compiler(*kind),
        SourceClassification::ProviderOwned(_) => SourceKind::ProviderOwned,
    }
}

/// One recognized compiler source or provider-owned file stored exactly once in the Stage 0
/// source-tree table.
///
/// The row owns the canonical physical path used for IO, classification, stable logical identity,
/// and explicit `ModuleId` ownership. Its zero-based position is a tree-local lookup handle, not a
/// compiler `SourceId`; the boundary `SourceDatabase` assigns the latter from registration rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceRecord {
    canonical_path: PathBuf,
    classification: SourceClassification,
    supported: bool,
    logical_identity: SourceLogicalIdentity,
    ownership: SourceOwnership,
}

impl SourceRecord {
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    /// The closed classification of this source record.
    ///
    /// Consumed by the directory provider path to validate that an authored provider
    /// target is an explicit provider-owned input, not a compiler semantic file.
    pub(crate) fn classification(&self) -> &SourceClassification {
        &self.classification
    }

    /// Whether this source's extension is supported by the active builder.
    ///
    /// `.moth` and `.mtf` are compiler-supported under every builder. A recognized optional kind
    /// (plain Markdown) is supported only when the builder selected it. The namespace checks this
    /// flag to produce `UnsupportedSourceFileKind` diagnostics for recognized but unsupported
    /// dependencies without a filesystem probe.
    pub(crate) fn supported(&self) -> bool {
        self.supported
    }

    pub(crate) fn logical_identity(&self) -> &SourceLogicalIdentity {
        &self.logical_identity
    }

    /// The ownership state of this source record.
    ///
    /// Consumed by the directory provider path to validate same-module ownership from
    /// indexed facts rather than reconstructing the module boundary from the dependency path.
    pub(crate) fn ownership(&self) -> SourceOwnership {
        self.ownership
    }
}

/// One classified source awaiting compiler-boundary registration.
///
/// The stable logical identity is the portable sort key. No compiler `SourceId` is assigned in
/// this Stage 0 pass.
struct ClassifiedSource {
    canonical_path: PathBuf,
    classification: SourceClassification,
    supported: bool,
    logical_identity: SourceLogicalIdentity,
    ownership: SourceOwnership,
}

/// Completed central source inventory produced after classification and deterministic sorting.
///
/// The collections hold zero-based source-row handles for Stage 0 ownership and namespace
/// resolution. The compiler-facing registration index is produced after these rows are sorted.
struct SourceInventory {
    sources: Vec<SourceRecord>,
    owned_source_indices: Vec<Vec<SourceRecordIndex>>,
    unrooted_source_indices: Vec<SourceRecordIndex>,
    canonical_path_to_source_index: FxHashMap<PathBuf, SourceRecordIndex>,
}

/// The bootstrap-registered build-only config source, resolved through its owning database.
///
/// WHAT: pairs the config source identity with the database that owns its canonical path.
/// WHY: bootstrap registers the physical config source before discovery. Carrying both facts
///      together lets discovery exclude the canonical target while the database stays the sole
///      path owner, so no second path copy enters build settings.
#[derive(Clone, Copy)]
pub(super) struct ProjectConfigSource<'a> {
    pub(super) source_id: SourceId,
    pub(super) source_files: &'a SourceDatabase,
}

impl<'a> ProjectConfigSource<'a> {
    /// The canonical physical path of the registered config source.
    fn canonical_path(&self) -> Option<&'a Path> {
        self.source_files
            .get(self.source_id)?
            .canonical_os_path
            .as_deref()
    }
}

/// Canonical module identities and traversal evidence for one directory build.
///
/// `module_identities` is the Stage 0 durable identity and topology table; the project module
/// graph built from it owns entry classification and compile-wave scheduling, and semantic
/// compilation derives its role-keyed root lookup from it. No second normal-only root table is
/// stored here.
///
/// `sources` is the central source-row table addressed by [`SourceRecordIndex`]. It is the sole
/// Stage 0 source inventory/ownership owner. The compiler-facing [`SourceRegistrationIndex`] is
/// assembled from its already-sorted canonical rows when a boundary constructs the
/// [`crate::compiler_frontend::source::SourceDatabase`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceTreeIndex {
    entry_root: PathBuf,
    package_identity: StablePackageIdentity,
    /// Registration provenance of this boundary's package, or `Ordinary` for the project.
    package_set_membership: PackageSetMembership,
    /// Bootstrap-registered config source for a project boundary.
    ///
    /// WHAT: the one build-only config identity that must stay outside ordinary source and
    ///       file-value admission even when its authored name is a symlink.
    /// WHY: file-value resolution borrows its canonical path from the owning source database, so
    ///      this ID is the single carried identity rather than another path copy.
    config_source_id: Option<SourceId>,
    module_identities: ModuleIdentityTable,
    sources: Vec<SourceRecord>,
    owned_source_indices: Vec<Vec<SourceRecordIndex>>,
    unrooted_source_indices: Vec<SourceRecordIndex>,
    canonical_path_to_source_index: FxHashMap<PathBuf, SourceRecordIndex>,
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
    /// Registration provenance that decides this boundary's damaged-package-set lane.
    package_set_membership: PackageSetMembership,
    /// Bootstrap-registered config source identity for a project boundary.
    ///
    /// WHAT: the one build-only config identity that must stay outside ordinary source
    ///       admission even when its authored name is a symlink to an ordinary-looking sibling.
    /// WHY: bootstrap owns config identity before discovery runs, so carrying the existing source
    ///      identity reuses it instead of re-resolving or inventing a second path mechanism.
    config_source: Option<ProjectConfigSource<'a>>,
    skip_policy: SourceTreeSkipPolicy,
    kind: SourceTreeBoundaryKind<'a>,
}

/// Boundary-specific policy for the shared source-tree traversal.
#[derive(Clone, Copy)]
enum SourceTreeBoundaryKind<'a> {
    Project {
        source_packages: &'a SourcePackageRegistry,
    },
    Package {
        package_prefix: &'a str,
    },
}

impl SourceTreeIndex {
    /// Build the index with one deterministic traversal of the configured entry root.
    ///
    /// The traversal also owns entry-root sibling `.moth` file/folder dependency-name collisions and
    /// entry-root folder/source-backed package-prefix collisions, using the same sorted directory
    /// entries it already reads. Skipped directories neither contribute collision facts nor get
    /// recursively scanned.
    ///
    /// Project and package boundaries share one traversal implementation through
    /// [`SourceTreeIndex::discover_for_boundary`]; this entry point supplies the project
    /// boundary descriptor and delegates to it. Package-boundary sibling collisions and root
    /// discovery are owned by the same traversal via [`SourceTreeIndex::discover_package`].
    ///
    /// Each source directory may contain one `@mod.moth`, one `@package.moth` and one
    /// `@entry.moth` root. Aliased or multiply-claimed roots are rejected through
    /// the existing structured config diagnostic lane. Every other `@*.moth`, `+*.moth` and
    /// `#*.moth` marker is rejected with a structured diagnostic that names the invalid filename
    /// and the three accepted root files. Entry roots carry the `Entry` role used by the project
    /// module graph to select entries; the graph, not this index, owns that selection. A
    /// project-root support root is an
    /// ordinary structural module when it lies inside the selected directory; it is never an entry
    /// candidate.
    ///
    /// The same traversal inventories every compiler-recognized source candidate (`.moth`,
    /// `.mtf` and `.md`) and every explicit
    /// provider-owned file whose extension is registered with the external provider registry but is
    /// not a compiler `SourceFileKind`. Unknown extensions never enter owned source sets. After
    /// deterministic `ModuleIdentityTable` construction, each recognized candidate is classified
    /// under its nearest containing normal or support root into the central [`SourceRecord`]
    /// table. Recognized candidates with no enclosing
    /// module root remain explicit unrooted [`SourceRecord`]s rather than being silently
    /// discarded. Sorted rows are handed to the boundary's compiler-facing registration index.
    ///
    /// WHY the context carries a config source: bootstrap registers the physical config source
    /// before discovery. Passing that source lets the traversal exclude its canonical target from
    /// ordinary-source admission without copying the path into build settings.
    pub(super) fn discover(
        entry_root: PathBuf,
        project_context: SourceTreeProjectContext<'_>,
        config: &Config,
        source_packages: &SourcePackageRegistry,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, PreparedFailure> {
        let SourceTreeProjectContext {
            validated_output_settings,
            config_source,
        } = project_context;
        let boundary = SourceTreeBoundary {
            entry_root: entry_root.clone(),
            package_identity: StablePackageIdentity::project_local(&config.project_name),
            package_set_membership: PackageSetMembership::Ordinary,
            config_source,
            skip_policy: SourceTreeSkipPolicy::from_validated_output_settings(
                &entry_root,
                validated_output_settings,
            ),
            kind: SourceTreeBoundaryKind::Project { source_packages },
        };
        Self::discover_for_boundary(
            boundary,
            source_file_kinds,
            external_import_providers,
            string_table,
        )
    }

    /// Build one source-package boundary index from its already-canonical root directory.
    /// WHAT: traverses one source-backed package root with its own stable package identity,
    /// source-row and module ownership tables. The package index becomes the filesystem owner for
    /// the package root's direct-child root discovery and for sibling `.moth` file/folder
    /// collisions throughout the package tree, using the same traversal implementation as the
    /// project index.
    /// WHY: each project or package boundary owns a separate `SourceTreeIndex`; compiler
    /// `SourceId`s are assigned later by that boundary's `SourceDatabase`. Reusing the project
    /// traversal avoids a parallel package traversal while preserving package-specific
    /// missing-root and multiple-root diagnostics at the package root directory.
    pub(crate) fn discover_package(
        canonical_root: PathBuf,
        package_identity: StablePackageIdentity,
        package_prefix: &str,
        package_set_membership: PackageSetMembership,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, PreparedFailure> {
        let boundary = SourceTreeBoundary {
            entry_root: canonical_root,
            package_identity,
            package_set_membership,
            config_source: None,
            skip_policy: SourceTreeSkipPolicy::default(),
            kind: SourceTreeBoundaryKind::Package { package_prefix },
        };
        Self::discover_for_boundary(
            boundary,
            source_file_kinds,
            external_import_providers,
            string_table,
        )
    }

    /// Run the shared boundary-parameterized source-tree traversal.
    /// This is the single filesystem inventory implementation. The boundary descriptor supplies
    /// the stable package identity, skip policy and entry-root prefix
    /// collision inputs, and selects package-specific root-directory classification. Every other
    /// step — per-directory collision checks, candidate inventory and nearest-module ownership —
    /// is shared between project and package boundaries. Compiler identity assignment happens in
    /// the boundary's `SourceDatabase`.
    fn discover_for_boundary(
        boundary: SourceTreeBoundary,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, PreparedFailure> {
        let SourceTreeBoundary {
            entry_root,
            package_identity: boundary_package,
            package_set_membership,
            config_source,
            skip_policy,
            kind,
        } = boundary;
        let config_canonical_path = config_source.and_then(|source| source.canonical_path());
        let config_source_id = config_source.map(|source| source.source_id);
        // One stable package identity shared by every structural module in this boundary:
        // normal roots, support roots and their role-neutral helper origin. It is derived from
        // the configured project name (project boundary) or the registry's `PackageOrigin` plus
        // package prefix (package boundary), never from an absolute path.
        let mut stats = SourceTreeDiscoveryStats::default();
        let mut queue = VecDeque::from([entry_root.clone()]);
        let mut visited_directories = BTreeSet::new();
        let mut records = Vec::new();
        let mut root_claims = Vec::new();
        let mut root_claim_by_canonical = FxHashMap::<PathBuf, usize>::default();
        let mut recognized_candidates: Vec<DiscoveredSourceCandidate> = Vec::new();

        while let Some(directory) = queue.pop_front() {
            // Symlink aliases are one physical subtree; canonicalize before indexing so aliases
            // cannot duplicate source identities, and do not traverse aliases outside this
            // boundary.
            let directory = fs::canonicalize(&directory)
                .map_err(|error| Self::directory_read_error(&directory, error))?;
            if !directory.starts_with(&entry_root)
                || !visited_directories.insert(directory.clone())
                || skip_policy.canonical_ancestor_is_skipped(&directory, &entry_root)
            {
                continue;
            }
            stats.dirs_visited += 1;

            let entries = Self::sorted_directory_entries(&directory)?;

            let mut subdirectories = Vec::new();
            let mut source_files_by_stem: BTreeMap<String, String> = BTreeMap::new();
            let mut dependency_folder_names: BTreeSet<String> = BTreeSet::new();
            let mut directory_roots: Vec<DiscoveredDirectoryRoot> = Vec::new();

            for entry in entries {
                let path = entry.path();
                // WHY: a non-symlink entry's listed type answers the kind checks, saving a `stat`.
                // Symlinks, and entries whose type cannot be read, keep the `Path` queries, which
                // follow the link to its target.
                let listed_file_type = entry.file_type().ok();
                let listed_type = listed_file_type.filter(|file_type| !file_type.is_symlink());
                let is_dir = match listed_type {
                    Some(listed) => listed.is_dir(),
                    None => path.is_dir(),
                };

                if is_dir {
                    // Root markers are reserved source filenames. A directory or directory
                    // symlink reachable under one of those names is a malformed authored root,
                    // not a traversable module directory. An invalid marker name stays a closed
                    // vocabulary error even when it is not a regular file.
                    if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                        if file_name_is_invalid_module_root_marker(file_name) {
                            return Err(invalid_module_root_file_name_error(
                                &directory,
                                file_name,
                                string_table,
                            ));
                        }
                        if file_name_is_module_root_file(file_name) {
                            return Err(reject_non_regular_module_root(
                                kind,
                                package_set_membership,
                                file_name,
                                &directory,
                                &path,
                                string_table,
                            ));
                        }
                    }

                    // A directory alias must be classified by its physical destination before it
                    // becomes a namespace claim. Otherwise an alias into a hidden, skipped or
                    // external tree can cause false file/folder or package-prefix collisions even
                    // when traversal later drops it. Normal directories need no second probe:
                    // their parent is already canonical.
                    let resolved_directory = if listed_type.is_some() {
                        None
                    } else {
                        Some(canonicalize_source_path(&path)?)
                    };
                    let eligible = match resolved_directory.as_deref() {
                        Some(resolved) => {
                            skip_policy.resolved_directory_is_eligible(resolved, &entry_root)
                        }
                        None => skip_policy.authored_directory_is_eligible(&path, &entry_root),
                    };

                    if !eligible {
                        stats.dirs_skipped += 1;
                    } else {
                        let folder_name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| {
                                non_utf8_filesystem_name_error(&path, "source tree folder name")
                            })?;
                        dependency_folder_names.insert(folder_name.to_owned());
                        subdirectories.push(path);
                    }
                    continue;
                }

                let is_file = match listed_type {
                    Some(listed) => listed.is_file(),
                    None => path.is_file(),
                };
                if !is_file {
                    // Filesystem `is_file` and `is_dir` cannot classify sockets, pipes and other
                    // non-regular entries. A reserved root-marker name in any existing form is an
                    // authored root, so canonicalize it and reject it instead of silently dropping
                    // the required root. A dangling marker symlink still fails canonicalization.
                    // An invalid marker spelling is a closed-vocabulary error without resolving
                    // the link, so a dangling invalid-name symlink is still visible to the author.
                    if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                        if file_name_is_invalid_module_root_marker(file_name) {
                            return Err(invalid_module_root_file_name_error(
                                &directory,
                                file_name,
                                string_table,
                            ));
                        }
                        if file_name_is_module_root_file(file_name) {
                            return Err(reject_non_regular_module_root(
                                kind,
                                package_set_membership,
                                file_name,
                                &directory,
                                &path,
                                string_table,
                            ));
                        }
                    }
                    continue;
                }
                stats.files_seen += 1;

                let file_name =
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| {
                            non_utf8_filesystem_name_error(&path, "source tree file name")
                        })?;

                // `@config.moth` is a reserved build-only configuration source, not a module
                // helper. Only the selected root's file participates in bootstrap; every nested
                // occurrence is inert to the enclosing build and must not enter module ownership
                // or dependency names.
                if file_name_is_config_file(file_name) {
                    continue;
                }

                // A `@config.moth` symlink may resolve through another authored name such as
                // `project.moth` or a `project.mtf` alias. That visible alias is skipped so the
                // build-only config source keeps its identity instead of claiming the reserved
                // `@project` namespace; a genuine project source still reaches the rejection.
                let claims_project_globals =
                    source_stem_from_file_name(file_name).is_some_and(claims_project_globals_name);
                if let SourceTreeBoundaryKind::Project { .. } = kind
                    && directory == entry_root
                    && claims_project_globals
                {
                    // An ordinary entry compares its listed path; a symlink resolves once here.
                    // Either the alias is the build-only config source and leaves admission, or
                    // it is a genuine project claim and is rejected below.
                    let config_target = config_canonical_path.is_some_and(|config_path| {
                        config_path == path.as_path()
                            || (listed_type.is_none()
                                && canonicalize_source_path(&path)
                                    .ok()
                                    .is_some_and(|canonical| canonical == config_path))
                    });
                    if config_target {
                        continue;
                    }
                    let diagnostic = project_structure_diagnostic(
                        InvalidConfigReason::ProjectGlobalsNameReserved,
                    );
                    let table = string_table.take_for_handoff();
                    return Err(PreparedFailure::Diagnosed(
                        PreparedDiagnosticBatch::capture_from_bag(
                            DiagnosticBag::from(diagnostic),
                            table,
                            None,
                            None,
                        ),
                    ));
                }

                // The authored root vocabulary is closed: only `@mod.moth`, `@package.moth` and
                // `@entry.moth` are roots. Every other `@*.moth`, `+*.moth` and `#*.moth` marker
                // is an authored structure error, never an ordinary helper or a no-op.
                if file_name_is_invalid_module_root_marker(file_name) {
                    return Err(invalid_module_root_file_name_error(
                        &directory,
                        file_name,
                        string_table,
                    ));
                }

                let is_module_root = file_name_is_module_root_file(file_name);
                let source_kind = recognized_source_kind(file_name);
                let source_supported = source_kind.is_some_and(|kind| {
                    source_file_kinds
                        .kind_for_extension(kind.extension())
                        .is_some()
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

                // WHY: `directory` is canonical, so a listed non-symlink file's path needs no
                // resolving syscall: the joined path is already the canonical spelling.
                let canonical_path = if listed_type.is_some() {
                    path.clone()
                } else {
                    canonicalize_source_path(&path)?
                };

                // A file symlink can point outside the selected boundary or into an excluded
                // subtree even when its visible spelling looks eligible. Directory ancestry
                // decides exclusion; an underscore-prefixed file name remains valid source.
                // Build-only config stays outside module ownership even through a file alias:
                // either the target is named `@config.moth`, or it is the canonical target
                // bootstrap already registered for a symlinked config source. An excluded root
                // target is a structural failure rather than a missing root; other excluded
                // targets stay silent.
                let canonical_parent = canonical_path.parent();
                if !canonical_path.starts_with(&entry_root)
                    || canonical_path
                        .file_name()
                        .is_some_and(|name| name == CONFIG_FILE_NAME)
                    || config_canonical_path
                        .is_some_and(|config_path| config_path == canonical_path.as_path())
                    || canonical_parent.is_some_and(|parent| {
                        skip_policy.canonical_ancestor_is_skipped(parent, &entry_root)
                    })
                {
                    if is_module_root {
                        return Err(module_root_outside_claim_error(
                            &directory,
                            &path,
                            &canonical_path,
                            string_table,
                        ));
                    }
                    continue;
                }

                // Authored module roots never supply dependency names: names come from the
                // containing module directory or registered package identity. Registering a
                // root's filename stem here would invent a collision with an unrelated asset
                // directory such as `@mod`.
                if !is_module_root && let Some(stem) = source_stem_from_file_name(file_name) {
                    source_files_by_stem.insert(stem.to_owned(), file_name.to_owned());
                }

                if let Some(kind) = source_kind {
                    let logical_candidate_path =
                        entry_root_relative_logical_path(&path, &entry_root)?;
                    recognized_candidates.push(DiscoveredSourceCandidate {
                        canonical_path: canonical_path.clone(),
                        classification: SourceClassification::CompilerSemantic(kind),
                        supported: source_supported,
                        logical_candidate_path,
                        root_role: is_module_root
                            .then(|| module_root_role_for_file_name(file_name))
                            .flatten(),
                    });
                } else if let Some(extension) = provider_extension {
                    // Provider-owned files are never module roots. They enter the inventory as
                    // explicit provider-owned candidates
                    // classified under their nearest enclosing module root, exactly like
                    // compiler semantic candidates.
                    let logical_candidate_path =
                        entry_root_relative_logical_path(&path, &entry_root)?;
                    recognized_candidates.push(DiscoveredSourceCandidate {
                        canonical_path: canonical_path.clone(),
                        classification: SourceClassification::ProviderOwned(extension),
                        supported: true,
                        logical_candidate_path,
                        root_role: None,
                    });
                }

                if !is_module_root {
                    continue;
                }

                let role = module_root_role_for_file_name(file_name)
                    .expect("a module root file name has a role after is_module_root_file");

                if role == ModuleRootRole::Normal {
                    stats.normal_root_files_seen += 1;
                } else if role == ModuleRootRole::Support {
                    stats.support_root_files_seen += 1;
                } else if role == ModuleRootRole::Entry {
                    stats.entry_root_files_seen += 1;
                }

                directory_roots.push(DiscoveredDirectoryRoot {
                    root_file: canonical_path,
                    visible_root_file: path.clone(),
                    role,
                });
            }

            // Check sibling source file/folder dependency-name collisions from the same sorted
            // entries. Skipped folders are absent from dependency_folder_names so they cannot
            // create false collisions.
            for (stem, file_name) in &source_files_by_stem {
                if let Some(folder_name) = dependency_folder_names
                    .iter()
                    .find(|folder_name| folder_name.eq_ignore_ascii_case(stem))
                {
                    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
                    let reason = InvalidConfigReason::SourceFileFolderCollision {
                        file_name: diagnostic_strings.intern(file_name),
                        folder_name: diagnostic_strings.intern(folder_name),
                        directory: diagnostic_strings.intern_path(&directory),
                    };
                    let diagnostic = project_structure_diagnostic(reason);
                    let table = string_table.take_for_handoff();
                    return Err(PreparedFailure::Diagnosed(
                        PreparedDiagnosticBatch::capture_from_bag(
                            DiagnosticBag::from(diagnostic),
                            table,
                            None,
                            None,
                        ),
                    ));
                }
            }
            if let SourceTreeBoundaryKind::Project { .. } = kind
                && directory == entry_root
                && dependency_folder_names
                    .iter()
                    .any(|name| claims_project_globals_name(name))
            {
                let diagnostic =
                    project_structure_diagnostic(InvalidConfigReason::ProjectGlobalsNameReserved);
                let table = string_table.take_for_handoff();
                return Err(PreparedFailure::Diagnosed(
                    PreparedDiagnosticBatch::capture_from_bag(
                        DiagnosticBag::from(diagnostic),
                        table,
                        None,
                        None,
                    ),
                ));
            }

            // A project folder whose entry-root-relative path equals a registered source package
            // prefix would claim that package's identity, whichever module owns the folder.
            // Ancestor folders alone may coexist with nested prefixes. Components compare
            // ASCII-case-insensitively, as package identities do, so the result doesn't depend on
            // the host filesystem. The canonical directory path keeps this independent of
            // traversal order. Package boundaries carry no prefix-collision packages, so this
            // check is skipped for them.
            if let SourceTreeBoundaryKind::Project {
                source_packages, ..
            } = kind
            {
                // Traversal skips canonical directories outside the entry root before this
                // point, so every visited project directory has an entry-relative path.
                let relative_directory = directory
                    .strip_prefix(&entry_root)
                    .expect("visited project directories stay inside the entry root");
                if let Some((package, folder_name)) = source_packages.iter().find_map(|package| {
                    let (parent, name) = split_package_prefix(&package.package_prefix);
                    if !package_parent_matches(parent, relative_directory) {
                        return None;
                    }
                    dependency_folder_names
                        .iter()
                        .find(|folder_name| folder_name_claims_package_component(folder_name, name))
                        .map(|folder_name| (package, folder_name))
                }) {
                    let colliding_folder = directory.join(folder_name);
                    return Err(entry_root_package_prefix_collision_error(
                        &package.package_prefix,
                        &colliding_folder,
                        string_table,
                    ));
                }
            }

            // Package boundaries require a root at the package root directory and report
            // package-specific missing/multiple-root diagnostics there. Every other directory
            // (and all project-boundary directories) uses the shared per-directory classifier,
            // which accepts at most one root per fixed role (normal, support or entry).
            let directory_root = match kind {
                SourceTreeBoundaryKind::Package { package_prefix } if directory == entry_root => {
                    classify_package_root_directory(
                        &directory,
                        &mut directory_roots,
                        package_prefix,
                        package_set_membership,
                        string_table,
                    )?
                }
                SourceTreeBoundaryKind::Project { .. } | SourceTreeBoundaryKind::Package { .. } => {
                    classify_directory_root(&mut directory_roots, string_table)?
                }
            };
            let ClassifiedDirectoryRoots { roots } = directory_root;
            if !roots.is_empty() {
                for (_, root) in roots.present() {
                    register_module_root_claim(
                        &mut root_claims,
                        &mut root_claim_by_canonical,
                        &directory,
                        root,
                        string_table,
                    )?;
                }

                // WHY: `directory` is canonical (canonicalized when popped), so it is the record's
                // identity; re-canonicalizing would repeat the same syscall for the same answer.
                let logical_module_path = logical_module_path_from(&directory, &entry_root)?;

                stats.module_roots_found += 1;
                let (normal_root, support_root, entry_root) = roots.into_parts();
                records.push(DiscoveredModuleRecord {
                    directory,
                    roots: ModuleRootFiles::new(
                        normal_root.map(|root| root.root_file),
                        support_root.map(|root| root.root_file),
                        entry_root.map(|root| root.root_file),
                    ),
                    logical_module_path,
                });
            }

            subdirectories.sort();
            queue.extend(subdirectories);
        }

        validate_module_root_claims(&root_claims, string_table)?;
        record_discovery_metrics(&stats);

        let records = records
            .into_iter()
            .map(|record| {
                ModuleIdentityRecord::new(
                    record.directory,
                    record.roots,
                    record.logical_module_path,
                    &boundary_package,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let module_identities = ModuleIdentityTable::from_records(records);
        let module_count = module_identities.module_ids().count();

        let classified = classify_owned_sources(&module_identities, recognized_candidates)?;
        validate_unique_source_logical_identities(&classified)?;

        let SourceInventory {
            sources,
            owned_source_indices,
            unrooted_source_indices,
            canonical_path_to_source_index,
        } = build_source_inventory(classified, module_count)?;

        Ok(Self {
            entry_root,
            package_identity: boundary_package,
            package_set_membership,
            config_source_id,
            module_identities,
            sources,
            owned_source_indices,
            unrooted_source_indices,
            canonical_path_to_source_index,
            stats,
        })
    }

    /// Collect local namespace facts without loading sources.
    ///
    /// WHAT: immediate namespace keys, one importable source claiming a reserved package
    /// namespace root, and the first local directory claiming a registered source-package prefix
    /// when this synthetic unit has no module topology.
    /// WHY: synthetic discovery needs these names even when their sources are not reachable, so a
    /// registered package never takes precedence over a local source or directory segment. A
    /// selection that owns module roots validates reserved-root ownership and registered
    /// source-package prefix collisions from its Stage 0 index instead; only a selection without
    /// them needs the bounded path walk below.
    ///
    /// `registered_package_prefixes` are the `@`-stripped registered source-package paths that
    /// directory compilation checks. Binding packages share import names but own no project-local
    /// filesystem path, so they contribute no exact-prefix directory collision. Only each source
    /// prefix's corresponding relative path below `source_root` is probed, so arbitrary descendant
    /// directories never enter the collision inventory.
    pub(super) fn single_file_namespace_inventory(
        source_root: &Path,
        has_module_topology: bool,
        registered_package_prefixes: &[&str],
    ) -> Result<SingleFileNamespaceInventory, PreparedFailure> {
        let skip_policy = SourceTreeSkipPolicy::default();
        let mut keys = BTreeSet::new();
        let mut reserved_root_claim = None;
        let mut root_directories = Vec::new();
        // One classified pass supplies every fact: files own helper keys and claim spellings, while
        // directories own namespace keys, reserved-root claims and the roots registered-prefix
        // probing descends from.
        for entry in Self::sorted_directory_entries(source_root)? {
            let Some(entry) = eligible_single_file_entry(&skip_policy, source_root, &entry)? else {
                continue;
            };
            let Some(directory_identity) = entry.directory_identity else {
                if let Some(stem) = ordinary_helper_source_stem(&entry.listed_name) {
                    // Ordinary supported sources may use a `+` stem (`+notes.mtf` contributes
                    // `+notes`); canonical root files and invalid markers keep no helper
                    // identity.
                    if !has_module_topology
                        && reserved_root_claim.is_none()
                        && reserved_package_namespace_claim_root(stem).is_some()
                    {
                        reserved_root_claim = Some((stem.to_owned(), entry.listed_path));
                    }
                    keys.insert(stem.to_owned());
                }
                continue;
            };

            let is_reserved_namespace_root = is_reserved_package_namespace_root(&entry.listed_name);
            // A reserved directory is never itself a package-collision key. An importable claim
            // below it aborts compilation, and a reserved directory without importable sources
            // must leave `@core/...` and `@builder/...` imports to their registered packages.
            if is_reserved_namespace_root && !has_module_topology && reserved_root_claim.is_none() {
                // The selected root is the first visited destination so a symlink alias back to
                // it cannot re-enter the same subtree.
                let mut visited_directories =
                    BTreeSet::from([canonicalize_source_path(source_root)?]);
                reserved_root_claim = reserved_root_directory_claim(
                    &entry.listed_path,
                    &entry.listed_name,
                    &skip_policy,
                    source_root,
                    &mut visited_directories,
                )?;
            }
            // The registered-prefix collision walk starts from these already-classified entries,
            // so it never repeats the root listing this scan owns.
            if !has_module_topology {
                root_directories.push(EligibleDirectoryEntry {
                    listed_path: entry.listed_path,
                    identity: directory_identity,
                });
            }
            if is_reserved_namespace_root {
                continue;
            }
            keys.insert(entry.listed_name);
        }

        let registered_prefix_collision = if has_module_topology {
            None
        } else {
            registered_package_prefix_collision(
                source_root,
                &root_directories,
                registered_package_prefixes,
                &skip_policy,
            )?
        };

        Ok(SingleFileNamespaceInventory {
            keys,
            reserved_root_claim,
            registered_prefix_collision,
        })
    }

    /// Build the bounded Stage 0 index for a directly selected module-root entry file.
    ///
    /// WHAT: returns the complete index for the selected file's directory tree, or `None` for an
    /// ordinary selected source file that owns no bounded module-root inventory.
    /// WHY: selected-entry compilation already scans this bounded tree for its root table.
    /// Returning the index lets the caller validate importable reserved-root claims from those
    /// same Stage 0 facts instead of scanning the tree again.
    pub(super) fn bounded_single_file_index(
        entry_file: &Path,
        config: &Config,
        source_packages: &SourcePackageRegistry,
        source_file_kinds: &SourceFileKindRegistry,
        external_import_providers: &ExternalImportProviderRegistry,
        string_table: &mut StringTable,
    ) -> Result<Option<SourceTreeIndex>, PreparedFailure> {
        let file_name = entry_file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| non_utf8_filesystem_name_error(entry_file, "single-file entry name"))?;
        // Only the three fixed root spellings own a bounded Stage 0 inventory.
        if !file_name_is_module_root_file(file_name) {
            return Ok(None);
        }

        let Some(root_directory) = entry_file.parent() else {
            return Ok(None);
        };
        let canonical_root = fs::canonicalize(root_directory).map_err(|error| {
            InfrastructureFailure::filesystem(
                CompilerStage::ProjectBuildAssembly,
                root_directory,
                format!("Failed to canonicalize single-file source root: {error}"),
            )
        })?;

        Self::discover(
            canonical_root,
            SourceTreeProjectContext {
                validated_output_settings: None,
                config_source: None,
            },
            config,
            source_packages,
            source_file_kinds,
            external_import_providers,
            string_table,
        )
        .map(Some)
    }

    pub(crate) fn entry_root(&self) -> &Path {
        &self.entry_root
    }

    /// Registration provenance that decides this boundary's damaged-package-set lane.
    pub(crate) fn package_set_membership(&self) -> PackageSetMembership {
        self.package_set_membership
    }

    /// The canonical physical path of this boundary's bootstrap config source.
    ///
    /// Consumers compare it with a resolved target so the build-only config keeps one physical
    /// identity and never enters ordinary source admission. The index stores only the source ID,
    /// so the boundary's `SourceDatabase` remains the path owner.
    pub(crate) fn config_source_path<'a>(
        &self,
        source_files: &'a SourceDatabase,
    ) -> Option<&'a Path> {
        source_files
            .get(self.config_source_id?)?
            .canonical_os_path
            .as_deref()
    }

    /// The stable package identity shared by every module in this boundary.
    pub(crate) fn stable_package_identity(&self) -> &StablePackageIdentity {
        &self.package_identity
    }

    /// The canonical `@package.moth` support facade of the module rooted at this package
    /// boundary's entry root.
    ///
    /// WHAT: the registered package's selected public-surface root file. Stage 0 package-root
    /// classification requires a directly contained support facade, so a co-located `@mod.moth`
    /// is only an internal normal-position scope and never the package interface. Returns `None`
    /// when no module is rooted at the entry root.
    /// WHY: the package-boundary index owner derives the resolver's narrow package-root view
    /// from this indexed fact instead of a separate `read_dir` or root-file discovery pass.
    pub(crate) fn package_public_root_file(&self) -> Option<&Path> {
        let module_id = self
            .module_identities
            .module_id_for_directory(&self.entry_root)?;
        let record = self.module_identities.record(module_id);
        record.support_root_file()
    }

    /// The role-keyed root lookup derived from the durable identity table.
    ///
    /// Focused Stage 0 tests need the same lookup semantic compilation consumes. Production
    /// callers build it once from [`Self::module_identities`] instead of storing a second copy.
    #[cfg(test)]
    pub(crate) fn module_roots(&self) -> ModuleRootTable {
        self.module_identities.derive_compilation_root_table()
    }

    /// The Stage 0 durable module identity and topology table.
    ///
    /// Consumed by the project module graph (built in `project_roots`) and by focused tests. The
    /// directory frontend derives its complete role-keyed root lookup from this identity table
    /// before semantic compilation.
    pub(crate) fn module_identities(&self) -> &ModuleIdentityTable {
        &self.module_identities
    }

    /// The central source-record table, addressed by zero-based [`SourceRecordIndex`] rows.
    ///
    /// The table owns Stage 0 classification, ownership and canonical IO paths. The compiler-facing
    /// registration index borrows its sorted canonical rows and assigns `SourceId`s separately.
    #[cfg(test)]
    pub(crate) fn sources(&self) -> &[SourceRecord] {
        &self.sources
    }

    /// Build the compact compiler-facing registration rows in this index's deterministic order.
    ///
    /// Rows are already sorted by [`SourceLogicalIdentity`]. The boundary source database consumes
    /// this order unchanged when assigning compiler identities. Each row carries the authored
    /// kind classified from the lexical file name, not a kind re-derived from the canonical
    /// extension.
    pub(crate) fn source_registration_index(&self) -> SourceRegistrationIndex<'_> {
        SourceRegistrationIndex::from_rows(self.sources.iter().map(|record| {
            (
                record.canonical_path(),
                source_kind_from_classification(record.classification()),
            )
        }))
    }

    /// One source record addressed by its zero-based tree row.
    pub(crate) fn source(&self, source_index: SourceRecordIndex) -> &SourceRecord {
        &self.sources[source_index.index()]
    }

    /// The source-row handles owned by one canonical module, indexed by `ModuleId`.
    ///
    /// Every recognized compiler source or provider-owned file whose nearest containing normal or
    /// support root is this module appears in portable module-relative source order.
    pub(crate) fn owned_source_indices(&self, module_id: ModuleId) -> &[SourceRecordIndex] {
        &self.owned_source_indices[module_id.index()]
    }

    /// The source-row handles for recognized or provider-owned files with no enclosing module root.
    #[cfg(test)]
    pub(crate) fn unrooted_source_indices(&self) -> &[SourceRecordIndex] {
        &self.unrooted_source_indices
    }

    /// Resolve one source-row handle by its canonical physical path.
    ///
    /// The directory namespace uses this to select the consuming file's owning boundary-local
    /// module. Returns `None` when no indexed record carries that canonical path.
    pub(crate) fn source_index_for_canonical_path(
        &self,
        canonical_path: &Path,
    ) -> Option<SourceRecordIndex> {
        self.canonical_path_to_source_index
            .get(canonical_path)
            .copied()
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> &SourceTreeDiscoveryStats {
        &self.stats
    }

    fn sorted_directory_entries(
        directory: &Path,
    ) -> Result<Vec<fs::DirEntry>, InfrastructureFailure> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| Self::directory_read_error(directory, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                InfrastructureFailure::filesystem(
                    CompilerStage::ProjectBuildAssembly,
                    directory,
                    format!("Failed to read directory entry while indexing source tree: {error}"),
                )
            })?;
        entries.sort_by_key(|entry| entry.path());
        Ok(entries)
    }

    fn directory_read_error(directory: &Path, error: std::io::Error) -> InfrastructureFailure {
        InfrastructureFailure::filesystem(
            CompilerStage::ProjectBuildAssembly,
            directory,
            format!("Failed to read directory while indexing source tree: {error}"),
        )
    }
}

/// Resolve one source path to its canonical identity.
///
/// Every caller treats a canonicalization failure as an infrastructure failure tied to the
/// authored path, so the source-tree traversal keeps one error shape for symlinks and missing
/// sources.
fn canonicalize_source_path(path: &Path) -> Result<PathBuf, InfrastructureFailure> {
    fs::canonicalize(path).map_err(|error| {
        InfrastructureFailure::filesystem(
            CompilerStage::ProjectBuildAssembly,
            path,
            format!("Failed to canonicalize source path: {error}"),
        )
    })
}

/// Classify one rooted directory's fixed role-keyed root inventory.
///
/// The closed filename vocabulary yields at most one root per role. Any two present roots that
/// resolve to one physical file are rejected because each role needs its own authored root
/// scope. A directory with no roots is not a structural module.
fn classify_directory_root(
    directory_roots: &mut Vec<DiscoveredDirectoryRoot>,
    string_table: &mut StringTable,
) -> Result<ClassifiedDirectoryRoots, PreparedFailure> {
    directory_roots.sort_by(|left, right| left.root_file.cmp(&right.root_file));

    let mut normal = Vec::new();
    let mut support = Vec::new();
    let mut entry = Vec::new();
    for root in directory_roots.drain(..) {
        match root.role {
            ModuleRootRole::Normal => normal.push(root),
            ModuleRootRole::Support => support.push(root),
            ModuleRootRole::Entry => entry.push(root),
            ModuleRootRole::Shared | ModuleRootRole::SyntheticProjectContext => {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "filesystem module roots may only be normal, support or entry roots"
                )
            }
        }
    }

    // Every present root must resolve to a distinct physical file so each role owns a separate
    // authored scope. A visible symlink's name decided its role above; this rejects two roles
    // that alias the same canonical target.
    let present_roots = normal
        .iter()
        .chain(support.iter())
        .chain(entry.iter())
        .collect::<Vec<_>>();
    for (index, left) in present_roots.iter().enumerate() {
        for right in present_roots.iter().skip(index + 1) {
            if left.root_file == right.root_file {
                return Err(aliased_root_files_error(
                    left.visible_root_file.clone(),
                    right.visible_root_file.clone(),
                    left.root_file.clone(),
                    string_table,
                ));
            }
        }
    }

    Ok(ClassifiedDirectoryRoots {
        roots: ModuleRootFiles::new(normal.pop(), support.pop(), entry.pop()),
    })
}

/// Record one canonical root-file claim and reject a second structural module claiming it.
///
/// The check runs during traversal, before any module identity or source identity is built. A
/// visible root marker and its canonical target are one claim; only a second authored root marker
/// resolving to the same file conflicts.
fn register_module_root_claim(
    claims: &mut Vec<DiscoveredModuleRootClaim>,
    claim_by_canonical: &mut FxHashMap<PathBuf, usize>,
    directory: &Path,
    root: &DiscoveredDirectoryRoot,
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    if let Some(previous_index) = claim_by_canonical.get(&root.root_file).copied() {
        let previous = &claims[previous_index];
        return Err(aliased_module_root_files_error(
            &previous.visible_root_file,
            &root.visible_root_file,
            &root.root_file,
            string_table,
        ));
    }

    claim_by_canonical.insert(root.root_file.clone(), claims.len());
    claims.push(DiscoveredModuleRootClaim {
        directory: directory.to_path_buf(),
        root_file: root.root_file.clone(),
        visible_root_file: root.visible_root_file.clone(),
    });
    Ok(())
}

/// Reject a root file that resolves outside the structural module it roots.
///
/// A root symlink may point at an ordinary source file in the same module, including an unrooted
/// subdirectory. It may not point into another structural module or an unrooted part of the entry
/// root, because Stage 0 would then assign the file to a different owner and the claiming module
/// would have no root source.
fn validate_module_root_claims(
    claims: &[DiscoveredModuleRootClaim],
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    let claimed_directories: FxHashSet<&Path> = claims
        .iter()
        .map(|claim| claim.directory.as_path())
        .collect();

    for claim in claims {
        let Some(canonical_parent) = claim.root_file.parent() else {
            return Err(module_root_outside_claim_error(
                &claim.directory,
                &claim.visible_root_file,
                &claim.root_file,
                string_table,
            ));
        };
        let owner = canonical_parent
            .ancestors()
            .find(|directory| claimed_directories.contains(*directory));
        if owner != Some(claim.directory.as_path()) {
            return Err(module_root_outside_claim_error(
                &claim.directory,
                &claim.visible_root_file,
                &claim.root_file,
                string_table,
            ));
        }
    }
    Ok(())
}

fn aliased_module_root_files_error(
    first_root: &Path,
    second_root: &Path,
    target: &Path,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let diagnostic = project_structure_diagnostic(InvalidConfigReason::AliasedModuleRootFiles {
        first_root: diagnostic_strings.intern_path(first_root),
        second_root: diagnostic_strings.intern_path(second_root),
        target: diagnostic_strings.intern_path(target),
    });
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        None,
    ))
}

fn module_root_outside_claim_error(
    directory: &Path,
    visible_root_file: &Path,
    target: &Path,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let diagnostic =
        project_structure_diagnostic(InvalidConfigReason::ModuleRootFileOutsideClaimingModule {
            root_file: diagnostic_strings.intern_path(visible_root_file),
            claiming_directory: diagnostic_strings.intern_path(directory),
            target: diagnostic_strings.intern_path(target),
        });
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        None,
    ))
}

/// Reject one reserved module-root name that is not a regular file in this boundary.
///
/// WHAT: resolves the visible marker first. A compiler-shipped package registration whose marker
///       cannot resolve, or resolves to a non-file entry, reports the incomplete installed set
///       with the original cause; every authored registration keeps its existing filesystem
///       failure or structure diagnostic.
/// WHY: registration provenance decides the lane, never the installation path, so a damaged
///       shipped facade does not surface as an author error while a custom package under the
///       installed `packages/` directory stays authored. The original OS error stays visible.
fn reject_non_regular_module_root(
    kind: SourceTreeBoundaryKind<'_>,
    package_set_membership: PackageSetMembership,
    file_name: &str,
    directory: &Path,
    visible_root_file: &Path,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let damaged_package = match kind {
        SourceTreeBoundaryKind::Package { package_prefix } => {
            damaged_package_set_report(package_set_membership)
                .map(|report| (package_prefix, report))
        }
        SourceTreeBoundaryKind::Project { .. } => None,
    };

    let canonical_target = match canonicalize_source_path(visible_root_file) {
        Ok(canonical_target) => canonical_target,
        Err(error) => {
            // Resolution can fail before classification when a symlinked facade target becomes
            // unsearchable, so the original OS error is retained in the shipped message.
            let Some((package_prefix, report)) = damaged_package else {
                return error.into();
            };
            let mut error = error;
            error.message = format!(
                "Source package @{package_prefix} cannot resolve its '{file_name}' module root \
                 from the installed package set: {}. {report}",
                error.message
            )
            .into_boxed_str();
            return error.into();
        }
    };

    let Some((package_prefix, report)) = damaged_package else {
        return module_root_not_regular_file_error(
            directory,
            visible_root_file,
            &canonical_target,
            string_table,
        );
    };
    InfrastructureFailure::filesystem(
        CompilerStage::ProjectBuildAssembly,
        visible_root_file,
        format!(
            "Source package @{package_prefix} has a reserved module-root name that is not a \
             regular file: '{file_name}'. {report}"
        ),
    )
    .into()
}

/// Reject a reserved root-marker name that resolves to a directory or other non-file entry.
///
/// The authored marker stays visible while the canonical target explains where the malformed
/// entry resolves. This is a user configuration failure, not a missing-root or IO fallback.
fn module_root_not_regular_file_error(
    directory: &Path,
    visible_root_file: &Path,
    target: &Path,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let diagnostic =
        project_structure_diagnostic(InvalidConfigReason::ModuleRootFileNotRegularFile {
            root_file: diagnostic_strings.intern_path(visible_root_file),
            directory: diagnostic_strings.intern_path(directory),
            target: diagnostic_strings.intern_path(target),
        });
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        None,
    ))
}

/// Reject one physical source file reached through two structural root roles.
///
/// Every present root role needs its own authored root scope. Canonicalizing two root names to one
/// file would collapse them into one source identity, so the filesystem structure fails before
/// module identity or semantic preparation begins.
fn aliased_root_files_error(
    left_root_file: PathBuf,
    right_root_file: PathBuf,
    target: PathBuf,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    let diagnostic = project_structure_diagnostic(InvalidConfigReason::AliasedModuleRootFiles {
        first_root: diagnostic_strings.intern_path(&left_root_file),
        second_root: diagnostic_strings.intern_path(&right_root_file),
        target: diagnostic_strings.intern_path(&target),
    });
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        None,
    ))
}

/// Classify the roots at one source-package root directory.
///
/// Every registered Moth-source package publishes its external API through a directly contained
/// `@package.moth`. A co-located `@mod.moth` keeps a distinct internal normal-position scope, while
/// `@entry.moth` only contributes a dormant entry and never a package surface. Missing or aliased
/// roots keep package-specific typed diagnostics, except for a registration the compiler ships
/// as part of its package set, where a missing support facade means that installed set is
/// incomplete. Registration provenance decides the lane; a custom root under the installed
/// `packages/` directory or reached through a symlink stays an authored structure diagnostic.
fn classify_package_root_directory(
    directory: &Path,
    directory_roots: &mut Vec<DiscoveredDirectoryRoot>,
    package_prefix: &str,
    package_set_membership: PackageSetMembership,
    string_table: &mut StringTable,
) -> Result<ClassifiedDirectoryRoots, PreparedFailure> {
    // The support facade is the only package-facing surface. A normal root alone is an internal
    // module scope, and an entry-only root publishes no importable API at all.
    let has_support_facade = directory_roots
        .iter()
        .any(|root| root.role == ModuleRootRole::Support);
    if !has_support_facade {
        if let Some(report) = damaged_package_set_report(package_set_membership) {
            return Err(InfrastructureFailure::filesystem(
                CompilerStage::ProjectBuildAssembly,
                directory,
                format!("Source package @{package_prefix} has no '@package.moth' support facade root. {report}"),
            )
            .into());
        }
        let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
        let diagnostic =
            project_structure_diagnostic(InvalidConfigReason::SourcePackageMissingRoot {
                prefix: diagnostic_strings.intern(package_prefix),
                root: diagnostic_strings.intern_path(directory),
            });
        let table = string_table.take_for_handoff();
        return Err(PreparedFailure::Diagnosed(
            PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from(diagnostic),
                table,
                None,
                None,
            ),
        ));
    }

    classify_directory_root(directory_roots, string_table)
}

/// Compute the source-relative logical module path for a canonical root directory.
///
/// `base` is the canonical entry root. A canonical root
/// directory discovered under the entry root always shares that prefix, so a `strip_prefix`
/// failure is a proven internal invariant: it means the entry root was not canonicalized before
/// indexing or the directory escaped the entry-root tree. Rather than silently falling back to an
/// absolute machine-local path (which would make `ModuleId` non-deterministic across machines),
/// surface it as an internal compiler error so the failure is never hidden.
fn logical_module_path_from(
    root_directory: &Path,
    base: &Path,
) -> Result<PathBuf, InfrastructureFailure> {
    root_directory
        .strip_prefix(base)
        .map(PathBuf::from)
        .map_err(|_| {
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Module root directory {root_directory:?} is not under the canonical base \
                 {base:?}; logical module path cannot fall back to an absolute path"
                ),
            )
        })
}

/// Split one registered package prefix into its parent path and final folder name.
///
/// A prefix with no `/` has an empty parent, which matches the entry root itself.
fn split_package_prefix(package_prefix: &str) -> (&str, &str) {
    package_prefix
        .rsplit_once('/')
        .unwrap_or(("", package_prefix))
}

/// Whether one folder name claims one registered package-prefix component.
///
/// Package identities compare components ASCII-case-insensitively on every host, so a folder's
/// filesystem spelling never decides an exact-prefix collision on its own.
fn folder_name_claims_package_component(folder_name: &str, component: &str) -> bool {
    folder_name.eq_ignore_ascii_case(component)
}

/// Whether an entry-root-relative directory is a registered package prefix's parent.
///
/// An empty parent is the entry root. Every parent component must match one directory component,
/// with no extra components left over.
fn package_parent_matches(parent: &str, relative_directory: &Path) -> bool {
    let mut components = relative_directory.components();
    parent
        .split('/')
        .filter(|segment| !segment.is_empty())
        .all(|segment| {
            components
                .next()
                .and_then(|component| component.as_os_str().to_str())
                .is_some_and(|component| folder_name_claims_package_component(component, segment))
        })
        && components.next().is_none()
}

/// Extract the dependency-name stem from a compiler-recognized source file name.
///
/// The caller must have already validated `file_name` as UTF-8 so that extension and stem
/// extraction can never silently skip a non-UTF-8 component.
fn source_stem_from_file_name(file_name: &str) -> Option<&str> {
    let path = Path::new(file_name);
    let extension = path.extension().and_then(|extension| extension.to_str())?;
    SourceFileKind::from_extension(extension)?;
    path.file_stem().and_then(|stem| stem.to_str())
}

/// The extensionless dependency name of one ordinary helper source.
///
/// WHAT: returns the stem for every recognized source kind except module-root spellings and
/// invalid `@`/`+`/`#` `.moth` markers.
/// WHY: the synthetic key scan and the reserved-directory claim walk must agree on which files
/// own importable helper identities; `+notes.mtf` remains an ordinary source named `+notes`.
fn ordinary_helper_source_stem(file_name: &str) -> Option<&str> {
    if file_name.starts_with('@') || file_name_is_invalid_module_root_marker(file_name) {
        return None;
    }
    source_stem_from_file_name(file_name)
}

/// Resolve the compiler-recognized source kind for one validated UTF-8 file name.
///
/// Returns `Some(kind)` for every recognized extension (`moth`, `mtf`, `md`) regardless of
/// whether the project's `SourceFileKindRegistry` supports it. The discovery pass computes
/// `supported` by asking the registry for the extension's kind, so the index can surface
/// recognized-but-unsupported optional files for structured dependency diagnostics without a
/// filesystem probe during dependency resolution.
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
) -> Result<PathBuf, InfrastructureFailure> {
    file_path
        .strip_prefix(module_root_directory)
        .map(PathBuf::from)
        .map_err(|_| {
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Owned source {file_path:?} is not under its nearest module root \
                     {module_root_directory:?}; module-relative source path cannot fall back to \
                     an absolute path"
                ),
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
) -> Result<String, InfrastructureFailure> {
    let relative_path = traversal_path
        .strip_prefix(entry_root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Discovered source candidate {traversal_path:?} is not under the entry root \
                 {entry_root:?}; logical candidate path cannot fall back to an absolute path"
            )
        });
    portable_relative_logical_path_from(&relative_path)
}

/// Classify every recognized candidate discovered during traversal and assign its portable logical
/// identity and explicit ownership, ready for deterministic `SourceId` assignment.
///
/// WHAT: classifies every recognized candidate under its nearest containing normal or support
/// root by walking parent directories through the identity table. A nested module root and all
/// files beneath it transfer to the nested module because the nearest-module walk finds it
/// first. Unrooted internal subdirectories stay owned by their nearest ancestor module. The
/// Recognized candidates with no enclosing module root become explicit deterministic unrooted
/// classified sources.
/// WHY: one authoritative classification is the single input for source ownership. Each
/// classified source carries its portable logical identity and explicit ownership.
/// Source ordering uses the stable identity's explicit ordering.
fn classify_owned_sources(
    module_identities: &ModuleIdentityTable,
    recognized_candidates: Vec<DiscoveredSourceCandidate>,
) -> Result<Vec<ClassifiedSource>, InfrastructureFailure> {
    // Two visible entries can resolve to one physical source (a root symlink and its target, or
    // two aliases). Retain one record and prefer the candidate that carries a discovered root
    // surface role, so a symlinked `@root` does not lose its surface to its plain target spelling.
    let mut candidates =
        Vec::<DiscoveredSourceCandidate>::with_capacity(recognized_candidates.len());
    let mut position_by_canonical = FxHashMap::<PathBuf, usize>::default();
    for candidate in recognized_candidates {
        if let Some(position) = position_by_canonical
            .get(&candidate.canonical_path)
            .copied()
        {
            if candidates[position].root_role.is_none() && candidate.root_role.is_some() {
                candidates[position] = candidate;
            }
            continue;
        }
        position_by_canonical.insert(candidate.canonical_path.clone(), candidates.len());
        candidates.push(candidate);
    }

    let mut classified = Vec::new();

    for candidate in candidates {
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
        let relative_path =
            relative_source_path_from(&candidate.canonical_path, record.root_directory())?;
        let source_origin = candidate
            .root_role
            .and_then(|role| record.surface_origin(role))
            .unwrap_or_else(|| record.shared_origin())
            .clone();
        let stable_identity =
            StableOwnedSourceIdentity::from_relative_source_path(source_origin, &relative_path)?;

        classified.push(ClassifiedSource {
            canonical_path: candidate.canonical_path,
            classification: candidate.classification,
            supported: candidate.supported,
            logical_identity: SourceLogicalIdentity::Owned(stable_identity),
            ownership: SourceOwnership::Owned(module_id),
        });
    }

    Ok(classified)
}

/// Build one classified unrooted source from a traversal candidate that has no enclosing module
/// root.
fn unrooted_classified(candidate: DiscoveredSourceCandidate) -> ClassifiedSource {
    let logical_path = UnrootedSourceLogicalPath::from_portable(candidate.logical_candidate_path);
    ClassifiedSource {
        canonical_path: candidate.canonical_path,
        classification: candidate.classification,
        supported: candidate.supported,
        logical_identity: SourceLogicalIdentity::Unrooted(logical_path),
        ownership: SourceOwnership::Unrooted,
    }
}

/// Reject duplicate logical identities before assigning dense `SourceId`s.
///
/// The duplicate key and both canonical physical paths stay in the invariant error so the
/// diagnostic retains useful identity and filesystem context without a separate path table.
fn validate_unique_source_logical_identities(
    classified: &[ClassifiedSource],
) -> Result<(), InfrastructureFailure> {
    let mut seen: FxHashMap<&SourceLogicalIdentity, &ClassifiedSource> = FxHashMap::default();
    for source in classified {
        let Some(left) = seen.insert(&source.logical_identity, source) else {
            continue;
        };
        let right = source;
        let message = format!(
            "Source tree index classified duplicate logical identity {:?} for physical sources {} \
             and {}; source identity must be unique before SourceId assignment",
            left.logical_identity,
            portable_path_text(&left.canonical_path),
            portable_path_text(&right.canonical_path),
        );
        return Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            message,
        ));
    }
    Ok(())
}

/// Sort classified sources and build the central Stage 0 source-row table plus ownership
/// projections.
///
/// The row order is the only ordering contract this owner publishes to the compiler-facing
/// registration index. No compiler `SourceId` is assigned here.
fn build_source_inventory(
    classified: Vec<ClassifiedSource>,
    module_count: usize,
) -> Result<SourceInventory, InfrastructureFailure> {
    let mut classified = classified;
    classified.sort_by(|left, right| left.logical_identity.cmp(&right.logical_identity));

    let mut sources = Vec::with_capacity(classified.len());
    let mut owned_source_indices: Vec<Vec<SourceRecordIndex>> =
        (0..module_count).map(|_| Vec::new()).collect();
    let mut unrooted_source_indices = Vec::new();
    let mut canonical_path_to_source_index: FxHashMap<PathBuf, SourceRecordIndex> =
        FxHashMap::default();

    for (index, source) in classified.into_iter().enumerate() {
        let source_index = SourceRecordIndex::from_index(index);
        if canonical_path_to_source_index
            .insert(source.canonical_path.clone(), source_index)
            .is_some()
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "Source tree index classified physical source {} more than once; each canonical \
                 source path must have exactly one SourceRecord",
                    portable_path_text(&source.canonical_path),
                ),
            ));
        }
        match source.ownership {
            SourceOwnership::Owned(module_id) => {
                owned_source_indices[module_id.index()].push(source_index);
            }
            SourceOwnership::Unrooted => {
                unrooted_source_indices.push(source_index);
            }
        }
        sources.push(SourceRecord {
            canonical_path: source.canonical_path,
            classification: source.classification,
            supported: source.supported,
            logical_identity: source.logical_identity,
            ownership: source.ownership,
        });
    }

    Ok(SourceInventory {
        sources,
        owned_source_indices,
        unrooted_source_indices,
        canonical_path_to_source_index,
    })
}

/// One eligible entry classified for the synthetic namespace inventory.
///
/// WHAT: retains the authored path and listed file name plus the directory identity when the
/// entry is a directory.
/// WHY: the top-level key scan needs the authored name, while registered-prefix probing needs the
/// canonical destination so parent identity matches the directory traversal. One classification
/// pass supplies both.
struct EligibleSingleFileEntry {
    listed_path: PathBuf,
    listed_name: String,
    /// `None` for a file; the directory's physical identity otherwise.
    directory_identity: Option<EligibleDirectoryIdentity>,
}

/// The physical identity of one eligible directory entry.
enum EligibleDirectoryIdentity {
    /// The listed path below its canonical parent is already canonical.
    Physical,
    /// A symlink alias, retaining its canonical destination.
    Aliased(PathBuf),
}

/// One eligible directory below a directory being probed for an exact registered prefix.
///
/// `listed_path` keeps the authored spelling below its canonical parent for the leaf diagnostic,
/// while the identity supplies the physical destination used for parent-component identity.
///
/// Aliases are identified by their destination because the directory traversal canonicalizes every
/// queued directory before comparing its relative path with a registered parent prefix. They remain
/// eligible leaf names because the traversal's leaf check reads listed folder names.
struct EligibleDirectoryEntry {
    listed_path: PathBuf,
    identity: EligibleDirectoryIdentity,
}

impl EligibleDirectoryEntry {
    /// The canonical destination used for parent-component identity and descent.
    fn canonical_path(&self) -> &Path {
        match &self.identity {
            EligibleDirectoryIdentity::Physical => &self.listed_path,
            EligibleDirectoryIdentity::Aliased(destination) => destination,
        }
    }

    /// Whether the authored path is already the canonical destination.
    fn is_physical(&self) -> bool {
        matches!(self.identity, EligibleDirectoryIdentity::Physical)
    }

    /// Whether the authored folder name claims one registered leaf component.
    fn listed_name_matches(&self, component: &str) -> bool {
        self.listed_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| folder_name_claims_package_component(name, component))
    }

    /// Whether this directory's canonical destination names one registered parent component.
    ///
    /// The directory traversal compares the relative canonical path, so a symlink alias must not
    /// claim the alias spelling as a physical parent identity.
    fn canonical_name_matches(&self, component: &str) -> bool {
        self.canonical_path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| folder_name_claims_package_component(name, component))
    }
}

/// Classify one immediate or recursive directory entry for the synthetic namespace inventory.
///
/// WHAT: applies the same listed-type, alias and skip-policy rules as the bounded single-file
/// scan and returns the entry's authored path, UTF-8 file name and directory identity when the
/// entry is a directory.
/// WHY: the top-level key scan and the reserved-directory source inspection must agree on which
/// entries can become importable synthetic namespace claims. Registered-prefix probing also needs
/// the canonical destination so it reaches the outcome directory compilation reaches after
/// canonicalizing queued directories.
fn eligible_single_file_entry(
    skip_policy: &SourceTreeSkipPolicy,
    source_root: &Path,
    entry: &fs::DirEntry,
) -> Result<Option<EligibleSingleFileEntry>, PreparedFailure> {
    let path = entry.path();
    // As in the traversal, a non-symlink entry's listed type avoids a metadata probe.
    // Symlinks and unreadable entry types resolve their physical destination once.
    let listed_file_type = entry.file_type().ok();
    let listed_type = listed_file_type.filter(|file_type| !file_type.is_symlink());
    let is_directory = match listed_type {
        Some(listed) => listed.is_dir(),
        None => path.is_dir(),
    };
    let is_file = match listed_type {
        Some(listed) => listed.is_file(),
        None => path.is_file(),
    };
    if !is_directory && !is_file {
        return Ok(None);
    }
    let directory_identity = if is_directory {
        match listed_type {
            Some(_) => {
                if !skip_policy.authored_directory_is_eligible(&path, source_root) {
                    return Ok(None);
                }
                // The parent is canonical, so a normal directory's listed path is already its
                // physical destination.
                Some(EligibleDirectoryIdentity::Physical)
            }
            None => {
                let canonical_directory = canonicalize_source_path(&path)?;
                if !skip_policy.resolved_directory_is_eligible(&canonical_directory, source_root) {
                    return Ok(None);
                }
                Some(EligibleDirectoryIdentity::Aliased(canonical_directory))
            }
        }
    } else {
        if listed_type.is_none() {
            // File names may start with `_`; only the physical directory ancestry excludes a
            // source alias. This keeps `_helper.moth` and eligible in-boundary aliases while
            // dropping visible aliases into hidden, fixed-skipped or external trees.
            let canonical_file = canonicalize_source_path(&path)?;
            let Some(canonical_parent) = canonical_file.parent() else {
                return Ok(None);
            };
            if !canonical_file.starts_with(source_root)
                || canonical_file
                    .file_name()
                    .is_some_and(|name| name == CONFIG_FILE_NAME)
                || skip_policy.canonical_ancestor_is_skipped(canonical_parent, source_root)
            {
                return Ok(None);
            }
        }
        None
    };

    let listed_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| non_utf8_filesystem_name_error(&path, "single-file namespace name"))?
        .to_owned();
    Ok(Some(EligibleSingleFileEntry {
        listed_path: path,
        listed_name,
        directory_identity,
    }))
}

/// List the eligible child directories of one canonical directory.
///
/// WHY: registered-prefix probing descends from an already canonical parent, so every child's
/// listed name and physical destination are both available from one listing. The destination is
/// what the directory traversal compares when it canonicalizes queued directories.
fn eligible_child_directories(
    directory: &Path,
    source_root: &Path,
    skip_policy: &SourceTreeSkipPolicy,
) -> Result<Vec<EligibleDirectoryEntry>, PreparedFailure> {
    let mut directories = Vec::new();
    for entry in SourceTreeIndex::sorted_directory_entries(directory)? {
        let Some(entry) = eligible_single_file_entry(skip_policy, source_root, &entry)? else {
            continue;
        };
        let Some(identity) = entry.directory_identity else {
            continue;
        };
        directories.push(EligibleDirectoryEntry {
            listed_path: entry.listed_path,
            identity,
        });
    }
    Ok(directories)
}

/// Find the first local directory below the synthetic source root whose relative path equals a
/// registered package prefix.
///
/// WHAT: splits each registered prefix into its parent path and leaf folder name. Parent components
/// follow physical canonical directories, mirroring the directory traversal after it canonicalizes
/// queued directories, and every case-equivalent candidate at every component is probed. The leaf
/// matches the authored folder name below its canonical parent, which is what the traversal's
/// per-directory collision check names.
/// WHY: directory compilation reports `EntryRootPackagePrefixCollision` for the same filesystem
/// path even when the local directory publishes no interface. An ordinary selected file owns no
/// Stage 0 traversal, so this bounded per-prefix walk is the only evidence available. Probing only
/// registered identities keeps arbitrary descendant directories out of the scan, and the first
/// registered prefix with a matching directory aborts compilation, so later prefixes need no probe.
fn registered_package_prefix_collision(
    source_root: &Path,
    root_directories: &[EligibleDirectoryEntry],
    registered_package_prefixes: &[&str],
    skip_policy: &SourceTreeSkipPolicy,
) -> Result<Option<(String, PathBuf)>, PreparedFailure> {
    for &registered_prefix in registered_package_prefixes {
        let (parent, leaf_component) = split_package_prefix(registered_prefix);
        let parent_components = parent
            .split('/')
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>();

        let colliding_directory = matching_leaf_directory(
            source_root,
            root_directories,
            &parent_components,
            leaf_component,
            skip_policy,
        )?;
        if let Some(colliding_directory) = colliding_directory {
            return Ok(Some((registered_prefix.to_owned(), colliding_directory)));
        }
    }
    Ok(None)
}

/// Resolve the parent components of one registered prefix and match its leaf directory.
///
/// `directories` are the eligible children of the canonical directory that already matched the
/// preceding parent components. Parent components follow physical directories only, because the
/// directory traversal compares canonical relative paths and a symlink alias therefore cannot add
/// a parent identity. Every case-equivalent candidate is followed, so one spelling that lacks the
/// deeper path cannot hide another that has it. The leaf is matched against every listed folder
/// name, including aliases, exactly like the traversal's per-directory leaf check.
fn matching_leaf_directory(
    source_root: &Path,
    directories: &[EligibleDirectoryEntry],
    parent_components: &[&str],
    leaf_component: &str,
    skip_policy: &SourceTreeSkipPolicy,
) -> Result<Option<PathBuf>, PreparedFailure> {
    let Some((parent_component, remaining_parent_components)) = parent_components.split_first()
    else {
        return Ok(directories
            .iter()
            .find(|directory| directory.listed_name_matches(leaf_component))
            .map(|directory| directory.listed_path.clone()));
    };

    for directory in directories.iter().filter(|directory| {
        directory.is_physical() && directory.canonical_name_matches(parent_component)
    }) {
        let children =
            eligible_child_directories(directory.canonical_path(), source_root, skip_policy)?;
        if let Some(colliding_directory) = matching_leaf_directory(
            source_root,
            &children,
            remaining_parent_components,
            leaf_component,
            skip_policy,
        )? {
            return Ok(Some(colliding_directory));
        }
    }
    Ok(None)
}

/// Find the first importable claim below one reserved-root directory when the selection owns no
/// module roots.
///
/// WHAT: applies the module-namespace importability rule using only the structural facts available
/// before synthetic module discovery. A normal or support facade module claims the reserved root;
/// an entry-only module publishes nothing, so its directory and private helpers stay legal; an
/// unrooted directory claims through owned helper sources or a nested importable module. Normal
/// facades and helper sources inherit `reserved_root` as their first importable component, while a
/// support facade is imported by its directory basename.
/// WHY: an ordinary selected file owns no module identities, so this bounded walk is the only
/// available evidence. It is deliberately coarser than directory-mode namespace validation: an
/// entry-only subtree is treated as one private scope, while directory compilation also inspects
/// every nested module's own namespace. Building an index here to match that exactly would add the
/// second traversal this path exists to avoid. Both modes therefore share the reserved-root
/// predicate and the diagnostic owner and differ only in which importable facts they can read.
fn reserved_root_directory_claim(
    directory: &Path,
    reserved_root: &str,
    skip_policy: &SourceTreeSkipPolicy,
    source_root: &Path,
    visited_directories: &mut BTreeSet<PathBuf>,
) -> Result<Option<(String, PathBuf)>, PreparedFailure> {
    debug_assert!(
        reserved_package_namespace_claim_root(reserved_root).is_some(),
        "the reserved subtree root must be one authored reserved component"
    );
    // Directory aliases are one physical subtree. Track canonical destinations exactly like the
    // Stage 0 traversal so a symlink back to the selected root or an ancestor cannot loop.
    let canonical_directory = canonicalize_source_path(directory)?;
    if !canonical_directory.starts_with(source_root)
        || !visited_directories.insert(canonical_directory.clone())
        || skip_policy.canonical_ancestor_is_skipped(&canonical_directory, source_root)
    {
        return Ok(None);
    }

    let mut has_normal_root = false;
    let mut has_support_root = false;
    let mut has_entry_root = false;
    let mut helper_source = None;
    let mut subdirectories = Vec::new();

    for entry in SourceTreeIndex::sorted_directory_entries(directory)? {
        let Some(entry) = eligible_single_file_entry(skip_policy, source_root, &entry)? else {
            continue;
        };
        if entry.directory_identity.is_some() {
            subdirectories.push(entry.listed_path);
            continue;
        }
        if let Some(role) = module_root_role_for_file_name(&entry.listed_name) {
            match role {
                ModuleRootRole::Normal => has_normal_root = true,
                ModuleRootRole::Support => has_support_root = true,
                ModuleRootRole::Entry => has_entry_root = true,
                ModuleRootRole::Shared | ModuleRootRole::SyntheticProjectContext => {}
            }
            continue;
        }
        if ordinary_helper_source_stem(&entry.listed_name).is_some() {
            helper_source.get_or_insert(entry.listed_path);
        }
    }

    if has_normal_root {
        // A normal facade is addressed by its owning directory's namespace key, whose first
        // component is the reserved root inherited from the caller.
        return Ok(Some((reserved_root.to_owned(), directory.to_path_buf())));
    }
    if has_support_root {
        // A support facade is imported by its directory basename, not by its nested path.
        let module_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .expect("eligible reserved subtree directories have UTF-8 names");
        return Ok(reserved_package_namespace_claim_root(module_name)
            .map(|root| (root.to_owned(), directory.to_path_buf())));
    }
    if has_entry_root {
        // An entry-only module publishes no namespace entry to its owner; its private helpers
        // and private subtree stay legal.
        return Ok(None);
    }
    if let Some(path) = helper_source {
        return Ok(Some((reserved_root.to_owned(), path)));
    }

    for path in subdirectories {
        if let Some(claim) = reserved_root_directory_claim(
            &path,
            reserved_root,
            skip_policy,
            source_root,
            visited_directories,
        )? {
            return Ok(Some(claim));
        }
    }
    Ok(None)
}

// Without both `timers` and `benchmark_counters` every body reference to
// `stats` expands away, so the parameter is intentionally unused there.
#[cfg_attr(
    not(all(feature = "timers", feature = "benchmark_counters")),
    allow(unused_variables)
)]
fn record_discovery_metrics(stats: &SourceTreeDiscoveryStats) {
    counter_observation!("source_tree_index.discovery_runs", 1.0);
    counter_observation!("source_tree_index.dirs_visited", stats.dirs_visited as f64);
    counter_observation!("source_tree_index.dirs_skipped", stats.dirs_skipped as f64);
    counter_observation!("source_tree_index.files_seen", stats.files_seen as f64);
    counter_observation!(
        "source_tree_index.normal_root_files_seen",
        stats.normal_root_files_seen as f64,
    );
    counter_observation!(
        "source_tree_index.support_root_files_seen",
        stats.support_root_files_seen as f64,
    );
    counter_observation!(
        "source_tree_index.entry_root_files_seen",
        stats.entry_root_files_seen as f64,
    );
    counter_observation!(
        "source_tree_index.module_roots_found",
        stats.module_roots_found as f64,
    );
}

#[cfg(test)]
#[path = "../tests/source_tree_index_tests.rs"]
mod tests;
