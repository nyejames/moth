//! Boundary-aware module namespaces for indexed source-dependency resolution.
//!
//! WHAT: builds one `ModuleNamespace` per indexed project or source-package module from Stage 0
//! indexed facts. Each namespace maps extensionless module-relative dependency paths to explicit
//! tagged entries (same-module source, provider-owned file, cross-module target). Registered
//! binding packages and source-backed package surfaces are explicit shared entries keyed by
//! dependency prefix.
//! WHY: indexed lookups avoid filesystem candidate probing and public-surface fallback,
//! and keep boundary identity attached to boundary-local `SourceId`/`ModuleId`. The
//! canonical path leaves the namespace only as the IO/enqueue handle for the current compiler.
//!
//! Resolution rules:
//! - normal compiler-semantic source lookup starts from the declaring source's owning module
//!   namespace and resolves bare module/package surfaces explicitly
//! - `@./` and parent traversal are rejected through structured diagnostics
//! - private child/support bypass is rejected through structured diagnostics
//! - no `read_dir`, `exists`, `canonicalize` or fallback-candidate probing runs after the
//!   indexes exist
//! - provider-backed paths follow the same owning-module-root contract as compiler-semantic paths

use super::facade_route_prepass::FacadeRouteIndex;
use super::module_identity::{ModuleId, ModuleIdentityTable};
use super::project_module_graph::{ProjectModuleGraph, is_support_visible_in_identity_table};
use super::project_structure_diagnostics::{
    entry_root_package_prefix_collision_error, reserved_package_namespace_root_claim_error,
};
use super::source_package_discovery::SourcePackageBoundaryIndexes;
use super::source_tree_index::{
    SourceClassification, SourceLogicalIdentity, SourceOwnership, SourceRecordIndex,
    SourceTreeIndex,
};

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::definition::damaged_package_set_report;
use crate::builder_surface::package_metadata::{
    package_paths_overlap, reserved_package_namespace_claim_root,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{imports, token_lexical};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticBag, InvalidImportPathReason, PreparedFailure,
    SourceSpanCapacityResource,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::public_export_route_conflicts::{
    fold_route_spelling, try_intern_route_spelling,
};
use crate::compiler_frontend::paths::path_resolution::{
    ProjectPathResolver, explicit_dependency_source_kind,
};
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, PublicExportRoute};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::source_packages::root_file::dependency_path_references_config_file;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
/// One explicit namespace entry for one dependency path within one module namespace.
///
/// Entries are tagged records with no precedence chain: a same-module source is never
/// interchangeable with a cross-module target, and resolution is a single lookup rather than an
/// ordered fallback.
#[derive(Clone, Debug)]
enum NamespaceEntry {
    /// A source file owned by the same module, addressed by its Stage 0 row.
    SameModuleSource {
        source_index: SourceRecordIndex,
        source_kind: SourceFileKind,
    },
    /// An explicit provider-owned file in the same module, keyed by its path with extension.
    SameModuleProvider { source_index: SourceRecordIndex },
    /// A child normal module or visible support package, resolved by boundary-local `ModuleId`
    /// and the explicit surface selected for this consumer position.
    CrossModule {
        target_module_id: ModuleId,
        surface: ModuleRootRole,
    },
    /// One declared public route beneath the same facade a `CrossModule` entry selects.
    ///
    /// WHAT: resolves to the facade's normal or support surface with an exact route suffix. The
    ///       entry is a distinct provider path, never a prefix or a directory.
    /// WHY: routes are content-derived namespace facts published by the facade prepass. Keeping
    ///      them in a separate variant stops the private-boundary bypass check from treating a
    ///      route as an importable module subtree.
    PublicRoute {
        target_module_id: ModuleId,
        surface: ModuleRootRole,
        route: PublicExportRoute,
    },
}

/// One boundary-aware namespace for one indexed module.
///
/// WHAT: the pre-computed lookup table from extensionless module-relative dependency path to one
/// explicit `NamespaceEntry`. Built once from indexed facts and consumed by the live reachable
/// traversal without further filesystem access.
#[derive(Clone, Debug)]
struct ModuleNamespace {
    entries: BTreeMap<String, NamespaceEntry>,
    /// Exact authored keys that collide with another visible identity.
    ///
    /// Entries remain present so private-boundary prefix checks still work, but resolution
    /// rejects an ambiguous key before selecting any target.
    ambiguous_keys: std::collections::BTreeSet<String>,
}

impl ModuleNamespace {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            ambiguous_keys: std::collections::BTreeSet::new(),
        }
    }

    /// Insert one visible identity without applying precedence.
    ///
    /// Exact and ASCII-case-only collisions mark every colliding spelling ambiguous. The first
    /// entry stays only as structural context for private-boundary detection; it can never win
    /// resolution while its key remains ambiguous.
    fn insert(&mut self, key: String, entry: NamespaceEntry) {
        let case_collisions = self
            .entries
            .keys()
            .filter(|existing| existing.eq_ignore_ascii_case(&key))
            .cloned()
            .collect::<Vec<_>>();

        if !case_collisions.is_empty() {
            self.ambiguous_keys.insert(key.clone());
            self.ambiguous_keys.extend(case_collisions);
        }

        self.entries.entry(key).or_insert(entry);
    }

    fn is_ambiguous(&self, key: &str) -> bool {
        self.ambiguous_keys
            .iter()
            .any(|ambiguous_key| ambiguous_key.eq_ignore_ascii_case(key))
    }
}

/// Which compilation boundary a resolved target belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamespaceBoundary {
    Project,
    Package,
}

/// The result of resolving one compiler-semantic dependency through a namespace.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedDependency {
    /// A source file in the same module, addressed by its Stage 0 row.
    ///
    /// The traversal retains this row handle until it joins the boundary source database's
    /// compiler identity at the file-reference or module-preparation boundary.
    SameModuleSource {
        source_index: SourceRecordIndex,
        consumer_module_id: ModuleId,
    },
    /// A source file in another module in the active project or package boundary.
    CrossModule {
        provider_module_id: ModuleId,
        provider_surface: ModuleRootRole,
        consumer_module_id: ModuleId,
        /// The exact public route beneath the provider facade, absent for its root surface.
        ///
        /// WHAT: an owned spelling that later binding passes to the provider interface view.
        /// WHY: the consumer's authored key loses the facade prefix during resolution, and the
        ///      provider interface may expose the same leaf under several routes. Carrying the
        ///      resolved route keeps binding from re-deriving it from a path it cannot split.
        route: Option<PublicExportRoute>,
        #[cfg(test)]
        root_file: PathBuf,
    },
    /// A source-backed package facade selected by its registered dependency prefix.
    SourcePackageSurface {
        consumer_module_id: ModuleId,
        dependency_prefix: String,
        #[cfg(test)]
        root_file: PathBuf,
    },
    /// A registered binding-backed package handled by frontend dependency binding.
    BindingPackage,
}

/// The Stage 0 owner of boundary-aware module namespaces for all indexed project and
/// source-package modules.
///
/// WHAT: stores one `ModuleNamespace` per project module and one set per source-package boundary,
/// plus the retained `SourcePackageBoundaryIndexes` for package-file lookups and canonical-path
/// resolution. Raw `SourceId`/`ModuleId` values never cross boundaries: each namespace is
/// self-contained with boundary-local IDs.
/// WHY: the build-system authority requires Core, Builder and dependency source packages to
/// compile as separate graphs with their own source indexes. This owner is the single Stage 0
/// home for the namespace lookup tables that replace filesystem probing and public-surface
/// fallback on the directory production path.
#[derive(Default)]
pub(crate) struct ModuleNamespaceSet {
    /// One namespace per project module, indexed by project `ModuleId`.
    project_namespaces: Vec<ModuleNamespace>,
    /// One set of namespaces per source-package boundary, keyed by dependency prefix.
    /// Each inner `Vec` is indexed by the package boundary's local `ModuleId`.
    package_namespaces: BTreeMap<String, Vec<ModuleNamespace>>,
    /// Registered Core, Builder and dependency package paths without their leading `@`.
    binding_package_paths: BTreeSet<String>,
    /// Retained source-package boundary indexes shared by indexed and synthetic consumers.
    package_boundary_indexes: SourcePackageBoundaryIndexes,
    /// Bounded local collision facts for each prepared synthetic owning root, without source IDs.
    single_file_dependency_keys: BTreeMap<PathBuf, BTreeSet<String>>,
}

/// Borrowed directory-project namespace context used by canonical header-owned discovery.
///
/// The namespace set and project index form one lookup authority: the set resolves boundary-local
/// identities and the index supplies the project declaring_source's ownership plus canonical IO handles.
/// Single-file synthetic compilation carries no value of this type.
#[derive(Clone, Copy)]
pub(crate) struct DirectoryDependencyResolution<'a> {
    namespace_set: &'a ModuleNamespaceSet,
    source_tree_index: &'a SourceTreeIndex,
    path_fork: &'a PathInternerFork,
    boundary: NamespaceBoundary,
    package_prefix: Option<&'a str>,
}

impl<'a> DirectoryDependencyResolution<'a> {
    pub(crate) fn project(
        namespace_set: &'a ModuleNamespaceSet,
        source_tree_index: &'a SourceTreeIndex,
        path_fork: &'a PathInternerFork,
    ) -> Self {
        Self {
            namespace_set,
            source_tree_index,
            path_fork,
            boundary: NamespaceBoundary::Project,
            package_prefix: None,
        }
    }

    pub(crate) fn package(
        namespace_set: &'a ModuleNamespaceSet,
        dependency_prefix: &'a str,
        source_tree_index: &'a SourceTreeIndex,
        path_fork: &'a PathInternerFork,
    ) -> Self {
        Self {
            namespace_set,
            source_tree_index,
            path_fork,
            boundary: NamespaceBoundary::Package,
            package_prefix: Some(dependency_prefix),
        }
    }

    pub(crate) fn is_project_boundary(self) -> bool {
        matches!(self.boundary, NamespaceBoundary::Project)
    }

    pub(crate) fn path_fork(self) -> &'a PathInternerFork {
        self.path_fork
    }
    /// Rebind this boundary context to the path fork that issued a retained provider path.
    ///
    /// Header preparation may extend the boundary table with module-local dependency paths. Those
    /// `PathId`s are valid only through the current syntax fork, not the immutable pre-discovery
    /// prefix used to construct the boundary context.
    pub(crate) fn with_path_fork<'b>(
        self,
        path_fork: &'b PathInternerFork,
    ) -> DirectoryDependencyResolution<'b>
    where
        'a: 'b,
    {
        DirectoryDependencyResolution {
            namespace_set: self.namespace_set,
            source_tree_index: self.source_tree_index,
            path_fork,
            boundary: self.boundary,
            package_prefix: self.package_prefix,
        }
    }

    pub(crate) fn resolve_dependency(
        self,
        provider: &RetainedDependencyPath,
        declaring_canonical_path: &Path,
        string_table: &mut StringTable,
    ) -> Result<ResolvedDependency, DiagnosticDraft> {
        self.namespace_set.resolve_dependency(
            provider,
            declaring_canonical_path,
            self.source_tree_index,
            self.boundary,
            self.package_prefix,
            string_table,
            self.path_fork,
        )
    }

    pub(crate) fn has_binding_package_dependency(
        self,
        dependency_path: PathId,
        string_table: &StringTable,
    ) -> bool {
        self.namespace_set
            .binding_package_prefix(dependency_path, string_table, self.path_fork)
            .is_some()
    }

    /// The boundary `SourceTreeIndex` used by canonical module preparation.
    pub(crate) fn source_tree_index(&self) -> &SourceTreeIndex {
        self.source_tree_index
    }

    pub(crate) fn resolve_provider_target(
        self,
        provider_path: PathId,
        declaring_canonical_path: &Path,
        dependency_span: Option<SourceSpan>,
        string_table: &StringTable,
    ) -> Result<PathBuf, DiagnosticDraft> {
        self.namespace_set.resolve_provider_target(
            provider_path,
            declaring_canonical_path,
            dependency_span,
            &self,
            string_table,
        )
    }
}

impl ModuleNamespaceSet {
    /// Iterate the independently indexed source-package boundaries in deterministic prefix order.
    ///
    /// Package compilation builds one graph and provider store per item from this view. The
    /// boundary-local indexes remain owned here beside their namespaces, so their `SourceId` and
    /// `ModuleId` values cannot be mixed with the project boundary or another package.
    pub(crate) fn source_package_boundaries(
        &self,
    ) -> impl Iterator<Item = (&str, &SourceTreeIndex)> {
        self.package_boundary_indexes.iter()
    }

    /// Build the complete namespace set from the project index, graph and package boundary
    /// indexes.
    ///
    /// The project `SourceTreeIndex` and `ProjectModuleGraph` provide the indexed facts for
    /// project module namespaces. The `SourcePackageBoundaryIndexes` provide both the
    /// source-backed package surface entries and the per-package module namespaces. The
    /// package indexes are moved into the set and retained for package-file lookups during
    /// resolution.
    #[cfg(test)]
    pub(crate) fn build(
        source_tree_index: &SourceTreeIndex,
        project_module_graph: &ProjectModuleGraph,
        package_boundary_indexes: SourcePackageBoundaryIndexes,
        binding_packages: &ExternalPackageRegistry,
        facade_routes: &FacadeRouteIndex,
        string_table: &mut StringTable,
    ) -> Result<Self, PreparedFailure> {
        let mut namespaces =
            Self::for_source_packages(package_boundary_indexes, binding_packages, string_table)?;
        namespaces.attach_project_namespaces(
            source_tree_index,
            project_module_graph,
            facade_routes,
            string_table,
        )?;
        Ok(namespaces)
    }

    /// Build the project boundary's namespaces after its facade routes are published.
    ///
    /// WHAT: builds one namespace per project module, inserts the declared public-route entries
    ///       and validates reserved-root claims.
    /// WHY: the caller may need the package boundary namespaces (and therefore package
    ///      diagnostics) before it prepares project facade headers. Keeping source-package and
    ///      project attachment separate preserves the existing package-before-project failure
    ///      ordering while still publishing project routes before project discovery.
    pub(super) fn attach_project_namespaces(
        &mut self,
        source_tree_index: &SourceTreeIndex,
        project_module_graph: &ProjectModuleGraph,
        facade_routes: &FacadeRouteIndex,
        string_table: &mut StringTable,
    ) -> Result<(), PreparedFailure> {
        self.project_namespaces = build_project_namespaces(source_tree_index, project_module_graph);
        // A reserved-root (`core`/`builder`) claim is the stronger authored failure. Validate it
        // before route publication so a route reaching a registered package cannot mask it.
        validate_reserved_package_namespace_claims(
            &self.project_namespaces,
            source_tree_index,
            string_table,
        )?;
        insert_facade_route_entries(
            &mut self.project_namespaces,
            source_tree_index.module_identities(),
            facade_routes,
            &self.package_boundary_indexes,
            string_table,
        )
    }

    /// Synthetic consumers keep local namespace claims separate from reachable source membership.
    /// Only immediate names below prepared owning roots become collision keys, and a reserved-named
    /// directory is inspected for importable claims; unrelated source bodies and arbitrary
    /// descendant directories never enter the key inventory.
    ///
    /// `bounded_index` is the Stage 0 index that a selected module-root entry already owns. That
    /// index is the reserved-root authority for those selections, so the directory walk never
    /// applies to a synthetic unit that has module namespaces.
    pub(crate) fn for_single_file(
        package_boundary_indexes: SourcePackageBoundaryIndexes,
        binding_packages: &ExternalPackageRegistry,
        project_path_resolver: &ProjectPathResolver,
        bounded_index: Option<&SourceTreeIndex>,
        string_table: &mut StringTable,
    ) -> Result<Self, PreparedFailure> {
        let mut namespaces =
            Self::for_source_packages(package_boundary_indexes, binding_packages, string_table)?;
        let has_module_topology = bounded_index.is_some();

        // A selected module-root entry carries the bounded Stage 0 module topology, so its
        // reserved-root claims come from module namespaces exactly as directory compilation does.
        // An ordinary selected file owns no module roots; its importable directory names are
        // checked by the inventory below.
        if let Some(index) = bounded_index {
            let project_module_graph = ProjectModuleGraph::from_source_tree_index(index);
            let project_namespaces = build_project_namespaces(index, &project_module_graph);
            validate_reserved_package_namespace_claims(&project_namespaces, index, string_table)?;
        }

        let roots = std::iter::once(project_path_resolver.entry_root().to_path_buf())
            .chain(project_path_resolver.module_roots().cloned())
            .collect::<BTreeSet<_>>();
        // Directory compilation's exact-prefix rule covers registered source packages only:
        // binding packages share import names but own no project-local filesystem path. The
        // synthetic probe therefore uses the same source-package prefixes, so a local folder
        // matching a binding path stays legal exactly as it does in directory compilation.
        let registered_package_prefixes = namespaces
            .package_boundary_indexes
            .iter()
            .map(|(package_prefix, _)| package_prefix)
            .collect::<Vec<_>>();
        for root in roots {
            let inventory = SourceTreeIndex::single_file_namespace_inventory(
                &root,
                has_module_topology,
                &registered_package_prefixes,
            )?;
            // A moduleless selection owns no Stage 0 index, so the bounded walk owns both authored
            // claims. The collision is reported first, matching directory compilation, which
            // reports it during traversal before namespace-claim validation.
            if !has_module_topology {
                if let Some((registered_prefix, local_directory)) =
                    inventory.registered_prefix_collision
                {
                    return Err(entry_root_package_prefix_collision_error(
                        &registered_prefix,
                        &local_directory,
                        string_table,
                    ));
                }
                if let Some((root_component, source)) = inventory.reserved_root_claim {
                    return Err(reserved_package_namespace_root_claim_error(
                        &root_component,
                        &source,
                        string_table,
                    ));
                }
            }
            namespaces
                .single_file_dependency_keys
                .insert(root, inventory.keys);
        }
        Ok(namespaces)
    }

    /// Build only the source-package boundary namespaces.
    ///
    /// WHY: project namespaces wait until the facade route prepass has published the project's
    ///      declared routes, while package boundaries keep their existing construction order.
    pub(super) fn for_source_packages(
        package_boundary_indexes: SourcePackageBoundaryIndexes,
        binding_packages: &ExternalPackageRegistry,
        string_table: &mut StringTable,
    ) -> Result<Self, PreparedFailure> {
        let package_namespaces = build_package_namespaces(&package_boundary_indexes, string_table)?;
        let binding_package_paths = binding_packages
            .package_paths()
            .map(|path| path.strip_prefix('@').unwrap_or(path).to_owned())
            .collect();

        Ok(Self {
            project_namespaces: Vec::new(),
            package_namespaces,
            binding_package_paths,
            package_boundary_indexes,
            single_file_dependency_keys: BTreeMap::new(),
        })
    }

    /// Resolve one compiler-semantic dependency through the boundary-aware namespace.
    ///
    /// WHAT: determines the declaring file's boundary and owning module from the project or
    /// package index, rejects obsolete `@./` and parent traversal, rejects explicit source
    /// extensions except the reserved config spelling, resolves source-backed package surfaces,
    /// and looks up the dependency path in the owning module's namespace. A config-named
    /// directory resolves through that namespace like any other module surface; an unresolved
    /// reserved spelling keeps the direct special-file diagnostic. Returns a tagged
    /// `ResolvedDependency` carrying the canonical path as the IO handle and the boundary-local
    /// `ModuleId` for project graph edges.
    /// WHY: the indexed source-tree facts are the single owner of filesystem-backed source
    /// surfaces and candidate resolution.
    #[allow(
        clippy::too_many_arguments,
        reason = "namespace resolution keeps the shared set, retained provider path, declaring file, source-tree lookup, boundary, package prefix, mutable string table and shared path fork as separate inputs"
    )]
    pub(crate) fn resolve_dependency(
        &self,
        provider: &RetainedDependencyPath,
        declaring_canonical_path: &Path,
        source_tree_index: &SourceTreeIndex,
        boundary: NamespaceBoundary,
        package_prefix: Option<&str>,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> Result<ResolvedDependency, DiagnosticDraft> {
        let dependency_path = provider.path;
        let dependency_span = Some(provider.span);
        // The exact reserved spelling is deferred to the indexed namespace: a directory with
        // that name publishes an ordinary module surface, while an unresolved spelling retains
        // the direct reserved-file reason below. Every other explicit source extension keeps
        // its ordinary rejection.
        let config_named_dependency =
            dependency_path_references_config_file(dependency_path, path_fork, string_table);
        reject_invalid_path_components(dependency_path, dependency_span, string_table, path_fork)?;
        reject_explicit_source_extension(
            dependency_path,
            dependency_span,
            &mut DiagnosticStringBuilder::new(string_table),
            path_fork,
        )?;

        let mut scratch = Vec::new();
        let prefix_components = path_fork.resolve_components(dependency_path, &mut scratch);
        let source_index = source_tree_index
            .source_index_for_canonical_path(declaring_canonical_path)
            .ok_or_else(|| imports::missing_import_target(dependency_path, dependency_span))?;
        let SourceOwnership::Owned(consumer_module_id) =
            source_tree_index.source(source_index).ownership()
        else {
            return Err(imports::missing_import_target(
                dependency_path,
                dependency_span,
            ));
        };
        let namespace = match boundary {
            NamespaceBoundary::Project => &self.project_namespaces[consumer_module_id.index()],
            NamespaceBoundary::Package => {
                let package_prefix = package_prefix.expect("package resolution carries its prefix");
                &self
                    .package_namespaces
                    .get(package_prefix)
                    .expect("package resolution prefix exists")[consumer_module_id.index()]
            }
        };

        let key = portable_dependency_key(prefix_components, string_table);

        // Only an actual cross-module facade may claim the reserved spelling. A same-module
        // source or provider with the same extensionless key, an ambiguous key, and any package
        // identity stay outside this spelling's source namespace.
        if config_named_dependency {
            let entry = namespace.entries.get(&key).filter(|entry| {
                matches!(entry, NamespaceEntry::CrossModule { .. }) && !namespace.is_ambiguous(&key)
            });
            let Some(entry) = entry else {
                return Err(imports::direct_special_file_import(
                    dependency_path,
                    dependency_span,
                ));
            };
            reject_hidden_directory_components(
                dependency_path,
                dependency_span,
                string_table,
                path_fork,
            )?;
            return resolve_entry(
                entry,
                source_tree_index,
                consumer_module_id,
                dependency_path,
                dependency_span,
                string_table,
            );
        }

        if namespace.is_ambiguous(&key) {
            return Err(imports::ambiguous_import_target(
                dependency_path,
                dependency_span,
            ));
        }

        // A published route is a namespace identity, not a physical directory. Resolve an exact
        // route before the hidden-directory guard, which exists to keep private physical paths
        // (and route misses) closed.
        if let Some(entry @ NamespaceEntry::PublicRoute { .. }) = namespace.entries.get(&key) {
            return resolve_entry(
                entry,
                source_tree_index,
                consumer_module_id,
                dependency_path,
                dependency_span,
                string_table,
            );
        }

        if let Some(package) = self.resolve_package_dependency(
            provider,
            consumer_module_id,
            namespace.entries.keys().map(String::as_str),
            &key,
        )? {
            return Ok(package);
        }

        // Package prefixes are validated independently and may contain underscore components, so
        // classify packages before treating an intermediate component as a hidden source directory.
        reject_hidden_directory_components(
            dependency_path,
            dependency_span,
            string_table,
            path_fork,
        )?;

        if let Some(entry) = namespace.entries.get(&key) {
            return resolve_entry(
                entry,
                source_tree_index,
                consumer_module_id,
                dependency_path,
                dependency_span,
                string_table,
            );
        }

        if find_module_bypass_prefix(&namespace.entries, &key).is_some() {
            return Err(imports::cross_module_import_not_exported(
                dependency_path,
                dependency_span,
            ));
        }

        Err(imports::missing_import_target(
            dependency_path,
            dependency_span,
        ))
    }

    fn binding_package_prefix<'a>(
        &'a self,
        dependency_path: PathId,
        string_table: &StringTable,
        path_fork: &PathInternerFork,
    ) -> Option<&'a str> {
        let mut scratch = Vec::new();
        let key = portable_dependency_key(
            path_fork.resolve_components(dependency_path, &mut scratch),
            string_table,
        );
        self.binding_package_prefix_for_key(&key)
    }

    fn binding_package_prefix_for_key<'a>(&'a self, provider_path: &str) -> Option<&'a str> {
        self.binding_package_paths
            .iter()
            .filter(|package_path| {
                provider_path == package_path.as_str()
                    || provider_path
                        .strip_prefix(package_path.as_str())
                        .is_some_and(|remainder| remainder.starts_with('/'))
            })
            .max_by_key(|package_path| package_path.len())
            .map(String::as_str)
    }
    fn resolve_provider_target(
        &self,
        provider_path: PathId,
        declaring_canonical_path: &Path,
        dependency_span: Option<SourceSpan>,
        resolution: &DirectoryDependencyResolution<'_>,
        string_table: &StringTable,
    ) -> Result<PathBuf, DiagnosticDraft> {
        if path_contains_dotdot(provider_path, resolution.path_fork, string_table) {
            return Err(imports::invalid_import_path(
                provider_path,
                InvalidImportPathReason::ParentDirectorySegment,
                dependency_span,
            ));
        }
        let source_index = resolution
            .source_tree_index
            .source_index_for_canonical_path(declaring_canonical_path)
            .ok_or_else(|| imports::missing_import_target(provider_path, dependency_span))?;
        let SourceOwnership::Owned(module_id) = resolution
            .source_tree_index
            .source(source_index)
            .ownership()
        else {
            return Err(imports::missing_import_target(
                provider_path,
                dependency_span,
            ));
        };
        // Module IDs and source rows are local to the context's project or package boundary.
        let namespace = match resolution.boundary {
            NamespaceBoundary::Project => &self.project_namespaces[module_id.index()],
            NamespaceBoundary::Package => {
                let package_prefix = resolution
                    .package_prefix
                    .expect("package resolution carries its prefix");
                &self
                    .package_namespaces
                    .get(package_prefix)
                    .expect("package resolution prefix exists")[module_id.index()]
            }
        };

        let mut scratch = Vec::new();
        let key = portable_dependency_key(
            resolution
                .path_fork
                .resolve_components(provider_path, &mut scratch),
            string_table,
        );
        if namespace.is_ambiguous(&key) {
            return Err(imports::ambiguous_import_target(
                provider_path,
                dependency_span,
            ));
        }

        match namespace.entries.get(&key) {
            Some(NamespaceEntry::SameModuleProvider { source_index }) => Ok(resolution
                .source_tree_index
                .source(*source_index)
                .canonical_path()
                .to_path_buf()),
            Some(NamespaceEntry::CrossModule { .. })
            | Some(NamespaceEntry::PublicRoute { .. })
            | Some(NamespaceEntry::SameModuleSource { .. }) => Err(
                imports::cross_module_import_not_exported(provider_path, dependency_span),
            ),
            None if find_module_bypass_prefix(&namespace.entries, &key).is_some() => Err(
                imports::cross_module_import_not_exported(provider_path, dependency_span),
            ),
            None => Err(imports::missing_import_target(
                provider_path,
                dependency_span,
            )),
        }
    }

    /// Resolve registered package surfaces through the same authority used by indexed modules.
    /// Package implementation files never enter the synthetic consumer's reachable source set.
    pub(crate) fn resolve_single_file_package_dependency(
        &self,
        provider: &RetainedDependencyPath,
        declaring_canonical_path: &Path,
        project_path_resolver: &ProjectPathResolver,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> Result<Option<ResolvedDependency>, DiagnosticDraft> {
        let dependency_span = Some(provider.span);
        // The exact reserved spelling may name an ordinary module facade in the bounded synthetic
        // topology. Let it reach the prepared Normal-root lookup instead of treating the spelling
        // as a package identity. The filesystem resolver retains the direct special-file
        // diagnostic when no facade owns the directory.
        let config_named_dependency =
            dependency_path_references_config_file(provider.path, path_fork, string_table);
        reject_invalid_path_components(provider.path, dependency_span, string_table, path_fork)?;
        reject_explicit_source_extension(
            provider.path,
            dependency_span,
            &mut DiagnosticStringBuilder::new(string_table),
            path_fork,
        )?;
        let package = if config_named_dependency {
            None
        } else {
            let mut scratch = Vec::new();
            let components = path_fork.resolve_components(provider.path, &mut scratch);
            let key = portable_dependency_key(components, string_table);
            let owning_root = project_path_resolver
                .module_root_for_file(declaring_canonical_path)
                .unwrap_or_else(|| project_path_resolver.entry_root().to_path_buf());
            let local_keys = self
                .single_file_dependency_keys
                .get(&owning_root)
                .into_iter()
                .flatten()
                .map(String::as_str);
            // Synthetic traversal belongs to the sole consumer in its one-node graph.
            self.resolve_package_dependency(provider, ModuleId::from_index(0), local_keys, &key)?
        };
        if package.is_none() {
            reject_hidden_directory_components(
                provider.path,
                dependency_span,
                string_table,
                path_fork,
            )?;
        }
        Ok(package)
    }

    fn resolve_package_dependency<'a>(
        &self,
        provider: &RetainedDependencyPath,
        consumer_module_id: ModuleId,
        mut local_dependency_keys: impl Iterator<Item = &'a str>,
        key: &str,
    ) -> Result<Option<ResolvedDependency>, DiagnosticDraft> {
        let dependency_path = provider.path;
        let dependency_span = Some(provider.span);
        let source_package_surface = self.find_source_package_surface(key);
        let binding_package_prefix = self.binding_package_prefix_for_key(key);

        if source_package_surface.is_some() && binding_package_prefix.is_some() {
            return Err(imports::ambiguous_import_target(
                dependency_path,
                dependency_span,
            ));
        }

        if let Some(binding_prefix) = binding_package_prefix {
            if local_dependency_keys
                .any(|entry_key| package_paths_overlap(entry_key, binding_prefix))
            {
                return Err(imports::ambiguous_import_target(
                    dependency_path,
                    dependency_span,
                ));
            }

            return Ok(Some(ResolvedDependency::BindingPackage));
        }

        if let Some((dependency_prefix, _root_file)) = source_package_surface {
            if local_dependency_keys.any(|entry_key| package_paths_overlap(entry_key, key)) {
                return Err(imports::ambiguous_import_target(
                    dependency_path,
                    dependency_span,
                ));
            }

            return Ok(Some(ResolvedDependency::SourcePackageSurface {
                consumer_module_id,
                dependency_prefix: dependency_prefix.to_owned(),
                #[cfg(test)]
                root_file: _root_file.to_path_buf(),
            }));
        }

        if self.is_source_package_private_path(key) {
            return Err(imports::cross_module_import_not_exported(
                dependency_path,
                dependency_span,
            ));
        }

        // Package identities reject ASCII case-only overlaps, so a case variant of a registered
        // package path can't fall through to a local target with the same spelling.
        let names_package_case_variant = self
            .package_boundary_indexes
            .iter()
            .map(|(prefix, _)| prefix)
            .chain(self.binding_package_paths.iter().map(String::as_str))
            .any(|prefix| key.len() >= prefix.len() && package_paths_overlap(prefix, key));
        if names_package_case_variant
            && local_dependency_keys.any(|entry_key| package_paths_overlap(entry_key, key))
        {
            return Err(imports::ambiguous_import_target(
                dependency_path,
                dependency_span,
            ));
        }

        Ok(None)
    }

    /// Match the complete registered package prefix without traversing its private facade.
    fn find_source_package_surface(&self, provider_path: &str) -> Option<(&str, &Path)> {
        for (dependency_prefix, package_index) in self.package_boundary_indexes.iter() {
            if dependency_prefix == provider_path {
                return package_index
                    .package_public_root_file()
                    .map(|root_file| (dependency_prefix, root_file));
            }
        }
        None
    }

    /// Whether a dependency tries to traverse below a registered source-package facade.
    fn is_source_package_private_path(&self, provider_path: &str) -> bool {
        self.package_boundary_indexes
            .iter()
            .any(|(dependency_prefix, _)| {
                provider_path
                    .strip_prefix(dependency_prefix)
                    .is_some_and(|remainder| remainder.starts_with('/'))
            })
    }
}

// ---------------------------------------------------------------------------
// Namespace construction
// ---------------------------------------------------------------------------

/// The first importable claim on a reserved package namespace root in one namespace set.
///
/// WHAT: scans every module namespace, regardless of module role, because each module's own
/// sources can resolve that namespace's entries. Returns the authored root spelling and the
/// source that publishes the entry.
/// WHY: project namespaces and registered source-package boundaries share the importability rule
/// but select different failure lanes, so one scan owns the fact and each caller owns its failure.
/// Only namespace entries count: the module's own reserved directory name never appears in its own
/// namespace, so it stays legal. Entries are the single Stage 0 record of what can be imported, so
/// this check reuses them instead of re-walking the filesystem or recomputing module ownership.
fn first_reserved_package_namespace_claim<'a>(
    namespaces: &'a [ModuleNamespace],
    source_tree_index: &'a SourceTreeIndex,
) -> Option<(&'a str, &'a Path)> {
    let identities = source_tree_index.module_identities();
    for module_id in identities.module_ids() {
        let namespace = &namespaces[module_id.index()];
        for (key, entry) in &namespace.entries {
            let Some(root) = reserved_package_namespace_claim_root(key) else {
                continue;
            };
            let source = match entry {
                NamespaceEntry::SameModuleSource { source_index, .. } => {
                    source_tree_index.source(*source_index).canonical_path()
                }
                NamespaceEntry::CrossModule {
                    target_module_id, ..
                } => identities.record(*target_module_id).root_directory(),
                // A route is only reachable beneath a facade entry that the same namespace would
                // already have validated. Its key can never claim a reserved first component on
                // its own, so checking it here would only restate the facade claim.
                NamespaceEntry::PublicRoute { .. } => continue,
                // Provider-owned files are ordinary resources, not compiler-semantic source
                // claims on the package namespace.
                NamespaceEntry::SameModuleProvider { .. } => continue,
            };
            return Some((root, source));
        }
    }
    None
}

/// Validate reserved-root claims in one already-built project namespace set.
fn validate_reserved_package_namespace_claims(
    project_namespaces: &[ModuleNamespace],
    source_tree_index: &SourceTreeIndex,
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    let Some((root, source)) =
        first_reserved_package_namespace_claim(project_namespaces, source_tree_index)
    else {
        return Ok(());
    };
    Err(reserved_package_namespace_root_claim_error(
        root,
        source,
        string_table,
    ))
}

/// Validate reserved-root claims in one already-built source-package namespace set.
///
/// A compiler-shipped package registration is installed input rather than authored project
/// source, so an importable claim inside it means the selected `packages/` set is damaged or was
/// modified. It takes the same infrastructure lane as a missing shipped facade. Third-party and
/// project-local registrations keep the authored diagnostic used for project namespaces.
fn validate_source_package_reserved_namespace_claims(
    package_namespaces: &[ModuleNamespace],
    source_tree_index: &SourceTreeIndex,
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    let Some((root, source)) =
        first_reserved_package_namespace_claim(package_namespaces, source_tree_index)
    else {
        return Ok(());
    };
    if let Some(report) = damaged_package_set_report(source_tree_index.package_set_membership()) {
        return Err(InfrastructureFailure::filesystem(
            CompilerStage::ProjectBuildAssembly,
            source,
            format!(
                "Source package publishes an importable claim on reserved namespace root \
                 '@{root}'. {report}"
            ),
        )
        .into());
    }
    Err(reserved_package_namespace_root_claim_error(
        root,
        source,
        string_table,
    ))
}

/// Build one `ModuleNamespace` per project module from the project index and graph.
fn build_project_namespaces(
    source_tree_index: &SourceTreeIndex,
    project_module_graph: &ProjectModuleGraph,
) -> Vec<ModuleNamespace> {
    let identities = source_tree_index.module_identities();
    let module_count = identities.module_ids().count();
    let mut namespaces = (0..module_count)
        .map(|_| ModuleNamespace::new())
        .collect::<Vec<_>>();

    for module_id in identities.module_ids() {
        let namespace = &mut namespaces[module_id.index()];
        populate_same_module_entries(namespace, source_tree_index, module_id);
        populate_project_cross_module_targets(
            namespace,
            identities,
            project_module_graph,
            module_id,
        );
    }

    namespaces
}

/// Build and validate one `ModuleNamespace` set per source-package boundary from the retained
/// package indexes.
///
/// Each boundary is validated from the namespaces just built, so registered source packages
/// cannot publish an importable claim on `@core` or `@builder` either. No separate walk is
/// needed: these entries are the same Stage 0 importability facts used for project modules.
fn build_package_namespaces(
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    string_table: &mut StringTable,
) -> Result<BTreeMap<String, Vec<ModuleNamespace>>, PreparedFailure> {
    let mut package_namespaces: BTreeMap<String, Vec<ModuleNamespace>> = BTreeMap::new();

    for (dependency_prefix, package_index) in package_boundary_indexes.iter() {
        let identities = package_index.module_identities();
        let module_count = identities.module_ids().count();
        let mut namespaces = (0..module_count)
            .map(|_| ModuleNamespace::new())
            .collect::<Vec<_>>();

        for module_id in identities.module_ids() {
            let namespace = &mut namespaces[module_id.index()];
            populate_same_module_entries(namespace, package_index, module_id);
            populate_package_cross_module_targets(namespace, identities, module_id);
        }

        validate_source_package_reserved_namespace_claims(
            &namespaces,
            package_index,
            string_table,
        )?;
        package_namespaces.insert(dependency_prefix.to_owned(), namespaces);
    }
    Ok(package_namespaces)
}

/// One planned public-route namespace entry for a consumer module.
struct PlannedRouteEntry {
    key: String,
    facade_module_id: ModuleId,
    surface: ModuleRootRole,
    route: PublicExportRoute,
    span: SourceSpan,
}

/// Which visible identity a declared public route collides with.
///
/// WHAT: separates the collision causes while one typed import diagnostic reports every kind to
///       the author.
/// WHY: the eager publication rule must name the collision kind without inventing a second
///      diagnostic family; focused tests assert the kind directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouteCollisionKind {
    ExistingNamespaceEntry,
    FacadeRoute,
    SourcePackage,
}

#[derive(Clone, Debug)]
struct RouteCollision {
    /// Retained for focused collision-cause tests; production reports the one typed import
    /// diagnostic for every kind.
    #[cfg_attr(not(test), allow(dead_code))]
    kind: RouteCollisionKind,
    route_key: String,
    span: SourceSpan,
    /// The competing facade's authored route operand when the collision has two content sites.
    related_span: Option<SourceSpan>,
}

/// Insert every declared route of every visible facade into one boundary's module namespaces.
///
/// WHAT: for each consumer namespace, plans `<facade-key>/<route>` entries only for facades whose
///       own `CrossModule` entry is present, rejects a collision involving any route even when no
///       consumer references it, then inserts the surviving entries through `ModuleNamespace::insert`.
/// WHY: public routes are content-derived facts published by the prepass. Inserting them through
///      the existing no-precedence table keeps exact and case-only collision policy identical to
///      every other visible identity, while planning first lets eager validation stay one
///      deterministic pass over the authored route claims.
fn insert_facade_route_entries(
    namespaces: &mut [ModuleNamespace],
    identities: &ModuleIdentityTable,
    routes: &FacadeRouteIndex,
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    let source_package_prefixes = package_boundary_indexes
        .iter()
        .map(|(prefix, _)| prefix)
        .collect::<Vec<_>>();

    for module_id in identities.module_ids() {
        let namespace = &mut namespaces[module_id.index()];
        let (planned, ambiguous_route_keys) = planned_route_entries(namespace, routes);
        for key in ambiguous_route_keys {
            namespace.ambiguous_keys.insert(key);
        }
        if planned.is_empty() {
            continue;
        }
        if let Some(collision) =
            first_route_collision(&planned, &namespace.entries, &source_package_prefixes)
        {
            return Err(route_collision_failure(collision, string_table));
        }
        for entry in planned {
            namespace.insert(
                entry.key,
                NamespaceEntry::PublicRoute {
                    target_module_id: entry.facade_module_id,
                    surface: entry.surface,
                    route: entry.route,
                },
            );
        }
    }

    Ok(())
}

/// Plan the route entries one consumer namespace may insert from its already-built facade entries.
///
/// WHY: route visibility is exactly facade-entry visibility. Reusing the `CrossModule` entries
///      preserves child/support selection, paired-surface suppression, root-local isolation and
///      entry-only exclusion without re-deriving any of them from topology.
fn planned_route_entries(
    namespace: &ModuleNamespace,
    routes: &FacadeRouteIndex,
) -> (Vec<PlannedRouteEntry>, Vec<String>) {
    let mut planned = BTreeMap::new();
    let mut ambiguous_route_keys = BTreeSet::new();
    for (facade_key, entry) in &namespace.entries {
        let NamespaceEntry::CrossModule {
            target_module_id,
            surface,
        } = entry
        else {
            continue;
        };
        if facade_key.is_empty() {
            continue;
        }
        // An ambiguous base key keeps the first retained row only as structural context. Its
        // route keys must stay unresolvable too: publishing them would select one of the
        // competing facades by insertion order. Record those keys as ambiguous so resolution
        // reports the base conflict rather than falling through to the bypass lane.
        if namespace.is_ambiguous(facade_key) {
            for claim in routes.routes(*target_module_id, *surface) {
                if !claim.route.is_root() {
                    ambiguous_route_keys.insert(format!("{facade_key}/{}", claim.route.as_str()));
                }
            }
            continue;
        }
        for claim in routes.routes(*target_module_id, *surface) {
            if claim.route.is_root() {
                continue;
            }
            let key = format!("{facade_key}/{}", claim.route.as_str());
            planned
                .entry((
                    key.clone(),
                    *target_module_id,
                    *surface,
                    claim.route.clone(),
                ))
                .or_insert(PlannedRouteEntry {
                    key,
                    facade_module_id: *target_module_id,
                    surface: *surface,
                    route: claim.route.clone(),
                    span: claim.span,
                });
        }
    }
    (
        planned.into_values().collect(),
        ambiguous_route_keys.into_iter().collect(),
    )
}

/// Return the first deterministic collision involving a planned route entry.
///
/// WHAT: rejects a route that folds onto any existing non-route entry, two routes from distinct
///       facades that fold together, and a route that overlaps a registered source-package
///       prefix.
/// WHY: these are the identities consumers can actually collide with under the current topology.
///      A route matching a private physical folder inside the exporting module has no namespace
///      entry in any consumer and therefore stays legal. Case-only conflicts between the same
///      facade's routes are authored-header diagnostics, not namespace collisions, so they stay
///      reserved for the facade's own preparation.
fn first_route_collision(
    planned: &[PlannedRouteEntry],
    entries: &BTreeMap<String, NamespaceEntry>,
    source_package_prefixes: &[&str],
) -> Option<RouteCollision> {
    for plan in planned {
        if entries
            .keys()
            .any(|existing| existing.eq_ignore_ascii_case(&plan.key))
        {
            return Some(RouteCollision {
                kind: RouteCollisionKind::ExistingNamespaceEntry,
                route_key: plan.key.clone(),
                span: plan.span,
                related_span: None,
            });
        }
    }

    let mut folded_route_groups: BTreeMap<String, Vec<&PlannedRouteEntry>> = BTreeMap::new();
    for plan in planned {
        folded_route_groups
            .entry(fold_route_spelling(&plan.key))
            .or_default()
            .push(plan);
    }
    for group in folded_route_groups.values() {
        if group.len() < 2 {
            continue;
        }
        let first_facade = (group[0].facade_module_id, group[0].surface);
        if group
            .iter()
            .any(|plan| (plan.facade_module_id, plan.surface) != first_facade)
        {
            // The later planned claim owns the diagnostic site, keeping the result independent
            // of map iteration or authored-file discovery order.
            let collision = group
                .last()
                .expect("a folded route group repeats its first key");
            return Some(RouteCollision {
                kind: RouteCollisionKind::FacadeRoute,
                route_key: collision.key.clone(),
                span: collision.span,
                related_span: group
                    .iter()
                    .find(|candidate| {
                        (candidate.facade_module_id, candidate.surface)
                            != (collision.facade_module_id, collision.surface)
                    })
                    .map(|candidate| candidate.span),
            });
        }
    }

    for plan in planned {
        if source_package_prefixes
            .iter()
            .any(|package_prefix| package_paths_overlap(package_prefix, &plan.key))
        {
            return Some(RouteCollision {
                kind: RouteCollisionKind::SourcePackage,
                route_key: plan.key.clone(),
                span: plan.span,
                related_span: None,
            });
        }
    }

    None
}

/// Build the typed import diagnostic for one eagerly rejected route collision.
///
/// WHAT: interns the colliding consumer-relative route key into a diagnostic-only path table and
///       reports it through the ambiguous-import family at the authored route operand span.
/// WHY: the route key is the importable spelling the author would write, and the operand span is
///      the only authored site that exists before any consumer references it. The affected facade
///      root or same-module source carries no source span of its own in Stage 0, so the diagnostic
///      names the route and leaves the other claimant as the key's visible identity.
fn route_collision_failure(
    collision: RouteCollision,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut path_fork = PathInternerFork::empty();
    let path = try_intern_route_spelling(&collision.route_key, &mut path_fork, string_table);
    let diagnostic = match (path, collision.related_span) {
        (None, _) => token_lexical::source_span_capacity(
            0,
            u32::MAX,
            SourceSpanCapacityResource::LogicalPath,
            Some(collision.span),
        ),
        (Some(path), Some(related_span)) => imports::ambiguous_import_target_with_previous(
            path,
            Some(collision.span),
            Some(related_span),
        ),
        (Some(path), None) => imports::ambiguous_import_target(path, Some(collision.span)),
    };
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        Some(Arc::new(path_fork.shared_table())),
    ))
}

/// Add same-module source entries for one module from its owned source rows.
///
/// Each owned source's extensionless module-relative logical path becomes a namespace key.
/// Special root files are excluded because direct root dependencies are rejected: every root
/// file's owning origin carries a Normal, Support, Entry or SyntheticProjectContext role (for
/// example `@mod.moth`, `@package.moth` or `@entry.moth`).
fn populate_same_module_entries(
    namespace: &mut ModuleNamespace,
    index: &SourceTreeIndex,
    module_id: ModuleId,
) {
    for source_index in index.owned_source_indices(module_id) {
        let record = index.source(*source_index);
        let SourceLogicalIdentity::Owned(owned_identity) = record.logical_identity() else {
            continue;
        };
        let relative_path = owned_identity.relative_source_path();

        let is_root_surface = record
            .logical_identity()
            .module_origin()
            .is_some_and(|origin| {
                matches!(
                    origin.role(),
                    ModuleRootRole::Normal
                        | ModuleRootRole::Support
                        | ModuleRootRole::Entry
                        | ModuleRootRole::SyntheticProjectContext
                )
            });
        if is_root_surface {
            continue;
        }

        match record.classification() {
            SourceClassification::CompilerSemantic(source_kind) => namespace.insert(
                extensionless_portable_path(relative_path),
                NamespaceEntry::SameModuleSource {
                    source_index: *source_index,
                    source_kind: *source_kind,
                },
            ),
            SourceClassification::ProviderOwned(_) => namespace.insert(
                relative_path.to_owned(),
                NamespaceEntry::SameModuleProvider {
                    source_index: *source_index,
                },
            ),
        }
    }
}

/// Add cross-module target entries for one project module: direct child modules and visible
/// support packages.
fn populate_project_cross_module_targets(
    namespace: &mut ModuleNamespace,
    identities: &ModuleIdentityTable,
    project_module_graph: &ProjectModuleGraph,
    module_id: ModuleId,
) {
    populate_cross_module_targets(namespace, identities, module_id, |support_id| {
        project_module_graph.is_support_visible_to_consumer(support_id, module_id)
    });
}

/// Add cross-module target entries for one package module: direct child modules and visible
/// support packages, using the package's identity table for support visibility.
fn populate_package_cross_module_targets(
    namespace: &mut ModuleNamespace,
    identities: &ModuleIdentityTable,
    module_id: ModuleId,
) {
    populate_cross_module_targets(namespace, identities, module_id, |support_id| {
        is_support_visible_in_identity_table(identities, support_id, module_id)
    });
}

/// Shared child/support namespace population for project and package boundary identity tables.
fn populate_cross_module_targets(
    namespace: &mut ModuleNamespace,
    identities: &ModuleIdentityTable,
    module_id: ModuleId,
    is_support_visible: impl Fn(ModuleId) -> bool,
) {
    let owning_path = identities
        .record(module_id)
        .stable_origin()
        .logical_module_path()
        .to_owned();

    for child_id in identities.direct_child_modules(module_id) {
        if identities.record(*child_id).normal_root_file().is_none() {
            continue;
        }

        if let Some(key) = relative_module_key(&owning_path, identities, *child_id) {
            namespace.insert(
                key,
                NamespaceEntry::CrossModule {
                    target_module_id: *child_id,
                    surface: ModuleRootRole::Normal,
                },
            );
        }
    }

    for support_id in identities.module_ids() {
        // The shared visibility predicate excludes self, but keep this local guard so namespace
        // construction can never expose a module's own support surface to itself.
        if support_id == module_id {
            continue;
        }
        if identities.record(support_id).support_root_file().is_none()
            || !is_support_visible(support_id)
        {
            continue;
        }
        if let Some(key) = support_package_key(identities, support_id) {
            // The consumer imports a paired normal surface through its normal module path, even
            // when that path passes through unrooted intermediate directories. Suppress the
            // support key only when the same structural module already has a normal entry in
            // this namespace. Key spelling cannot decide this: the support key is a basename.
            let normal_surface_already_visible = namespace.entries.values().any(|entry| {
                matches!(
                    entry,
                    NamespaceEntry::CrossModule {
                        target_module_id,
                        surface: ModuleRootRole::Normal,
                    } if *target_module_id == support_id
                )
            });
            if normal_surface_already_visible {
                continue;
            }
            namespace.insert(
                key,
                NamespaceEntry::CrossModule {
                    target_module_id: support_id,
                    surface: ModuleRootRole::Support,
                },
            );
        }
    }
}

fn relative_module_key(
    owning_path: &str,
    identities: &ModuleIdentityTable,
    target_id: ModuleId,
) -> Option<String> {
    let target_path = identities
        .record(target_id)
        .stable_origin()
        .logical_module_path();

    if owning_path.is_empty() {
        if target_path.is_empty() {
            return None;
        }
        return Some(target_path.to_owned());
    }

    let prefix = format!("{owning_path}/");
    target_path
        .strip_prefix(&prefix)
        .map(|remainder| remainder.to_owned())
        .filter(|key| !key.is_empty())
}

/// The package name exposed by one support root.
///
/// Support packages are injected by the containing directory's name. Consumers never encode the
/// package's physical path from their own module root.
fn support_package_key(identities: &ModuleIdentityTable, support_id: ModuleId) -> Option<String> {
    identities
        .record(support_id)
        .stable_origin()
        .logical_module_path()
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

// ---------------------------------------------------------------------------
// Dependency resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a namespace entry to a `ResolvedDependency`, looking up the canonical path and root file
/// from the owning boundary's `SourceTreeIndex`.
///
/// For a same-module source, the entry's `supported` flag is checked so a recognized-but-
/// unsupported extension (for example `.md` without builder selection) produces the
/// `UnsupportedSourceFileKind` diagnostic without a filesystem probe.
fn resolve_entry(
    entry: &NamespaceEntry,
    index: &SourceTreeIndex,
    consumer_module_id: ModuleId,
    dependency_path: PathId,
    dependency_span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<ResolvedDependency, DiagnosticDraft> {
    match entry {
        NamespaceEntry::SameModuleSource {
            source_index,
            source_kind,
        } => {
            let record = index.source(*source_index);
            if !record.supported() {
                let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
                let extension_id = diagnostic_strings.intern(source_kind.extension());
                return Err(imports::unsupported_source_file_kind(
                    dependency_path,
                    extension_id,
                    dependency_span,
                ));
            }
            Ok(ResolvedDependency::SameModuleSource {
                source_index: *source_index,
                consumer_module_id,
            })
        }
        NamespaceEntry::CrossModule {
            target_module_id,
            surface,
        } => Ok(ResolvedDependency::CrossModule {
            provider_module_id: *target_module_id,
            provider_surface: *surface,
            consumer_module_id,
            route: None,
            #[cfg(test)]
            root_file: cross_module_root_file(index, *target_module_id, *surface),
        }),
        NamespaceEntry::PublicRoute {
            target_module_id,
            surface,
            route,
        } => Ok(ResolvedDependency::CrossModule {
            provider_module_id: *target_module_id,
            provider_surface: *surface,
            consumer_module_id,
            route: Some(route.clone()),
            #[cfg(test)]
            root_file: cross_module_root_file(index, *target_module_id, *surface),
        }),
        NamespaceEntry::SameModuleProvider { .. } => Err(imports::missing_import_target(
            dependency_path,
            dependency_span,
        )),
    }
}

/// The provider root handle recorded with one resolved cross-module dependency in tests.
#[cfg(test)]
fn cross_module_root_file(
    index: &SourceTreeIndex,
    target_module_id: ModuleId,
    surface: ModuleRootRole,
) -> PathBuf {
    index
        .module_identities()
        .record(target_module_id)
        .root_file(surface)
        .unwrap_or_else(|| {
            panic!(
                "cross-module namespace selected missing {surface:?} surface for ModuleId {}",
                target_module_id.index()
            )
        })
        .to_path_buf()
}

/// Reject path components that cannot participate in a module-root-relative dependency.
fn reject_invalid_path_components(
    dependency_path: PathId,
    dependency_span: Option<SourceSpan>,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), DiagnosticDraft> {
    let mut scratch = Vec::new();
    let components = path_fork.resolve_components(dependency_path, &mut scratch);
    if components
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
    {
        return Err(imports::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::ParentDirectorySegment,
            dependency_span,
        ));
    }

    if components
        .first()
        .map(|component| string_table.resolve(*component))
        .is_some_and(|component| component == "." || component == "..")
    {
        return Err(imports::bare_file_import(dependency_path, dependency_span));
    }

    Ok(())
}

/// Reject dependency paths that traverse a proven hidden source directory.
///
/// Only non-final components are proven directories: a component followed by another path
/// component must contain that child. The final component may name `_helper.moth`, and a single
/// `_private` is ambiguous without indexed directory facts, so neither is rejected merely for
/// starting with `_`.
fn reject_hidden_directory_components(
    dependency_path: PathId,
    dependency_span: Option<SourceSpan>,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), DiagnosticDraft> {
    let mut scratch = Vec::new();
    let components = path_fork.resolve_components(dependency_path, &mut scratch);
    let Some((_final_component, directory_components)) = components.split_last() else {
        return Ok(());
    };
    if directory_components
        .iter()
        .any(|component| string_table.resolve(*component).starts_with('_'))
    {
        return Err(imports::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::HiddenDirectory,
            dependency_span,
        ));
    }

    Ok(())
}

/// Reject explicit compiler-semantic source extensions.
fn reject_explicit_source_extension(
    dependency_path: PathId,
    dependency_span: Option<SourceSpan>,
    strings: &mut DiagnosticStringBuilder<'_>,
    path_fork: &PathInternerFork,
) -> Result<(), DiagnosticDraft> {
    if let Some(kind) =
        explicit_dependency_source_kind(dependency_path, path_fork, strings.strings())
    {
        let diagnostic = if kind == SourceFileKind::Moth {
            imports::explicit_moth_extension(dependency_path, dependency_span)
        } else {
            let extension_id = strings.intern(kind.extension());
            imports::explicit_source_extension(dependency_path, extension_id, dependency_span)
        };
        return Err(diagnostic);
    }

    Ok(())
}

/// Convert dependency-path components to a portable forward-slash namespace key.
fn portable_dependency_key(
    components: &[crate::compiler_frontend::symbols::string_interning::StringId],
    string_table: &StringTable,
) -> String {
    components
        .iter()
        .map(|component| string_table.resolve(*component))
        .collect::<Vec<_>>()
        .join("/")
}

fn path_contains_dotdot(
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> bool {
    let mut scratch = Vec::new();
    path_fork
        .resolve_components(dependency_path, &mut scratch)
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
}

/// Strip the final component's extension from a portable forward-slash source path.
///
/// WHY: the input is already a logical `/` spelling, so a native `Path` round trip adds nothing.
/// The display conversion it fed also rewrote a literal backslash in a Unix filename into a
/// separator, collapsing distinct logical identities. Only the final `/` component may change; a
/// leading dot with no later dot is a dotfile and keeps its name.
fn extensionless_portable_path(relative_path: &str) -> String {
    let file_name_start = relative_path
        .rfind('/')
        .map_or(0, |separator| separator + 1);
    let file_name = &relative_path[file_name_start..];

    let Some(extension_dot) = file_name.rfind('.') else {
        return relative_path.to_owned();
    };
    if extension_dot == 0 {
        return relative_path.to_owned();
    }

    relative_path[..file_name_start + extension_dot].to_owned()
}

/// Check whether a namespace key is a sub-path of a cross-module entry, indicating a private
/// bypass of a module or support boundary.
fn find_module_bypass_prefix<'a>(
    entries: &'a BTreeMap<String, NamespaceEntry>,
    key: &str,
) -> Option<&'a str> {
    for (entry_key, entry) in entries {
        if !matches!(entry, NamespaceEntry::CrossModule { .. }) {
            continue;
        }
        if key.starts_with(&format!("{entry_key}/")) {
            return Some(entry_key.as_str());
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/module_namespace_tests.rs"]
mod tests;
