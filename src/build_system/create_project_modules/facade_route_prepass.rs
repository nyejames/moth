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
//! across a boundary. Registered source-package boundaries can call the same per-boundary helper
//! when their route prepublication phase lands; this module has no project-index assumption other
//! than the identities it is handed.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerStage, DiagnosticBag, PreparedFailure};
use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareOutput;
use crate::compiler_frontend::headers::public_export_route_conflicts::{
    CaseFoldedRouteIndex, CaseRouteClaim, RouteClaimOrder, fold_route_spelling,
    try_intern_route_spelling,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, PublicExportRoute};
#[cfg(test)]
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::source::{SourceDatabase, SourceSpan, SourceSpanBuilders};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::style_directives::{
    PublicExportRouteOperand, StyleDirectiveRegistry,
};
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathInternerFork};
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
///      operand, so neither a boundary-local `PathId` nor an authored path table row may escape
///      the preparation that issued it.
#[derive(Debug)]
pub(super) struct FacadeRouteClaim {
    pub(super) route: PublicExportRoute,
    pub(super) span: SourceSpan,
    pub(super) path_syntax: PathSyntaxId,
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
                    path_syntax: PathSyntaxId::NONE,
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
/// WHAT: owns at most one prepared source per `(ModuleId, ModuleRootRole)` facade and hands it to
///       the walk by move.
/// WHY: the prepass and the walk must consume the same tokenized/parsed output. A take-only cache
///      makes a second preparation of one facade an explicit internal error instead of a silent
///      reparse.
#[derive(Default)]
pub(crate) struct FacadePreparationCache {
    by_facade: BTreeMap<(ModuleId, ModuleRootRole), FacadePreparedSource>,
    /// Whether the producing pass prepared every present facade root in this boundary.
    ///
    /// WHY: a project BFS must adopt, never re-prepare, a root the prepass already handled. Package
    ///      boundaries until Phase 3 pass an ordinary default cache and keep their existing path.
    exhaustive: bool,
}

impl FacadePreparationCache {
    /// Construct the cache produced by a complete boundary facade prepass.
    fn exhaustive() -> Self {
        Self {
            by_facade: BTreeMap::new(),
            exhaustive: true,
        }
    }

    /// Whether a present facade root missing from this cache is a compiler invariant failure.
    pub(super) fn is_exhaustive(&self) -> bool {
        self.exhaustive
    }

    /// Move one cached facade preparation into its module's discovery walk.
    pub(super) fn take(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
    ) -> Option<FacadePreparedSource> {
        self.by_facade.remove(&(module_id, surface))
    }

    fn insert(
        &mut self,
        module_id: ModuleId,
        surface: ModuleRootRole,
        prepared: FacadePreparedSource,
    ) -> Result<(), InfrastructureFailure> {
        if self
            .by_facade
            .insert((module_id, surface), prepared)
            .is_some()
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "facade route prepass prepared ModuleId {} {surface:?} facade more than once",
                    module_id.index()
                ),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.by_facade.len()
    }
}

/// One boundary's completed facade route prepublication.
///
/// WHAT: the owned route summaries, the facade preparations awaiting their walks, and the shared
///       source-origin table those walks reuse instead of rebuilding it.
/// WHY: all three are produced by the same per-boundary traversal and consumed by the same
///      namespace build and module inventory. Keeping them together avoids a second origin table
///      or a parallel facade scan.
pub(super) struct FacadeRoutePrepass {
    pub(super) routes: FacadeRouteIndex,
    pub(super) cache: FacadePreparationCache,
    pub(super) source_module_origins: Arc<SourceModuleOriginTable>,
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

    // `nodes()` is already the canonical `ModuleId` order the module walk seeds from, so neither
    // the preparation nor its diagnostics depend on filesystem traversal.
    for node in project_module_graph.nodes() {
        let module_id = node.module_id();
        #[cfg(feature = "timers")]
        let timing_module_key = crate::timing::register_timing_module_for_preparation(
            timing_boundary,
            module_id.index() as u32,
            node.stable_origin().logical_module_path(),
        );
        #[cfg(feature = "timers")]
        let timing_context = Some(crate::timing::TimingContext::for_module(timing_module_key));
        for (surface, root_path) in node.root_files().present() {
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
            let input = input_result.map_err(|error| {
                let path_table = Arc::new(syntax.path_fork_mut().shared_table());
                error.into_failure(syntax.string_table_mut(), path_table)
            })?;
            let output = syntax.prepare_source(input, source_spans)?;

            let claims = collect_route_claims(&output, &syntax);
            validate_facade_route_case_conflicts(&claims, string_table)?;
            for claim in claims {
                routes.insert_claim(module_id, surface, claim);
            }
            let prepared = syntax.into_facade_prepared_source(output, string_base_len);
            cache
                .insert(module_id, surface, prepared)
                .map_err(PreparedFailure::from)?;
        }
    }

    Ok(FacadeRoutePrepass {
        routes,
        cache,
        source_module_origins,
    })
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
    // namespace fact; case-distinct spellings stay separate so the conflict rule can see them.
    let mut claims = BTreeMap::<String, FacadeRouteClaim>::new();
    let mut collect = |route: PublicExportRouteOperand| {
        let spelling = syntax.render_portable_path(route.path);
        let order = RouteClaimOrder::new(route.span.source(), route.path_syntax);
        match claims.entry(spelling.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(FacadeRouteClaim {
                    route: PublicExportRoute::from_canonical_spelling(spelling),
                    span: route.span,
                    path_syntax: route.path_syntax,
                });
            }
            Entry::Occupied(mut entry) => {
                // The same exact spelling can arrive from the declaration header and a
                // re-export clause. Diagnostic order and collision spans must use the earliest
                // authored operand regardless of collection-pass order.
                let existing = entry.get();
                if order.precedes(RouteClaimOrder::new(
                    existing.span.source(),
                    existing.path_syntax,
                )) {
                    entry.insert(FacadeRouteClaim {
                        route: PublicExportRoute::from_canonical_spelling(spelling),
                        span: route.span,
                        path_syntax: route.path_syntax,
                    });
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
    let mut claims = claims.into_values().collect::<Vec<_>>();
    // A BTreeMap iteration is spelling order, which puts `@FORMS` before `@forms`. The conflict
    // diagnostic must order claims by authored position, exactly as the collector does.
    claims.sort_by_key(|claim| claim.path_syntax);
    claims
}

/// Reject a same-facade case-only route conflict in the prepass lane.
///
/// WHAT: folds every distinct authored route of one facade and reports the first conflicting pair
///       with both authored operands.
/// WHY: the namespace would otherwise make both consumer keys ambiguous and abort discovery at a
///      consumer import with MOTH-IMPORT-0006 before the facade's own MOTH-RULE-0092 can fire.
///      This uses the same shared folding/ordering rule as the header collector.
fn validate_facade_route_case_conflicts(
    claims: &[FacadeRouteClaim],
    string_table: &mut StringTable,
) -> Result<(), PreparedFailure> {
    let mut folded = CaseFoldedRouteIndex::<usize>::default();
    for (index, claim) in claims.iter().enumerate() {
        let key = fold_route_spelling(claim.route.as_str());
        if let Err((earlier, later)) = folded.insert(
            key,
            CaseRouteClaim::new(
                index,
                claim.span,
                RouteClaimOrder::new(claim.span.source(), claim.path_syntax),
            ),
        ) {
            return Err(case_conflicting_route_failure(
                &claims[earlier.id],
                &claims[later.id],
                string_table,
            ));
        }
    }
    Ok(())
}

/// Build the two-site `MOTH-RULE-0092` failure for one prepass-discovered route conflict.
fn case_conflicting_route_failure(
    earlier: &FacadeRouteClaim,
    later: &FacadeRouteClaim,
    string_table: &mut StringTable,
) -> PreparedFailure {
    let mut path_fork = PathInternerFork::empty();
    let earlier_path =
        try_intern_route_spelling(earlier.route.as_str(), &mut path_fork, string_table)
            .expect("an authored route re-interns within a fresh path domain");
    let later_path = try_intern_route_spelling(later.route.as_str(), &mut path_fork, string_table)
        .expect("an authored route re-interns within a fresh path domain");
    let diagnostic = generics_traits_api::case_conflicting_public_export_route(
        later_path,
        earlier_path,
        Some(earlier.span),
        Some(later.span),
    );
    let table = string_table.take_for_handoff();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        table,
        None,
        Some(Arc::new(path_fork.shared_table())),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_takes_one_preparation_per_facade_exactly_once() {
        let mut cache = FacadePreparationCache::default();
        let module_id = ModuleId::from_index(3);
        let prepared = empty_prepared_source();
        cache
            .insert(module_id, ModuleRootRole::Normal, prepared)
            .expect("the first facade preparation must be accepted");

        assert_eq!(cache.len(), 1);
        assert!(
            cache.take(module_id, ModuleRootRole::Normal).is_some(),
            "the facade walk must receive its cached preparation"
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

        let package_cache = FacadePreparationCache::default();
        assert!(
            !package_cache.is_exhaustive(),
            "package walks keep the ordinary preparation path until their prepass lands"
        );
    }

    #[test]
    fn cache_rejects_a_second_preparation_of_one_facade() {
        let mut cache = FacadePreparationCache::default();
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
