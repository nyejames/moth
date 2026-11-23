//! Stage 0 public-export route prepublication for project facade roots.
//!
//! WHAT: prepares every project `@mod.moth` and `@package.moth` facade root exactly once after the
//!       project source table exists and before consumer namespaces are built, then derives a
//!       small owned route summary from the retained header facts. The prepared output and its
//!       local identity domain travel in a build-owned cache that the module walk later adopts.
//! WHY: an explicit dependency route such as `@child/forms` is a namespace fact, so consumer
//!      namespaces need it before any module's header-owned discovery runs. Preparing the facade
//!      here keeps that knowledge content-derived without a second parser, source scan or
//!      filesystem fallback, and the cache preserves Stage 0's exactly-once preparation contract.
//!
//! The summary is deliberately tiny: one owned route spelling and its authored operand span per
//! declared route. It never carries a prepared interface, a whole `Header` or a donor `PathId`
//! across a boundary; the per-file `PathSyntaxId` only orders claims while this module builds the
//! summary. Registered source-package boundaries run the same per-boundary helper into their own
//! namespace sets; this module has no project-index assumption other than the identities it is
//! handed.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::CASE_CONFLICTING_PUBLIC_EXPORT_ROUTE;
use crate::compiler_frontend::compiler_messages::{CompilerStage, PreparedFailure};
use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareOutput;
use crate::compiler_frontend::headers::public_export_route_conflicts::RouteClaimOrder;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, PublicExportRoute};
#[cfg(test)]
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::source::{SourceDatabase, SourceSpan, SourceSpanBuilders};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::style_directives::{
    PublicExportRouteOperand, StyleDirectiveRegistry,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
#[cfg(test)]
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::num::NonZeroU32;
use std::sync::Arc;

use super::module_identity::ModuleId;
use super::module_preparation::{
    FacadePreparedSource, ModulePreparationContext, ModuleSyntaxDiscovery, RegisteredModuleSources,
};
use super::project_module_graph::ProjectModuleGraph;
use super::source_discovery::prepare_owned_source_input;
use super::source_loading::SelectedSourceTextMap;
use super::source_tree_index::SourceTreeIndex;

/// One authored public-route claim on one facade surface.
///
/// WHAT: the owned canonical route spelling relative to the declaring facade plus the exact
///       `$export` operand span.
/// WHY: Stage 0 keys namespace entries by owned text and reports eager collisions at the authored
///      operand, so no boundary-local `PathId` or authored path table row escapes the preparation
///      that issued it.
#[derive(Debug)]
pub(super) struct FacadeRouteClaim {
    pub(super) route: PublicExportRoute,
    pub(super) span: SourceSpan,
}

/// Public-route summaries for one compilation boundary, keyed by facade identity and surface.
///
/// WHAT: maps each `(ModuleId, ModuleRootRole)` facade to its authored route claims in deterministic
///       encounter order.
/// WHY: namespace construction needs exactly the claims of the facade whose entry it is about to
///      insert. A build-local `ModuleId` is the correct key here because the summary never crosses
///      its boundary.
#[derive(Debug, Default)]
pub(crate) struct FacadeRouteIndex {
    by_facade: BTreeMap<(ModuleId, ModuleRootRole), Vec<FacadeRouteClaim>>,
}

impl FacadeRouteIndex {
    /// The authored route claims for one facade surface, empty when the facade declares none.
    pub(super) fn routes(
        &self,
        module_id: ModuleId,
        surface: ModuleRootRole,
    ) -> &[FacadeRouteClaim] {
        self.by_facade
            .get(&(module_id, surface))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Build a deterministic route summary for focused namespace tests.
    ///
    /// WHY: collision tests need authored route facts without compiling a full project through
    ///      the Stage 0 prepass; production still constructs this index only from prepared
    ///      facade headers.
    #[cfg(test)]
    pub(crate) fn from_claims(
        claims: impl IntoIterator<Item = (ModuleId, ModuleRootRole, PublicExportRoute)>,
    ) -> Self {
        let mut index = Self::default();
        for (module_id, surface, route) in claims {
            index.insert_claim(
                module_id,
                surface,
                FacadeRouteClaim {
                    route,
                    span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
                },
            );
        }
        index
    }

    fn insert_claim(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
        claim: FacadeRouteClaim,
    ) {
        self.by_facade
            .entry((module_id, surface))
            .or_default()
            .push(claim);
    }
}

/// Build-owned cache of facade preparations awaiting their module's discovery walk.
///
/// WHAT: owns at most one preparation outcome per `(ModuleId, ModuleRootRole)` facade and hands it
///       to the walk by move.
/// WHY: the prepass and the walk must consume the same tokenized/parsed output or original
///      diagnostic. A take-only cache makes a second preparation of one facade an explicit
///      internal error instead of a silent reparse.
pub(crate) struct FacadePreparationCache {
    /// One preparation outcome per facade, moved out by the facade's walk.
    ///
    /// WHY: a diagnosed facade cannot be handed to the walk as a preparation, but the walk still
    ///      owns the boundary-ordered report. One outcome map keeps success and failure under the
    ///      same exactly-once contract instead of checking two maps for duplicate preparation.
    by_facade: BTreeMap<(ModuleId, ModuleRootRole), Result<FacadePreparedSource, PreparedFailure>>,
    /// Whether the producing pass handled every present facade root in this boundary.
    ///
    /// WHY: a project or package walk must adopt or report, never re-prepare, a root the prepass
    ///      already handled. Only focused tests construct a cache without a prepass and keep the
    ///      ordinary preparation path; production has no non-exhaustive producer.
    #[cfg(test)]
    exhaustive: bool,
}

impl FacadePreparationCache {
    /// Construct the cache produced by a complete boundary facade prepass.
    fn exhaustive() -> Self {
        Self {
            by_facade: BTreeMap::new(),
            #[cfg(test)]
            exhaustive: true,
        }
    }

    /// Construct a non-exhaustive cache for focused walks that deliberately skip the prepass.
    #[cfg(test)]
    pub(super) fn for_focused_test_without_prepass() -> Self {
        Self {
            by_facade: BTreeMap::new(),
            exhaustive: false,
        }
    }

    /// Whether a present facade root missing from this cache is a compiler invariant failure.
    pub(super) fn is_exhaustive(&self) -> bool {
        #[cfg(test)]
        {
            self.exhaustive
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    /// Move one facade's preparation or original diagnostic into its discovery walk.
    pub(super) fn take(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
    ) -> Option<Result<FacadePreparedSource, PreparedFailure>> {
        self.by_facade.remove(&(module_id, surface))
    }

    fn insert(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
        prepared: FacadePreparedSource,
    ) -> Result<(), InfrastructureFailure> {
        self.insert_outcome(module_id, surface, Ok(prepared))
    }

    fn insert_failure(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
        failure: PreparedFailure,
    ) -> Result<(), InfrastructureFailure> {
        self.insert_outcome(module_id, surface, Err(failure))
    }

    fn insert_outcome(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
        outcome: Result<FacadePreparedSource, PreparedFailure>,
    ) -> Result<(), InfrastructureFailure> {
        if self
            .by_facade
            .insert((module_id, surface), outcome)
            .is_some()
        {
            return Err(self.multiple_preparation_error(module_id, surface));
        }
        Ok(())
    }

    fn multiple_preparation_error(
        &self,
        module_id: ModuleId,
        surface: ModuleRootRole,
    ) -> InfrastructureFailure {
        InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "facade route prepass prepared ModuleId {} {surface:?} facade more than once",
                module_id.index()
            ),
        )
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.by_facade.len()
    }
}

/// Which owner reports a facade source-preparation diagnostic.
///
/// WHAT: the project boundary validates its config before this pass, so its prepass fails fast;
///       a package boundary defers source diagnostics to its ordinary walk so config-first and
///       package-order diagnostics remain intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FacadeDiagnosticOwner {
    Prepass,
    BoundaryWalk,
}

/// One boundary's completed facade route prepublication.
///
/// WHAT: the owned route summaries, the facade preparations awaiting their walks, the shared
///       source-origin table those walks reuse instead of rebuilding it, the facades whose own
///       preparation raised a source diagnostic, and any non-source prepass failure retained for
///       boundary-ordered reporting.
/// WHY: all facts are produced by the same per-boundary traversal and consumed by the same
///      namespace build and module inventory. Keeping them together avoids a second origin table
///      or a parallel facade scan.
pub(super) struct FacadeRoutePrepass {
    pub(super) routes: FacadeRouteIndex,
    pub(super) cache: FacadePreparationCache,
    pub(super) source_module_origins: Arc<SourceModuleOriginTable>,
    /// Facades whose source preparation produced a diagnostic the boundary walk must own.
    pub(super) unavailable_facades: Vec<(ModuleId, ModuleRootRole)>,
    /// A route-publication or infrastructure failure retained for this boundary's walk turn.
    pub(super) failure: Option<PreparedFailure>,
}

/// Prepare every facade root in one boundary and derive its public route summary.
///
/// WHAT: walks graph nodes in deterministic `ModuleId` order and, for each present normal and
///       support root, runs the existing owned-source and header preparation once.
/// WHY: the caller needs route facts before it builds namespaces, while the later module walk must
///       reuse this exact preparation. The pass stops after header preparation: it resolves no
///       provider, copies no public interface and publishes no semantic fact.
#[allow(
    clippy::too_many_arguments,
    reason = "the prepass keeps indexed sources, preparation settings, live span/selected-text owners and shared string/path builders as separate inputs"
)]
pub(super) fn prepublish_facade_routes(
    source_tree_index: &SourceTreeIndex,
    project_module_graph: &ProjectModuleGraph,
    source_files: &SourceDatabase,
    style_directives: &StyleDirectiveRegistry,
    module_error_allowance: Option<NonZeroU32>,
    diagnostic_owner: FacadeDiagnosticOwner,
    source_spans: &mut SourceSpanBuilders<'_>,
    selected_source_texts: &mut SelectedSourceTextMap,
    string_table: &mut StringTable,
    path_interner: &PathInternerBuilder,
    #[cfg(feature = "timers")] timing_boundary: crate::timing::TimingBoundaryId,
) -> Result<FacadeRoutePrepass, PreparedFailure> {
    let source_origin_lookup = project_module_graph
        .build_source_origin_lookup(source_tree_index)
        .map_err(PreparedFailure::from)?;
    let source_module_origins = Arc::new(SourceModuleOriginTable::from_graph_ownership(
        source_files,
        &source_origin_lookup,
    ));
    drop(source_origin_lookup);

    let preparation_context = ModulePreparationContext {
        source_files,
        style_directives,
        module_error_allowance,
    };
    let string_fork_source = string_table.fork_source();
    let path_fork_source = path_interner.fork_source();

    let mut routes = FacadeRouteIndex::default();
    let mut cache = FacadePreparationCache::exhaustive();
    let mut unavailable_facades = Vec::new();
    let mut boundary_failure = None;

    // `nodes()` is already the canonical `ModuleId` order the module walk seeds from, so neither
    // the preparation nor its diagnostics depend on filesystem traversal.
    'boundary: for node in project_module_graph.nodes() {
        let module_id = node.module_id();
        #[cfg(feature = "timers")]
        let timing_module_key = crate::timing::register_timing_module_for_preparation(
            timing_boundary,
            module_id.index() as u32,
            node.stable_origin().logical_module_path(),
        );
        #[cfg(feature = "timers")]
        let timing_context = Some(crate::timing::TimingContext::for_module(timing_module_key));
        'facades: for (surface, root_path) in node.root_files().present() {
            if !matches!(surface, ModuleRootRole::Normal | ModuleRootRole::Support) {
                continue;
            }
            let Some(facade_source_index) =
                source_tree_index.source_index_for_canonical_path(root_path)
            else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "facade route prepass found no source row for ModuleId {} {surface:?} facade",
                        module_id.index()
                    ),
                )
                .into());
            };

            let string_fork = string_fork_source.fork_for_module();
            let string_base_len = string_fork.base_len();
            let (local_string_table, _) = string_fork.into_parts();
            let path_fork = path_fork_source.fork_for_module();
            // The prepass prepares only this facade root and retains no module inventory, so it
            // carries no candidate list or prepared-output slots.
            let mut syntax = preparation_context.begin_syntax_discovery(
                node.stable_origin().clone(),
                RegisteredModuleSources {
                    candidate_source_ids: Vec::new(),
                    source_module_origins: Arc::clone(&source_module_origins),
                },
                node.root_files().clone(),
                None,
                local_string_table,
                path_fork,
                selected_source_texts,
                #[cfg(feature = "timers")]
                timing_context,
            )?;

            let input_result = crate::timed_stage_attributed!(
                crate::timing::TimingMetric::FrontendPrepare,
                timing_context,
                {
                    let (syntax_string_table, selected_source_texts, syntax_path_fork) =
                        syntax.source_preparation_inputs_and_path_fork_mut();
                    prepare_owned_source_input(
                        facade_source_index,
                        source_tree_index,
                        source_files,
                        source_spans,
                        style_directives,
                        syntax_string_table,
                        syntax_path_fork,
                        selected_source_texts,
                    )
                },
            );
            let input = match input_result {
                Ok(input) => input,
                Err(error) => {
                    let path_table = Arc::new(syntax.path_fork_mut().shared_table());
                    let failure = error.into_failure(syntax.string_table_mut(), path_table);
                    match diagnostic_owner {
                        FacadeDiagnosticOwner::BoundaryWalk
                            if matches!(failure, PreparedFailure::Diagnosed(_)) =>
                        {
                            // Cache the facade's one original diagnostic for its boundary walk.
                            if let Err(error) = cache.insert_failure(module_id, surface, failure) {
                                boundary_failure = Some(error.into());
                                break 'boundary;
                            }
                            unavailable_facades.push((module_id, surface));
                            continue 'facades;
                        }
                        FacadeDiagnosticOwner::BoundaryWalk => {
                            boundary_failure = Some(failure);
                            break 'boundary;
                        }
                        FacadeDiagnosticOwner::Prepass => return Err(failure),
                    }
                }
            };
            let output = match syntax.prepare_source(input, source_spans) {
                Ok(output) => output,
                Err(failure) => {
                    // A same-facade route conflict is a publication failure, not an ordinary
                    // source diagnostic. Deferring it would let a healthy competing facade make
                    // the route ambiguous and mask MOTH-RULE-0092 with a consumer import error
                    // during the boundary walk.
                    let may_defer_to_walk = !is_case_conflicting_route_failure(&failure);
                    match diagnostic_owner {
                        FacadeDiagnosticOwner::BoundaryWalk
                            if may_defer_to_walk
                                && matches!(failure, PreparedFailure::Diagnosed(_)) =>
                        {
                            if let Err(error) = cache.insert_failure(module_id, surface, failure) {
                                boundary_failure = Some(error.into());
                                break 'boundary;
                            }
                            unavailable_facades.push((module_id, surface));
                            continue 'facades;
                        }
                        FacadeDiagnosticOwner::BoundaryWalk => {
                            boundary_failure = Some(failure);
                            break 'boundary;
                        }
                        FacadeDiagnosticOwner::Prepass => return Err(failure),
                    }
                }
            };

            for claim in collect_route_claims(&output, &syntax) {
                routes.insert_claim(module_id, surface, claim);
            }
            let prepared = syntax.into_facade_prepared_source(output, string_base_len);
            if let Err(error) = cache.insert(module_id, surface, prepared) {
                match diagnostic_owner {
                    FacadeDiagnosticOwner::BoundaryWalk => {
                        boundary_failure = Some(PreparedFailure::from(error));
                        break 'boundary;
                    }
                    FacadeDiagnosticOwner::Prepass => return Err(PreparedFailure::from(error)),
                }
            }
        }
    }

    Ok(FacadeRoutePrepass {
        routes,
        cache,
        source_module_origins,
        unavailable_facades,
        failure: boundary_failure,
    })
}

/// Whether one facade preparation failure is the same-facade case-conflicting route diagnostic.
///
/// WHY: the header owner validates route case conflicts once, but Stage 0 must not defer that
///      publication failure to a walk where a competing facade's ambiguity can be reported first.
fn is_case_conflicting_route_failure(failure: &PreparedFailure) -> bool {
    matches!(
        failure,
        PreparedFailure::Diagnosed(batch)
            if batch.contains_code(CASE_CONFLICTING_PUBLIC_EXPORT_ROUTE.descriptor().code)
    )
}

/// Derive one facade's authored public routes from its retained header facts.
///
/// WHAT: reads the optional route from each public declaration header and each public
///       direct-selection dependency clause, rendering the authored path identity to owned text.
/// WHY: these are already-parsed facts on the same output the module walk will retain. Re-reading
///      source, walking tokens or reconstructing routes from path metadata would be a second parse
///      of one directive fact.
fn collect_route_claims(
    output: &FileFrontendPrepareOutput,
    syntax: &ModuleSyntaxDiscovery<'_, '_>,
) -> Vec<FacadeRouteClaim> {
    // One summary per exact route spelling. Repeated `$export @forms` groups are the same
    // namespace fact; case-distinct spellings stay separate so namespace route collision
    // detection can see both. The interim value keeps only authored position facts, so the final
    // claim can take ownership of the spelling key instead of storing a second copy.
    let mut claims = BTreeMap::<String, (SourceSpan, PathSyntaxId)>::new();
    let mut collect = |route: PublicExportRouteOperand| {
        let spelling = syntax.render_portable_path(route.path);
        let order = RouteClaimOrder::new(route.span.source(), route.path_syntax);
        match claims.entry(spelling) {
            Entry::Vacant(entry) => {
                entry.insert((route.span, route.path_syntax));
            }
            Entry::Occupied(mut entry) => {
                // The same exact spelling can arrive from the declaration header and a
                // re-export clause. Diagnostic order and collision spans must use the earliest
                // authored operand regardless of collection-pass order.
                let (existing_span, existing_path_syntax) = *entry.get();
                if order.precedes(RouteClaimOrder::new(
                    existing_span.source(),
                    existing_path_syntax,
                )) {
                    entry.insert((route.span, route.path_syntax));
                }
            }
        }
    };

    for header in &output.headers {
        if let Some(route) = header.export_mode.route() {
            collect(route);
        }
    }
    for clause in &output.file_dependency_clauses {
        if let Some(route) = clause.export_mode.route() {
            collect(route);
        }
    }
    let mut claims = claims.into_iter().collect::<Vec<_>>();
    // A BTreeMap iteration is spelling order, which puts `@FORMS` before `@forms`. A package's
    // first reported route collision must follow authored position, exactly as header
    // preparation orders its own route claims.
    claims.sort_by_key(|(_, (_, path_syntax))| *path_syntax);
    claims
        .into_iter()
        .map(|(spelling, (span, _))| FacadeRouteClaim {
            route: PublicExportRoute::from_canonical_spelling(spelling),
            span,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_takes_one_preparation_per_facade_exactly_once() {
        let mut cache = FacadePreparationCache::for_focused_test_without_prepass();
        let module_id = ModuleId::from_index(3);
        let prepared = empty_prepared_source();
        cache
            .insert(module_id, ModuleRootRole::Normal, prepared)
            .expect("the first facade preparation must be accepted");

        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .take(module_id, ModuleRootRole::Normal)
                .expect("the facade walk must receive its cached preparation")
                .is_ok(),
            "a prepared facade stays a success outcome"
        );
        assert!(
            cache.take(module_id, ModuleRootRole::Normal).is_none(),
            "a second walk must not see the same facade preparation"
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn exhaustive_cache_reports_that_every_present_facade_must_be_prepared() {
        let mut cache = FacadePreparationCache::exhaustive();
        assert!(cache.is_exhaustive());
        assert!(
            cache
                .take(ModuleId::from_index(4), ModuleRootRole::Normal)
                .is_none(),
            "a missing facade preparation must be observable as a cache miss"
        );

        let ordinary_cache = FacadePreparationCache::for_focused_test_without_prepass();
        assert!(
            !ordinary_cache.is_exhaustive(),
            "a cache constructed without a prepass keeps the ordinary preparation path"
        );
    }

    #[test]
    fn cache_rejects_a_second_preparation_of_one_facade() {
        let mut cache = FacadePreparationCache::for_focused_test_without_prepass();
        let module_id = ModuleId::from_index(2);
        cache
            .insert(module_id, ModuleRootRole::Support, empty_prepared_source())
            .expect("the first facade preparation must be accepted");
        let error = cache
            .insert(module_id, ModuleRootRole::Support, empty_prepared_source())
            .expect_err("a second preparation of one facade is a build invariant violation");

        assert!(
            format!("{error:?}").contains("prepared ModuleId 2 Support facade more than once"),
            "the error must name the duplicated facade: {error:?}"
        );
    }

    #[test]
    fn cache_takes_one_failure_per_facade_exactly_once() {
        let mut cache = FacadePreparationCache::for_focused_test_without_prepass();
        let module_id = ModuleId::from_index(5);
        cache
            .insert_failure(
                module_id,
                ModuleRootRole::Support,
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "test facade preparation failure",
                )
                .into(),
            )
            .expect("the first facade failure must be accepted");

        assert!(
            cache
                .take(module_id, ModuleRootRole::Support)
                .expect("the facade walk must receive its retained failure")
                .is_err(),
            "a diagnosed facade stays a failure outcome"
        );
        assert!(
            cache.take(module_id, ModuleRootRole::Support).is_none(),
            "a second walk must not see the same facade failure"
        );
    }

    fn empty_prepared_source() -> FacadePreparedSource {
        FacadePreparedSource {
            output: FileFrontendPrepareOutput::empty_for_test(),
            string_table: StringTable::new(),
            string_base_len: 0,
            path_fork: PathInternerFork::empty(),
            source_byte_len: 0,
            warnings: Vec::new(),
        }
    }
}
