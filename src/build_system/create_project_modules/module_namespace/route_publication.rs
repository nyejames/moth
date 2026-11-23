//! Public-export route publication for boundary-aware module namespaces.
//!
//! WHAT: validates and publishes every declared facade route (`<facade>/<route>`) into the
//! consumer module namespaces, and builds the collision failures that report route claims.
//! WHY: this is the one publication step between the facade route prepass and namespace
//! resolution. Keeping it beside the namespace resolver lets both use the same entry and
//! ambiguity tables without exposing a second route registry.

use super::super::facade_route_prepass::FacadeRouteIndex;
use super::super::module_identity::{ModuleId, ModuleIdentityTable};
use super::super::source_loading::SelectedSourceTextMap;
use super::super::source_package_discovery::SourcePackageBoundaryIndexes;
use super::super::source_tree_index::SourceTreeIndex;
use super::{ModuleNamespace, ModuleNamespaceSet, NamespaceEntry};

use crate::builder_surface::package_metadata::package_paths_overlap;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{imports, token_lexical};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticBag, PreparedFailure, SourceSpanCapacityResource,
};
use crate::compiler_frontend::headers::public_export_route_conflicts::{
    fold_route_spelling, try_intern_route_spelling,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, PublicExportRoute, StablePackageIdentity,
};
use crate::compiler_frontend::source::{
    FrozenIdentityHandle, LocalSpan, SourceDatabase, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One planned public-route namespace entry for a consumer module.
pub(super) struct PlannedRouteEntry {
    pub(super) key: String,
    /// The facade base key every route under this claim is scoped to, folded for comparison.
    ///
    /// WHY: two routes with one folded key are latent only when they sit behind the same
    ///      ambiguous base. Distinct bases own independent visibility and must collide eagerly.
    pub(super) base_key: String,
    pub(super) facade_module_id: ModuleId,
    pub(super) surface: ModuleRootRole,
    pub(super) route: PublicExportRoute,
    pub(super) span: SourceSpan,
}

/// Route claims that fold to one consumer-visible key, split by publication status.
///
/// WHY: selectable and ambiguous-base claims obey different collision rules. Two selectable
///      facades conflict with each other, a selectable claim conflicts with any ambiguous claim,
///      and ambiguous claims conflict with one another only when their folded base keys differ.
#[derive(Default)]
struct RouteClaimGroup<'plan> {
    planned: Vec<&'plan PlannedRouteEntry>,
    ambiguous: Vec<&'plan PlannedRouteEntry>,
}

#[derive(Clone, Debug)]
pub(super) struct RouteCollision {
    pub(super) route_key: String,
    pub(super) span: SourceSpan,
    /// The competing facade's authored site when the collision has two content sites.
    ///
    /// A registered package prefix collision uses its published route operand when one matches,
    /// otherwise its root facade start.
    pub(super) related_span: Option<SourceSpan>,
    /// The registered package whose authored root facade supplies `related_span`.
    ///
    /// Present only when a local route overlaps a registered source-package prefix and that
    /// package retained a root facade site. The report then needs the package source database and
    /// stable identity as an explicit frozen domain instead of resolving the facade's
    /// boundary-local `SourceId` in the project database.
    pub(super) related_package: Option<PackageRouteCollisionSite>,
    /// The registered source-package prefix competing with this route, when one overlaps.
    ///
    /// WHAT: the deterministic first overlapping prefix, independent of whether its root facade
    ///       site is available yet.
    /// WHY: a package boundary attaches its routes before later packages publish theirs. Retaining
    ///      the prefix lets the publisher report at its own turn with the competing site resolved
    ///      once every package claim is known, instead of degrading to a one-site report.
    ///      Binding-backed packages own no authored site and leave this `None`.
    pub(super) registered_package_prefix: Option<String>,
}

/// A package-internal route that overlaps another registered package's namespace prefix.
///
/// WHAT: keeps the authored `$export` operand and both participating prefixes after package route
///       attachment stops.
/// WHY: registered packages prepare in prefix order, so the competing package's root facade and
///      published routes may not exist yet. The publisher reports the collision at its own turn,
///      as HEAD did, and only then resolves the competing site.
pub(in crate::build_system::create_project_modules) struct DeferredPackageRouteCollision {
    pub(in crate::build_system::create_project_modules) publisher_prefix: String,
    pub(in crate::build_system::create_project_modules) provider_prefix: String,
    pub(in crate::build_system::create_project_modules) route_key: String,
    pub(in crate::build_system::create_project_modules) span: SourceSpan,
}

/// One registered package boundary whose authored site competes with a local route.
///
/// `span` is the exact authored site used for the previous-claim label: the published route
/// operand when one collides, otherwise the root facade start. `root_facade` carries the file the
/// donor finalizer must load in that package's database, which is needed only for the facade-start
/// fallback because discovery has already selected a published route operand's file.
#[derive(Clone, Debug)]
pub(super) struct PackageRouteCollisionSite {
    pub(super) dependency_prefix: String,
    pub(super) identity: StablePackageIdentity,
    pub(super) root_facade: Option<PathBuf>,
    pub(super) span: SourceSpan,
}

/// The source facts needed to resolve a registered package's authored root facade later.
#[derive(Clone, Debug)]
pub(in crate::build_system::create_project_modules) struct PackageRootFacadeSite {
    pub(super) identity: StablePackageIdentity,
    pub(super) path: PathBuf,
    pub(super) span: SourceSpan,
}

impl PackageRootFacadeSite {
    /// The root facade's canonical path, used as the donor's selected site when the collision is
    /// with the package prefix rather than one published `$export` operand.
    pub(in crate::build_system::create_project_modules) fn path(&self) -> &Path {
        &self.path
    }

    /// Retain one indexed package's root facade while its source slot is available.
    ///
    /// WHY: project route attachment runs after package preparation but before package walks, so
    ///      the root facade's package-local `SourceId` must be captured from the package source
    ///      owner instead of reinterpreted in the project database later.
    pub(in crate::build_system::create_project_modules) fn from_package(
        package_index: &SourceTreeIndex,
        identity: StablePackageIdentity,
        source_files: &SourceDatabase,
    ) -> Option<Self> {
        let root_facade = package_index.package_public_root_file()?;
        let slot = source_files.get_by_canonical_path(root_facade)?;
        Some(Self {
            identity,
            path: root_facade.to_path_buf(),
            span: SourceSpan::new(slot.id, LocalSpan::source_start()),
        })
    }
}

/// The package boundary that must render one collision label through its own source database.
///
/// WHAT: retains the colliding published package's registered prefix and stable identity.
/// WHY: the project premerge report resolves the competing provider in the project source
///      database, but the package `$export` operand carries a package-local `SourceId`. The
///      report therefore needs the package's finished source database and stable identity as an
///      explicit foreign domain instead of reinterpreting that ID in the project database.
#[derive(Clone, Debug)]
pub(in crate::build_system::create_project_modules) struct PackageRouteDonorRequest {
    pub(in crate::build_system::create_project_modules) dependency_prefix: String,
    pub(in crate::build_system::create_project_modules) identity: StablePackageIdentity,
    /// One boundary-local source the donor must load and finalize before freezing the report.
    ///
    /// A provider namespace entry can be absent from the package's selected-source set, so its
    /// slot exists but has no line index yet. The donor finish loads this authored file into the
    /// package's own source database. Route operands are already selected and leave this `None`.
    pub(in crate::build_system::create_project_modules) selected_site: Option<PathBuf>,
}

/// The package boundaries that must render a two-site collision through their own databases.
///
/// WHAT: distinguishes the boundary owning the diagnostic primary span from the boundary owning
///       its previous-site label. A project collision needs only the label domain; a collision
///       between two registered packages needs both.
/// WHY: each site carries a package-local `SourceId`. A report can mix those identities only by
///      registering the exact source database that owns each one as a frozen report domain; no
///      registry is needed because a route collision has at most two authored sites.
#[derive(Default)]
pub(in crate::build_system::create_project_modules) struct PackageRouteDonorRequests {
    pub(in crate::build_system::create_project_modules) primary: Option<PackageRouteDonorRequest>,
    pub(in crate::build_system::create_project_modules) related: Option<PackageRouteDonorRequest>,
}

/// A route-publication failure and the package source owners a cross-boundary report needs.
pub(in crate::build_system::create_project_modules) struct RoutePublicationFailure {
    pub(in crate::build_system::create_project_modules) failure: PreparedFailure,
    /// Present only when `failure` names a site in one or two foreign package boundaries.
    pub(in crate::build_system::create_project_modules) package_donors:
        Option<Box<PackageRouteDonorRequests>>,
    /// Prepared package boundaries whose route claims participate in this collision.
    pub(in crate::build_system::create_project_modules) affected_prefixes: Vec<String>,
}

impl RoutePublicationFailure {
    pub(super) fn single_domain(failure: PreparedFailure) -> Self {
        Self {
            failure,
            package_donors: None,
            affected_prefixes: Vec::new(),
        }
    }

    fn single_domain_in(affected_prefixes: Vec<String>, failure: PreparedFailure) -> Self {
        Self {
            failure,
            package_donors: None,
            affected_prefixes,
        }
    }

    /// A diagnosed collision whose report must resolve one or two package-owned sites.
    ///
    /// The affected prefixes are the donor domains in their reporting order, so they cannot skew
    /// from the requests that the caller will later finalize.
    fn with_package_donors(
        failure: PreparedFailure,
        package_donors: PackageRouteDonorRequests,
    ) -> Self {
        let affected_prefixes = [&package_donors.primary, &package_donors.related]
            .into_iter()
            .flatten()
            .map(|donor| donor.dependency_prefix.clone())
            .collect();
        Self {
            failure,
            package_donors: Some(Box::new(package_donors)),
            affected_prefixes,
        }
    }

    /// Collapse a failure whose report owns one source database.
    ///
    /// WHY: package-boundary publication and the test-only namespace builder resolve every
    ///      collision site in one database. A donor request here would mean the project claim
    ///      site was selected outside project attachment, which is an orchestration invariant.
    pub(super) fn into_single_domain_prepared(self) -> PreparedFailure {
        match self.package_donors {
            None => self.failure,
            Some(donors) => {
                let prefix = donors
                    .primary
                    .as_ref()
                    .or(donors.related.as_ref())
                    .map(|donor| donor.dependency_prefix.as_str())
                    .unwrap_or("<unresolved>");
                PreparedFailure::from(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "route collision in @{prefix} carried a foreign package site into a single-database report"
                    ),
                ))
            }
        }
    }
}

/// Borrowed source databases for the registered package boundaries under attachment.
///
/// WHAT: maps each registered prefix to the database that owns its boundary-local `SourceId`s.
/// WHY: cross-package route validation must resolve another package's namespace entry in that
///      package's own database. The project report never translates those IDs; the map only
///      borrows the package inventories during eager route validation.
pub(in crate::build_system::create_project_modules) type PackageRouteSources<'a> =
    BTreeMap<&'a str, &'a SourceDatabase>;

/// Insert every declared route of every visible facade into one boundary's module namespaces.
///
/// WHAT: for each consumer namespace, plans `<facade-key>/<route>` entries only for facades whose
///       own `CrossModule` entry is present, rejects a collision involving any route even when no
///       consumer references it, then inserts the surviving planned entries directly.
/// WHY: public routes are content-derived facts published by the prepass. Validating every claim
///      before insertion keeps exact and case-only collision policy identical to every other
///      visible identity, while one deterministic pass over the authored route claims avoids a
///      second per-route scan of the namespace table.
///
/// Returns the first collision when publication stops. The caller owns the reporting policy:
/// project attachment resolves every package site immediately, while a package boundary must
/// defer a registered-package overlap until later packages publish their root facades.
pub(super) fn insert_facade_route_entries(
    namespaces: &mut [ModuleNamespace],
    identities: &ModuleIdentityTable,
    routes: &FacadeRouteIndex,
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    binding_package_paths: &BTreeSet<String>,
    route_collision_site: Option<RouteCollisionSite<'_>>,
    mut selected_source_texts: Option<&mut SelectedSourceTextMap>,
) -> Result<Option<RouteCollision>, RoutePublicationFailure> {
    let registered_source_package_paths = package_boundary_indexes
        .iter()
        .map(|(prefix, _)| prefix)
        .collect::<Vec<_>>();

    for module_id in identities.module_ids() {
        let namespace = &mut namespaces[module_id.index()];
        let (planned, ambiguous_plans) = planned_route_entries(namespace, routes);
        // Validation covers ambiguous-base claims too: a route that a healthy child facade or a
        // registered package already owns cannot stay latent behind the base ambiguity.
        match first_route_collision(
            &planned,
            &ambiguous_plans,
            &namespace.entries,
            &registered_source_package_paths,
            binding_package_paths,
            route_collision_site,
            selected_source_texts.as_deref_mut(),
        ) {
            Ok(Some(collision)) => return Ok(Some(collision)),
            Err(failure) => return Err(failure),
            Ok(None) => {}
        }
        namespace.ambiguous_keys.extend(ambiguous_plans.into_keys());
        // The validation pass above checked every surviving planned key against existing entries
        // and against its sibling routes, so a second `insert` scan cannot discover a collision.
        // Same-facade case-only route conflicts are header diagnostics emitted by the prepass, so
        // no reachable entry relies on `insert`'s ambiguity tracking here. Insert directly to keep
        // route publication linear in the number of published routes.
        for entry in planned {
            namespace.entries.insert(
                entry.key,
                NamespaceEntry::PublicRoute {
                    target_module_id: entry.facade_module_id,
                    surface: entry.surface,
                    route: entry.route,
                },
            );
        }
    }

    Ok(None)
}

/// Plan the route entries one consumer namespace may insert from its already-built facade entries.
///
/// WHY: route visibility is exactly facade-entry visibility. Reusing the `CrossModule` claimants
///      preserves child/support selection, paired-surface suppression, root-local isolation and
///      entry-only exclusion without re-deriving any of them from topology.
///
/// Returns the publication plans and every claim whose base facade key is ambiguous, grouped by
/// raw route key. One key may hold claims behind distinct folded bases, so the caller owns both
/// resolving the base conflict and rejecting a collision between independent bases.
pub(super) fn planned_route_entries(
    namespace: &ModuleNamespace,
    routes: &FacadeRouteIndex,
) -> (
    Vec<PlannedRouteEntry>,
    BTreeMap<String, Vec<PlannedRouteEntry>>,
) {
    let mut planned = BTreeMap::new();
    // One raw route key can be reached from distinct ambiguous bases. Keep every claim so the
    // caller can compare folded bases instead of letting map insertion discard one base.
    let mut ambiguous_plans: BTreeMap<String, Vec<PlannedRouteEntry>> = BTreeMap::new();
    for (facade_key, claimants) in &namespace.facade_claimants {
        if facade_key.is_empty() {
            continue;
        }
        // Every claimant behind an ambiguous base still contributes route identities. Publishing
        // any one of them would select a facade by insertion order, so record each route spelling
        // as ambiguous; retaining the claims here is what makes a later `@base/<other-route>`
        // resolve to the base conflict instead of a route miss. Identical route claims behind one
        // folded base stay latent until selected; claims behind distinct bases are retained
        // separately so their collision is not hidden by raw-key deduplication.
        let base_is_ambiguous = namespace.is_ambiguous(facade_key);
        let folded_base = fold_route_spelling(facade_key);
        for (target_module_id, surface) in claimants {
            for claim in routes.routes(*target_module_id, *surface) {
                if claim.route.is_root() {
                    continue;
                }
                let key = format!("{facade_key}/{}", claim.route.as_str());
                if base_is_ambiguous {
                    ambiguous_plans
                        .entry(key.clone())
                        .or_default()
                        .push(PlannedRouteEntry {
                            key,
                            base_key: folded_base.clone(),
                            facade_module_id: *target_module_id,
                            surface: *surface,
                            route: claim.route.clone(),
                            span: claim.span,
                        });
                    continue;
                }
                // The formatted key already fixes the facade and spelling, so the plan identity
                // needs no second route copy.
                planned
                    .entry((key.clone(), *target_module_id, *surface))
                    .or_insert_with(|| PlannedRouteEntry {
                        key,
                        base_key: folded_base.clone(),
                        facade_module_id: *target_module_id,
                        surface: *surface,
                        route: claim.route.clone(),
                        span: claim.span,
                    });
            }
        }
    }
    (planned.into_values().collect(), ambiguous_plans)
}

/// The boundary's source facts for resolving a competing namespace entry's authored location.
///
/// WHY: a route that collides with a visible child facade or owned source has two authored sites.
///      The route operand already supplies the primary span; this site resolves the entry that
///      owns the other claim so both can be reported without a second source scan.
#[derive(Clone, Copy)]
pub(super) struct RouteCollisionSite<'a> {
    pub(super) source_tree_index: &'a SourceTreeIndex,
    pub(super) source_files: &'a SourceDatabase,
    /// Root facade sites retained for project route attachment, keyed by package prefix.
    ///
    /// Project attachment passes the map once every package root facade has been indexed, so a
    /// local route overlapping a registered prefix can name that package's authored site. Package
    /// attachment leaves it `None`: a package-internal collision keeps the single authored route
    /// operand and does not reinterpret a foreign package database.
    pub(super) package_root_facades: Option<&'a BTreeMap<String, PackageRootFacadeSite>>,
    /// Registered package root routes, keyed by prefix and then boundary-relative spelling.
    ///
    /// A project facade can collide with one exact package route under a shared prefix. That
    /// route's `$export @operand` is the authored competing site; the root facade start is only a
    /// fallback when the collision is with the package prefix rather than one published route.
    pub(super) package_public_routes: Option<&'a BTreeMap<String, BTreeMap<String, SourceSpan>>>,
}

/// Where a published package route's collision site can be rendered.
#[derive(Clone, Copy)]
pub(super) enum ProjectRouteClaimSite<'a> {
    /// The route operand has no usable consumer source database in this report context.
    PackageOperand,
    /// The consumer is the same package boundary, so its local entry resolves in that database.
    PackageLocal {
        source_tree_index: &'a SourceTreeIndex,
        source_files: &'a SourceDatabase,
    },
    /// The consumer is the project boundary; its matching local entry supplies the primary site.
    ///
    /// The competing package operand remains a foreign site: it resolves only through the
    /// package boundary's own source database and stable identity, never through the project DB.
    ProjectLocal {
        source_tree_index: &'a SourceTreeIndex,
        source_files: &'a SourceDatabase,
    },
}

impl<'a> ProjectRouteClaimSite<'a> {
    pub(super) fn from_source_database(
        source_tree_index: &'a SourceTreeIndex,
        source_files: Option<&'a SourceDatabase>,
    ) -> Self {
        match source_files {
            Some(source_files) => Self::ProjectLocal {
                source_tree_index,
                source_files,
            },
            None => Self::PackageOperand,
        }
    }
}

/// Reject an external package route that another registered package already names.
///
/// WHAT: folds every published root route into `<prefix>/<route>` and compares it with every
///       other registered prefix and binding path through the shared overlap predicate.
/// WHY: two published routes are always separated by their own prefixes, but a route can consume
///      another package's entire external root spelling (`@tools/forms` for package `tools/forms`).
///      Waiting for an importer to witness that collision reports one site and depends on a
///      consumer the project may not have. Both packages have authored root surfaces, so the
///      collision carries the route operand as primary and the other package's root facade as its
///      related site. A binding path owns no authored content site and keeps the established
///      single-site diagnostic.
pub(super) fn validate_package_route_prefix_collisions(
    package_public_routes: &BTreeMap<String, BTreeMap<String, SourceSpan>>,
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    binding_package_paths: &BTreeSet<String>,
    package_sources: &PackageRouteSources<'_>,
) -> Result<(), RoutePublicationFailure> {
    // One folded index per prefix set converts the per-route prefix scan from O(P + B) to
    // O(log(P + B)). Folded keys keep the same case-insensitive boundary rule.
    let source_prefixes = package_boundary_indexes
        .iter()
        .map(|(package_prefix, _)| package_prefix)
        .fold(BTreeMap::new(), |mut index, prefix| {
            index.entry(fold_route_spelling(prefix)).or_insert(prefix);
            index
        });
    let binding_prefixes = binding_package_paths.iter().map(String::as_str).fold(
        BTreeMap::new(),
        |mut index, prefix| {
            index.entry(fold_route_spelling(prefix)).or_insert(prefix);
            index
        },
    );
    for (package_prefix, routes) in package_public_routes {
        for (route_spelling, published_span) in routes {
            let route_key = format!("{package_prefix}/{route_spelling}");
            let Some(related_prefix) =
                overlapping_registered_prefix(&source_prefixes, &route_key, package_prefix)
            else {
                if overlapping_registered_prefix(&binding_prefixes, &route_key, package_prefix)
                    .is_some()
                {
                    let primary = package_report_identity(
                        package_boundary_indexes,
                        package_prefix,
                        &route_key,
                    )?;
                    let failure = prepared_route_collision_failure(
                        RouteCollision {
                            route_key,
                            span: *published_span,
                            related_span: None,
                            related_package: None,
                            registered_package_prefix: None,
                        },
                        Some(&primary),
                        None,
                        false,
                    );
                    return Err(RoutePublicationFailure::with_package_donors(
                        failure,
                        PackageRouteDonorRequests {
                            primary: Some(PackageRouteDonorRequest {
                                dependency_prefix: package_prefix.to_owned(),
                                identity: primary,
                                selected_site: None,
                            }),
                            related: None,
                        },
                    ));
                }
                continue;
            };
            let (related_span, related_site) = provider_root_facade_site(
                package_boundary_indexes,
                package_sources,
                related_prefix,
                &route_key,
            )?;
            // Resolve each identity only after a real collision is known; the route map itself
            // carries no duplicated stable identity.
            let primary_identity =
                package_report_identity(package_boundary_indexes, package_prefix, &route_key)?;
            let related_identity =
                package_report_identity(package_boundary_indexes, related_prefix, &route_key)?;
            let failure = prepared_route_collision_failure(
                RouteCollision {
                    route_key,
                    span: *published_span,
                    related_span: Some(related_span),
                    related_package: None,
                    registered_package_prefix: None,
                },
                Some(&primary_identity),
                Some(&related_identity),
                false,
            );
            return Err(RoutePublicationFailure::with_package_donors(
                failure,
                PackageRouteDonorRequests {
                    primary: Some(PackageRouteDonorRequest {
                        dependency_prefix: package_prefix.to_owned(),
                        identity: primary_identity,
                        selected_site: None,
                    }),
                    related: Some(PackageRouteDonorRequest {
                        dependency_prefix: related_prefix.to_owned(),
                        identity: related_identity,
                        selected_site: Some(related_site),
                    }),
                },
            ));
        }
    }
    Ok(())
}

/// One deferred package-prefix collision resolved to its authored sites without reading text.
///
/// WHAT: the publisher's identity and span, the competing provider's label identity and span, and
///       the provider donor request that must be finalized before the label can render.
/// WHY: a deferred collision that loses the report race must never read its provider's sources.
///      Keeping resolution to metadata lets the publisher's turn decide whether the donor is
///      needed, and lets that turn confirm the donor database has a line index before the
///      two-site diagnostic is built.
pub(in crate::build_system::create_project_modules) struct DeferredCollisionSite {
    pub(in crate::build_system::create_project_modules) publisher_prefix: String,
    pub(in crate::build_system::create_project_modules) route_key: String,
    pub(in crate::build_system::create_project_modules) primary_span: SourceSpan,
    pub(in crate::build_system::create_project_modules) primary_identity: StablePackageIdentity,
    pub(in crate::build_system::create_project_modules) related_identity:
        Option<StablePackageIdentity>,
    pub(in crate::build_system::create_project_modules) related_span: Option<SourceSpan>,
    /// The competing package's donor request. `selected_site` names the root facade when the
    /// collision is with the prefix rather than one published `$export` operand.
    pub(in crate::build_system::create_project_modules) related_request:
        Option<PackageRouteDonorRequest>,
}

/// Resolve one deferred package-internal prefix collision to metadata once its publisher's turn
/// is about to report it.
///
/// WHAT: selects the provider's exact published route operand when one matches the colliding key,
///       otherwise its indexed root facade start, and returns both authored sites and the donor
///       request without reading either site.
/// WHY: attachment runs in package-prefix order, so a later registered provider had no root
///      facade site when the publishing boundary stopped. Resolution must not read a donor that
///      belongs to a collision an earlier report will discard.
pub(in crate::build_system::create_project_modules) fn resolve_deferred_package_route_site(
    namespace_set: &ModuleNamespaceSet,
    deferred: DeferredPackageRouteCollision,
    provider_root_facade: Option<PackageRootFacadeSite>,
    provider_authored_routes: Option<&BTreeMap<String, SourceSpan>>,
) -> Result<DeferredCollisionSite, RoutePublicationFailure> {
    let DeferredPackageRouteCollision {
        publisher_prefix,
        provider_prefix,
        route_key,
        span,
    } = deferred;
    let primary_identity = package_report_identity(
        &namespace_set.package_boundary_indexes,
        &publisher_prefix,
        &route_key,
    )?;
    // Prefer the exact published `$export` operand. When the provider's own attachment failed,
    // its prepass still captured those authored claims; they stay out of import resolution.
    let folded_route_key = fold_route_spelling(&route_key);
    let published_span = namespace_set
        .package_public_routes
        .get(&provider_prefix)
        .or(provider_authored_routes)
        .and_then(|routes| {
            routes
                .iter()
                .find(|(route_spelling, _)| {
                    fold_route_spelling(&format!("{provider_prefix}/{route_spelling}"))
                        == folded_route_key
                })
                .map(|(_, span)| *span)
        });
    let provider_identity = namespace_set
        .package_boundary_indexes
        .get(&provider_prefix)
        .map(SourceTreeIndex::stable_package_identity)
        .cloned();
    let related_site = match provider_identity {
        Some(identity) => match published_span {
            Some(span) => Some((span, None, identity)),
            None => provider_root_facade.map(|site| {
                let selected_site = Some(site.path().to_path_buf());
                (site.span, selected_site, identity)
            }),
        },
        None => None,
    };
    let (related_identity, related_span, related_request) = match related_site {
        Some((related_span, selected_site, identity)) => {
            let request = PackageRouteDonorRequest {
                dependency_prefix: provider_prefix,
                identity: identity.clone(),
                selected_site,
            };
            (Some(identity), Some(related_span), Some(request))
        }
        None => (None, None, None),
    };
    Ok(DeferredCollisionSite {
        publisher_prefix,
        route_key,
        primary_span: span,
        primary_identity,
        related_identity,
        related_span,
        related_request,
    })
}

/// Build the deferred collision's diagnostic after its publisher's turn has won.
///
/// WHAT: a two-site report with the publisher's primary request when the competing donor is
///       renderable, otherwise the publisher-only report with its ordinary logical display.
/// WHY: the related donor is attached by the caller after finalization; a donor without a line
///      index must not produce a dangling label or switch the primary to canonical-host display.
pub(in crate::build_system::create_project_modules) fn deferred_package_collision_failure(
    site: &DeferredCollisionSite,
    related_donor_available: bool,
) -> RoutePublicationFailure {
    let (Some(related_span), Some(related_identity)) =
        (site.related_span, site.related_identity.as_ref())
    else {
        return single_site_package_collision(
            site.route_key.clone(),
            site.primary_span,
            site.publisher_prefix.clone(),
            site.primary_identity.clone(),
        );
    };
    if !related_donor_available {
        return single_site_package_collision(
            site.route_key.clone(),
            site.primary_span,
            site.publisher_prefix.clone(),
            site.primary_identity.clone(),
        );
    }
    let failure = prepared_route_collision_failure(
        RouteCollision {
            route_key: site.route_key.clone(),
            span: site.primary_span,
            related_span: Some(related_span),
            related_package: None,
            registered_package_prefix: None,
        },
        Some(&site.primary_identity),
        Some(related_identity),
        false,
    );
    RoutePublicationFailure::with_package_donors(
        failure,
        PackageRouteDonorRequests {
            primary: Some(PackageRouteDonorRequest {
                dependency_prefix: site.publisher_prefix.clone(),
                identity: site.primary_identity.clone(),
                selected_site: None,
            }),
            related: None,
        },
    )
}

/// Build the publisher-only collision report used when the competing site has no indexed location.
///
/// The publisher keeps its ordinary package-relative display: without a competing label there is
/// no second spelling to disambiguate, so the report matches HEAD's own prepass failure.
fn single_site_package_collision(
    route_key: String,
    span: SourceSpan,
    publisher_prefix: String,
    primary_identity: StablePackageIdentity,
) -> RoutePublicationFailure {
    let failure = prepared_route_collision_failure_with_primary_display(
        RouteCollision {
            route_key,
            span,
            related_span: None,
            related_package: None,
            registered_package_prefix: None,
        },
        Some(&primary_identity),
        None,
        false,
        PrimaryDomainDisplay::Logical,
    );
    RoutePublicationFailure::with_package_donors(
        failure,
        PackageRouteDonorRequests {
            primary: Some(PackageRouteDonorRequest {
                dependency_prefix: publisher_prefix,
                identity: primary_identity,
                selected_site: None,
            }),
            related: None,
        },
    )
}

/// Visit every folded-index entry whose key overlaps `folded_key` at a path boundary.
///
/// The candidate set is the exact key, every ancestor, and every descendant. The prefix index is
/// pairwise disjoint, so at most one descendant can overlap in practice, while the claims index
/// can hold several that the caller ranks. Visiting in BTreeMap order keeps both scans
/// deterministic.
fn visit_overlapping_folded_entries<'index, Value>(
    folded_entries: &'index BTreeMap<String, Value>,
    folded_key: &str,
    mut visit: impl FnMut(&'index Value),
) {
    if let Some(value) = folded_entries.get(folded_key) {
        visit(value);
    }
    for (boundary, _) in folded_key.match_indices('/') {
        if let Some(value) = folded_entries.get(&folded_key[..boundary]) {
            visit(value);
        }
    }
    let descendant_prefix = format!("{folded_key}/");
    let descendant_range = (
        Bound::Included(descendant_prefix.as_str()),
        Bound::Unbounded,
    );
    for (key, value) in folded_entries.range::<str, _>(descendant_range) {
        if !key.starts_with(&descendant_prefix) {
            break;
        }
        visit(value);
    }
}

/// Find one registered prefix that overlaps `route_key`, excluding the publishing package.
fn overlapping_registered_prefix<'a>(
    folded_prefixes: &'a BTreeMap<String, &'a str>,
    route_key: &str,
    package_prefix: &str,
) -> Option<&'a str> {
    let folded = fold_route_spelling(route_key);
    let mut best: Option<&'a str> = None;
    visit_overlapping_folded_entries(folded_prefixes, &folded, |prefix| {
        let prefix = *prefix;
        if prefix != package_prefix && best.is_none_or(|current| prefix < current) {
            best = Some(prefix);
        }
    });
    best
}

/// One consumer namespace entry that a published package route can collide with.
struct PackageNamespaceClaim<'a> {
    consumer_prefix: &'a str,
    module_index: usize,
    key: &'a str,
    entry: &'a NamespaceEntry,
}

/// One package-route claim that overlaps a provider in another package boundary.
struct PackageRouteNamespaceCollision<'a> {
    /// Earliest prepared boundary whose walk can report this collision.
    earliest_prefix: &'a str,
    route_key: String,
    package_prefix: &'a str,
    published_span: SourceSpan,
    claim: &'a PackageNamespaceClaim<'a>,
}

fn namespace_collision_rank<'entry>(
    collision: &'entry PackageRouteNamespaceCollision<'_>,
) -> (&'entry str, &'entry str, &'entry str, usize, &'entry str) {
    (
        collision.earliest_prefix,
        collision.route_key.as_str(),
        collision.claim.consumer_prefix,
        collision.claim.module_index,
        collision.claim.key,
    )
}

/// Keep the deterministic first collision for the earliest affected boundary.
fn consider_namespace_collision<'a>(
    best: &mut Option<PackageRouteNamespaceCollision<'a>>,
    publishing_prefix: &'a str,
    route_key: &str,
    published_span: SourceSpan,
    claim: &'a PackageNamespaceClaim<'a>,
) {
    if claim.consumer_prefix == publishing_prefix {
        return;
    }
    let earliest_prefix = publishing_prefix.min(claim.consumer_prefix);
    let candidate_rank = (
        earliest_prefix,
        route_key,
        claim.consumer_prefix,
        claim.module_index,
        claim.key,
    );
    if best
        .as_ref()
        .is_some_and(|current| namespace_collision_rank(current) <= candidate_rank)
    {
        return;
    }
    *best = Some(PackageRouteNamespaceCollision {
        earliest_prefix,
        route_key: route_key.to_owned(),
        package_prefix: publishing_prefix,
        published_span,
        claim,
    });
}

/// Reject a published external package route that another package's namespace already claims.
///
/// WHAT: scans every registered package's boundary-relative namespace entries, excluding the
///       publishing package itself, with the same overlap predicate consumers use.
/// WHY: package namespaces are the only place another package can own the route key locally
///      (`tools/forms` in a package other than `tools`). Without this pass the ambiguity stays
///      latent until that package imports `@tools/forms`, where resolution can name only the
///      importer's span. Both authored sites exist now, so reject eagerly and carry the route
///      operand as primary and the competing package provider as its related site.
pub(super) fn validate_package_route_namespace_collisions<'a>(
    package_public_routes: &'a BTreeMap<String, BTreeMap<String, SourceSpan>>,
    package_namespaces: &'a BTreeMap<String, Vec<ModuleNamespace>>,
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    package_sources: &PackageRouteSources<'_>,
) -> Result<(), RoutePublicationFailure> {
    // One folded-key index replaces a per-route scan of every package namespace entry. The
    // candidate set is exact, each ancestor, and every descendant; the same ancestor-aware
    // identity rule as consumer validation is retained without the quadratic walk.
    let mut claims: BTreeMap<String, Vec<PackageNamespaceClaim<'_>>> = BTreeMap::new();
    for (consumer_prefix, namespaces) in package_namespaces {
        for (module_index, namespace) in namespaces.iter().enumerate() {
            for (key, entry) in &namespace.entries {
                claims
                    .entry(fold_route_spelling(key))
                    .or_default()
                    .push(PackageNamespaceClaim {
                        consumer_prefix,
                        module_index,
                        key,
                        entry,
                    });
            }
        }
    }
    let mut best = None;
    for (package_prefix, routes) in package_public_routes {
        for (route_spelling, published_span) in routes {
            let route_key = format!("{package_prefix}/{route_spelling}");
            let folded = fold_route_spelling(&route_key);
            visit_overlapping_folded_entries(&claims, &folded, |claimants| {
                for claim in claimants {
                    consider_namespace_collision(
                        &mut best,
                        package_prefix,
                        &route_key,
                        *published_span,
                        claim,
                    );
                }
            });
        }
    }
    let Some(selected) = best else {
        return Ok(());
    };
    let PackageRouteNamespaceCollision {
        route_key,
        package_prefix,
        published_span,
        claim,
        ..
    } = selected;
    let consumer_prefix = claim.consumer_prefix;
    let Some(consumer_index) = package_boundary_indexes.get(consumer_prefix) else {
        return Err(unavailable_boundary_source_failure(
            &route_key,
            consumer_prefix,
            "a registered boundary index",
        ));
    };
    let Some(consumer_source) = package_sources.get(consumer_prefix).copied() else {
        return Err(unavailable_boundary_source_failure(
            &route_key,
            consumer_prefix,
            "a retained source database",
        ));
    };
    // The matched provider belongs to another package boundary. Resolve only its
    // registered identity here; its text is loaded into that package's own source
    // database by the donor finish, never through the project selection map.
    let collision_prefixes = vec![package_prefix.to_owned(), consumer_prefix.to_owned()];
    let Some(provider_path) = project_local_claim_path(claim.entry, consumer_index) else {
        return Err(unavailable_collision_site_failure(
            &route_key,
            collision_prefixes,
        ));
    };
    let provider_path = provider_path.to_path_buf();
    let Some(local_span) =
        project_local_claim_span(claim.entry, consumer_index, consumer_source, None)
    else {
        return Err(unavailable_collision_site_failure(
            &route_key,
            collision_prefixes,
        ));
    };
    let primary_identity =
        package_report_identity(package_boundary_indexes, package_prefix, &route_key)?;
    let related_identity =
        package_report_identity(package_boundary_indexes, consumer_prefix, &route_key)?;
    let failure = prepared_route_collision_failure(
        RouteCollision {
            route_key,
            span: published_span,
            related_span: Some(local_span),
            related_package: None,
            registered_package_prefix: None,
        },
        Some(&primary_identity),
        Some(&related_identity),
        false,
    );
    Err(RoutePublicationFailure::with_package_donors(
        failure,
        PackageRouteDonorRequests {
            primary: Some(PackageRouteDonorRequest {
                dependency_prefix: package_prefix.to_owned(),
                identity: primary_identity,
                selected_site: None,
            }),
            related: Some(PackageRouteDonorRequest {
                dependency_prefix: consumer_prefix.to_owned(),
                identity: related_identity,
                selected_site: Some(provider_path),
            }),
        },
    ))
}

/// Fail closed when a registered package boundary cannot supply one of its own source facts.
fn unavailable_boundary_source_failure(
    route_key: &str,
    package_prefix: &str,
    detail: &str,
) -> RoutePublicationFailure {
    RoutePublicationFailure::single_domain_in(
        vec![package_prefix.to_owned()],
        PreparedFailure::from(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "route collision at {route_key} names source package @{package_prefix} without {detail}"
            ),
        )),
    )
}

/// Reject a published external package route that a consumer namespace already claims locally.
///
/// WHAT: folds every registered root route into `<prefix>/<route>` and compares it, through the
///       shared overlap predicate, with every visible entry of every consumer. Ambiguous keys are
///       covered by their retained base entry, which the same ancestor-aware rule matches.
/// WHY: external package selection must not silently outrank a local provider. Validation is
///      eager so an unreferenced collision still reports. Inside the same package boundary, the
///      authored route operand is primary and the local provider is its related site; both
///      resolve in that boundary's source database. A project consumer names the matching local
///      entry's file as primary and requests the package source owner as an explicit foreign
///      report domain for the competing `$export` operand, so the label never reinterprets a
///      package `SourceId` in the project database.
pub(super) fn validate_published_package_routes_against_consumers(
    namespaces: &[&ModuleNamespace],
    package_public_routes: &BTreeMap<String, BTreeMap<String, SourceSpan>>,
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    claim_site: ProjectRouteClaimSite<'_>,
    mut selected_source_texts: Option<&mut SelectedSourceTextMap>,
) -> Result<(), RoutePublicationFailure> {
    for (package_prefix, routes) in package_public_routes {
        validate_package_route_claims(
            namespaces.iter().copied(),
            package_prefix,
            package_boundary_indexes,
            routes
                .iter()
                .map(|(spelling, span)| (spelling.as_str(), *span)),
            claim_site,
            selected_source_texts.as_deref_mut(),
        )?;
    }
    Ok(())
}

/// Check one package boundary's published root claims against the consumers that can see them.
pub(super) fn validate_package_route_claims<'namespace, 'route>(
    namespaces: impl Iterator<Item = &'namespace ModuleNamespace> + Clone,
    package_prefix: &str,
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    published_routes: impl Iterator<Item = (&'route str, SourceSpan)>,
    claim_site: ProjectRouteClaimSite<'_>,
    mut selected_source_texts: Option<&mut SelectedSourceTextMap>,
) -> Result<(), RoutePublicationFailure> {
    for (route_spelling, published_span) in published_routes {
        let route_key = format!("{package_prefix}/{route_spelling}");
        let Some(matched_local) = namespaces.clone().find_map(|namespace| {
            namespace
                .entries
                .iter()
                .find(|(key, _)| package_paths_overlap(key, &route_key))
        }) else {
            continue;
        };
        let (span, related_span, related_donor) = match claim_site {
            ProjectRouteClaimSite::PackageOperand => (published_span, None, None),
            ProjectRouteClaimSite::PackageLocal {
                source_tree_index,
                source_files,
            } => {
                let Some(local_span) = project_local_claim_span(
                    matched_local.1,
                    source_tree_index,
                    source_files,
                    selected_source_texts.as_deref_mut(),
                ) else {
                    return Err(unavailable_collision_site_failure(
                        &route_key,
                        vec![package_prefix.to_owned()],
                    ));
                };
                (published_span, Some(local_span), None)
            }
            ProjectRouteClaimSite::ProjectLocal {
                source_tree_index,
                source_files,
            } => {
                let Some(local_span) = project_local_claim_span(
                    matched_local.1,
                    source_tree_index,
                    source_files,
                    selected_source_texts.as_deref_mut(),
                ) else {
                    return Err(unavailable_collision_site_failure(
                        &route_key,
                        vec![package_prefix.to_owned()],
                    ));
                };
                // The local provider is primary and resolves in the project database. The
                // package operand becomes a related frozen label owned by the package domain.
                let package_identity =
                    package_report_identity(package_boundary_indexes, package_prefix, &route_key)?;
                (
                    local_span,
                    Some(published_span),
                    Some(PackageRouteDonorRequest {
                        dependency_prefix: package_prefix.to_owned(),
                        identity: package_identity,
                        // The route operand was selected before discovery, so its slot has text.
                        selected_site: None,
                    }),
                )
            }
        };
        let failure = prepared_route_collision_failure(
            RouteCollision {
                route_key,
                span,
                related_span,
                related_package: None,
                registered_package_prefix: None,
            },
            None,
            related_donor.as_ref().map(|donor| &donor.identity),
            related_donor.is_some(),
        );
        return Err(match related_donor {
            Some(related) => RoutePublicationFailure::with_package_donors(
                failure,
                PackageRouteDonorRequests {
                    primary: None,
                    related: Some(related),
                },
            ),
            None => {
                RoutePublicationFailure::single_domain_in(vec![package_prefix.to_owned()], failure)
            }
        });
    }
    Ok(())
}

/// Resolve one registered package's stable identity only when a collision needs a report domain.
fn package_report_identity(
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    package_prefix: &str,
    route_key: &str,
) -> Result<StablePackageIdentity, RoutePublicationFailure> {
    package_boundary_indexes
        .get(package_prefix)
        .map(SourceTreeIndex::stable_package_identity)
        .cloned()
        .ok_or_else(|| {
            RoutePublicationFailure::single_domain_in(
                vec![package_prefix.to_owned()],
                PreparedFailure::from(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "route collision at {route_key} names unregistered source package @{package_prefix}"
                    ),
                )),
            )
        })
}

/// Resolve another registered package's authored root facade in that package's own database.
///
/// WHY: every registered source package is indexed with a root `@package.moth`, and the package
///      source map is built from those same inventories before project attachment. Missing either
///      fact is index/source-database skew, so the ambiguity must not degrade to a one-site
///      report or reinterpret a boundary-local `SourceId`.
fn provider_root_facade_site(
    package_boundary_indexes: &SourcePackageBoundaryIndexes,
    package_sources: &PackageRouteSources<'_>,
    provider_prefix: &str,
    route_key: &str,
) -> Result<(SourceSpan, PathBuf), RoutePublicationFailure> {
    let unavailable =
        |detail: &str| unavailable_boundary_source_failure(route_key, provider_prefix, detail);
    let index = package_boundary_indexes
        .get(provider_prefix)
        .ok_or_else(|| unavailable("a registered boundary index"))?;
    let source_files = package_sources
        .get(provider_prefix)
        .copied()
        .ok_or_else(|| unavailable("a retained source database"))?;
    let root_facade = index
        .package_public_root_file()
        .ok_or_else(|| unavailable("an indexed root facade"))?;
    let slot = source_files
        .get_by_canonical_path(root_facade)
        .ok_or_else(|| unavailable("a registered root facade source"))?;
    Ok((
        SourceSpan::new(slot.id, LocalSpan::source_start()),
        root_facade.to_path_buf(),
    ))
}

/// Fail closed when a detected collision's own site does not resolve.
///
/// WHY: `None` is unreachable for a valid indexed compilation. `matched_local` entries come only
///      from present Stage 0 rows and present root files in the same boundary whose
///      `SourceDatabase` registered them, so a missing span proves index/database skew or a
///      foreign `ModuleId`. Skipping the collision would let an ambiguous package identity
///      survive as `Ok`, so the contradiction takes the typed invariant lane instead.
fn unavailable_collision_site_failure(
    route_key: &str,
    affected_prefixes: Vec<String>,
) -> RoutePublicationFailure {
    RoutePublicationFailure::single_domain_in(
        affected_prefixes,
        PreparedFailure::from(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "route collision at {route_key} names a namespace entry absent from the boundary source table"
            ),
        )),
    )
}

/// Resolve one visible namespace entry to a project-local source span.
fn project_local_claim_span(
    entry: &NamespaceEntry,
    source_tree_index: &SourceTreeIndex,
    source_files: &SourceDatabase,
    selected_source_texts: Option<&mut SelectedSourceTextMap>,
) -> Option<SourceSpan> {
    let path = project_local_claim_path(entry, source_tree_index)?;
    source_span_for_path(path, source_files, selected_source_texts)
}

/// Resolve one visible namespace entry to its authored source file without loading text.
fn project_local_claim_path<'a>(
    entry: &'a NamespaceEntry,
    source_tree_index: &'a SourceTreeIndex,
) -> Option<&'a Path> {
    match entry {
        NamespaceEntry::SameModuleSource { source_index, .. }
        | NamespaceEntry::SameModuleProvider { source_index } => {
            Some(source_tree_index.source(*source_index).canonical_path())
        }
        NamespaceEntry::CrossModule {
            target_module_id,
            surface,
        }
        | NamespaceEntry::PublicRoute {
            target_module_id,
            surface,
            ..
        } => source_tree_index
            .module_identities()
            .record(*target_module_id)
            .root_file(*surface),
    }
}

fn source_span_for_path(
    path: &Path,
    source_files: &SourceDatabase,
    selected_source_texts: Option<&mut SelectedSourceTextMap>,
) -> Option<SourceSpan> {
    // A setup failure retains only selected source snapshots. Load the matched local provider so
    // the rendered diagnostic has the same source excerpt as any other authored site.
    if let Some(selected) = selected_source_texts {
        let _ = selected.load(path);
    }
    let slot = source_files.get_by_canonical_path(path)?;
    Some(SourceSpan::new(slot.id, LocalSpan::source_start()))
}

/// Return the first deterministic collision involving a planned or ambiguous-base route entry.
///
/// WHAT: rejects a route that folds onto any existing non-route entry, two selectable routes from
///       distinct facades that fold together, a selectable route colliding with an ambiguous-base
///       route, and a route that overlaps a registered source or binding package prefix.
/// WHY: these are the identities consumers can actually collide with under the current topology.
///      A route matching a private physical folder inside the exporting module has no namespace
///      entry in any consumer and therefore stays legal. Case-only conflicts between the same
///      facade's routes are authored-header diagnostics, not namespace collisions, so they stay
///      reserved for the facade's own preparation. Identical route claims behind the same folded
///      ambiguous base stay latent until a consumer selects them; claims behind distinct
///      ambiguous bases own independent route identities and collide eagerly.
pub(super) fn first_route_collision(
    planned: &[PlannedRouteEntry],
    ambiguous_plans: &BTreeMap<String, Vec<PlannedRouteEntry>>,
    entries: &BTreeMap<String, NamespaceEntry>,
    registered_source_package_paths: &[&str],
    registered_binding_package_paths: &BTreeSet<String>,
    collision_site: Option<RouteCollisionSite<'_>>,
    mut selected_source_texts: Option<&mut SelectedSourceTextMap>,
) -> Result<Option<RouteCollision>, RoutePublicationFailure> {
    if planned.is_empty() && ambiguous_plans.is_empty() {
        return Ok(None);
    }
    // One folded index replaces a per-claim scan of every existing key. Entries may retain
    // case-only claimants as ambiguity, so the first sorted spelling is the deterministic site.
    let folded_entries = entries
        .iter()
        .map(|(key, entry)| (fold_route_spelling(key), entry))
        .collect::<BTreeMap<_, _>>();
    for plan in planned.iter().chain(ambiguous_plans.values().flatten()) {
        if let Some(entry) = folded_entries.get(&fold_route_spelling(&plan.key)) {
            let related_span = match collision_site {
                None => None,
                Some(site) => {
                    let Some(span) = project_local_claim_span(
                        entry,
                        site.source_tree_index,
                        site.source_files,
                        selected_source_texts.as_deref_mut(),
                    ) else {
                        // The entry came from the same indexed boundary as the route plan, so a
                        // missing span proves index/database skew or a foreign `ModuleId`.
                        // Dropping the collision would let an ambiguous route survive as `Ok`.
                        return Err(unavailable_collision_site_failure(&plan.key, Vec::new()));
                    };
                    Some(span)
                }
            };
            return Ok(Some(RouteCollision {
                route_key: plan.key.clone(),
                span: plan.span,
                related_span,
                related_package: None,
                registered_package_prefix: None,
            }));
        }
    }

    let mut folded_route_groups: BTreeMap<String, RouteClaimGroup<'_>> = BTreeMap::new();
    for plan in planned {
        folded_route_groups
            .entry(fold_route_spelling(&plan.key))
            .or_default()
            .planned
            .push(plan);
    }
    for plan in ambiguous_plans.values().flatten() {
        folded_route_groups
            .entry(fold_route_spelling(&plan.key))
            .or_default()
            .ambiguous
            .push(plan);
    }
    for group in folded_route_groups.values() {
        if group.planned.len() >= 2 {
            let first_facade = (group.planned[0].facade_module_id, group.planned[0].surface);
            if group
                .planned
                .iter()
                .any(|plan| (plan.facade_module_id, plan.surface) != first_facade)
            {
                // The later planned claim owns the diagnostic site, keeping the result
                // independent of map iteration or authored-file discovery order.
                let collision = group
                    .planned
                    .last()
                    .expect("a folded route group repeats its first key");
                return Ok(Some(RouteCollision {
                    route_key: collision.key.clone(),
                    span: collision.span,
                    related_span: group
                        .planned
                        .iter()
                        .find(|candidate| {
                            (candidate.facade_module_id, candidate.surface)
                                != (collision.facade_module_id, collision.surface)
                        })
                        .map(|candidate| candidate.span),
                    related_package: None,
                    registered_package_prefix: None,
                }));
            }
        }
        // A selectable route and an ambiguous-base route with the same folded key cannot both
        // survive publication: selecting the key would reject the healthy claim because of the
        // unrelated base ambiguity. Ambiguous claims under one folded base stay latent; distinct
        // bases are compared below.
        if let (Some(selectable), Some(ambiguous)) =
            (group.planned.first(), group.ambiguous.first())
        {
            return Ok(Some(RouteCollision {
                route_key: ambiguous.key.clone(),
                span: ambiguous.span,
                related_span: Some(selectable.span),
                related_package: None,
                registered_package_prefix: None,
            }));
        }
        if let Some(collision) = group.ambiguous.last()
            && let Some(related) = group
                .ambiguous
                .iter()
                .find(|candidate| candidate.base_key != collision.base_key)
        {
            return Ok(Some(RouteCollision {
                route_key: collision.key.clone(),
                span: collision.span,
                related_span: Some(related.span),
                related_package: None,
                registered_package_prefix: None,
            }));
        }
    }

    for plan in planned.iter().chain(ambiguous_plans.values().flatten()) {
        let mut registered_package_prefix: Option<&str> = None;
        let mut related_package = None;
        for package_prefix in registered_source_package_paths {
            if !package_paths_overlap(package_prefix, &plan.key) {
                continue;
            }
            let candidate_prefix = *package_prefix;
            if registered_package_prefix.is_none_or(|current| candidate_prefix < current) {
                registered_package_prefix = Some(candidate_prefix);
            }
            let Some(site) = collision_site
                .and_then(|site| site.package_root_facades)
                .and_then(|facades| facades.get(candidate_prefix))
            else {
                continue;
            };
            // Prefer the exact published package route: its `$export @operand` is the authored
            // claim competing with this project route. A prefix-only collision falls back to the
            // root facade start, which is still a site in the package's own source database.
            let folded_plan = fold_route_spelling(&plan.key);
            let published_span = collision_site
                .and_then(|site| site.package_public_routes)
                .and_then(|routes| routes.get(candidate_prefix))
                .and_then(|routes| {
                    routes
                        .iter()
                        .find(|(route_spelling, _)| {
                            fold_route_spelling(&format!("{candidate_prefix}/{route_spelling}"))
                                == folded_plan
                        })
                        .map(|(_, span)| *span)
                });
            let (span, root_facade) = match published_span {
                Some(span) => (span, None),
                None => (site.span, Some(site.path.clone())),
            };
            let candidate = PackageRouteCollisionSite {
                dependency_prefix: candidate_prefix.to_owned(),
                identity: site.identity.clone(),
                root_facade,
                span,
            };
            if related_package
                .as_ref()
                .is_none_or(|current: &PackageRouteCollisionSite| {
                    candidate.dependency_prefix < current.dependency_prefix
                })
            {
                related_package = Some(candidate);
            }
        }
        if let Some(package_prefix) = registered_package_prefix {
            return Ok(Some(RouteCollision {
                route_key: plan.key.clone(),
                span: plan.span,
                related_span: related_package.as_ref().map(|package| package.span),
                related_package,
                registered_package_prefix: Some(package_prefix.to_owned()),
            }));
        }
        if registered_binding_package_paths
            .iter()
            .any(|package_prefix| package_paths_overlap(package_prefix, &plan.key))
        {
            // A binding package owns no authored content site, so the collision keeps the
            // established single-site report but is still rejected eagerly.
            return Ok(Some(RouteCollision {
                route_key: plan.key.clone(),
                span: plan.span,
                related_span: None,
                related_package: None,
                registered_package_prefix: None,
            }));
        }
    }

    Ok(None)
}

/// Build the typed import diagnostic for one eagerly rejected route collision.
///
/// WHAT: interns the colliding consumer-relative route key into a diagnostic-only path table,
///       reports the primary site, and adds a previous-claim label when `related_span` names the
///       competing authored operand.
/// WHY: the route key is the importable spelling the author would write. Two route operands or a
///      route operand and a visible local provider both resolve in the boundary's source context,
///      so both spans are retained. A registered package operand may belong to another boundary;
///      the domain arguments then attach frozen identity handles so that package's database
///      resolves the span instead of the report's default project context.
pub(super) fn route_collision_failure(mut collision: RouteCollision) -> RoutePublicationFailure {
    let Some(package) = collision.related_package.take() else {
        return RoutePublicationFailure::single_domain(prepared_route_collision_failure(
            collision, None, None, false,
        ));
    };
    let failure = prepared_route_collision_failure(collision, None, Some(&package.identity), true);
    RoutePublicationFailure::with_package_donors(
        failure,
        PackageRouteDonorRequests {
            primary: None,
            related: Some(PackageRouteDonorRequest {
                dependency_prefix: package.dependency_prefix,
                identity: package.identity,
                selected_site: package.root_facade,
            }),
        },
    )
}

/// Which spelling a package-domain primary site uses.
#[derive(Clone, Copy)]
enum PrimaryDomainDisplay {
    /// The package's boundary-relative path, as an ordinary package diagnostic renders it.
    Logical,
    /// The physical path, used when two boundaries can spell the same logical file.
    CanonicalHost,
}

/// Build the collision failure, optionally pointing either authored site at a frozen donor domain.
///
/// A domain argument is set only for a cross-boundary report. `canonical_local_primary` opts a
/// project-local primary into the same physical-path presentation as the package label, because
/// the two boundaries can spell the same logical file. Without a related domain no label handle
/// is attached: the constructors that create a label are the only ones that own label index zero.
pub(super) fn prepared_route_collision_failure(
    collision: RouteCollision,
    primary_domain: Option<&StablePackageIdentity>,
    related_domain: Option<&StablePackageIdentity>,
    canonical_local_primary: bool,
) -> PreparedFailure {
    prepared_route_collision_failure_with_primary_display(
        collision,
        primary_domain,
        related_domain,
        canonical_local_primary,
        PrimaryDomainDisplay::CanonicalHost,
    )
}

fn prepared_route_collision_failure_with_primary_display(
    collision: RouteCollision,
    primary_domain: Option<&StablePackageIdentity>,
    related_domain: Option<&StablePackageIdentity>,
    canonical_local_primary: bool,
    primary_domain_display: PrimaryDomainDisplay,
) -> PreparedFailure {
    // A publication failure can be retained past its boundary's preparation, so it must not
    // consume the live boundary string table. The diagnostic's path rows pair with this fresh
    // local table and stay self-contained until the failure's turn.
    let mut diagnostic_strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = try_intern_route_spelling(
        &collision.route_key,
        &mut path_fork,
        &mut diagnostic_strings,
    );
    // Only the constructor that actually creates a label may attach a label handle. A capacity
    // diagnostic has no labels, and the setter asserts on an out-of-range label index.
    let has_related_label = path.is_some() && collision.related_span.is_some();
    let mut diagnostic = match (path, collision.related_span) {
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
    if let Some(identity) = primary_domain {
        // A cross-boundary collision is exactly where two packages can spell the same logical
        // file, so those donor sites opt into canonical-host display. A lone package site keeps
        // the package's ordinary boundary-relative spelling.
        let handle = match primary_domain_display {
            PrimaryDomainDisplay::Logical => FrozenIdentityHandle::for_domain(identity.clone()),
            PrimaryDomainDisplay::CanonicalHost => {
                FrozenIdentityHandle::for_collision_domain(identity.clone())
            }
        };
        diagnostic = diagnostic.with_primary_frozen_identity_handle(handle);
    } else if canonical_local_primary {
        diagnostic = diagnostic
            .with_primary_frozen_identity_handle(FrozenIdentityHandle::for_collision_local());
    }
    if has_related_label && let Some(identity) = related_domain {
        diagnostic = diagnostic.with_label_frozen_identity_handle(
            0,
            FrozenIdentityHandle::for_collision_domain(identity.clone()),
        );
    }
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        diagnostic_strings,
        None,
        Some(Arc::new(path_fork.shared_table())),
    ))
}
