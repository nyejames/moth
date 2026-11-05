//! TIR-native active-context subtree copying.
//!
//! WHAT: deep-copies finalized TIR subtrees into fresh trees while applying an
//! optional active slot-plan context to unresolved `Slot` placeholders and
//! rebasing retained view overlays to the copied identity domain.
//!
//! WHY: control-flow bodies and runtime slot wrapper roots must be copied
//! without mutating the stored originals, while still honoring the active
//! slot-plan cursor semantics required by nested runtime slot wrappers.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::expression_overlays::ReachableExpressionSiteCache;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
    TemplateSlotPlanId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    RetainedControlFlowRenderUnit, RetainedNumericIntakeRole, TemplateIrNode, TemplateIrNodeKind,
    TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::slot_plan::convert_runtime_slot_site;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::{
    ReachableExpressionRoot, collect_reachable_expression_sites,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;

use crate::compiler_frontend::ast::templates::tir::copy_state::TirCopyState;
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolutionOverlay,
    TirSlotResolutionOverlayId, TirWrapperContextOverlay, TirWrapperContextOverlayId,
};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;

/// Destination identities minted for one source occurrence/site in a copy pass.
///
/// A source template can be referenced more than once inside the same copied
/// subtree, and every reference receives its own fresh identities. The common
/// case is exactly one destination, so it stays inline and duplicates only spill
/// into a vector when the same source structure is copied again. Destinations
/// are appended in recording order and each remembers the pass-wide record index
/// that minted it, so a scope lookup binary-searches a contiguous slice instead
/// of scanning every destination of a reused source identity.
pub(super) enum CopiedIdentityDestinations<Id> {
    One(CopiedIdentity<Id>),
    Many(Vec<CopiedIdentity<Id>>),
}

/// The pass-wide record-index range minted while one copy scope was active.
///
/// Scopes nest: a child template copy starts after its parent and finishes
/// before it, so every identity recorded while the child is active lands in the
/// child's `[start_record, end_record)` slice. A source identity's destinations
/// are appended in exactly this record order, which makes that slice contiguous
/// in its destination list. Representing a scope by the records it produced,
/// rather than by a numeric scope id, also removes any ordering assumption: a
/// parent that records a shared identity again after a nested copy simply
/// appends after it instead of shifting earlier destinations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CopyIdentityScope {
    pub(super) start_record: u64,
    pub(super) end_record: u64,
}

/// One copied identity and the pass-wide record index that minted it.
#[derive(Clone, Copy, Debug)]
pub(super) struct CopiedIdentity<Id> {
    record: u64,
    copied: Id,
}

impl<Id: Copy> CopiedIdentityDestinations<Id> {
    /// Appends one destination.
    ///
    /// Record indexes grow with every recorded identity, which is precisely the
    /// order destinations arrive here, so appending keeps the list ordered by
    /// record index. A shared identity recorded by nested and enclosing scopes
    /// in any interleaving therefore stays linear in the number of copies
    /// instead of shifting earlier entries on every enclosing-scope record.
    fn push(&mut self, copied: CopiedIdentity<Id>) {
        match self {
            Self::One(first) => {
                debug_assert!(
                    first.record < copied.record,
                    "destinations must be appended in record order"
                );
                let first = *first;
                *self = Self::Many(vec![first, copied]);
            }
            Self::Many(copied_ids) => {
                debug_assert!(
                    copied_ids
                        .last()
                        .is_none_or(|last| last.record < copied.record),
                    "destinations must be appended in record order"
                );
                copied_ids.push(copied);
            }
        }
    }

    /// Iterates the destinations recorded inside `scope`.
    ///
    /// A destination belongs to the scope exactly when its record index falls in
    /// the scope's `[start_record, end_record)` range. Because the list is
    /// appended in record order, the matching destinations form one contiguous
    /// slice located with two binary searches: `O(log n + copies)` instead of a
    /// scan or an ordered insertion that shifts earlier entries.
    pub(super) fn iter_in_scope(&self, scope: CopyIdentityScope) -> impl Iterator<Item = Id> + '_ {
        let copied = match self {
            Self::One(copied) => std::slice::from_ref(copied),
            Self::Many(copied_ids) => copied_ids.as_slice(),
        };
        let range_start = copied.partition_point(|entry| entry.record < scope.start_record);
        let range_end = copied.partition_point(|entry| entry.record < scope.end_record);
        copied[range_start..range_end]
            .iter()
            .map(|entry| entry.copied)
    }
}

/// Records one copied identity for `source` at the pass-wide `record` index.
pub(super) fn record_copied_identity<Key, Id>(
    identities: &mut FxHashMap<Key, CopiedIdentityDestinations<Id>>,
    record: u64,
    source: Key,
    copied: Id,
) where
    Key: Eq + std::hash::Hash,
    Id: Copy,
{
    let copied = CopiedIdentity { record, copied };
    match identities.entry(source) {
        Entry::Vacant(entry) => {
            entry.insert(CopiedIdentityDestinations::One(copied));
        }
        Entry::Occupied(mut entry) => entry.get_mut().push(copied),
    }
}

/// Records all source identity copies minted while one subtree root is copied.
///
/// A copied child reference retains its contextual dimensions, so those
/// dimensions must be rebuilt against the copied key space before the reference
/// is published. One map serves the whole recording pass: every destination is
/// tagged with the pass-wide record index that minted it, and a reference
/// context selects the destinations whose record index falls inside its own
/// `CopyIdentityScope` with a binary range search. A parent scope therefore
/// never rehashes or duplicates descendant mappings, and callers that never
/// rebase a context allocate no map at all. The same pass owns its reachability
/// memo, so a nested reference rebase records the copied child summaries an
/// enclosing rebase later inherits instead of re-walking the same descendant
/// copies.
#[derive(Default)]
struct TirCopyIdentityRemap {
    expression_sites: FxHashMap<ExpressionSiteId, CopiedIdentityDestinations<ExpressionSiteId>>,
    slot_occurrences: FxHashMap<SlotOccurrenceId, CopiedIdentityDestinations<SlotOccurrenceId>>,
    child_occurrences:
        FxHashMap<ChildTemplateOccurrenceId, CopiedIdentityDestinations<ChildTemplateOccurrenceId>>,
    reachability: ReachableExpressionSiteCache,
    next_record: u64,
}

impl TirCopyIdentityRemap {
    /// Marks the record index at which the scope starts recording.
    fn begin_scope(&mut self) -> u64 {
        self.next_record
    }

    /// Closes a scope opened by `begin_scope` and returns its record range.
    ///
    /// Every identity recorded since the matching `begin_scope` call is inside
    /// the returned range, including identities minted by nested template
    /// copies, which matches the template-wide selection a reference context
    /// needs.
    fn end_scope(&mut self, start_record: u64) -> CopyIdentityScope {
        CopyIdentityScope {
            start_record,
            end_record: self.next_record,
        }
    }

    fn next_record(&mut self) -> u64 {
        // At most three distinct u32 identity domains can be recorded per copy.
        let record = self.next_record;
        self.next_record += 1;
        record
    }

    fn record_expression_site(&mut self, source: ExpressionSiteId, copied: ExpressionSiteId) {
        let record = self.next_record();
        record_copied_identity(&mut self.expression_sites, record, source, copied);
    }

    fn record_slot_occurrence(&mut self, source: SlotOccurrenceId, copied: SlotOccurrenceId) {
        let record = self.next_record();
        record_copied_identity(&mut self.slot_occurrences, record, source, copied);
    }

    fn record_child_occurrence(
        &mut self,
        source: ChildTemplateOccurrenceId,
        copied: ChildTemplateOccurrenceId,
    ) {
        let record = self.next_record();
        record_copied_identity(&mut self.child_occurrences, record, source, copied);
    }
}

/// Copies a finalized TIR subtree into a fresh tree, applying an optional active
/// slot-plan context to any unresolved `Slot` placeholders.
///
/// WHAT: walks the source subtree starting at `source_node_id` and pushes a
///       freshly allocated mirror into the same store. `Slot` nodes are converted
///       to `RuntimeSlotSite` nodes when `active_slot_plan` matches the cursor,
///       exactly as atom-based materialization would do. `ChildTemplate` and
///       `InsertContribution` references are deep-copied as fresh template
///       entries; runtime-slot-handoff children keep their own plan and are not
///       reprocessed under the parent's active plan.
/// WHY: this lets `materialize_loop` reuse a module-local finalized loop body root
///      without mutating the stored root in place, while still honoring the
///      active slot-plan cursor semantics required by nested runtime slot
///      wrappers. Runtime slot handoff planning also uses it to copy a
///      module-local finalized wrapper root when that root is runtime-kind-safe.
pub(crate) fn copy_tir_subtree_with_active_slot_plan(
    source_node_id: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<TemplateIrNodeId, TemplateError> {
    increment_ast_counter(AstCounter::TirCopyPasses);
    copy_tir_node_with_active_slot_plan(source_node_id, active_slot_plan, store, copy_state, None)
}

/// Copies a finalized TIR subtree and rebases a value-carried view context onto
/// the copied identity domain while retaining a source template's conditional
/// child wrapper set for reachability.
///
/// WHAT: performs the deep copy, then rewrites every occurrence/site key of
///       `view_context` that names a node from the copied subtree. Every fresh
///       copy of a reused source site receives the override. Expression entries
///       whose source site was not copied are kept only while the rebased context
///       still reaches that unchanged site through wrapper applications, resolved
///       slot sources, descendant reference contexts or shared runtime slot
///       plans; the rest are dropped because they belong to a replaced structural
///       root. The other overlay dimensions keep every entry whose occurrence key
///       is still part of an unchanged reference. `retained_template_wrapper_set`
///       names the source template's `conditional_child_wrapper_set` when the
///       copied node will become the root of a structurally derived template
///       that preserves it.
/// WHY: occurrence-keyed overlays (notably `wrapper_context` for `$children(..)`
///      wrappers) describe the *source* root. A runtime slot application replaces
///      that root with a scratch copy whose child occurrences are fresh, so the
///      receiver's context must move to the copied keys or fixed direct children
///      silently lose their inherited wrappers and named content. Wrapper
///      templates stay shared, so their expression overrides must keep applying
///      after the rebase. Runtime-slot plan materialization copies a receiver
///      template's *bare root* and later publishes it through
///      `push_runtime_slot_derived_template`, which preserves that template-level
///      wrapper set; rebasing from the node root alone cannot discover the set,
///      so overrides that only the unchanged wrapper templates use would
///      otherwise be removed as unreachable.
pub(crate) fn copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
    source_node_id: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    view_context: TemplateViewContext,
    retained_template_wrapper_set: Option<TemplateWrapperSetId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<(TemplateIrNodeId, TemplateViewContext), TemplateError> {
    // A fully empty context has no occurrence or expression key to rebase, so
    // the copy must not retain identity maps for it. Nested reference contexts
    // still record their own scoped identities while the tree is copied.
    if view_context == TemplateViewContext::default() {
        let copied_node_id = copy_tir_subtree_with_active_slot_plan(
            source_node_id,
            active_slot_plan,
            store,
            copy_state,
        )?;
        return Ok((copied_node_id, view_context));
    }

    let (copied_node_id, identity_remap, identity_scope) = copy_tir_subtree_collecting_identities(
        source_node_id,
        active_slot_plan,
        store,
        copy_state,
    )?;
    let reached_root = match retained_template_wrapper_set {
        Some(conditional_child_wrapper_set) => ReachableExpressionRoot::DerivedNode {
            node: copied_node_id,
            conditional_child_wrapper_set,
        },
        None => ReachableExpressionRoot::Node(copied_node_id),
    };
    let copied_context = remap_view_context(
        store,
        view_context,
        &identity_remap,
        identity_scope,
        reached_root,
    )
    .map_err(TemplateError::from)?;
    Ok((copied_node_id, copied_context))
}

/// Copies one subtree while recording every minted identity in one flat map.
///
/// The returned scope addresses exactly the copies made by this pass, including
/// all nested template copies, so the caller can rebase a context against the
/// whole pass without any per-scope map folding.
fn copy_tir_subtree_collecting_identities(
    source_node_id: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<(TemplateIrNodeId, TirCopyIdentityRemap, CopyIdentityScope), TemplateError> {
    increment_ast_counter(AstCounter::TirCopyPasses);
    let mut identity_remap = TirCopyIdentityRemap::default();
    let root_scope_start = identity_remap.begin_scope();
    let copied_node_id = copy_tir_node_with_active_slot_plan(
        source_node_id,
        active_slot_plan,
        store,
        copy_state,
        Some(&mut identity_remap),
    )?;
    let root_scope = identity_remap.end_scope(root_scope_start);
    Ok((copied_node_id, identity_remap, root_scope))
}

/// Recursively copies one TIR node, translating child-template references into
/// fresh template entries and applying the active slot-plan cursor to `Slot`
/// placeholders.
fn copy_tir_node_with_active_slot_plan(
    source_node_id: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
    mut identity_remap: Option<&mut TirCopyIdentityRemap>,
) -> Result<TemplateIrNodeId, TemplateError> {
    let source_node = store.get_node(source_node_id).cloned().ok_or_else(|| {
        TemplateError::from(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "active-context TIR copy: source node ID was not present in the store.",
        ))
    })?;

    let span = source_node.span;

    match source_node.kind {
        TemplateIrNodeKind::Text {
            text,
            byte_len,
            origin,
        } => {
            copy_state.record_text_node(byte_len);

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Text {
                    text,
                    byte_len,
                    origin,
                },
                span,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::DynamicExpression {
            expression,
            origin,
            site_id,
        } => {
            copy_state.record_dynamic_expression();

            let copied_site_id = remap_expression_site_id(store);
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_expression_site(site_id, copied_site_id);
            }
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::DynamicExpression {
                    expression,
                    origin,
                    site_id: copied_site_id,
                },
                span,
            ));
            store
                .replicate_pending_numeric_intakes_for_copied_node(source_node_id, node_id)
                .map_err(TemplateError::from)?;
            Ok(node_id)
        }

        TemplateIrNodeKind::Slot { mut placeholder } => {
            if let Some(plan_id) = active_slot_plan {
                let site_id = copy_state
                    .next_runtime_slot_site_for_key(plan_id, &placeholder.key, store)
                    .ok_or_else(|| {
                        TemplateError::from(InfrastructureFailure::internal(CompilerStage::AstConstruction,
                            "active-context TIR copy: no matching runtime slot site for a slot placeholder.",
                        ))
                    })?;

                return Ok(convert_runtime_slot_site(
                    plan_id, site_id, store, copy_state, span,
                ));
            }

            copy_state.record_slot();
            let source_occurrence_id = placeholder.occurrence_id;
            let copied_occurrence_id = store.next_slot_occurrence_id();
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_slot_occurrence(source_occurrence_id, copied_occurrence_id);
            }
            placeholder.occurrence_id = copied_occurrence_id;
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Slot { placeholder },
                span,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::RuntimeSlotSite { plan, site } => {
            copy_state.record_existing_runtime_slot_site();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RuntimeSlotSite { plan, site },
                span,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::RuntimeSlotContributionSource {
            plan,
            source,
            shape,
            control_flow,
        } => {
            copy_state.record_runtime_slot_contribution_source();
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RuntimeSlotContributionSource {
                    plan,
                    source,
                    shape,
                    control_flow,
                },
                span,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let needs_context_rebase = reference.context != TemplateViewContext::default();

            // A child context only names copies inside its own template, so a
            // copy without any enclosing recording pass records into a local
            // map that is dropped after this reference is rebased. When an
            // enclosing pass is recording, the child shares its flat map and
            // its scope range selects exactly this template copy.
            let mut scoped_identity_remap = None;
            let child_identity_remap = match identity_remap.as_deref_mut() {
                Some(identity_remap) => Some(identity_remap),
                None if needs_context_rebase => {
                    scoped_identity_remap = Some(TirCopyIdentityRemap::default());
                    scoped_identity_remap.as_mut()
                }
                None => None,
            };

            let (new_child_id, child_scope) = copy_tir_template_with_active_slot_plan(
                reference.root,
                active_slot_plan,
                store,
                copy_state,
                child_identity_remap,
            )?;
            copy_state.record_child_template();

            let reference_context = if needs_context_rebase {
                let child_scope =
                    child_scope.expect("a recorded child-template copy must report its copy scope");
                let child_identity_remap: &TirCopyIdentityRemap =
                    match identity_remap.as_deref_mut() {
                        Some(identity_remap) => identity_remap,
                        None => scoped_identity_remap
                            .as_ref()
                            .expect("a child context copy must record identities"),
                    };
                remap_view_context(
                    store,
                    reference.context,
                    child_identity_remap,
                    child_scope,
                    ReachableExpressionRoot::Template(new_child_id),
                )?
            } else {
                reference.context
            };
            let copied_occurrence_id = store.next_child_template_occurrence_id();
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_child_occurrence(occurrence_id, copied_occurrence_id);
            }
            let reference = reference
                .with_root(new_child_id)
                .with_context(reference_context);
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id: copied_occurrence_id,
                },
                span,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let (new_child_id, _child_scope) = copy_tir_template_with_active_slot_plan(
                template,
                active_slot_plan,
                store,
                copy_state,
                identity_remap.as_deref_mut(),
            )?;
            copy_state.record_insert_contribution();

            let authored_name = store.insert_contribution_name(source_node_id);
            let node_id = store.push_insert_contribution(new_child_id, authored_name, span);
            Ok(node_id)
        }

        TemplateIrNodeKind::Sequence { children } => {
            copy_state.enter_depth();
            let new_children = children
                .into_iter()
                .map(|child_id| {
                    copy_tir_node_with_active_slot_plan(
                        child_id,
                        active_slot_plan,
                        store,
                        copy_state,
                        identity_remap.as_deref_mut(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            copy_state.exit_depth();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: new_children,
                },
                span,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            copy_state.record_control_flow();
            copy_state.enter_depth();
            let new_body = copy_tir_node_with_active_slot_plan(
                body,
                active_slot_plan,
                store,
                copy_state,
                identity_remap.as_deref_mut(),
            )?;
            let copied_selector_site_id = remap_expression_site_id(store);
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_expression_site(selector_site_id, copied_selector_site_id);
            }
            copy_state.exit_depth();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Conditional {
                    selector,
                    selector_site_id: copied_selector_site_id,
                    body: new_body,
                },
                span,
            ));
            store
                .replicate_pending_numeric_intakes_for_copied_node(source_node_id, node_id)
                .map_err(TemplateError::from)?;
            Ok(node_id)
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
        } => {
            copy_state.record_control_flow();
            copy_state.enter_depth();
            let new_body = copy_tir_node_with_active_slot_plan(
                body,
                active_slot_plan,
                store,
                copy_state,
                identity_remap.as_deref_mut(),
            )?;
            let header_sites = remap_loop_header_expression_sites(
                store,
                header_sites,
                identity_remap.as_deref_mut(),
            );
            let new_aggregate_wrapper = aggregate_wrapper
                .map(|wrapper_id| {
                    copy_tir_node_with_active_slot_plan(
                        wrapper_id,
                        active_slot_plan,
                        store,
                        copy_state,
                        identity_remap.as_deref_mut(),
                    )
                })
                .transpose()?;
            copy_state.exit_depth();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Loop {
                    header,
                    header_sites,
                    body: new_body,
                    aggregate_wrapper: new_aggregate_wrapper,
                },
                span,
            ));
            store
                .replicate_pending_numeric_intakes_for_copied_node(source_node_id, node_id)
                .map_err(TemplateError::from)?;
            Ok(node_id)
        }

        TemplateIrNodeKind::AggregateOutput => {
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::AggregateOutput,
                span,
            ));
            Ok(node_id)
        }

        // A copied retained position is its own document-order splice: the copy re-mints every
        // expression site the position owns, copies the subtree its role names, and registers its
        // own intake under the same body domain.
        TemplateIrNodeKind::RetainedNumericIntake(role) => {
            let copied_role = match role {
                RetainedNumericIntakeRole::HeadExpression { expression_site } => {
                    copy_state.record_dynamic_expression();

                    let copied_site = remap_expression_site_id(store);
                    if let Some(identity_remap) = identity_remap.as_deref_mut() {
                        identity_remap.record_expression_site(expression_site, copied_site);
                    }

                    RetainedNumericIntakeRole::HeadExpression {
                        expression_site: copied_site,
                    }
                }

                RetainedNumericIntakeRole::ConditionalLoopCondition {
                    header_sites,
                    body,
                    render_unit,
                } => {
                    copy_state.record_control_flow();
                    let (copied_body, copied_render_unit) = copy_retained_control_flow_body(
                        body,
                        render_unit,
                        active_slot_plan,
                        store,
                        copy_state,
                        identity_remap.as_deref_mut(),
                    )?;

                    RetainedNumericIntakeRole::ConditionalLoopCondition {
                        header_sites: remap_loop_header_expression_sites(
                            store,
                            header_sites,
                            identity_remap.as_deref_mut(),
                        ),
                        body: copied_body,
                        render_unit: copied_render_unit,
                    }
                }

                RetainedNumericIntakeRole::LoopRange {
                    header_sites,
                    body,
                    render_unit,
                } => {
                    copy_state.record_control_flow();
                    let (copied_body, copied_render_unit) = copy_retained_control_flow_body(
                        body,
                        render_unit,
                        active_slot_plan,
                        store,
                        copy_state,
                        identity_remap.as_deref_mut(),
                    )?;

                    // The copy's own range-header row and operand records register in the
                    // replication call below, so the copy completes into its own concrete loop.
                    RetainedNumericIntakeRole::LoopRange {
                        header_sites: remap_loop_header_expression_sites(
                            store,
                            header_sites,
                            identity_remap.as_deref_mut(),
                        ),
                        body: copied_body,
                        render_unit: copied_render_unit,
                    }
                }

                RetainedNumericIntakeRole::LoopCollection {
                    header_sites,
                    body,
                    render_unit,
                } => {
                    copy_state.record_control_flow();
                    let (copied_body, copied_render_unit) = copy_retained_control_flow_body(
                        body,
                        render_unit,
                        active_slot_plan,
                        store,
                        copy_state,
                        identity_remap.as_deref_mut(),
                    )?;

                    // The copy's own binding row registers in the replication call below, so the
                    // copy completes into its own concrete loop.
                    RetainedNumericIntakeRole::LoopCollection {
                        header_sites: remap_loop_header_expression_sites(
                            store,
                            header_sites,
                            identity_remap.as_deref_mut(),
                        ),
                        body: copied_body,
                        render_unit: copied_render_unit,
                    }
                }

                RetainedNumericIntakeRole::ConditionalSelector {
                    selector_site_id,
                    body,
                    kind,
                    render_unit,
                } => {
                    copy_state.record_control_flow();
                    let (copied_body, copied_render_unit) = copy_retained_control_flow_body(
                        body,
                        render_unit,
                        active_slot_plan,
                        store,
                        copy_state,
                        identity_remap.as_deref_mut(),
                    )?;

                    // The copy's own selector row (pattern) and fragment record register in the
                    // replication call below, so the copy completes into its own conditional.
                    let copied_selector_site = remap_expression_site_id(store);
                    if let Some(identity_remap) = identity_remap {
                        identity_remap
                            .record_expression_site(selector_site_id, copied_selector_site);
                    }

                    RetainedNumericIntakeRole::ConditionalSelector {
                        selector_site_id: copied_selector_site,
                        body: copied_body,
                        kind,
                        render_unit: copied_render_unit,
                    }
                }
            };

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RetainedNumericIntake(copied_role),
                span,
            ));
            store
                .replicate_pending_numeric_intakes_for_copied_node(source_node_id, node_id)
                .map_err(TemplateError::from)?;
            Ok(node_id)
        }
    }
}

/// Copies a retained control-flow body and its optional prepared loop wrapper.
fn copy_retained_control_flow_body(
    body: TemplateIrNodeId,
    render_unit: Option<RetainedControlFlowRenderUnit>,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
    mut identity_remap: Option<&mut TirCopyIdentityRemap>,
) -> Result<(TemplateIrNodeId, Option<RetainedControlFlowRenderUnit>), TemplateError> {
    copy_state.enter_depth();
    let copied_body = copy_tir_node_with_active_slot_plan(
        body,
        active_slot_plan,
        store,
        copy_state,
        identity_remap.as_deref_mut(),
    )?;

    let copied_render_unit = match render_unit {
        Some(RetainedControlFlowRenderUnit::Conditional) => {
            Some(RetainedControlFlowRenderUnit::Conditional)
        }
        Some(RetainedControlFlowRenderUnit::Loop { aggregate_wrapper }) => {
            let copied_aggregate_wrapper = copy_tir_node_with_active_slot_plan(
                aggregate_wrapper,
                active_slot_plan,
                store,
                copy_state,
                identity_remap,
            )?;
            Some(RetainedControlFlowRenderUnit::Loop {
                aggregate_wrapper: copied_aggregate_wrapper,
            })
        }
        None => None,
    };
    copy_state.exit_depth();

    Ok((copied_body, copied_render_unit))
}

/// Deep-copies one template entry and its root subtree, applying the active
/// slot-plan context only when the source template is not itself a runtime slot
/// handoff.
///
/// WHAT: returns a fresh `TemplateIrId` whose root is a copy of the source
///       template's root, plus the copy scope minted for that template when the
///       caller is recording identities. If the source carries its own
///       `runtime_slot_plan`, the active plan is suppressed for the copy so
///       nested runtime slot applications remain independent.
/// WHY: `ChildTemplate` and `InsertContribution` nodes inside a copied body may
///      reference helper templates that already own a runtime slot plan. Those
///      must not have the parent's active cursor applied to them. The returned
///      scope selects only the copies made inside this template, so a reference
///      context is rebased against its own subtree without folding or rehashing
///      descendant mappings into the enclosing scope.
fn copy_tir_template_with_active_slot_plan(
    source_template_id: TemplateIrId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
    mut identity_remap: Option<&mut TirCopyIdentityRemap>,
) -> Result<(TemplateIrId, Option<CopyIdentityScope>), TemplateError> {
    let (source_root, has_runtime_slot_plan) = {
        let source_template = store.get_template(source_template_id).ok_or_else(|| {
            TemplateError::from(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "active-context TIR copy: source template ID was not present in the store.",
            ))
        })?;
        (
            source_template.root,
            source_template.runtime_slot_plan.is_some(),
        )
    };

    // Runtime slot handoff templates already resolved their own placeholders.
    // Copy them under a suppressed active plan so the outer cursor does not leak
    // into the nested application.
    let effective_active_slot_plan = if has_runtime_slot_plan {
        None
    } else {
        active_slot_plan
    };

    let mut child_state = TirCopyState::new();
    child_state.runtime_slot_site_cursor = copy_state.runtime_slot_site_cursor.clone();

    let scope_start = identity_remap
        .as_deref_mut()
        .map(TirCopyIdentityRemap::begin_scope);
    let new_root = copy_tir_node_with_active_slot_plan(
        source_root,
        effective_active_slot_plan,
        store,
        &mut child_state,
        identity_remap.as_deref_mut(),
    )?;
    let identity_scope = match (scope_start, identity_remap.as_deref_mut()) {
        (Some(scope_start), Some(identity_remap)) => Some(identity_remap.end_scope(scope_start)),
        _ => None,
    };

    // Propagate the cursor state for the active plan back to the parent copy.
    // The child template's own text/child/slot counts stay in its own summary.
    copy_state.runtime_slot_site_cursor = child_state.runtime_slot_site_cursor;

    let template_id = store.push_structurally_derived_template(
        source_template_id,
        new_root,
        crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(),
    )?;
    if let Some(identity_remap) = identity_remap {
        identity_remap
            .reachability
            .register_copied_template(template_id);
    }
    Ok((template_id, identity_scope))
}

fn remap_expression_site_id(store: &mut TemplateIrStore) -> ExpressionSiteId {
    store.next_expression_site_id()
}

fn remap_loop_header_expression_sites(
    store: &mut TemplateIrStore,
    header_sites: TemplateLoopHeaderExpressionSites,
    mut identity_remap: Option<&mut TirCopyIdentityRemap>,
) -> TemplateLoopHeaderExpressionSites {
    match header_sites {
        TemplateLoopHeaderExpressionSites::Conditional { condition } => {
            let copied_condition = remap_expression_site_id(store);
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_expression_site(condition, copied_condition);
            }
            TemplateLoopHeaderExpressionSites::Conditional {
                condition: copied_condition,
            }
        }
        TemplateLoopHeaderExpressionSites::Range { start, end, step } => {
            let copied_start = remap_expression_site_id(store);
            let copied_end = remap_expression_site_id(store);
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_expression_site(start, copied_start);
                identity_remap.record_expression_site(end, copied_end);
            }
            TemplateLoopHeaderExpressionSites::Range {
                start: copied_start,
                end: copied_end,
                step: step.map(|source_step| {
                    let copied_step = remap_expression_site_id(store);
                    if let Some(identity_remap) = identity_remap.as_deref_mut() {
                        identity_remap.record_expression_site(source_step, copied_step);
                    }
                    copied_step
                }),
            }
        }
        TemplateLoopHeaderExpressionSites::Collection { iterable, step } => {
            let copied_iterable = remap_expression_site_id(store);
            if let Some(identity_remap) = identity_remap.as_deref_mut() {
                identity_remap.record_expression_site(iterable, copied_iterable);
            }
            TemplateLoopHeaderExpressionSites::Collection {
                iterable: copied_iterable,
                step: step.map(|source_step| {
                    let copied_step = remap_expression_site_id(store);
                    if let Some(identity_remap) = identity_remap {
                        identity_remap.record_expression_site(source_step, copied_step);
                    }
                    copied_step
                }),
            }
        }
    }
}

fn remap_view_context(
    store: &mut TemplateIrStore,
    context: TemplateViewContext,
    identity_remap: &TirCopyIdentityRemap,
    identity_scope: CopyIdentityScope,
    reached_root: ReachableExpressionRoot,
) -> Result<TemplateViewContext, InfrastructureFailure> {
    // Overlay dimensions stay present even when their copied subtree has no
    // matching entries. Presence is part of the exact view identity.
    //
    // Wrapper-context and slot-resolution entries are rebased first because the
    // expression overlay keeps overrides that still name the unchanged wrapper
    // templates and resolved sources those dimensions reference.
    let slot_resolution = context
        .slot_resolution
        .map(|overlay_id| {
            remap_slot_resolution_overlay(store, overlay_id, identity_remap, identity_scope)
        })
        .transpose()?;
    let wrapper_context = context
        .wrapper_context
        .map(|overlay_id| {
            remap_wrapper_context_overlay(store, overlay_id, identity_remap, identity_scope)
        })
        .transpose()?;
    let expression_overlay = context
        .expression_overlay
        .map(|overlay_id| {
            remap_expression_overlay(
                store,
                overlay_id,
                identity_remap,
                identity_scope,
                reached_root,
                wrapper_context,
                slot_resolution,
            )
        })
        .transpose()?;

    Ok(TemplateViewContext {
        expression_overlay,
        slot_resolution,
        wrapper_context,
    })
}

fn remap_expression_overlay(
    store: &mut TemplateIrStore,
    overlay_id: TirExpressionOverlayId,
    identity_remap: &TirCopyIdentityRemap,
    identity_scope: CopyIdentityScope,
    reached_root: ReachableExpressionRoot,
    wrapper_context: Option<TirWrapperContextOverlayId>,
    slot_resolution: Option<TirSlotResolutionOverlayId>,
) -> Result<TirExpressionOverlayId, InfrastructureFailure> {
    // Sites still reachable through unchanged store-owned structure keep their
    // original identity. The walk starts at the copied root, so descendant
    // reference contexts and retained plan roots belong to the same domain the
    // production readers see. Enumerating only the value-carried overlay
    // dimensions misses both.
    let reachable_sites = collect_reachable_expression_sites(
        store,
        reached_root,
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution,
            wrapper_context,
        },
        &identity_remap.reachability,
    )?;
    let source_overrides = &store
        .expression_overlay(overlay_id)
        .ok_or_else(|| {
            InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                "active-context TIR copy: expression overlay {overlay_id} was not present in the store."
            ))
        })?
        .overrides;

    // Only the source overlay names the sites this rebase can retain. Querying
    // those candidates keeps the work proportional to the overlay instead of
    // materialising every site reachable from the copied root, whose shared
    // summaries stay unflattened.
    let retained_sites =
        reachable_sites.retained_candidates(source_overrides.iter().map(|(site_id, _)| *site_id));

    // One source site can need several entries: every fresh copy of the source
    // tree uses its own site identity, while an unchanged wrapper set that
    // shares the same template keeps the original site. A site with neither use
    // has no meaning in the rebased view and must not leak.
    let mut overrides = Vec::with_capacity(source_overrides.len());
    for (site_id, expression) in source_overrides {
        if let Some(copied_site_ids) = identity_remap.expression_sites.get(site_id) {
            for copied_site_id in copied_site_ids.iter_in_scope(identity_scope) {
                overrides.push((copied_site_id, *expression));
            }
        }

        if retained_sites.contains(site_id) {
            overrides.push((*site_id, *expression));
        }
    }

    store.allocate_expression_overlay(TirExpressionOverlay { overrides })
}

fn remap_slot_resolution_overlay(
    store: &mut TemplateIrStore,
    overlay_id: TirSlotResolutionOverlayId,
    identity_remap: &TirCopyIdentityRemap,
    identity_scope: CopyIdentityScope,
) -> Result<TirSlotResolutionOverlayId, InfrastructureFailure> {
    let source_overlay = store.slot_resolution_overlay(overlay_id).ok_or_else(|| {
        InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
            "active-context TIR copy: slot-resolution overlay {overlay_id} was not present in the store."
        ))
    })?;
    let mut resolutions = Vec::with_capacity(source_overlay.resolutions.len());
    for (occurrence_id, resolution) in &source_overlay.resolutions {
        if let Some(copied_occurrence_ids) = identity_remap.slot_occurrences.get(occurrence_id) {
            for copied_occurrence_id in copied_occurrence_ids.iter_in_scope(identity_scope) {
                resolutions.push((copied_occurrence_id, resolution.clone()));
            }
        }
    }

    store.allocate_slot_resolution_overlay(TirSlotResolutionOverlay { resolutions })
}

fn remap_wrapper_context_overlay(
    store: &mut TemplateIrStore,
    overlay_id: TirWrapperContextOverlayId,
    identity_remap: &TirCopyIdentityRemap,
    identity_scope: CopyIdentityScope,
) -> Result<TirWrapperContextOverlayId, InfrastructureFailure> {
    let source_overlay = store.wrapper_context_overlay(overlay_id).ok_or_else(|| {
        InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
            "active-context TIR copy: wrapper-context overlay {overlay_id} was not present in the store."
        ))
    })?;
    let mut contexts = Vec::with_capacity(source_overlay.contexts.len());
    for (occurrence_id, context) in &source_overlay.contexts {
        if let Some(copied_occurrence_ids) = identity_remap.child_occurrences.get(occurrence_id) {
            for copied_occurrence_id in copied_occurrence_ids.iter_in_scope(identity_scope) {
                contexts.push((copied_occurrence_id, context.clone()));
            }
        }
    }

    store.allocate_wrapper_context_overlay(TirWrapperContextOverlay { contexts })
}
