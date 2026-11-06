//! TIR expression-overlay collection and precedence.
//!
//! WHAT: collects structural and effective expression payloads into the keyed
//!       overlay input used by AST finalization.
//! WHY: overlay normalization has different ownership from read-only
//!      expression-site traversal. This module owns its temporary authority
//!      layers, precedence rules and cycle-guarded structural collection while
//!      `TirView` remains the production read authority.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashSet, hash_map::Entry};
use std::rc::Rc;

use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::ids::{ExpressionSiteId, TemplateWrapperSetId};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolutionKind,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::slot_plan::runtime_slot_plan_roots;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{
    TirView, TirViewIdentity, structural_transition_context, validate_context,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId, TemplateViewContext,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use rustc_hash::{FxHashMap, FxHashSet};

/// Collects committed structural expression IDs reachable from a template.
///
/// WHAT: traverses template structure below `template_id`, including runtime
///       slot-plan roots owned by that template, records dynamic-expression
///       payloads, conditional selectors and loop-header expressions keyed by their
///       `ExpressionSiteId`.
/// WHY: focused walker tests compare raw structural coverage with the effective
///      production collector without exposing collector internals.
/// This test-only entry intentionally remains raw structural collection: it
/// does not manufacture a durable view or import overlay authority.
#[cfg(test)]
pub(crate) fn collect_tir_expression_overlay_payloads<'store>(
    store: &'store TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<Vec<(ExpressionSiteId, ExprId)>, InfrastructureFailure> {
    let mut collector = ExpressionOverlayPayloadCollector::<'store>::new(None, None);
    collector.collect_template(store, template_id)?;
    Ok(collector.into_payloads())
}

/// Collects effective expression payloads from an exact root view.
///
/// The caller supplies the durable view so collection cannot reconstruct a
/// root, phase and context triple independently from the view authority.
pub(crate) fn collect_effective_tir_expression_overlay_payloads<'store>(
    root_view: &TirView<'store>,
    expressions: &'store AstExpressionBuilder,
) -> Result<Vec<(ExpressionSiteId, ExprId)>, InfrastructureFailure> {
    let mut collector =
        ExpressionOverlayPayloadCollector::new(Some(root_view.clone()), Some(expressions));
    collector.collect_template(root_view.store(), root_view.root_ref())?;
    Ok(collector.into_payloads())
}

/// Root of an expression-site key-space walk.
///
/// WHAT: distinguishes a published template entry from a bare node root, such as
///       a scratch subtree copy whose template entry does not exist yet.
/// WHY: reachability serves both retained template references and value-carried
///      subtree roots. A template root additionally carries conditional wrapper
///      and runtime slot-plan dimensions that a bare node root does not. A
///      derived node root is a bare node that will be published as the root of a
///      structurally derived template retaining the source template's
///      `conditional_child_wrapper_set`, so those wrappers stay reachable.
///      Callers pass node roots only for structure minted by the active copy
///      pass; those nodes own fresh identities rather than retained ones.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ReachableExpressionRoot {
    Template(TemplateIrId),
    Node(TemplateIrNodeId),
    DerivedNode {
        node: TemplateIrNodeId,
        conditional_child_wrapper_set: TemplateWrapperSetId,
    },
}

/// Immutable union of retained expression sites reachable through one view.
///
/// WHAT: records only the sites one reachability walk observed itself, shares
///       every child summary that walk inherited by reference, and carries a
///       compact transitive index of the recorded summaries those children
///       reach.
/// WHY: an enclosing copy rebase reaches the whole descendant union. Storing
///      that union flattened in every cache entry makes a nested chain retain
///      sets of size `1..n` and copy them on every cache hit. Sharing keeps one
///      entry proportional to the sites and inherited edges that walk itself
///      observed. The coverage index answers membership without re-walking the
///      descendant graph: a candidate site is retained exactly when the root
///      records it directly or when a summary that directly records it lies in
///      the root's coverage.
pub(crate) struct ReachableExpressionSites<'cache> {
    root: Rc<ReachableExpressionSiteSummary>,
    cache: &'cache ReachableExpressionSiteCache,
}

/// One inclusive range of dense summary creation indices.
///
/// Summary indices are assigned in recording order, so a nested chain's
/// coverage merges into a single range instead of growing one entry per level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReachableSummaryRange {
    start: u32,
    end: u32,
}

/// The summaries that directly recorded one expression site.
///
/// A site can be observed by independent walks because shared store-owned
/// structures are re-walked rather than inherited when a cache edge is absent.
/// Owners are appended in summary recording order, so the multi-owner list
/// stays sorted by dense summary index. The common case has exactly one owner,
/// so the first owner stays inline.
enum ReachableSiteOwners {
    One(u32),
    Many(Vec<u32>),
}

impl ReachableSiteOwners {
    fn record(&mut self, summary_id: u32) {
        match self {
            Self::One(owner) => {
                if *owner != summary_id {
                    *self = Self::Many(vec![*owner, summary_id]);
                }
            }
            Self::Many(owners) => owners.push(summary_id),
        }
    }

    fn is_in_coverage(&self, coverage: &[ReachableSummaryRange]) -> bool {
        match self {
            Self::One(summary_id) => summary_range_contains(coverage, *summary_id),
            // Owner ids are sorted, so each range only needs one probe.
            Self::Many(summary_ids) => coverage.iter().any(|range| {
                let first_in_range =
                    summary_ids.partition_point(|summary_id| *summary_id < range.start);
                summary_ids
                    .get(first_in_range)
                    .is_some_and(|summary_id| *summary_id <= range.end)
            }),
        }
    }
}

struct ReachableExpressionSiteSummary {
    /// Dense recording index; also names this summary inside a parent's
    /// coverage.
    id: u32,
    sites: FxHashSet<ExpressionSiteId>,
    inherited: Vec<Rc<ReachableExpressionSiteSummary>>,
    /// Sorted, merged, non-overlapping ranges of every summary this summary
    /// reaches, including itself. Ranges are inclusive.
    coverage: Vec<ReachableSummaryRange>,
}

impl ReachableExpressionSites<'_> {
    /// Returns whether `site_id` is reachable through any summary in the union.
    ///
    /// Test-only: production rebases ask [`Self::retained_candidates`] so one
    /// lookup can answer the whole source overlay.
    #[cfg(test)]
    pub(crate) fn contains(&self, site_id: &ExpressionSiteId) -> bool {
        self.any_summary(|summary| summary.sites.contains(site_id))
    }

    /// Returns the candidates this root still reaches.
    ///
    /// WHAT: tests the caller-provided candidate sites against the root's own
    ///       directly recorded sites and against an index of the summaries that
    ///       directly record each candidate.
    /// WHY: an overlay rebase asks about the sites its source overlay names, not
    ///      about every site in the reachable union. Descending the shared
    ///      summary graph per query made `n` nested rebases pay `O(n)` each
    ///      when a candidate was absent or deep, even though every summary was
    ///      already recorded. The owner index turns a candidate into the
    ///      summaries that observed it, and a summary's coverage ranges answer
    ///      whether the root reaches one of those summaries. Each candidate is
    ///      therefore resolved from compact identities and the numbered summary
    ///      DAG instead of a repeated descendant walk.
    pub(crate) fn retained_candidates(
        &self,
        candidates: impl IntoIterator<Item = ExpressionSiteId>,
    ) -> FxHashSet<ExpressionSiteId> {
        if self.root.sites.is_empty() && self.root.inherited.is_empty() {
            return FxHashSet::default();
        }

        let candidates: FxHashSet<ExpressionSiteId> = candidates.into_iter().collect();
        if candidates.is_empty() {
            return candidates;
        }

        let owners = self.cache.site_owners.borrow();
        candidates
            .into_iter()
            .filter(|site_id| {
                self.root.sites.contains(site_id)
                    || owners
                        .get(site_id)
                        .is_some_and(|owners| owners.is_in_coverage(&self.root.coverage))
            })
            .collect()
    }

    /// Number of distinct retained sites in the union.
    ///
    /// Test-only: production callers query their candidate sites instead of
    /// materialising the union.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        let mut sites = FxHashSet::default();
        self.any_summary(|summary| {
            sites.extend(summary.sites.iter().copied());
            false
        });
        sites.len()
    }

    #[cfg(test)]
    fn any_summary(
        &self,
        mut predicate: impl FnMut(&ReachableExpressionSiteSummary) -> bool,
    ) -> bool {
        // A summary only references summaries recorded before it, so the shared
        // graph is acyclic. Deduplicating allocations still keeps the traversal
        // linear when an enclosing walk recorded the same summary through
        // several structural edges.
        let mut visited: FxHashSet<*const ReachableExpressionSiteSummary> = FxHashSet::default();
        let mut pending = vec![&*self.root];
        while let Some(summary) = pending.pop() {
            if !visited.insert(summary as *const ReachableExpressionSiteSummary) {
                continue;
            }
            if predicate(summary) {
                return true;
            }
            pending.extend(summary.inherited.iter().map(|inherited| &**inherited));
        }
        false
    }
}

/// Whether the sorted coverage ranges contain one summary index.
fn summary_range_contains(coverage: &[ReachableSummaryRange], summary_id: u32) -> bool {
    coverage
        .binary_search_by(|range| {
            if range.end < summary_id {
                Ordering::Less
            } else if range.start > summary_id {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

/// Memo of copy-time reachability summaries for one copy pass.
///
/// WHAT: keys one shared retained-site summary by copied template root and the
///       reachability-relevant context dimensions, and names every template the
///       pass itself published.
/// WHY: a copied child root is walked once while its reference context is
///      rebased. Enclosing rebases reach the same `(template, context)` edge
///      and inherit that shared summary instead of walking the descendant tree
///      again or copying its site set. The cache lives inside the copy pass's
///      identity-remap scope, so no entry can outlive the structure it
///      describes. Expression overlays never affect structural reachability, so
///      the key normalizes that dimension away. Recording also indexes each
///      directly observed site by the dense summaries that recorded it, so a
///      later candidate query resolves against summary coverage instead of
///      re-walking the descendant graph. Interior mutability keeps the existing
///      immutable remap borrow at the copy call site.
#[derive(Default)]
pub(crate) struct ReachableExpressionSiteCache {
    summaries:
        RefCell<FxHashMap<(TemplateIrId, TemplateViewContext), Rc<ReachableExpressionSiteSummary>>>,
    /// Every recorded summary that directly observed an expression site, keyed
    /// by the site's compact dense identity.
    site_owners: RefCell<FxHashMap<ExpressionSiteId, ReachableSiteOwners>>,
    copied_templates: RefCell<FxHashSet<TemplateIrId>>,
    /// Dense creation index assigned to every summary this cache builds.
    next_summary_id: Cell<u32>,
    #[cfg(test)]
    hits: Cell<usize>,
}

impl ReachableExpressionSiteCache {
    /// Allocates the next dense summary index.
    fn allocate_summary_id(&self) -> u32 {
        let summary_id = self.next_summary_id.get();
        self.next_summary_id.set(
            summary_id
                .checked_add(1)
                .expect("TIR reachability summary count exceeds u32::MAX; this is a compiler bug"),
        );
        summary_id
    }

    /// Records one template published by the copy pass that owns this cache.
    ///
    /// Copied templates own freshly minted expression sites. Those sites exist
    /// only in the copied view and can never be named by a source overlay, so
    /// walks suppress them. Sites reached through shared store-owned structures
    /// (wrapper templates, resolved slot sources and retained plans) stay in the
    /// summary because those identities survive the copy unchanged.
    pub(crate) fn register_copied_template(&self, template_id: TemplateIrId) {
        self.copied_templates.borrow_mut().insert(template_id);
    }

    fn is_copied_template(&self, template_id: TemplateIrId) -> bool {
        self.copied_templates.borrow().contains(&template_id)
    }

    fn summary(
        &self,
        template_id: TemplateIrId,
        context: TemplateViewContext,
    ) -> Option<Rc<ReachableExpressionSiteSummary>> {
        let summary = self
            .summaries
            .borrow()
            .get(&(template_id, reachability_context(context)))
            .cloned();
        #[cfg(test)]
        {
            if summary.is_some() {
                self.hits.set(self.hits.get() + 1);
            }
        }
        summary
    }

    fn record(
        &self,
        root: ReachableExpressionRoot,
        context: TemplateViewContext,
        summary: &Rc<ReachableExpressionSiteSummary>,
    ) {
        let ReachableExpressionRoot::Template(template_id) = root else {
            return;
        };
        self.summaries.borrow_mut().insert(
            (template_id, reachability_context(context)),
            Rc::clone(summary),
        );

        // Only recorded summaries can be inherited by another walk, so only
        // their directly observed sites need an owner entry. A node root's own
        // sites stay in its summary and are tested directly by
        // `retained_candidates`.
        let mut site_owners = self.site_owners.borrow_mut();
        for site_id in &summary.sites {
            match site_owners.entry(*site_id) {
                Entry::Vacant(entry) => {
                    entry.insert(ReachableSiteOwners::One(summary.id));
                }
                Entry::Occupied(mut entry) => entry.get_mut().record(summary.id),
            }
        }
    }

    /// Number of enclosing walks that inherited a recorded child summary.
    #[cfg(test)]
    pub(crate) fn cached_lookup_hits(&self) -> usize {
        self.hits.get()
    }

    /// Number of sites stored directly across recorded summaries.
    ///
    /// Inherited sites stay shared through child summary references rather than
    /// being copied into each enclosing entry, so a nested chain must stay
    /// proportional to its levels instead of recording sets of size `1..n`.
    #[cfg(test)]
    pub(crate) fn directly_recorded_site_count(&self) -> usize {
        self.summaries
            .borrow()
            .values()
            .map(|summary| summary.sites.len())
            .sum()
    }

    /// Number of inherited-summary references recorded across summaries.
    #[cfg(test)]
    pub(crate) fn inherited_summary_count(&self) -> usize {
        self.summaries
            .borrow()
            .values()
            .map(|summary| summary.inherited.len())
            .sum()
    }

    /// Number of coverage ranges recorded across summaries.
    ///
    /// A nested chain merges each new dense summary index into the inherited
    /// range, so it stays at one range per summary. A summary whose inherited
    /// ranges are separated by recorded summaries outside its closure, such as
    /// an independently queried sibling above a shared child, records more than
    /// one range; the ranges remain exact.
    #[cfg(test)]
    pub(crate) fn coverage_range_count(&self) -> usize {
        self.summaries
            .borrow()
            .values()
            .map(|summary| summary.coverage.len())
            .sum()
    }

    /// Number of expression sites that have a directly recording summary.
    #[cfg(test)]
    pub(crate) fn owned_site_count(&self) -> usize {
        self.site_owners.borrow().len()
    }
}

/// Drops the expression dimension, which reachability never reads.
fn reachability_context(context: TemplateViewContext) -> TemplateViewContext {
    TemplateViewContext {
        expression_overlay: None,
        ..context
    }
}

/// Collects every retained expression site the root and context still reach.
///
/// WHAT: walks `root` with the same structural transitions `TirView` applies,
///       following the wrapper context of child occurrences, slot-owned wrapper
///       sets, resolved slot sources, template-level conditional wrappers and
///       retained runtime slot-plan roots, recording each reached site that
///       predates the copy pass in the same walk. Sites owned by templates this
///       pass published are suppressed: they are fresh identities no source
///       overlay can name. Retained numeric-intake positions are skipped,
///       matching every other overlay reader's completed-tree contract. When
///       `root` names a template, the collected summary is recorded in `cache`
///       so an enclosing rebase can inherit it instead of walking the template
///       again.
/// WHY: an expression overlay is the authority of one exact view context, and
///      expression authority is granted by `ExpressionSiteId` alone. A rebased
///      context may therefore keep an original site identity only while that
///      identity stays reachable through unchanged store-owned structure
///      (wrapper templates, resolved slot sources and shared plans).
///      Collecting site IDs during the reachability walk avoids a second
///      structural traversal and the payload map that traversal would build for
///      values this caller never reads. The cache reuses those per-root results
///      across the nested rebases of one copy pass; without it every enclosing
///      rebase would re-walk all descendant trees and a nested chain would cost
///      quadratic work in the copied node count. The cached summary shares
///      inherited child summaries by reference instead of cloning their site
///      sets, so an enclosing walk records only its own sites and edges: a
///      nested chain retains linear memory and pays constant-time cache hits
///      rather than copying flattened sets of size `1..n`. Dense summary
///      indices give each entry a compact coverage range over the recorded
///      summaries it reaches; that index is what makes later candidate queries
///      independent of descendant depth.
pub(crate) fn collect_reachable_expression_sites<'cache>(
    store: &TemplateIrStore,
    root: ReachableExpressionRoot,
    context: TemplateViewContext,
    cache: &'cache ReachableExpressionSiteCache,
) -> Result<ReachableExpressionSites<'cache>, InfrastructureFailure> {
    // The expression dimension cannot affect reachability: structural and wrapper
    // transitions always inherit the parent overlay. Validate the carried
    // dimensions once so a missing overlay fails here instead of on first use.
    validate_context(store, context, "TIR expression reachability")?;

    let mut walk = ReachableExpressionSiteWalk {
        cache,
        sites: FxHashSet::default(),
        inherited_sites: Vec::new(),
        visited_views: FxHashSet::default(),
        visited_nodes: FxHashSet::default(),
        visited_wrapper_sets: FxHashSet::default(),
    };
    walk.walk_root(store, root, context)?;

    let summary_id = cache.allocate_summary_id();
    let coverage = summary_coverage(&walk.inherited_sites, summary_id);
    let summary = Rc::new(ReachableExpressionSiteSummary {
        id: summary_id,
        sites: walk.sites,
        inherited: walk.inherited_sites,
        coverage,
    });
    cache.record(root, context, &summary);
    Ok(ReachableExpressionSites {
        root: summary,
        cache,
    })
}

/// Builds the exact coverage ranges of one summary from its inherited children.
///
/// WHAT: unions every inherited child's ranges with this summary's own dense
///       index, then sorts and merges overlapping or adjacent ranges.
/// WHY: coverage is a transitive set of summary indices, so a query can test
///      membership without walking the descendant graph. Dense post-order
///      indices make the common nested chain one merged range; a summary graph
///      with recorded summaries outside the closure between inherited ranges,
///      such as a shared child inherited by independent parents, leaves holes
///      between its ranges.
fn summary_coverage(
    inherited: &[Rc<ReachableExpressionSiteSummary>],
    summary_id: u32,
) -> Vec<ReachableSummaryRange> {
    let mut ranges: Vec<ReachableSummaryRange> = inherited
        .iter()
        .flat_map(|summary| summary.coverage.iter().copied())
        .collect();
    ranges.push(ReachableSummaryRange {
        start: summary_id,
        end: summary_id,
    });
    ranges.sort_unstable_by_key(|range| range.start);

    // Merge in place so one summary's coverage needs a single allocation.
    let mut merged_len = 0;
    for read in 0..ranges.len() {
        let range = ranges[read];
        if merged_len > 0 && range.start <= ranges[merged_len - 1].end.saturating_add(1) {
            ranges[merged_len - 1].end = ranges[merged_len - 1].end.max(range.end);
        } else {
            ranges[merged_len] = range;
            merged_len += 1;
        }
    }
    ranges.truncate(merged_len);
    ranges
}

/// Reachability state accumulated by one expression-site walk.
struct ReachableExpressionSiteWalk<'cache> {
    cache: &'cache ReachableExpressionSiteCache,
    /// Sites observed by this walk itself. Child summaries stay shared in
    /// `inherited_sites` rather than being copied in here.
    sites: FxHashSet<ExpressionSiteId>,
    inherited_sites: Vec<Rc<ReachableExpressionSiteSummary>>,
    visited_views: FxHashSet<(TemplateIrId, TemplateViewContext)>,
    visited_nodes: FxHashSet<(TemplateIrNodeId, TemplateViewContext, bool)>,
    visited_wrapper_sets: FxHashSet<(TemplateWrapperSetId, TemplateViewContext)>,
}

impl ReachableExpressionSiteWalk<'_> {
    fn walk_root(
        &mut self,
        store: &TemplateIrStore,
        root: ReachableExpressionRoot,
        context: TemplateViewContext,
    ) -> Result<(), InfrastructureFailure> {
        match root {
            ReachableExpressionRoot::Template(template_id) => {
                self.walk_template(store, template_id, context)
            }
            // Both node roots are freshly copied structure owned by this pass,
            // so their own sites are not retained identities.
            ReachableExpressionRoot::Node(node_id) => {
                self.walk_node(store, node_id, context, false)
            }
            ReachableExpressionRoot::DerivedNode {
                node,
                conditional_child_wrapper_set,
            } => {
                // The derived template preserves the source template-level
                // wrapper set, so its wrapper templates remain reachable even
                // though the copied bare root does not carry the set itself.
                self.walk_wrapper_set(store, conditional_child_wrapper_set, context)?;
                self.walk_node(store, node, context, false)
            }
        }
    }

    fn walk_template(
        &mut self,
        store: &TemplateIrStore,
        template_id: TemplateIrId,
        context: TemplateViewContext,
    ) -> Result<(), InfrastructureFailure> {
        if !self.visited_views.insert((template_id, context)) {
            return Ok(());
        }

        let template = store.get_template(template_id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("TIR expression reachability referenced missing template {template_id}."),
            )
        })?;

        if let Some(wrapper_set_id) = template.conditional_child_wrapper_set {
            self.walk_wrapper_set(store, wrapper_set_id, context)?;
        }

        // A retained plan keeps its own original render roots across a copy, so
        // those roots are reachable through the copied template and their
        // expression sites keep their original identities.
        if let Some(slot_plan_id) = template.runtime_slot_plan {
            for root in runtime_slot_plan_roots(store, slot_plan_id)? {
                self.walk_node(store, root, context, true)?;
            }
        }

        let record_sites = !self.cache.is_copied_template(template_id);
        self.walk_node(store, template.root, context, record_sites)
    }

    fn walk_wrapper_set(
        &mut self,
        store: &TemplateIrStore,
        wrapper_set_id: TemplateWrapperSetId,
        parent_context: TemplateViewContext,
    ) -> Result<(), InfrastructureFailure> {
        if !self
            .visited_wrapper_sets
            .insert((wrapper_set_id, parent_context))
        {
            return Ok(());
        }

        let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "TIR expression reachability referenced missing wrapper set {wrapper_set_id}."
                ),
            )
        })?;

        for wrapper in &wrapper_set.wrappers {
            let wrapper_context =
                structural_transition_context(parent_context, wrapper.phase, wrapper.context);
            self.walk_template(store, wrapper.root, wrapper_context)?;
        }

        Ok(())
    }

    fn walk_node(
        &mut self,
        store: &TemplateIrStore,
        node_id: TemplateIrNodeId,
        context: TemplateViewContext,
        record_sites: bool,
    ) -> Result<(), InfrastructureFailure> {
        if !self.visited_nodes.insert((node_id, context, record_sites)) {
            return Ok(());
        }

        let node = store.get_node(node_id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("TIR expression reachability referenced missing node {node_id}."),
            )
        })?;

        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                for child in children {
                    self.walk_node(store, *child, context, record_sites)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::DynamicExpression { site_id, .. } => {
                if record_sites {
                    self.sites.insert(*site_id);
                }
                Ok(())
            }

            TemplateIrNodeKind::Conditional {
                selector_site_id,
                body,
                ..
            } => {
                if record_sites {
                    self.sites.insert(*selector_site_id);
                }
                self.walk_node(store, *body, context, record_sites)
            }

            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper,
            } => {
                visit_loop_header_expression_sites(header, header_sites, |site_id, _| {
                    if record_sites {
                        self.sites.insert(site_id);
                    }
                    Ok(())
                })?;
                self.walk_node(store, *body, context, record_sites)?;
                if let Some(wrapper) = aggregate_wrapper {
                    self.walk_node(store, *wrapper, context, record_sites)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::ChildTemplate {
                reference,
                occurrence_id,
            } => {
                if let Some(overlay_id) = context.wrapper_context {
                    let overlay = store.wrapper_context_overlay(overlay_id).ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::AstConstruction,
                            format!(
                                "TIR expression reachability referenced missing wrapper-context overlay {overlay_id}."
                            ),
                        )
                    })?;
                    if let Some(wrapper_context) = overlay.context_for_occurrence(*occurrence_id)
                        && !wrapper_context.skip_parent_child_wrappers
                        && let Some(wrapper_set_id) = wrapper_context.inherited_wrapper_set
                    {
                        self.walk_wrapper_set(store, wrapper_set_id, context)?;
                    }
                }

                let child_context =
                    structural_transition_context(context, reference.phase, reference.context);
                // A copied child root was already walked when its reference
                // context was rebased. Inherit that exact summary by reference
                // instead of walking the descendant tree or copying its sites
                // once more per enclosing rebase.
                if let Some(inherited) = self.cache.summary(reference.root, child_context) {
                    if !inherited.sites.is_empty() || !inherited.inherited.is_empty() {
                        self.inherited_sites.push(inherited);
                    }
                    return Ok(());
                }
                self.walk_template(store, reference.root, child_context)
            }

            TemplateIrNodeKind::InsertContribution { template } => {
                self.walk_template(store, *template, context)
            }

            TemplateIrNodeKind::Slot { placeholder } => {
                // Slot-owned wrappers are applied around the injected content by
                // the same view that resolves this occurrence.
                if let Some(wrapper_set_id) = placeholder.child_wrapper_set {
                    self.walk_wrapper_set(store, wrapper_set_id, context)?;
                }
                if !placeholder.skip_parent_child_wrappers
                    && let Some(wrapper_set_id) = placeholder.applied_child_wrapper_set
                {
                    self.walk_wrapper_set(store, wrapper_set_id, context)?;
                }

                let Some(overlay_id) = context.slot_resolution else {
                    return Ok(());
                };
                let overlay = store.slot_resolution_overlay(overlay_id).ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        format!(
                            "TIR expression reachability referenced missing slot-resolution overlay {overlay_id}."
                        ),
                    )
                })?;
                if let Some(resolution) =
                    overlay.resolution_for_occurrence(placeholder.occurrence_id)
                    && let TirSlotResolutionKind::Resolved { sources } = &resolution.kind
                {
                    // Resolved sources retain the resolving view's context.
                    for source in sources {
                        self.walk_template(store, *source, context)?;
                    }
                }
                Ok(())
            }

            TemplateIrNodeKind::Text { .. }
            | TemplateIrNodeKind::AggregateOutput
            | TemplateIrNodeKind::RuntimeSlotSite { .. }
            | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(()),

            // A retained position is completed before overlay readers run, matching
            // every other overlay reader's completed-tree contract.
            TemplateIrNodeKind::RetainedNumericIntake(_) => Ok(()),
        }
    }
}

/// Visits every expression site a loop header owns, in source order.
///
/// WHAT: validates that the header variant and its stored site payload agree,
///       then yields each site together with its structural expression.
/// WHY: reachability only needs the site IDs, while payload collection needs the
///      structural expressions as override fallbacks. Sharing this traversal
///      keeps loop-header shape validation identical on both paths.
fn visit_loop_header_expression_sites(
    header: &TemplateLoopHeader,
    header_sites: &TemplateLoopHeaderExpressionSites,
    mut visit: impl FnMut(ExpressionSiteId, ExprId) -> Result<(), InfrastructureFailure>,
) -> Result<(), InfrastructureFailure> {
    match (header, header_sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
        ) => visit(*site_id, *condition),

        (
            TemplateLoopHeader::Range { range, .. },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            visit(*start, range.start)?;
            visit(*end, range.end)?;

            match (step, &range.step) {
                (Some(step_site_id), Some(step_expression)) => {
                    visit(*step_site_id, *step_expression)
                }
                (None, None) => Ok(()),
                _ => Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR expression overlay collection found mismatched range loop step site.",
                )),
            }
        }

        (
            TemplateLoopHeader::Collection { iterable, step, .. },
            TemplateLoopHeaderExpressionSites::Collection {
                iterable: site_id,
                step: step_site,
            },
        ) => {
            visit(*site_id, *iterable)?;

            match (step_site, step) {
                (Some(step_site), Some(step_expression)) => visit(*step_site, *step_expression),
                (None, None) => Ok(()),
                _ => Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR expression overlay collection found mismatched collection loop step site.",
                )),
            }
        }

        _ => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "TIR expression overlay collection found mismatched loop-header expression sites.",
        )),
    }
}

/// Replaces selected expression entries while preserving the other view
/// dimensions and every untouched expression override.
pub(crate) fn replace_expression_overlay_entries(
    store: &mut TemplateIrStore,
    base: TemplateViewContext,
    replacements: impl IntoIterator<Item = (ExpressionSiteId, ExprId)>,
) -> Result<TemplateViewContext, InfrastructureFailure> {
    let mut replacement_entries = Vec::new();
    let mut replacement_site_ids = FxHashSet::default();
    for (site_id, expression) in replacements {
        if !replacement_site_ids.insert(site_id) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "TIR expression overlay replacement received duplicate expression site {site_id}"
                ),
            ));
        }
        replacement_entries.push((site_id, expression));
    }

    let existing_overrides = if let Some(overlay_id) = base.expression_overlay {
        store
            .expression_overlay(overlay_id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                    "TIR expression overlay replacement referenced missing expression overlay {overlay_id}"
                ))
            })?
            .overrides
            .iter()
            .filter(|(site_id, _)| !replacement_site_ids.contains(site_id))
            .map(|(site_id, expression)| (*site_id, *expression))
            .collect()
    } else {
        Vec::new()
    };

    if replacement_entries.is_empty() {
        return Ok(base);
    }

    let mut overrides = existing_overrides;
    overrides.extend(replacement_entries);
    overrides.sort_unstable_by_key(|(site_id, _)| site_id.index());

    let overlay_id = store.allocate_expression_overlay(TirExpressionOverlay { overrides })?;
    Ok(TemplateViewContext {
        expression_overlay: Some(overlay_id),
        ..base
    })
}

/// Descendant expression overlays are temporary authority layers. The first
/// layer containing a site wins, so an outer structural context remains more
/// authoritative than an overlay introduced by a nested child reference.
#[derive(Default)]
struct ExpressionOverlayAuthority {
    layers: Vec<TirExpressionOverlayId>,
}

impl ExpressionOverlayAuthority {
    fn push_context(
        &mut self,
        store: &TemplateIrStore,
        context: TemplateViewContext,
    ) -> Result<bool, InfrastructureFailure> {
        let Some(overlay_id) = context.expression_overlay else {
            return Ok(false);
        };

        if store.expression_overlay(overlay_id).is_none() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "TIR expression overlay collection referenced missing expression overlay {overlay_id}"
                ),
            ));
        }

        self.layers.push(overlay_id);
        Ok(true)
    }

    fn pop_layer(&mut self) {
        let _ = self.layers.pop();
    }

    fn expression_for_site(
        &self,
        store: &TemplateIrStore,
        site_id: ExpressionSiteId,
    ) -> Result<Option<(ExprId, usize)>, InfrastructureFailure> {
        for (layer_index, overlay_id) in self.layers.iter().copied().enumerate() {
            let overlay = store.expression_overlay(overlay_id).ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                    "TIR expression overlay collection referenced missing expression overlay {overlay_id}"
                ))
            })?;

            if let Some(expression) = overlay.expression_for_site(site_id) {
                return Ok(Some((expression, layer_index + 1)));
            }
        }

        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ExpressionOverlayTemplateTraversalKey {
    template_id: TemplateIrId,
    view: Option<TirViewIdentity>,
}

struct ExpressionOverlayPayloadCollector<'store> {
    // Temporary exact view used to collect a normalized root overlay. It is
    // restored after each named transition and is never retained by a caller.
    effective_view: Option<TirView<'store>>,
    authority: ExpressionOverlayAuthority,
    expressions: Option<&'store AstExpressionBuilder>,
    payloads: FxHashMap<ExpressionSiteId, (ExprId, Option<usize>)>,
    active_nodes: HashSet<(TemplateIrNodeId, Option<TirViewIdentity>)>,
    completed_nodes: HashSet<(TemplateIrNodeId, Option<TirViewIdentity>)>,
    active_templates: HashSet<ExpressionOverlayTemplateTraversalKey>,
    completed_templates: HashSet<ExpressionOverlayTemplateTraversalKey>,
    active_slot_plans: HashSet<(TemplateSlotPlanId, Option<TirViewIdentity>)>,
    completed_slot_plans: HashSet<(TemplateSlotPlanId, Option<TirViewIdentity>)>,
}

impl<'store> ExpressionOverlayPayloadCollector<'store> {
    fn new(
        effective_view: Option<TirView<'store>>,
        expressions: Option<&'store AstExpressionBuilder>,
    ) -> Self {
        Self {
            expressions,
            effective_view,
            authority: ExpressionOverlayAuthority::default(),
            payloads: FxHashMap::default(),
            active_nodes: HashSet::new(),
            completed_nodes: HashSet::new(),
            active_templates: HashSet::new(),
            completed_templates: HashSet::new(),
            active_slot_plans: HashSet::new(),
            completed_slot_plans: HashSet::new(),
        }
    }

    fn into_payloads(self) -> Vec<(ExpressionSiteId, ExprId)> {
        let mut payloads: Vec<_> = self
            .payloads
            .into_iter()
            .map(|(site_id, (expression, _))| (site_id, expression))
            .collect();
        payloads.sort_unstable_by_key(|(site_id, _)| site_id.index());
        payloads
    }

    fn effective_expression(
        &self,
        store: &'store TemplateIrStore,
        site_id: ExpressionSiteId,
        structural_expression: ExprId,
    ) -> Result<(ExprId, Option<usize>), InfrastructureFailure> {
        if let Some(view) = &self.effective_view
            && let Some(expressions) = self.expressions
            && let Some(expression) = view.effective_expression_for_site(expressions, site_id)?
        {
            return Ok((expression.id(), Some(0)));
        }

        if let Some((expression, precedence)) =
            self.authority.expression_for_site(store, site_id)?
        {
            return Ok((expression, Some(precedence)));
        }

        Ok((structural_expression, None))
    }

    fn record_payload(
        &mut self,
        site_id: ExpressionSiteId,
        expression: ExprId,
        precedence: Option<usize>,
    ) {
        match self.payloads.entry(site_id) {
            Entry::Vacant(entry) => {
                entry.insert((expression, precedence));
            }
            Entry::Occupied(mut entry) => {
                let existing_precedence = entry.get().1;
                let should_replace = match (precedence, existing_precedence) {
                    (Some(candidate), Some(existing)) => candidate < existing,
                    (Some(_), None) => true,
                    (None, _) => false,
                };

                if should_replace {
                    entry.insert((expression, precedence));
                }
            }
        }
    }

    fn collect_template(
        &mut self,
        store: &'store TemplateIrStore,
        template_id: TemplateIrId,
    ) -> Result<(), InfrastructureFailure> {
        let traversal_key = ExpressionOverlayTemplateTraversalKey {
            template_id,
            view: self.current_view_identity(),
        };
        if self.effective_view.is_none() && self.completed_templates.contains(&traversal_key) {
            return Ok(());
        }

        if !self.active_templates.insert(traversal_key) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR expression overlay collection found a recursive child-template reference.",
            ));
        }

        let (root, runtime_slot_plan) = store
            .get_template(template_id)
            .map(|template| (template.root, template.runtime_slot_plan))
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR expression overlay collection referenced a missing child template.",
                )
            })?;
        let result = if let Some(slot_plan_id) = runtime_slot_plan {
            self.collect_runtime_slot_application(store, root, slot_plan_id)
        } else {
            self.collect_node(store, root)
        };

        self.active_templates.remove(&traversal_key);
        if result.is_ok() {
            self.completed_templates.insert(traversal_key);
        }

        result
    }

    fn collect_runtime_slot_application(
        &mut self,
        store: &'store TemplateIrStore,
        wrapper_root: TemplateIrNodeId,
        slot_plan_id: TemplateSlotPlanId,
    ) -> Result<(), InfrastructureFailure> {
        let traversal_key = (slot_plan_id, self.current_view_identity());
        if self.effective_view.is_none() && self.completed_slot_plans.contains(&traversal_key) {
            return self.collect_node(store, wrapper_root);
        }

        if !self.active_slot_plans.insert(traversal_key) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR expression overlay collection found a recursive runtime slot plan.",
            ));
        }

        let roots = runtime_slot_plan_roots(store, slot_plan_id)?;

        let result = self.collect_node(store, wrapper_root).and_then(|()| {
            for root in roots {
                self.collect_node(store, root)?;
            }

            Ok(())
        });

        self.active_slot_plans.remove(&traversal_key);
        if result.is_ok() {
            self.completed_slot_plans.insert(traversal_key);
        }

        result
    }

    fn collect_node(
        &mut self,
        store: &'store TemplateIrStore,
        node_id: TemplateIrNodeId,
    ) -> Result<(), InfrastructureFailure> {
        let traversal_key = (node_id, self.current_view_identity());
        if self.effective_view.is_none() && self.completed_nodes.contains(&traversal_key) {
            return Ok(());
        }

        if !self.active_nodes.insert(traversal_key) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR expression overlay collection found a recursive node reference.",
            ));
        }

        let result = self.collect_node_contents(store, node_id);

        self.active_nodes.remove(&traversal_key);
        if result.is_ok() {
            self.completed_nodes.insert(traversal_key);
        }

        result
    }

    fn collect_node_contents(
        &mut self,
        store: &'store TemplateIrStore,
        node_id: TemplateIrNodeId,
    ) -> Result<(), InfrastructureFailure> {
        let node = store.get_node(node_id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR expression overlay collection referenced a missing node.",
            )
        })?;

        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                for &child in children {
                    self.collect_node(store, child)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::DynamicExpression {
                expression,
                site_id,
                ..
            } => {
                let (expression, precedence) =
                    self.effective_expression(store, *site_id, *expression)?;
                self.record_payload(*site_id, expression, precedence);
                Ok(())
            }

            TemplateIrNodeKind::Conditional {
                selector,
                selector_site_id,
                body,
            } => {
                let (expression, precedence) = self.effective_expression(
                    store,
                    *selector_site_id,
                    selector.condition_expression(),
                )?;
                self.record_payload(*selector_site_id, expression, precedence);
                self.collect_node(store, *body)?;
                Ok(())
            }

            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper,
                ..
            } => {
                self.collect_loop_header_payloads(store, header, header_sites)?;
                self.collect_node(store, *body)?;

                if let Some(wrapper) = aggregate_wrapper {
                    self.collect_node(store, *wrapper)?;
                }

                Ok(())
            }

            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                self.collect_structural_child(store, *reference)
            }

            TemplateIrNodeKind::InsertContribution { template } => self.collect_in_transition(
                store,
                *template,
                Some(|view: &TirView<'store>| view.structural_helper(*template)),
            ),

            TemplateIrNodeKind::Text { .. }
            | TemplateIrNodeKind::Slot { .. }
            | TemplateIrNodeKind::AggregateOutput
            | TemplateIrNodeKind::RuntimeSlotSite { .. }
            | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(()),
            // A retained position contributes no expression payload: its value is installed by
            // body-root completion, and every overlay reader runs on the completed tree.
            TemplateIrNodeKind::RetainedNumericIntake(_) => Ok(()),
        }
    }

    fn collect_loop_header_payloads(
        &mut self,
        store: &'store TemplateIrStore,
        header: &TemplateLoopHeader,
        header_sites: &TemplateLoopHeaderExpressionSites,
    ) -> Result<(), InfrastructureFailure> {
        visit_loop_header_expression_sites(header, header_sites, |site_id, expression| {
            let (expression, precedence) = self.effective_expression(store, site_id, expression)?;
            self.record_payload(site_id, expression, precedence);
            Ok(())
        })
    }

    fn collect_structural_child(
        &mut self,
        store: &'store TemplateIrStore,
        reference: TemplateTirChildReference,
    ) -> Result<(), InfrastructureFailure> {
        let added_layer = if self.effective_view.is_some() {
            self.authority.push_context(store, reference.context)?
        } else {
            false
        };

        let result = self.collect_in_transition(
            store,
            reference.root,
            Some(|view: &TirView<'store>| view.structural_child(reference)),
        );

        if added_layer {
            self.authority.pop_layer();
        }

        result
    }

    fn current_view_identity(&self) -> Option<TirViewIdentity> {
        self.effective_view.as_ref().map(TirView::identity)
    }

    fn collect_in_transition(
        &mut self,
        store: &'store TemplateIrStore,
        template_id: TemplateIrId,
        transition: Option<
            impl FnOnce(&TirView<'store>) -> Result<TirView<'store>, InfrastructureFailure>,
        >,
    ) -> Result<(), InfrastructureFailure> {
        let parent_view = self.effective_view.take();
        let child_view = match (parent_view.as_ref(), transition) {
            (Some(parent_view), Some(transition)) => match transition(parent_view) {
                Ok(child_view) => Some(child_view),
                Err(error) => {
                    self.effective_view = Some(parent_view.clone());
                    return Err(error);
                }
            },
            _ => None,
        };

        self.effective_view = child_view;
        let result = self.collect_template(store, template_id);
        self.effective_view = parent_view;
        result
    }
}
