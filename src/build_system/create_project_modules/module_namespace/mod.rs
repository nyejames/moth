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
//!
//! Public-route publication lives in the `route_publication` child. It consumes the facade route
//! prepass index, validates `<facade>/<route>` claims against these namespace tables and builds the
//! collision failures. This module keeps namespace construction, resolution and the boundary
//! wrappers that invoke publication. Facade preparation, route claims and their cache belong to
//! `facade_route_prepass` and never move here.

use super::facade_route_prepass::FacadeRouteIndex;
use super::module_identity::{ModuleId, ModuleIdentityTable};
use super::project_module_graph::{ProjectModuleGraph, is_support_visible_in_identity_table};
use super::project_structure_diagnostics::{
    entry_root_package_prefix_collision_error, reserved_package_namespace_root_claim_error,
};
use super::source_loading::SelectedSourceTextMap;
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
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, InvalidImportPathReason, PreparedFailure,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::paths::path_resolution::{
    ProjectPathResolver, explicit_dependency_source_kind,
};
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, PublicExportRoute};
use crate::compiler_frontend::source::{SourceDatabase, SourceSpan};
use crate::compiler_frontend::source_packages::root_file::dependency_path_references_config_file;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

mod route_publication;

pub(super) use route_publication::{
    DeferredPackageRouteCollision, PackageRootFacadeSite, PackageRouteDonorRequest,
    PackageRouteDonorRequests, PackageRouteSources, RoutePublicationFailure,
    deferred_package_collision_failure, resolve_deferred_package_route_site,
};
use route_publication::{
    ProjectRouteClaimSite, RouteCollisionSite, insert_facade_route_entries,
    route_collision_failure, validate_package_route_claims,
    validate_package_route_namespace_collisions, validate_package_route_prefix_collisions,
    validate_published_package_routes_against_consumers,
};

#[cfg(test)]
use route_publication::{
    PlannedRouteEntry, RouteCollision, first_route_collision, planned_route_entries,
    prepared_route_collision_failure,
};

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
    ambiguous_keys: BTreeSet<String>,
    /// Every facade identity inserted behind each consumer-relative key.
    ///
    /// WHAT: retains all `CrossModule` claimants even when the entry table keeps only the first
    ///       spelling. A key with one retained entry can still hide several facades from a
    ///       case-folded or support-basename collision.
    /// WHY: route planning and eager collision validation must inspect every visible facade, not
    ///      merely the structurally retained row that resolution uses as context.
    facade_claimants: BTreeMap<String, Vec<(ModuleId, ModuleRootRole)>>,
    /// Visible facade keys whose own preparation raised a source diagnostic.
    ///
    /// WHAT: consumer-relative keys such as `widgets` whose facade could not prepare.
    /// WHY: a lookup beneath one of these keys must reach the facade's walk-owned diagnostic
    ///      instead of falling through to the route-miss lane at `MOTH-IMPORT-0015`.
    unavailable_facades: BTreeSet<String>,
}

impl ModuleNamespace {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            ambiguous_keys: BTreeSet::new(),
            facade_claimants: BTreeMap::new(),
            unavailable_facades: BTreeSet::new(),
        }
    }

    fn mark_unavailable_facade(&mut self, facade_key: String) {
        self.unavailable_facades.insert(facade_key);
    }

    fn is_under_unavailable_facade(&self, key: &str) -> bool {
        self.unavailable_facades
            .iter()
            .any(|facade_key| is_same_or_nested_path(key, facade_key))
    }

    /// Insert one visible identity without applying precedence.
    ///
    /// Exact and ASCII-case-only collisions mark every colliding spelling ambiguous. The first
    /// entry stays only as structural context for private-boundary detection; it can never win
    /// resolution while its key remains ambiguous.
    fn insert(&mut self, key: String, entry: NamespaceEntry) {
        if let NamespaceEntry::CrossModule {
            target_module_id,
            surface,
        } = &entry
        {
            let claimants = self.facade_claimants.entry(key.clone()).or_default();
            let claimant = (*target_module_id, *surface);
            if !claimants.contains(&claimant) {
                claimants.push(claimant);
            }
        }

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

/// Whether `key` names `prefix` itself or a path nested below it at a `/` boundary.
///
/// Case-sensitive: namespace keys and package prefixes are already folded through their owning
/// case-collision rules, so these lookups never fold spellings a second time.
fn is_same_or_nested_path(key: &str, prefix: &str) -> bool {
    key == prefix
        || key
            .strip_prefix(prefix)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

/// One resolved registered package surface and the public route the consumer named.
///
/// WHY: the exact prefix and the public route are separate ownership facts, so resolution returns
///      them together without reconstructing the prefix from the authored key or searching the
///      package boundary again.
struct RegisteredPackageSurface<'a> {
    dependency_prefix: &'a str,
    route: PublicExportRoute,
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
        /// The exact public route on the package root facade; `ROOT` selects its own surface.
        ///
        /// WHY: the consumer's authored key loses the registered prefix, and the completed
        ///      package interface may expose the same leaf under several routes. Carrying the
        ///      resolved route keeps later binding from re-deriving it from a path it cannot split.
        route: PublicExportRoute,
        #[cfg(test)]
        root_file: PathBuf,
    },
    /// A package facade or registered package boundary whose own preparation failed.
    ///
    /// WHY: the consumer must not turn an unavailable facade into a route-miss diagnostic. The
    ///      facade's boundary walk owns its source diagnostic and reports it at that boundary's
    ///      turn; this variant only keeps discovery from aborting earlier with `0015`.
    UnavailableSourcePackage,
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
    /// Published routes of each registered package boundary's root support facade.
    ///
    /// WHAT: maps a registered package prefix to its root `@package.moth` routes, keyed by their
    ///       one owned canonical spelling and valued by the authored operand span.
    /// WHY: an external consumer names `@<prefix>/<route>`. The route must resolve through the
    ///      exact owning package root, and the package's internal namespaces cannot supply this
    ///      fact because their keys are boundary-relative. Retaining only the root support
    ///      facade's owned spellings keeps internal child-facade routes private to the package.
    ///      No package identity is stored here: the boundary index owns it, and a collision looks
    ///      it up only when the package's report domain is actually needed.
    package_public_routes: BTreeMap<String, BTreeMap<String, SourceSpan>>,
    /// Registered package prefixes whose root support facade or boundary-wide publication failed.
    ///
    /// WHY: a lookup beneath such a prefix belongs to the failing boundary's walk diagnostic, not
    ///      to the private-route or route-miss lanes. A failed internal child facade stays scoped
    ///      to its own namespace key and does not make an unrelated external path unavailable.
    unavailable_source_packages: BTreeSet<String>,
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

/// The outcome of publishing one package boundary's declared facade routes.
///
/// WHY: a collision with a registered package prefix needs that package's authored site, which a
/// later registered boundary has not published yet. Reporting it during attachment would degrade
/// the diagnostic to one site, so the orchestration keeps the collision pending until every
/// package claim exists and reports it at the publisher's turn, as HEAD did.
pub(super) enum PackageRouteAttachment {
    /// Every planned route was published in this boundary.
    Published,
    /// Publication stopped on another registered package's prefix; the caller reports at this
    /// boundary's turn with the competing site resolved after every package's claims are known.
    Deferred(DeferredPackageRouteCollision),
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

    /// The registered package prefixes in deterministic package order.
    ///
    /// WHAT: an owned snapshot of the prefixes, so a caller that also mutates this set can iterate
    ///       every boundary without holding a borrow of the registry.
    /// WHY: package route prepublication prepares each boundary and then attaches its routes to
    ///      this same set. Re-reading the registry after each mutation would either hold a
    ///      forbidden borrow or need a second copy of the registration order.
    pub(crate) fn source_package_prefixes(&self) -> Vec<String> {
        self.package_boundary_indexes
            .iter()
            .map(|(prefix, _)| prefix.to_owned())
            .collect()
    }

    /// One registered package boundary's index, addressed by its exact prefix.
    pub(crate) fn source_package_boundary(&self, package_prefix: &str) -> Option<&SourceTreeIndex> {
        self.package_boundary_indexes.get(package_prefix)
    }

    /// Publish one registered package boundary's declared facade routes.
    ///
    /// WHAT: inserts every route of the boundary's normal and support facades into that boundary's
    ///       module namespaces, then retains the root support facade's routes for external
    ///       `@<prefix>/<route>` selection.
    /// WHY: internal package consumers resolve their own facades exactly as project modules do,
    ///      while external consumers may select only the package's published root surface. Route
    ///      publication happens before either the package walk or project namespaces run, so a
    ///      consumer never sees a partially published boundary.
    pub(super) fn attach_package_facade_routes(
        &mut self,
        package_prefix: &str,
        routes: &FacadeRouteIndex,
        source_files: &SourceDatabase,
        selected_source_texts: &mut SelectedSourceTextMap,
    ) -> Result<PackageRouteAttachment, PreparedFailure> {
        let Self {
            package_namespaces,
            package_boundary_indexes,
            package_public_routes,
            binding_package_paths,
            ..
        } = self;
        let package_index = package_boundary_indexes
            .get(package_prefix)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "facade route prepublication named unregistered source package @{package_prefix}"
                    ),
                )
            })
            .map_err(PreparedFailure::from)?;
        let namespaces = package_namespaces
            .get_mut(package_prefix)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "source package @{package_prefix} has no prepared namespace set for route publication"
                    ),
                )
            })
            .map_err(PreparedFailure::from)?;
        match insert_facade_route_entries(
            namespaces,
            package_index.module_identities(),
            routes,
            package_boundary_indexes,
            binding_package_paths,
            Some(RouteCollisionSite {
                source_tree_index: package_index,
                source_files,
                package_root_facades: None,
                package_public_routes: None,
            }),
            Some(&mut *selected_source_texts),
        ) {
            Ok(None) => {}
            Ok(Some(mut collision)) => {
                if let Some(provider_prefix) = collision.registered_package_prefix.take() {
                    return Ok(PackageRouteAttachment::Deferred(
                        DeferredPackageRouteCollision {
                            publisher_prefix: package_prefix.to_owned(),
                            provider_prefix,
                            route_key: collision.route_key,
                            span: collision.span,
                        },
                    ));
                }
                // Every other collision keeps one authored site in this boundary. That includes a
                // binding-package prefix overlap, which owns no package source site to report.
                return Err(route_collision_failure(collision).into_single_domain_prepared());
            }
            Err(failure) => return Err(failure.into_single_domain_prepared()),
        }

        let Some(root_module_id) = package_index
            .module_identities()
            .module_id_for_directory(package_index.entry_root())
        else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Source package @{package_prefix} has no module rooted at its indexed entry root"
            );
        };
        let root_claims = routes.routes(root_module_id, ModuleRootRole::Support);
        if root_claims.is_empty() {
            return Ok(PackageRouteAttachment::Published);
        }
        // Only this boundary's newly published claims need the internal eager check. Claims from
        // earlier packages have already passed this scan, and the project pass validates every
        // package against project-local identities once project namespaces exist.
        // Only this package's own namespaces share the source database that resolves a matching
        // local provider. Another package's boundary-local ModuleIds and SourceIds are foreign.
        validate_package_route_claims(
            namespaces.iter(),
            package_prefix,
            package_boundary_indexes,
            root_claims
                .iter()
                .map(|claim| (claim.route.as_str(), claim.span)),
            ProjectRouteClaimSite::PackageLocal {
                source_tree_index: package_index,
                source_files,
            },
            Some(selected_source_texts),
        )
        .map_err(RoutePublicationFailure::into_single_domain_prepared)?;
        package_public_routes.insert(
            package_prefix.to_owned(),
            root_claims
                .iter()
                .map(|claim| (claim.route.as_str().to_owned(), claim.span))
                .collect(),
        );
        Ok(PackageRouteAttachment::Published)
    }

    /// Mark every facade of one package boundary as unavailable to its internal consumers.
    ///
    /// WHAT: for each consumer namespace, marks the visible `CrossModule` key that targets a
    ///       facade whose own preparation raised a source diagnostic.
    /// WHY: internal lookups under that key must reach the boundary walk's diagnostic instead of
    ///      being misreported as an undeclared route.
    pub(super) fn mark_package_facades_unavailable(
        &mut self,
        package_prefix: &str,
        unavailable_facades: &[(ModuleId, ModuleRootRole)],
    ) {
        let Some(namespaces) = self.package_namespaces.get_mut(package_prefix) else {
            return;
        };
        for namespace in namespaces {
            let facade_keys = namespace
                .entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    NamespaceEntry::CrossModule {
                        target_module_id,
                        surface,
                    } if unavailable_facades
                        .iter()
                        .any(|(id, role)| id == target_module_id && role == surface) =>
                    {
                        Some(key.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            for key in facade_keys {
                namespace.mark_unavailable_facade(key);
            }
        }
    }

    /// Mark one registered package boundary as unavailable to external consumers.
    ///
    /// WHY: its own prepass failure or a failed facade makes every unresolved lookup beneath the
    ///      prefix a boundary-owned diagnostic rather than a route miss.
    pub(super) fn mark_source_package_unavailable(&mut self, package_prefix: &str) {
        self.unavailable_source_packages
            .insert(package_prefix.to_owned());
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
        namespaces
            .attach_project_namespaces(
                source_tree_index,
                None,
                &BTreeMap::new(),
                None,
                project_module_graph,
                facade_routes,
                string_table,
            )
            .map_err(RoutePublicationFailure::into_single_domain_prepared)?;
        Ok(namespaces)
    }

    /// Whether any prepared package published at least one external route.
    ///
    /// A package set with no routes has no cross-boundary claim to validate, so the caller can
    /// skip both the package source map and every folded collision index.
    pub(super) fn publishes_package_routes(&self) -> bool {
        self.package_public_routes
            .values()
            .any(|routes| !routes.is_empty())
    }

    /// Validate cross-package route claims as soon as every package namespace is known.
    ///
    /// WHAT: checks published package routes against registered package prefixes, registered
    ///       package namespaces and binding paths before project namespaces are attached.
    /// WHY: these claims depend only on package facts, so a collision must not wait for a
    ///      consumer import. Keeping the failure in this walk preserves the established package
    ///      prepass ordering before any project publication failure.
    pub(super) fn validate_package_route_collisions(
        &self,
        package_sources: &PackageRouteSources<'_>,
    ) -> Result<(), RoutePublicationFailure> {
        validate_package_route_prefix_collisions(
            &self.package_public_routes,
            &self.package_boundary_indexes,
            &self.binding_package_paths,
            package_sources,
        )?;
        validate_package_route_namespace_collisions(
            &self.package_public_routes,
            &self.package_namespaces,
            &self.package_boundary_indexes,
            package_sources,
        )
    }

    /// Build the project boundary's namespaces after its facade routes are published.
    ///
    /// WHAT: builds one namespace per project module, inserts the declared public-route entries
    ///       and validates reserved-root claims.
    /// WHY: the caller may need the package boundary namespaces (and therefore package
    ///      diagnostics) before it prepares project facade headers. Keeping source-package and
    ///      project attachment separate preserves the existing package-before-project failure
    ///      ordering while still publishing project routes before project discovery.
    #[allow(
        clippy::too_many_arguments,
        reason = "project attachment keeps the project index, source database, package facade sites, selection map, graph, facade routes and string table as separate inputs"
    )]
    pub(super) fn attach_project_namespaces(
        &mut self,
        source_tree_index: &SourceTreeIndex,
        source_files: Option<&SourceDatabase>,
        package_root_facades: &BTreeMap<String, PackageRootFacadeSite>,
        mut selected_source_texts: Option<&mut SelectedSourceTextMap>,
        project_module_graph: &ProjectModuleGraph,
        facade_routes: &FacadeRouteIndex,
        string_table: &mut StringTable,
    ) -> Result<(), RoutePublicationFailure> {
        self.project_namespaces = build_project_namespaces(source_tree_index, project_module_graph);
        // A reserved-root (`core`/`builder`) claim is the stronger authored failure. Validate it
        // before route publication so a route reaching a registered package cannot mask it.
        validate_reserved_package_namespace_claims(
            &self.project_namespaces,
            source_tree_index,
            string_table,
        )
        .map_err(RoutePublicationFailure::single_domain)?;
        let Self {
            project_namespaces,
            package_boundary_indexes,
            binding_package_paths,
            package_public_routes,
            ..
        } = self;
        let route_collision_site = source_files.map(|source_files| RouteCollisionSite {
            source_tree_index,
            source_files,
            package_root_facades: Some(package_root_facades),
            package_public_routes: Some(package_public_routes),
        });
        if let Some(collision) = insert_facade_route_entries(
            project_namespaces,
            source_tree_index.module_identities(),
            facade_routes,
            package_boundary_indexes,
            binding_package_paths,
            route_collision_site,
            selected_source_texts.as_deref_mut(),
        )? {
            // Project attachment runs after every package inventory, so the competing package
            // site is already known and reports here without deferral.
            return Err(route_collision_failure(collision));
        }
        // Cross-package claims were validated once every package inventory existed, before any
        // project preparation. This attachment only adds project-local consumers.
        let consumers = project_namespaces.iter().collect::<Vec<_>>();
        validate_published_package_routes_against_consumers(
            &consumers,
            package_public_routes,
            package_boundary_indexes,
            ProjectRouteClaimSite::from_source_database(source_tree_index, source_files),
            selected_source_texts,
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
            package_public_routes: BTreeMap::new(),
            unavailable_source_packages: BTreeSet::new(),
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
            namespace
                .entries
                .keys()
                .chain(namespace.ambiguous_keys.iter())
                .map(String::as_str),
            &key,
        )? {
            return Ok(package);
        }

        if namespace.is_under_unavailable_facade(&key) {
            return Ok(ResolvedDependency::UnavailableSourcePackage);
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
            .filter(|package_path| is_same_or_nested_path(provider_path, package_path.as_str()))
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
        let source_package_surface = self.find_source_package_dependency(key);
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

        if let Some(surface) = source_package_surface {
            if local_dependency_keys.any(|entry_key| package_paths_overlap(entry_key, key)) {
                return Err(imports::ambiguous_import_target(
                    dependency_path,
                    dependency_span,
                ));
            }

            #[cfg(test)]
            let root_file = self
                .package_boundary_indexes
                .get(surface.dependency_prefix)
                .and_then(SourceTreeIndex::package_public_root_file)
                .expect("a resolved package surface must name a registered support facade")
                .to_path_buf();

            return Ok(Some(ResolvedDependency::SourcePackageSurface {
                consumer_module_id,
                dependency_prefix: surface.dependency_prefix.to_owned(),
                route: surface.route,
                #[cfg(test)]
                root_file,
            }));
        }

        if self
            .unavailable_source_packages
            .iter()
            .any(|prefix| is_same_or_nested_path(key, prefix))
        {
            return Ok(Some(ResolvedDependency::UnavailableSourcePackage));
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

    /// Match one registered package surface without traversing its private implementation.
    ///
    /// WHAT: accepts the exact registered prefix with its root route, or one exact declared root
    ///       facade route below that prefix. Anything else beneath a prefix stays private.
    /// WHY: the registered prefix is the only ownership fact. A nested suffix is importable only
    ///      when the package root support facade published that exact spelling; there is no
    ///      filesystem probe, case-folded fallback or deeper-path acceptance.
    fn find_source_package_dependency(
        &self,
        provider_path: &str,
    ) -> Option<RegisteredPackageSurface<'_>> {
        for (dependency_prefix, package_index) in self.package_boundary_indexes.iter() {
            if dependency_prefix == provider_path {
                // A boundary without a published support facade exposes no external surface.
                package_index.package_public_root_file()?;
                return Some(RegisteredPackageSurface {
                    dependency_prefix,
                    route: PublicExportRoute::ROOT,
                });
            }
            let Some(route_spelling) = provider_path
                .strip_prefix(dependency_prefix)
                .and_then(|remainder| remainder.strip_prefix('/'))
            else {
                continue;
            };
            let route_is_published = self
                .package_public_routes
                .get(dependency_prefix)
                .is_some_and(|routes| routes.contains_key(route_spelling));
            if !route_is_published || package_index.package_public_root_file().is_none() {
                continue;
            }
            return Some(RegisteredPackageSurface {
                dependency_prefix,
                route: PublicExportRoute::from_canonical_spelling(route_spelling.to_owned()),
            });
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
/// For a same-module source, the source record's `supported` flag is checked so a recognized-but-
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
fn portable_dependency_key(components: &[StringId], string_table: &StringTable) -> String {
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
#[path = "../tests/module_namespace_tests.rs"]
mod tests;
