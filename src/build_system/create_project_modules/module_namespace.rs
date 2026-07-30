//! Boundary-aware module namespaces for indexed source-import resolution.
//!
//! WHAT: builds one `ModuleNamespace` per indexed project or source-package module from Stage 0
//! indexed facts. Each namespace maps extensionless module-relative import paths to explicit
//! tagged entries (same-module source, provider-owned file, cross-module target). Registered
//! binding packages and source-backed package surfaces are explicit shared entries keyed by
//! import prefix.
//! WHY: replaces filesystem candidate probing and public-surface fallback with indexed
//! lookups, keeping boundary identity attached to boundary-local `SourceId`/`ModuleId`. The
//! canonical path leaves the namespace only as the IO/enqueue handle for the current compiler.
//!
//! Resolution rules:
//! - normal compiler-semantic source lookup starts from the importing source's owning module
//!   namespace and resolves bare module/package surfaces explicitly
//! - `@./` and parent traversal are rejected through structured diagnostics
//! - private child/support bypass is rejected through structured diagnostics
//! - no `read_dir`, `exists`, `canonicalize` or fallback-candidate probing runs after the
//!   indexes exist
//! - provider-backed relative forms such as `@./drawing.js` remain separate and are not
//!   broadened by this owner

use super::module_identity::{ModuleId, ModuleIdentityTable};
use super::project_module_graph::{ProjectModuleGraph, is_support_visible_in_identity_table};
use super::source_package_discovery::SourcePackageBoundaryIndexes;
use super::source_tree_index::{
    SourceClassification, SourceId, SourceLogicalIdentity, SourceOwnership, SourceTreeIndex,
};

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidImportPathReason};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::path_normalization::{
    import_contains_dotdot, is_relative_import_path,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source_packages::root_file::{
    import_path_references_config_file, import_path_references_hash_root_file,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One explicit namespace entry for one import path within one module namespace.
///
/// Entries are tagged records with no precedence chain: a same-module source is never
/// interchangeable with a cross-module target, and resolution is a single lookup rather than an
/// ordered fallback.
#[derive(Clone, Debug)]
enum NamespaceEntry {
    /// A source file owned by the same module, resolved by boundary-local `SourceId`.
    SameModuleSource {
        source_id: SourceId,
        source_kind: SourceFileKind,
    },
    /// An explicit provider-owned file in the same module, keyed by its path with extension.
    SameModuleProvider { source_id: SourceId },
    /// A child normal module or visible support package, resolved by boundary-local `ModuleId`.
    CrossModule { target_module_id: ModuleId },
}

/// One boundary-aware namespace for one indexed module.
///
/// WHAT: the pre-computed lookup table from extensionless module-relative import path to one
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

/// The result of resolving one compiler-semantic import through a namespace.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedImport {
    /// A source file in the same module (project or package boundary).
    ///
    /// The traversal queues the canonical path as the IO handle with the indexed source kind.
    /// `source_id` is the boundary-local `SourceId` from the namespace entry, carried directly so
    /// the semantic-set builder records same-owner membership without re-resolving through paths.
    /// `consumer_module_id` is the importing file's owning module inside the active boundary.
    SameModuleSource {
        source_id: SourceId,
        #[cfg(test)]
        canonical_path: PathBuf,
        consumer_module_id: ModuleId,
    },
    /// A cross-module target in the active project or package boundary.
    ///
    /// The traversal inserts a provider-before-consumer edge by `ModuleId` and queues the
    /// target module's root file.
    CrossModule {
        provider_module_id: ModuleId,
        consumer_module_id: ModuleId,
        #[cfg(test)]
        root_file: PathBuf,
    },
    /// A source-backed package facade selected by its registered import prefix.
    SourcePackageSurface {
        consumer_module_id: ModuleId,
        import_prefix: String,
        #[cfg(test)]
        root_file: PathBuf,
    },
    /// A registered binding-backed package handled by frontend import binding.
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
/// fallback on the migrated directory production path.
pub(crate) struct ModuleNamespaceSet {
    /// One namespace per project module, indexed by project `ModuleId`.
    project_namespaces: Vec<ModuleNamespace>,
    /// One set of namespaces per source-package boundary, keyed by import prefix.
    /// Each inner `Vec` is indexed by the package boundary's local `ModuleId`.
    package_namespaces: BTreeMap<String, Vec<ModuleNamespace>>,
    /// Registered Core, Builder and dependency package paths without their leading `@`.
    binding_package_paths: BTreeSet<String>,
    /// Retained source-package boundary indexes for the lifetime of directory Stage 0.
    package_boundary_indexes: SourcePackageBoundaryIndexes,
}

/// Borrowed directory-project namespace context used by canonical header-owned discovery.
///
/// The namespace set and project index form one lookup authority: the set resolves boundary-local
/// identities and the index supplies the project importer's ownership plus canonical IO handles.
/// Single-file synthetic compilation carries no value of this type.
#[derive(Clone, Copy)]
pub(crate) struct DirectoryImportResolution<'a> {
    namespace_set: &'a ModuleNamespaceSet,
    source_tree_index: &'a SourceTreeIndex,
    boundary: NamespaceBoundary,
    package_prefix: Option<&'a str>,
}

impl<'a> DirectoryImportResolution<'a> {
    pub(crate) fn project(
        namespace_set: &'a ModuleNamespaceSet,
        source_tree_index: &'a SourceTreeIndex,
    ) -> Self {
        Self {
            namespace_set,
            source_tree_index,
            boundary: NamespaceBoundary::Project,
            package_prefix: None,
        }
    }

    pub(crate) fn package(
        namespace_set: &'a ModuleNamespaceSet,
        import_prefix: &'a str,
        source_tree_index: &'a SourceTreeIndex,
    ) -> Self {
        Self {
            namespace_set,
            source_tree_index,
            boundary: NamespaceBoundary::Package,
            package_prefix: Some(import_prefix),
        }
    }

    pub(crate) fn resolve_import(
        self,
        provider: &crate::compiler_frontend::paths::const_paths::StructuralProviderReference,
        importing_canonical_path: &Path,
        string_table: &mut StringTable,
    ) -> Result<ResolvedImport, CompilerDiagnostic> {
        self.namespace_set.resolve_import(
            provider,
            importing_canonical_path,
            self.source_tree_index,
            self.boundary,
            self.package_prefix,
            string_table,
        )
    }

    pub(crate) fn has_binding_package_import(
        self,
        import_path: &InternedPath,
        string_table: &StringTable,
    ) -> bool {
        self.namespace_set
            .binding_package_prefix(import_path, string_table)
            .is_some()
    }

    /// The boundary `SourceTreeIndex` used by canonical module preparation.
    pub(crate) fn source_tree_index(&self) -> &SourceTreeIndex {
        self.source_tree_index
    }

    pub(crate) fn resolve_provider_target(
        self,
        provider_path: &InternedPath,
        importing_canonical_path: &Path,
        import_location: &SourceLocation,
        string_table: &mut StringTable,
    ) -> Result<PathBuf, CompilerDiagnostic> {
        self.namespace_set.resolve_provider_target(
            provider_path,
            importing_canonical_path,
            import_location,
            self.source_tree_index,
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
    pub(crate) fn build(
        source_tree_index: &SourceTreeIndex,
        project_module_graph: &ProjectModuleGraph,
        package_boundary_indexes: SourcePackageBoundaryIndexes,
        binding_packages: &ExternalPackageRegistry,
    ) -> Self {
        let project_namespaces = build_project_namespaces(source_tree_index, project_module_graph);

        let package_namespaces = build_package_namespaces(&package_boundary_indexes);
        let binding_package_paths = binding_packages
            .package_paths()
            .map(|path| path.strip_prefix('@').unwrap_or(path).to_owned())
            .collect();

        Self {
            project_namespaces,
            package_namespaces,
            binding_package_paths,
            package_boundary_indexes,
        }
    }

    /// Resolve one compiler-semantic import through the boundary-aware namespace.
    ///
    /// WHAT: determines the importing file's boundary and owning module from the project or
    /// package index, rejects obsolete `@./` and parent traversal, rejects explicit source
    /// extensions, resolves source-backed package surfaces, and looks up the import path in
    /// the owning module's namespace. Returns a tagged `ResolvedImport` carrying the canonical
    /// path as the IO handle and the boundary-local `ModuleId` for project graph edges.
    /// WHY: replaces the legacy filesystem source-surface fallback and candidate probing with
    /// indexed facts.
    pub(crate) fn resolve_import(
        &self,
        provider: &crate::compiler_frontend::paths::const_paths::StructuralProviderReference,
        importing_canonical_path: &Path,
        source_tree_index: &SourceTreeIndex,
        boundary: NamespaceBoundary,
        package_prefix: Option<&str>,
        string_table: &mut StringTable,
    ) -> Result<ResolvedImport, CompilerDiagnostic> {
        let import_path = &provider.path;
        let import_location = &provider.path_location;
        reject_invalid_path_components(import_path, import_location, string_table)?;
        reject_direct_special_file_import(provider, string_table)?;
        reject_explicit_source_extension(import_path, import_location, string_table)?;

        let full_components = provider.path.as_components();
        let prefix_components = structural_provider_components(provider);
        let source_id = source_tree_index
            .source_id_for_canonical_path(importing_canonical_path)
            .ok_or_else(|| {
                CompilerDiagnostic::missing_import_target(
                    import_path.clone(),
                    import_location.clone(),
                )
            })?;
        let SourceOwnership::Owned(consumer_module_id) =
            source_tree_index.source(source_id).ownership()
        else {
            return Err(CompilerDiagnostic::missing_import_target(
                import_path.clone(),
                import_location.clone(),
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
        let index = source_tree_index;

        let key = portable_import_key(prefix_components, string_table);

        if namespace.is_ambiguous(&key) {
            return Err(CompilerDiagnostic::ambiguous_import_target(
                import_path.clone(),
                import_location.clone(),
            ));
        }

        let source_package_surface = self.find_source_package_surface(&key);
        let binding_package_prefix = self.binding_package_prefix_for_key(&key);

        if source_package_surface.is_some() && binding_package_prefix.is_some() {
            return Err(CompilerDiagnostic::ambiguous_import_target(
                import_path.clone(),
                import_location.clone(),
            ));
        }

        if let Some(binding_prefix) = binding_package_prefix {
            if namespace_conflicts_with_package_prefix(namespace, binding_prefix) {
                return Err(CompilerDiagnostic::ambiguous_import_target(
                    import_path.clone(),
                    import_location.clone(),
                ));
            }

            return Ok(ResolvedImport::BindingPackage);
        }

        if let Some((import_prefix, _root_file)) = source_package_surface {
            if namespace_conflicts_with_package_prefix(namespace, &key) {
                return Err(CompilerDiagnostic::ambiguous_import_target(
                    import_path.clone(),
                    import_location.clone(),
                ));
            }

            return Ok(ResolvedImport::SourcePackageSurface {
                consumer_module_id,
                import_prefix: import_prefix.to_owned(),
                #[cfg(test)]
                root_file: _root_file.to_path_buf(),
            });
        }

        if self.is_source_package_private_path(&key) {
            return Err(CompilerDiagnostic::cross_module_import_not_exported(
                import_path.clone(),
                import_location.clone(),
            ));
        }

        // A grouped item under an owned directory may name its source file directly. Package
        // and prefix collisions have already been classified above, so an exact same-module
        // source cannot win through lookup order.
        if provider.from_grouped {
            let complete_key = portable_import_key(full_components, string_table);
            if namespace.is_ambiguous(&complete_key) {
                return Err(CompilerDiagnostic::ambiguous_import_target(
                    import_path.clone(),
                    import_location.clone(),
                ));
            }
            if let Some(entry) = namespace.entries.get(&complete_key)
                && matches!(entry, NamespaceEntry::SameModuleSource { .. })
            {
                return resolve_entry(
                    entry,
                    index,
                    consumer_module_id,
                    import_path,
                    import_location,
                    string_table,
                );
            }
        }

        if let Some(entry) = namespace.entries.get(&key) {
            return resolve_entry(
                entry,
                index,
                consumer_module_id,
                import_path,
                import_location,
                string_table,
            );
        }

        if find_module_bypass_prefix(&namespace.entries, &key).is_some() {
            return Err(CompilerDiagnostic::cross_module_import_not_exported(
                import_path.clone(),
                import_location.clone(),
            ));
        }

        Err(CompilerDiagnostic::missing_import_target(
            import_path.clone(),
            import_location.clone(),
        ))
    }

    fn binding_package_prefix<'a>(
        &'a self,
        import_path: &InternedPath,
        string_table: &StringTable,
    ) -> Option<&'a str> {
        let key = portable_import_key(import_path.as_components(), string_table);
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
        provider_path: &InternedPath,
        importing_canonical_path: &Path,
        import_location: &SourceLocation,
        project_source_tree_index: &SourceTreeIndex,
        string_table: &mut StringTable,
    ) -> Result<PathBuf, CompilerDiagnostic> {
        if import_contains_dotdot(provider_path, string_table) {
            return Err(CompilerDiagnostic::invalid_import_path(
                provider_path.clone(),
                InvalidImportPathReason::ParentDirectorySegment,
                import_location.clone(),
            ));
        }

        let (namespace, index, importer_relative_path) = match project_source_tree_index
            .source_id_for_canonical_path(importing_canonical_path)
        {
            Some(source_id) => {
                let importer_record = project_source_tree_index.source(source_id);
                let SourceOwnership::Owned(module_id) = importer_record.ownership() else {
                    return Err(CompilerDiagnostic::missing_import_target(
                        provider_path.clone(),
                        import_location.clone(),
                    ));
                };
                (
                    &self.project_namespaces[module_id.index()],
                    project_source_tree_index,
                    owned_relative_source_path(importer_record),
                )
            }
            None => {
                let (package_prefix, package_index, module_id) = self
                    .find_package_namespace_owner(importing_canonical_path)
                    .ok_or_else(|| {
                        CompilerDiagnostic::missing_import_target(
                            provider_path.clone(),
                            import_location.clone(),
                        )
                    })?;
                let namespaces = self
                    .package_namespaces
                    .get(package_prefix)
                    .expect("package prefix found by find_package_namespace_owner");
                let source_id = package_index
                    .source_id_for_canonical_path(importing_canonical_path)
                    .expect("package owner lookup found the importing source");
                (
                    &namespaces[module_id.index()],
                    package_index,
                    owned_relative_source_path(package_index.source(source_id)),
                )
            }
        };

        let key = provider_import_key(provider_path, importer_relative_path, string_table);
        if namespace.is_ambiguous(&key) {
            return Err(CompilerDiagnostic::ambiguous_import_target(
                provider_path.clone(),
                import_location.clone(),
            ));
        }

        match namespace.entries.get(&key) {
            Some(NamespaceEntry::SameModuleProvider { source_id }) => {
                Ok(index.source(*source_id).canonical_path().to_path_buf())
            }
            Some(NamespaceEntry::CrossModule { .. })
            | Some(NamespaceEntry::SameModuleSource { .. }) => {
                Err(CompilerDiagnostic::cross_module_import_not_exported(
                    provider_path.clone(),
                    import_location.clone(),
                ))
            }
            None if find_module_bypass_prefix(&namespace.entries, &key).is_some() => {
                Err(CompilerDiagnostic::cross_module_import_not_exported(
                    provider_path.clone(),
                    import_location.clone(),
                ))
            }
            None => Err(CompilerDiagnostic::missing_import_target(
                provider_path.clone(),
                import_location.clone(),
            )),
        }
    }

    /// Check whether the complete provider path matches a source-backed package prefix and return
    /// the package's root file.
    fn find_source_package_surface(&self, provider_path: &str) -> Option<(&str, &Path)> {
        for (import_prefix, package_index) in self.package_boundary_indexes.iter() {
            if import_prefix == provider_path {
                return package_index
                    .root_file_for_entry_root()
                    .map(|root_file| (import_prefix, root_file));
            }
        }
        None
    }

    /// Whether an import tries to traverse below a registered source-package facade.
    fn is_source_package_private_path(&self, provider_path: &str) -> bool {
        self.package_boundary_indexes
            .iter()
            .any(|(import_prefix, _)| {
                provider_path
                    .strip_prefix(import_prefix)
                    .is_some_and(|remainder| remainder.starts_with('/'))
            })
    }

    /// Find the package boundary, index and owning module for a canonical importing path.
    fn find_package_namespace_owner(
        &self,
        canonical_path: &Path,
    ) -> Option<(&str, &SourceTreeIndex, ModuleId)> {
        for (import_prefix, package_index) in self.package_boundary_indexes.iter() {
            if let Some(source_id) = package_index.source_id_for_canonical_path(canonical_path) {
                let ownership = package_index.source(source_id).ownership();
                if let SourceOwnership::Owned(module_id) = ownership {
                    return Some((import_prefix, package_index, module_id));
                }
            }
        }
        None
    }
}

/// Return the provider-prefix components for one parsed import item.
///
/// Grouped import paths retain their requested item as the last component. Stage 0 resolves the
/// complete grouped path against the namespace first (see `resolve_import`): when it denotes an
/// indexed same-module compiler-semantic source the item file is queued directly, otherwise the
/// provider prefix returned here selects a module, source-package or binding-package facade.
/// Non-grouped paths are already provider paths and are returned unchanged.
fn structural_provider_components(
    provider: &crate::compiler_frontend::paths::const_paths::StructuralProviderReference,
) -> &[crate::compiler_frontend::symbols::string_interning::StringId] {
    let components = provider.path.as_components();
    if provider.from_grouped {
        &components[..components.len().saturating_sub(1)]
    } else {
        components
    }
}

// ---------------------------------------------------------------------------
// Namespace construction
// ---------------------------------------------------------------------------

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

/// Build one `ModuleNamespace` set per source-package boundary from the retained package indexes.
fn build_package_namespaces(
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
) -> BTreeMap<String, Vec<ModuleNamespace>> {
    let mut package_namespaces: BTreeMap<String, Vec<ModuleNamespace>> = BTreeMap::new();

    for (import_prefix, package_index) in package_boundary_indexes.iter() {
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

        package_namespaces.insert(import_prefix.to_owned(), namespaces);
    }

    package_namespaces
}

/// Add same-module source entries for one module from its owned source IDs.
///
/// Each owned source's extensionless module-relative logical path becomes a namespace key.
/// Root files (names starting with `#` or `+`) are excluded because direct root imports are
/// rejected.
fn populate_same_module_entries(
    namespace: &mut ModuleNamespace,
    index: &SourceTreeIndex,
    module_id: ModuleId,
) {
    for source_id in index.owned_source_ids(module_id) {
        let record = index.source(*source_id);
        let SourceLogicalIdentity::Owned(owned_identity) = record.logical_identity() else {
            continue;
        };
        let relative_path = owned_identity.relative_source_path();

        let file_name = Path::new(relative_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if file_name.starts_with('#') || file_name.starts_with('+') {
            continue;
        }

        match record.classification() {
            SourceClassification::CompilerSemantic(source_kind) => namespace.insert(
                extensionless_portable_path(relative_path),
                NamespaceEntry::SameModuleSource {
                    source_id: *source_id,
                    source_kind: *source_kind,
                },
            ),
            SourceClassification::ProviderOwned(_) => namespace.insert(
                relative_path.to_owned(),
                NamespaceEntry::SameModuleProvider {
                    source_id: *source_id,
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
    let owning_path = identities
        .record(module_id)
        .stable_origin()
        .logical_module_path()
        .to_owned();

    for child_id in identities.direct_child_modules(module_id) {
        if identities.record(*child_id).role() != ModuleRootRole::Normal {
            continue;
        }

        if let Some(key) = relative_module_key(&owning_path, identities, *child_id) {
            namespace.insert(
                key,
                NamespaceEntry::CrossModule {
                    target_module_id: *child_id,
                },
            );
        }
    }

    for support_id in identities.module_ids() {
        if support_id == module_id {
            continue;
        }
        if !project_module_graph.is_support_visible_to_consumer(support_id, module_id) {
            continue;
        }
        if let Some(key) = support_package_key(identities, support_id) {
            namespace.insert(
                key,
                NamespaceEntry::CrossModule {
                    target_module_id: support_id,
                },
            );
        }
    }
}

/// Add cross-module target entries for one package module: direct child modules and visible
/// support packages, using the package's identity table for support visibility.
fn populate_package_cross_module_targets(
    namespace: &mut ModuleNamespace,
    identities: &ModuleIdentityTable,
    module_id: ModuleId,
) {
    let owning_path = identities
        .record(module_id)
        .stable_origin()
        .logical_module_path()
        .to_owned();

    for child_id in identities.direct_child_modules(module_id) {
        if identities.record(*child_id).role() != ModuleRootRole::Normal {
            continue;
        }

        if let Some(key) = relative_module_key(&owning_path, identities, *child_id) {
            namespace.insert(
                key,
                NamespaceEntry::CrossModule {
                    target_module_id: *child_id,
                },
            );
        }
    }

    for support_id in identities.module_ids() {
        if support_id == module_id {
            continue;
        }
        if !is_support_visible_in_identity_table(identities, support_id, module_id) {
            continue;
        }
        if let Some(key) = support_package_key(identities, support_id) {
            namespace.insert(
                key,
                NamespaceEntry::CrossModule {
                    target_module_id: support_id,
                },
            );
        }
    }
}

/// Compute the extensionless module-relative import key for one target module, relative to the
/// owning module's logical path.
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
// Import resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a namespace entry to a `ResolvedImport`, looking up the canonical path and root file
/// from the owning boundary's `SourceTreeIndex`.
///
/// For a same-module source, the entry's `supported` flag is checked so a recognized-but-
/// unsupported extension (for example `.mtf` without builder registration) produces the same
/// `UnsupportedSourceFileKind` diagnostic the old filesystem-backed resolver produced, without
/// a filesystem probe.
fn resolve_entry(
    entry: &NamespaceEntry,
    index: &SourceTreeIndex,
    consumer_module_id: ModuleId,
    import_path: &InternedPath,
    import_location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<ResolvedImport, CompilerDiagnostic> {
    match entry {
        NamespaceEntry::SameModuleSource {
            source_id,
            source_kind,
        } => {
            let record = index.source(*source_id);
            if !record.supported() {
                let extension_id = string_table.intern(source_kind.extension());
                return Err(CompilerDiagnostic::unsupported_source_file_kind(
                    import_path.clone(),
                    extension_id,
                    import_location.clone(),
                ));
            }
            Ok(ResolvedImport::SameModuleSource {
                source_id: *source_id,
                #[cfg(test)]
                canonical_path: record.canonical_path().to_path_buf(),
                consumer_module_id,
            })
        }
        NamespaceEntry::CrossModule { target_module_id } => {
            #[cfg(test)]
            let root_file = index
                .module_identities()
                .record(*target_module_id)
                .root_file()
                .to_path_buf();
            Ok(ResolvedImport::CrossModule {
                provider_module_id: *target_module_id,
                consumer_module_id,
                #[cfg(test)]
                root_file,
            })
        }
        NamespaceEntry::SameModuleProvider { .. } => Err(
            CompilerDiagnostic::missing_import_target(import_path.clone(), import_location.clone()),
        ),
    }
}

/// Reject direct module-root and config paths before namespace absence can change the diagnostic.
fn reject_direct_special_file_import(
    provider: &crate::compiler_frontend::paths::const_paths::StructuralProviderReference,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    if import_path_references_hash_root_file(&provider.path, provider.from_grouped, string_table)
        || import_path_references_config_file(&provider.path, provider.from_grouped, string_table)
    {
        return Err(CompilerDiagnostic::direct_special_file_import(
            provider.path.clone(),
            provider.path_location.clone(),
        ));
    }

    Ok(())
}

/// Reject path components that cannot participate in a module-root-relative import.
fn reject_invalid_path_components(
    import_path: &InternedPath,
    import_location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), CompilerDiagnostic> {
    if import_contains_dotdot(import_path, string_table) {
        return Err(CompilerDiagnostic::invalid_import_path(
            import_path.clone(),
            InvalidImportPathReason::ParentDirectorySegment,
            import_location.clone(),
        ));
    }

    if is_relative_import_path(import_path, string_table) {
        return Err(CompilerDiagnostic::bare_file_import(
            import_path.clone(),
            import_location.clone(),
        ));
    }

    Ok(())
}

/// Reject explicit compiler-semantic source extensions after direct special files are classified.
fn reject_explicit_source_extension(
    import_path: &InternedPath,
    import_location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), CompilerDiagnostic> {
    if let Some(extension) = explicit_source_extension(import_path, string_table) {
        let diagnostic = if extension == SourceFileKind::Moth.extension() {
            CompilerDiagnostic::explicit_moth_extension(
                import_path.clone(),
                import_location.clone(),
            )
        } else {
            let extension_id = string_table.intern(&extension);
            CompilerDiagnostic::explicit_source_extension(
                import_path.clone(),
                extension_id,
                import_location.clone(),
            )
        };
        return Err(diagnostic);
    }

    Ok(())
}

/// Check whether any path component carries a recognised source-file extension.
fn explicit_source_extension(
    import_path: &InternedPath,
    string_table: &StringTable,
) -> Option<String> {
    for component in import_path.as_components() {
        let segment = string_table.resolve(*component);
        let Some(extension) = Path::new(segment)
            .extension()
            .and_then(|extension| extension.to_str())
        else {
            continue;
        };
        if SourceFileKind::from_extension(extension).is_some() {
            return Some(extension.to_owned());
        }
    }
    None
}

/// Convert import-path components to a portable forward-slash namespace key.
fn portable_import_key(
    components: &[crate::compiler_frontend::symbols::string_interning::StringId],
    string_table: &StringTable,
) -> String {
    components
        .iter()
        .map(|component| string_table.resolve(*component))
        .collect::<Vec<_>>()
        .join("/")
}

/// Convert a provider-owned path to its module-relative namespace key.
///
/// Provider files retain their explicit extension and may use the dedicated leading `./` form;
/// that marker selects the current module but is not part of the indexed key.
fn provider_import_key(
    import_path: &InternedPath,
    importer_relative_path: &str,
    string_table: &StringTable,
) -> String {
    let components = import_path
        .as_components()
        .iter()
        .map(|component| string_table.resolve(*component))
        .collect::<Vec<_>>();

    if components
        .first()
        .is_some_and(|component| *component == ".")
    {
        let relative_target = components[1..].join("/");
        return Path::new(importer_relative_path)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(relative_target)
            .to_string_lossy()
            .replace('\\', "/");
    }

    components.join("/")
}

fn owned_relative_source_path(record: &super::source_tree_index::SourceRecord) -> &str {
    match record.logical_identity() {
        SourceLogicalIdentity::Owned(identity) => identity.relative_source_path(),
        SourceLogicalIdentity::Unrooted(_) => "",
    }
}

/// Strip the file extension from a portable forward-slash source path.
fn extensionless_portable_path(relative_path: &str) -> String {
    let path = Path::new(relative_path);
    path.with_extension("").to_string_lossy().replace('\\', "/")
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

fn namespace_conflicts_with_package_prefix(
    namespace: &ModuleNamespace,
    package_prefix: &str,
) -> bool {
    let folded_package = package_prefix.to_ascii_lowercase();
    namespace.entries.keys().any(|entry_key| {
        let folded_entry = entry_key.to_ascii_lowercase();
        folded_entry == folded_package
            || folded_entry.starts_with(&format!("{folded_package}/"))
            || folded_package.starts_with(&format!("{folded_entry}/"))
    })
}
