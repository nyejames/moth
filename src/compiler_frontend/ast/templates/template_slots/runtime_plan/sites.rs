//! Runtime wrapper slot-site planning.
//!
//! WHAT: assigns stable IDs to concrete wrapper `$slot` occurrences and builds
//! one TIR render root per site from plan-qualified contribution markers.
//!
//! WHY: runtime applications must evaluate each source once while repeated slot
//! placeholders can still carry different child-wrapper and `$fresh`
//! metadata. Ordinary contributions compose wrappers around the source marker;
//! control-flow contributions attach them conditionally to the marker's fresh
//! template root so a skipped source emits no wrapper. Named markers carry their
//! source's shape and control-flow class so wrapper-local named slots make the
//! same wrapper decisions as structural expansion. Each exact wrapper reference
//! is prepared once per receiving application: its shared prefix/suffix edges and
//! named content stay resolved, and every child rebuilds only the implicit
//! loose-fill path instead of descending the whole decorator composite again.

use super::push_named_source_markers;
use super::types::{RuntimeSlotContributionSourceDraft, RuntimeSlotSiteId};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::{
    DerivedTemplateMetadata, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateIrStore, TemplateIrSummary, TemplateSlotPlanId, TemplateSlotSitePlan,
    TemplateWrapperSetId, TirCopyState, TirSlotPlaceholder, TirSlotPlaceholderRef,
    apply_slot_wrapper_sets_to_deferred_fill, attach_conditional_wrapper_set,
    collect_tir_slot_layout_from_root, collect_tir_slot_layout_with_context,
    conditional_wrapper_set_for_control_flow, copy_tir_subtree_with_active_slot_plan,
    fill_direct_child_wrapper_names_into, fill_named_slots_in_wrapper_root,
    fill_named_slots_in_wrapper_template, push_runtime_slot_contribution_source,
    summarize_existing_root,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use std::rc::Rc;

use rustc_hash::{FxHashMap, FxHashSet};

struct RuntimeSlotSiteDraft {
    site: RuntimeSlotSiteId,
    placeholder: TirSlotPlaceholderRef,
}

/// Plan-qualified contribution markers keyed by receiving slot name.
type NamedSlotMarkers = FxHashMap<StringId, Vec<TemplateIrNodeId>>;

/// One wrapper reference prepared for this receiving application.
///
/// WHAT: keeps the wrapper root after structural named occurrences and inherited
///       overlay contexts are resolved, the node paths that still await this
///       application's loose fill, and the exact reference transition the derived
///       application must carry.
/// WHY: every source and slot occurrence under one application replays the same
///      exact wrapper reference and the same named content. Resolving that
///      content once leaves only the implicit child insertion point for each
///      fill, so prefix/suffix edges are shared rather than walked and
///      re-derived per child.
#[derive(Clone)]
pub(super) struct PreparedRuntimeWrapper {
    wrapper_root: TemplateIrNodeId,
    wrapper_span: Option<crate::compiler_frontend::source::SourceSpan>,
    /// Validated summary of `wrapper_ref.root`, the template the per-child version
    /// is derived from.
    ///
    /// WHAT: keeps the exact facts publication would otherwise recompute from the
    ///       prepared root.
    /// WHY: each fill changes only the loose-fill occurrence, so a child version's
    ///      summary is this source summary adjusted by that fill's replacement.
    ///      Re-walking the unchanged prefix and suffix graph would make every child
    ///      pay for the whole decorator composite again.
    source_summary: TemplateIrSummary,
    /// The wrapper's loose-fill target, or `None` when the wrapper is structurally
    /// slotless and must be prepended unchanged before the fill. A wrapper with any
    /// slot target but no loose-fill key is rejected while preparing, so `None`
    /// never hides a named-only wrapper here.
    target_key: Option<SlotKey>,
    /// Nodes whose subtree contains an occurrence of `target_key`. Per-fill
    /// injection visits only these nodes, so a decorator composite reuses its
    /// prepared prefix and suffix edges instead of descending them per child.
    fill_paths: Rc<FxHashSet<TemplateIrNodeId>>,
    /// The plan-qualified named markers this application replays. Shared behind
    /// one pointer because each fill applies them only to slot-owned child
    /// wrapper sets; the wrapper edges themselves were resolved once.
    named_fill: Option<Rc<NamedSlotMarkers>>,
    wrapper_ref: TemplateWrapperReference,
}

pub(super) struct RuntimeWrapperSitePlanBuilder<'a> {
    pub(super) sources: &'a [RuntimeSlotContributionSourceDraft],
    pub(super) slot_plan_id: TemplateSlotPlanId,
    pub(super) store: &'a mut TemplateIrStore,
    pub(super) copy_state: &'a mut TirCopyState,
    pub(super) expressions: &'a AstExpressionBuilder,
    pub(super) const_values: &'a ConstValueStore,
    pub(super) string_table: &'a StringTable,
    /// Prepared wrappers keyed by the exact reference this application receives.
    /// The builder is scoped to one receiving application, so the plan identity
    /// and named markers behind every entry are identical.
    pub(super) resolved_wrappers: FxHashMap<TemplateWrapperReference, PreparedRuntimeWrapper>,
}

impl RuntimeWrapperSitePlanBuilder<'_> {
    pub(super) fn build_slot_sites(
        mut self,
        wrapper_tir_root: TemplateIrNodeId,
    ) -> Result<Vec<TemplateSlotSitePlan>, TemplateError> {
        let mut drafts = Vec::new();
        self.collect_site_drafts(wrapper_tir_root, &mut drafts)?;

        let mut slot_sites = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let render_root =
                self.build_site_render_root(&draft.placeholder, draft.placeholder.span)?;
            slot_sites.push(TemplateSlotSitePlan {
                site: draft.site,
                key: draft.placeholder.key.clone(),
                render_root,
                span: draft.placeholder.span,
            });
        }

        Ok(slot_sites)
    }

    fn collect_site_drafts(
        &mut self,
        wrapper_tir_root: TemplateIrNodeId,
        drafts: &mut Vec<RuntimeSlotSiteDraft>,
    ) -> Result<(), TemplateError> {
        // Walk the TIR tree in document order to discover every unresolved slot
        // placeholder, including those nested inside child templates,
        // conditionals and loops. TIR is the sole authority for slot-placeholder
        // discovery.
        // Site IDs are assigned in traversal order, matching the cursor-based
        // assignment the final materialization pass will use.
        let layout = collect_tir_slot_layout_from_root(self.store, wrapper_tir_root)?;

        for placeholder in layout.placeholders {
            let site = RuntimeSlotSiteId(drafts.len());
            drafts.push(RuntimeSlotSiteDraft { site, placeholder });
        }

        Ok(())
    }

    fn build_site_render_root(
        &mut self,
        placeholder: &TirSlotPlaceholderRef,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let mut fill_roots = Vec::new();

        // Every source of one site replays the same conditional wrapper sets against the
        // same plan sources, so one resolution per distinct set is enough for this site.
        let mut resolved_wrapper_sets: FxHashMap<TemplateWrapperSetId, TemplateWrapperSetId> =
            FxHashMap::default();

        for source_index in 0..self.sources.len() {
            if self.sources[source_index].source.target != placeholder.key {
                continue;
            }

            let source = self.sources[source_index].clone();
            let source_has_control_flow = source.control_flow;
            let source_root = push_runtime_slot_contribution_source(
                self.store,
                self.slot_plan_id,
                source.source.source,
                source.shape,
                source.control_flow,
                source.source.span,
            );
            let wrapped_root = if source_has_control_flow {
                if let Some(wrapper_set_id) = conditional_wrapper_set_for_control_flow(
                    self.store,
                    placeholder.child_wrapper_set,
                    placeholder.applied_child_wrapper_set,
                    placeholder.skip_parent_child_wrappers,
                    source.shape,
                )? {
                    let wrapper_set_id = self.fill_named_markers_in_wrapper_set(
                        &mut resolved_wrapper_sets,
                        wrapper_set_id,
                    )?;
                    attach_conditional_wrapper_set(self.store, source_root, wrapper_set_id)?
                } else {
                    source_root
                }
            } else {
                self.apply_site_wrappers(placeholder, &source, source_root)?
            };
            fill_roots.push(wrapped_root);
        }

        // Routing owns presence: a source of this key suppresses the fallback even when
        // the source renders empty. Only an occurrence with no routed source carries
        // its own default into the site tree.
        if fill_roots.is_empty()
            && let Some(default) = placeholder.default
        {
            let mut default_placeholder = TirSlotPlaceholder::with_wrapper_sets(
                placeholder.key.clone(),
                self.store.next_slot_occurrence_id(),
                Some(default),
                None,
                None,
                false,
            );
            // The plan selected this fallback, so no later injection may fill the
            // occurrence the site tree materializes.
            default_placeholder.closed = true;
            fill_roots.push(self.store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Slot {
                    placeholder: default_placeholder,
                },
                span,
            )));
        }

        Ok(collapse_render_roots(fill_roots, span, self.store))
    }

    fn apply_site_wrappers(
        &mut self,
        placeholder: &TirSlotPlaceholderRef,
        source: &RuntimeSlotContributionSourceDraft,
        source_root: TemplateIrNodeId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let shape = source.shape;

        // Source plans are distinct from site plans so repeated placeholders can
        // apply their own child-wrapper metadata without re-evaluating the source.
        let (mut fill_root, wrapped_as_child) = match placeholder.child_wrapper_set {
            Some(child_wrapper_set)
                if !shape.skips_parent_child_wrappers()
                    && shape.is_child_template_contribution() =>
            {
                (
                    self.wrap_site_fill_with_tir_child_wrappers(source_root, child_wrapper_set)?,
                    true,
                )
            }

            _ => (source_root, shape.is_child_template_contribution()),
        };

        if !placeholder.skip_parent_child_wrappers
            && wrapped_as_child
            && let Some(applied_child_wrapper_set) = placeholder.applied_child_wrapper_set
        {
            fill_root =
                self.wrap_site_fill_with_tir_child_wrappers(fill_root, applied_child_wrapper_set)?;
        }

        Ok(fill_root)
    }

    /// Applies an ordered TIR wrapper set around a fill root.
    ///
    /// WHAT: resolves complete wrapper references from the wrapper-set side
    ///       table and iterates them forward (innermost-first).
    /// WHY: `TemplateWrapperSet::wrappers` is stored innermost-to-outermost, so
    ///      forward consumption yields the `outermost(innermost(fill))` nesting
    ///      the store contract requires.
    fn wrap_site_fill_with_tir_child_wrappers(
        &mut self,
        mut fill_root: TemplateIrNodeId,
        wrapper_set_id: TemplateWrapperSetId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let Some(wrapper_set) = self.store.get_wrapper_set(wrapper_set_id) else {
            return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
                "Runtime slot site planning found a slot wrapper-set ID that was not present in the TIR store.",
            )
            .into());
        };

        let wrapper_refs = wrapper_set.wrappers.clone();
        for wrapper_ref in wrapper_refs {
            add_ast_counter(AstCounter::TemplateWrapperApplications, 1);
            fill_root = self.apply_wrapper_reference(wrapper_ref, fill_root)?;
        }

        Ok(fill_root)
    }

    /// Applies one exact wrapper reference around a fill root.
    ///
    /// WHAT: reuses the receiving application's prepared form of the exact reference,
    ///       then injects this fill root at the prepared loose-fill paths.
    /// WHY: every source and repeated slot occurrence under this application receives
    ///      the same wrapper reference and the same named content. Preparing both once
    ///      avoids walking and deriving the prefix/suffix trees per child; the per-fill
    ///      injection now rebuilds only the target's ancestor paths so each occurrence
    ///      still gets its own fill and occurrence identities.
    fn apply_wrapper_reference(
        &mut self,
        wrapper_ref: TemplateWrapperReference,
        fill_root: TemplateIrNodeId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        // The prepared snapshot is cheap: the reference and target are copyable and
        // the fill paths and named markers are shared behind pointers, so no
        // per-fill map clone remains.
        let prepared = match self.resolved_wrappers.get(&wrapper_ref) {
            Some(prepared) => prepared.clone(),
            None => {
                let prepared = self.prepare_wrapper_reference(wrapper_ref)?;
                self.resolved_wrappers.insert(wrapper_ref, prepared.clone());
                prepared
            }
        };

        self.apply_prepared_wrapper(prepared, fill_root)
    }

    /// Resolves one wrapper reference for this application exactly once.
    ///
    /// WHAT: collects the reference's exact layout, builds one plan marker per named
    ///       source the wrapper declares, resolves structural named occurrences and
    ///       inherited overlay contexts into a named-filled template, settles every
    ///       non-target slot, records the target's fill paths, and records the exact
    ///       reference transition the application must carry.
    /// WHY: inherited overlay wrappers keep named occurrences outside the wrapper's
    ///      structural root, and a wrapper value can contain nested decorated helpers.
    ///      The named fill must see the same exact reference context as the layout used
    ///      for routing; doing it once per application leaves only the implicit loose
    ///      fill and slot-owned child wrappers for each child.
    fn prepare_wrapper_reference(
        &mut self,
        wrapper_ref: TemplateWrapperReference,
    ) -> Result<PreparedRuntimeWrapper, TemplateError> {
        let (source_root, wrapper_span, wrapper_source_summary) = {
            let wrapper_template = self.store.get_template(wrapper_ref.root).ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                    "Runtime slot site planning found a slot wrapper template ref {} that was not present in the TIR store.",
                    wrapper_ref
                ))
            })?;
            (
                wrapper_template.root,
                wrapper_template.span,
                wrapper_template.summary.clone(),
            )
        };
        let layout = collect_tir_slot_layout_with_context(
            self.store,
            wrapper_ref.root,
            wrapper_ref.context,
        )?;

        if !layout.schema.has_any_slots() {
            return Ok(PreparedRuntimeWrapper {
                wrapper_root: source_root,
                wrapper_span,
                source_summary: wrapper_source_summary,
                target_key: None,
                fill_paths: Rc::new(FxHashSet::default()),
                named_fill: None,
                wrapper_ref,
            });
        }

        let Some(target_key) = layout.schema.loose_fill_target_key() else {
            return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
                "Runtime slot site planning could not apply a TIR slot wrapper to a loose contribution.",
            )
            .into());
        };

        let named_fill =
            push_named_source_markers(self.store, self.slot_plan_id, self.sources, &layout.schema);

        // Resolve every named occurrence and composition-owned child-edge overlay
        // once. Slot-owned wrapper sets stay with the per-fill pass: they wrap this
        // application's child, not the shared prefix/suffix edges.
        let named_filled_template = fill_named_slots_in_wrapper_root(
            self.store,
            self.expressions,
            self.const_values,
            wrapper_ref.root,
            &layout.schema,
            &named_fill,
            self.string_table,
        )?;
        let mut prepared_template_id = named_filled_template.unwrap_or(wrapper_ref.root);
        let (mut prepared_root, mut source_summary) = {
            let prepared_template = self.store.get_template(prepared_template_id).ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                    "Runtime slot site planning found prepared wrapper template {prepared_template_id} missing from the TIR store."
                ))
            })?;
            (prepared_template.root, prepared_template.summary.clone())
        };

        // The loose fill belongs to each child, so its occurrences stay unresolved.
        // Every other slot is occurrence-local fallback or empty output: an unfilled
        // named occurrence, an unreachable positional occurrence or a defaulted
        // non-target occurrence is settled here, once, instead of on every child path.
        let resolved = inject_runtime_slot_fill(
            prepared_root,
            &[],
            None,
            Some(&target_key),
            None,
            self.store,
            self.copy_state,
        )?;
        if resolved.changed {
            let summary = apply_fill_summary(source_summary, &resolved.summary_delta)?;
            prepared_template_id = self.derive_wrapper_template_with_summary(
                prepared_template_id,
                resolved.root,
                summary.clone(),
            )?;
            prepared_root = resolved.root;
            source_summary = summary;
        }

        let mut fill_paths = FxHashSet::default();
        collect_fill_paths(self.store, prepared_root, &target_key, &mut fill_paths)?;
        if fill_paths.is_empty() {
            return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
                "Runtime slot site planning prepared a slot wrapper whose loose-fill occurrence disappeared.",
            )
            .into());
        }

        // The wrapper's fixed direct children keep their occurrence-keyed overlay context,
        // so the same named markers must also reach the child-wrapper sets stored there.
        // Reusing the markers keeps one plan source per named contribution while still
        // replaying it at every wrapper occurrence this application owns. A wrapper with no
        // named targets has nothing to fill, so the overlay walk is skipped.
        let prepared_context = if layout.schema.named_slots.is_empty() {
            wrapper_ref.context
        } else {
            fill_direct_child_wrapper_names_into(
                self.store,
                self.expressions,
                self.const_values,
                wrapper_ref.context,
                &named_fill,
                self.string_table,
            )?
        };
        let wrapper_ref = TemplateWrapperReference::new(
            prepared_template_id,
            wrapper_ref.phase,
            prepared_context,
        );

        Ok(PreparedRuntimeWrapper {
            wrapper_root: prepared_root,
            wrapper_span,
            source_summary,
            target_key: Some(target_key),
            fill_paths: Rc::new(fill_paths),
            named_fill: (!named_fill.is_empty()).then(|| Rc::new(named_fill)),
            wrapper_ref,
        })
    }

    /// Injects one fill root into an already prepared wrapper reference.
    fn apply_prepared_wrapper(
        &mut self,
        prepared: PreparedRuntimeWrapper,
        fill_root: TemplateIrNodeId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let PreparedRuntimeWrapper {
            wrapper_root,
            wrapper_span,
            source_summary,
            target_key,
            fill_paths,
            named_fill,
            wrapper_ref,
        } = prepared;

        // A slotless wrapper has no structural insertion point. It stays an exact
        // reference and renders before the fill, preserving its phase and context.
        let Some(target_key) = target_key else {
            let wrapper_reference = wrapper_ref.into_structural_child_reference();
            let unchanged_wrapper = self.push_wrapper_child(wrapper_reference, wrapper_span);
            return Ok(collapse_render_roots(
                vec![unchanged_wrapper, fill_root],
                wrapper_span,
                self.store,
            ));
        };

        // Every named occurrence and every unreachable slot is already resolved in
        // the prepared root, so only the loose-fill key remains to inject. The fill
        // paths keep this pass on the target's ancestor chain instead of re-walking
        // the prefix and suffix edges.
        let injections = [RuntimeSlotInjection {
            key: target_key,
            fill_root,
        }];

        // A target slot's own wrapper set wraps this child, so it still
        // receives the application's named markers while the fill is applied. The
        // map is shared with the prepared snapshot rather than cloned per fill.
        let empty_named_fill: NamedSlotMarkers = FxHashMap::default();
        let named_fill = named_fill.as_deref().unwrap_or(&empty_named_fill);
        let slot_fill_wrappers = RuntimeSlotFillWrappers {
            named_fill,
            expressions: self.expressions,
            const_values: self.const_values,
            string_table: self.string_table,
        };
        let injected = inject_runtime_slot_fill(
            wrapper_root,
            &injections,
            Some(&slot_fill_wrappers),
            None,
            Some(&*fill_paths),
            self.store,
            self.copy_state,
        )?;

        // The prepared source summary is already validated. This child changed only
        // the loose-fill occurrence, so its version reuses those facts plus the
        // fill delta instead of re-walking the unchanged decorator edges.
        let summary = apply_fill_summary(source_summary, &injected.summary_delta)?;
        self.push_composed_wrapper_child(wrapper_ref, injected.root, summary, wrapper_span)
    }

    /// Derives each wrapper in a set with its own named slot markers filled.
    ///
    /// WHAT: builds markers per wrapper from that wrapper's declared names, fills that
    ///       wrapper's structural occurrences, slot-owned wrapper sets and wrapper-local
    ///       direct-child overlay, and returns the replacement set. Resolutions are
    ///       memoised in the caller's per-site wrapper-set map.
    /// WHY: conditional wrapper application stores only wrapper references, and its
    ///       injection happens at runtime handoff after the store becomes read-only.
    ///       Keeping the fill per wrapper prevents two wrappers that declare the same name
    ///       from replaying each other's markers inside one occurrence. A wrapper's fixed
    ///       direct children carry nested wrapper sets in its reference context,
    ///       and wrappers stored on its slots are applied only once a runtime child is
    ///       injected, so both paths need the markers before handoff begins.
    fn fill_named_markers_in_wrapper_set(
        &mut self,
        resolved_wrapper_sets: &mut FxHashMap<TemplateWrapperSetId, TemplateWrapperSetId>,
        wrapper_set_id: TemplateWrapperSetId,
    ) -> Result<TemplateWrapperSetId, TemplateError> {
        fill_named_markers_in_wrapper_set(
            self.store,
            self.slot_plan_id,
            self.sources,
            self.expressions,
            self.const_values,
            self.string_table,
            resolved_wrapper_sets,
            wrapper_set_id,
        )
    }

    fn push_composed_wrapper_child(
        &mut self,
        wrapper_ref: TemplateWrapperReference,
        new_root: TemplateIrNodeId,
        summary: TemplateIrSummary,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let derived_id =
            self.derive_wrapper_template_with_summary(wrapper_ref.root, new_root, summary)?;
        let reference = wrapper_ref.into_composed_child_reference(derived_id);
        Ok(self.push_wrapper_child(reference, span))
    }

    fn derive_wrapper_template_with_summary(
        &mut self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
        summary: TemplateIrSummary,
    ) -> Result<TemplateIrId, TemplateError> {
        Ok(self.store.push_derived_template_with_summary(
            source,
            new_root,
            summary,
            DerivedTemplateMetadata::preserve_source(),
        )?)
    }

    fn push_wrapper_child(
        &mut self,
        reference: TemplateTirChildReference,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> TemplateIrNodeId {
        let occurrence_id = self.store.next_child_template_occurrence_id();
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference,
                occurrence_id,
            },
            span,
        ))
    }
}

/// One slot key and its fill root for a wrapper application.
struct RuntimeSlotInjection {
    key: SlotKey,
    fill_root: TemplateIrNodeId,
}

struct RuntimeSlotInjectionResult {
    root: TemplateIrNodeId,
    changed: bool,
    /// Facts this pass changed relative to the traversed template's source summary.
    ///
    /// WHAT: records the slots the pass replaced or resolved to empty, the combined
    ///       additive facts of every inserted fill, and the deepest fill reach.
    /// WHY: callers that version a template around the rebuilt root can reuse the
    ///      source's already validated summary instead of re-walking the unchanged
    ///      prefix and suffix edges. Counts from nested child-template scopes stay
    ///      out because a parent summary records only the child reference itself.
    summary_delta: RuntimeFillSummaryDelta,
}

/// One fill pass's summary changes relative to its source template.
///
/// WHAT: accumulates removed slots and the additive shape facts of inserted fills,
///       with the deepest relative depth a fill introduces.
/// WHY: wrapper preparation and per-child application both derive template versions
///      from a validated source plus local fill changes. This delta keeps summary
///      publication proportional to the changed path instead of the whole decorator.
#[derive(Default)]
struct RuntimeFillSummaryDelta {
    removed_slots: u32,
    added: TemplateIrSummary,
    max_depth: usize,
}

impl RuntimeFillSummaryDelta {
    /// Records one `Slot` replaced by a fill root at this pass's relative depth.
    fn record_fill(&mut self, fill: &TemplateIrSummary) {
        self.removed_slots += 1;
        self.added.add_fill_facts(fill);
        self.max_depth = self.max_depth.max(fill.max_depth);
    }

    /// Records one unmatched `Slot` resolved to empty output.
    fn record_removed_slot(&mut self) {
        self.removed_slots += 1;
    }

    /// Folds a rebuilt structural child into this template scope.
    ///
    /// WHY: sequence children, conditional bodies and loop bodies sit one level
    ///      below this node in the summary's depth model, while their replacement
    ///      counts belong to the same template summary.
    fn merge_structural_child(
        &mut self,
        child: RuntimeFillSummaryDelta,
    ) -> Result<(), InfrastructureFailure> {
        self.removed_slots += child.removed_slots;
        self.added.add_fill_facts(&child.added);
        let child_depth = child.max_depth.checked_add(1).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR summary depth overflowed usize; this is a compiler bug.",
            )
        })?;
        self.max_depth = self.max_depth.max(child_depth);
        Ok(())
    }
}

/// Applies a pass's fill delta to a source template's published summary.
fn apply_fill_summary(
    mut source: TemplateIrSummary,
    delta: &RuntimeFillSummaryDelta,
) -> Result<TemplateIrSummary, TemplateError> {
    source.apply_fill_replacements(delta.removed_slots, &delta.added, delta.max_depth)?;
    Ok(source)
}

/// Services and named markers needed to apply wrappers stored on a wrapper's own slots.
///
/// WHAT: carries the plan's flattened named markers and the expression, constant and
///       string owners that wrapper composition needs.
/// WHY: the injected fill is a plan-qualified marker, so the slot's own
///      child wrappers cannot be inferred from the injected site root.
///      Site-gated fills keep the existing unconditional application contract
///      because their site planning already gated them; named markers stay
///      available to the nested wrappers a receiving slot applies.
struct RuntimeSlotFillWrappers<'a> {
    named_fill: &'a NamedSlotMarkers,
    expressions: &'a AstExpressionBuilder,
    const_values: &'a ConstValueStore,
    string_table: &'a StringTable,
}

/// Derives each wrapper in a set with its own named slot markers filled.
///
/// WHAT: builds markers per wrapper from that wrapper's declared names, fills that
///       wrapper's structural occurrences, slot-owned wrapper sets and wrapper-local
///       direct-child overlay, and returns the replacement set. One resolution per
///       distinct set is memoised in the caller's per-site wrapper-set map.
/// WHY: conditional wrapper application stores only wrapper references, and its
///       injection happens at runtime handoff after the store becomes read-only.
///       Keeping the fill per wrapper prevents two wrappers that declare the same name
///       from replaying each other's markers inside one occurrence. A wrapper's fixed
///       direct children carry nested wrapper sets in its reference context,
///       and wrappers stored on its slots are applied only once a runtime child is
///       injected, so both paths need the markers before handoff begins. Sibling
///       sources of one site replay the same set against the same sources, and every
///       repeat fill within that site would otherwise publish equivalent derived
///       templates that the store cannot deduplicate by identity.
#[allow(
    clippy::too_many_arguments,
    reason = "resolving a set needs the store, plan sources, expression/constant/string owners, the per-site resolution map and the set ID"
)]
fn fill_named_markers_in_wrapper_set(
    store: &mut TemplateIrStore,
    slot_plan_id: TemplateSlotPlanId,
    sources: &[RuntimeSlotContributionSourceDraft],
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    string_table: &StringTable,
    resolved_wrapper_sets: &mut FxHashMap<TemplateWrapperSetId, TemplateWrapperSetId>,
    wrapper_set_id: TemplateWrapperSetId,
) -> Result<TemplateWrapperSetId, TemplateError> {
    // The plan may have no named source at all, but a wrapper can still declare
    // named occurrences that need missing-name closure before handoff. The
    // per-wrapper layout below is the fast path: it skips preparation only when
    // that wrapper declares no named target.
    if let Some(resolved_set_id) = resolved_wrapper_sets.get(&wrapper_set_id) {
        return Ok(*resolved_set_id);
    }

    let wrappers = store
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            InfrastructureFailure::internal(CompilerStage::AstConstruction,
                "Runtime slot site planning found a wrapper set that was not present in the TIR store.",
            )
        })?
        .wrappers
        .to_vec();

    let mut filled = Vec::with_capacity(wrappers.len());
    let mut changed = false;
    for wrapper in wrappers {
        let layout = collect_tir_slot_layout_with_context(store, wrapper.root, wrapper.context)?;

        // The collected layout already proves there is no named target to resolve.
        // Skip both preparation walks; an empty marker set only has closure work when
        // the wrapper actually declares named occurrences.
        if layout.schema.named_slots.is_empty() {
            filled.push(wrapper);
            continue;
        }

        let named_fill = push_named_source_markers(store, slot_plan_id, sources, &layout.schema);

        // An empty marker set still fixes the wrapper's unfilled named occurrences:
        // handoff cannot derive templates, so their fallbacks must be settled now.
        let filled_root = fill_named_slots_in_wrapper_template(
            store,
            expressions,
            const_values,
            wrapper.root,
            &named_fill,
            string_table,
        )?;
        let filled_context = fill_direct_child_wrapper_names_into(
            store,
            expressions,
            const_values,
            wrapper.context,
            &named_fill,
            string_table,
        )?;

        if filled_root.is_none() && filled_context == wrapper.context {
            filled.push(wrapper);
            continue;
        }

        changed = true;
        filled.push(TemplateWrapperReference::new(
            filled_root.unwrap_or(wrapper.root),
            wrapper.phase,
            filled_context,
        ));
    }

    let resolved_set_id = if changed {
        store.push_or_reuse_wrapper_set(&filled)
    } else {
        wrapper_set_id
    };
    resolved_wrapper_sets.insert(wrapper_set_id, resolved_set_id);
    Ok(resolved_set_id)
}

/// Records every node on a path from `root` to an occurrence of `target_key`.
///
/// WHAT: mirrors the injection walk's structural branches and inserts a node when
///       its subtree contains the loose-fill target.
/// WHY: a prepared wrapper settles every other slot once and keeps only the
///       implicit child insertion point unresolved. Rebuilding only these paths
///       per child reuses the prepared prefix and suffix edges instead of
///       descending them again for every contribution.
fn collect_fill_paths(
    store: &TemplateIrStore,
    root: TemplateIrNodeId,
    target_key: &SlotKey,
    paths: &mut FxHashSet<TemplateIrNodeId>,
) -> Result<bool, TemplateError> {
    let node = store.get_node(root).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "Runtime slot site planning could not read a wrapper node while collecting fill paths.",
        )
    })?;

    let contains_target = match &node.kind {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.key == *target_key,

        TemplateIrNodeKind::Sequence { children } => {
            let mut found = false;
            for child_id in children {
                found |= collect_fill_paths(store, *child_id, target_key, paths)?;
            }
            found
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_template = store.get_template(reference.root).ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                    "Runtime slot site planning found child template {} missing while collecting wrapper fill paths.",
                    reference.root
                ))
            })?;
            collect_fill_paths(store, child_template.root, target_key, paths)?
        }

        TemplateIrNodeKind::Conditional { body, .. } => {
            collect_fill_paths(store, *body, target_key, paths)?
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let body_contains = collect_fill_paths(store, *body, target_key, paths)?;
            let aggregate_contains = match aggregate_wrapper {
                Some(wrapper_id) => collect_fill_paths(store, *wrapper_id, target_key, paths)?,
                None => false,
            };
            body_contains || aggregate_contains
        }

        _ => false,
    };

    if contains_target {
        paths.insert(root);
    }

    Ok(contains_target)
}

/// Injects slot fills and applies each receiving slot's own child wrappers.
///
/// WHAT: fills every matching occurrence, then applies the wrappers stored on the
///       matched placeholder before the surrounding wrapper template renders the
///       fill. Site-gated fills keep their unconditional wrapper application because
///       their site planning already gated them. An optional fill-path set prunes
///       subtrees that cannot contain the target, and an optional preserved key
///       keeps an occurrence out of this pass.
/// WHY: a wrapper applied around a runtime contribution still owns its own
///      child-wrapper metadata. Deferring that metadata to handoff would require
///      deriving templates from an immutable store, so the site tree must carry the
///      wrappers structurally, exactly as authored slot expansion does, including
///      `$fresh` and conditional-emission behavior.
fn inject_runtime_slot_fill(
    wrapper_root: TemplateIrNodeId,
    injections: &[RuntimeSlotInjection],
    slot_wrappers: Option<&RuntimeSlotFillWrappers<'_>>,
    preserve_key: Option<&SlotKey>,
    fill_paths: Option<&FxHashSet<TemplateIrNodeId>>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<RuntimeSlotInjectionResult, TemplateError> {
    if fill_paths.is_some_and(|paths| !paths.contains(&wrapper_root)) {
        return Ok(RuntimeSlotInjectionResult {
            root: wrapper_root,
            changed: false,
            summary_delta: RuntimeFillSummaryDelta::default(),
        });
    }

    let node = store.get_node(wrapper_root).cloned().ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "Child-wrapper TIR path could not read the wrapper root node.",
        )
    })?;
    let span = node.span;

    match node.kind {
        TemplateIrNodeKind::Slot { placeholder } => {
            let Some(injection) = injections
                .iter()
                .find(|injection| injection.key == placeholder.key)
            else {
                // A preserved occurrence belongs to a later per-fill pass.
                if preserve_key.is_some_and(|key| placeholder.key == *key) {
                    return Ok(RuntimeSlotInjectionResult {
                        root: wrapper_root,
                        changed: false,
                        summary_delta: RuntimeFillSummaryDelta::default(),
                    });
                }

                // Routing owns presence. An occurrence with no matching injection
                // keeps its own fallback for handoff materialization. A matching
                // injection, including an explicitly empty one, suppresses that
                // fallback by replacing the occurrence with its fill root.
                if placeholder.default.is_some() {
                    return Ok(RuntimeSlotInjectionResult {
                        root: wrapper_root,
                        changed: false,
                        summary_delta: RuntimeFillSummaryDelta::default(),
                    });
                }

                let mut summary_delta = RuntimeFillSummaryDelta::default();
                summary_delta.record_removed_slot();
                return Ok(RuntimeSlotInjectionResult {
                    root: empty_render_root(store, span),
                    changed: true,
                    summary_delta,
                });
            };

            let copied_fill = copy_tir_subtree_with_active_slot_plan(
                injection.fill_root,
                None,
                store,
                copy_state,
            )?;
            let root = match slot_wrappers {
                Some(slot_wrappers) => apply_slot_wrapper_sets_to_deferred_fill(
                    store,
                    slot_wrappers.expressions,
                    slot_wrappers.const_values,
                    &placeholder,
                    copied_fill,
                    slot_wrappers.named_fill,
                    slot_wrappers.string_table,
                )?,
                None => copied_fill,
            };

            // The fill is the only part of the source template this occurrence
            // changes, so its established facts are all a version needs to know.
            let fill_summary = summarize_existing_root(store, root)?;
            let mut summary_delta = RuntimeFillSummaryDelta::default();
            summary_delta.record_fill(&fill_summary);

            Ok(RuntimeSlotInjectionResult {
                root,
                changed: true,
                summary_delta,
            })
        }

        TemplateIrNodeKind::Sequence { children } => {
            let mut injected_children = Vec::with_capacity(children.len());
            let mut changed = false;
            let mut summary_delta = RuntimeFillSummaryDelta::default();
            for child_id in children {
                let RuntimeSlotInjectionResult {
                    root,
                    changed: child_changed,
                    summary_delta: child_delta,
                } = inject_runtime_slot_fill(
                    child_id,
                    injections,
                    slot_wrappers,
                    preserve_key,
                    fill_paths,
                    store,
                    copy_state,
                )?;
                changed |= child_changed;
                if child_changed {
                    summary_delta.merge_structural_child(child_delta)?;
                }
                injected_children.push(root);
            }

            if !changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
                    summary_delta: RuntimeFillSummaryDelta::default(),
                });
            }

            Ok(RuntimeSlotInjectionResult {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Sequence {
                        children: injected_children,
                    },
                    span,
                )),
                changed: true,
                summary_delta,
            })
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let (child_root, child_source_summary) = {
                let child_template = store.get_template(reference.root).ok_or_else(|| {
                    InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                        "Runtime slot site planning found child template {} missing from the TIR store.",
                        reference.root
                    ))
                })?;
                (child_template.root, child_template.summary.clone())
            };

            let injected = inject_runtime_slot_fill(
                child_root,
                injections,
                slot_wrappers,
                preserve_key,
                fill_paths,
                store,
                copy_state,
            )?;
            if !injected.changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
                    summary_delta: RuntimeFillSummaryDelta::default(),
                });
            }

            // The child is its own template summary scope: the parent records one
            // child reference either way, so only the derived child's own facts
            // move. Its version reuses the validated source summary plus this
            // pass's fill delta instead of re-walking the child's unchanged edges.
            let child_summary = apply_fill_summary(child_source_summary, &injected.summary_delta)?;
            let derived_id = store.push_derived_template_with_summary(
                reference.root,
                injected.root,
                child_summary,
                DerivedTemplateMetadata::preserve_source(),
            )?;
            Ok(RuntimeSlotInjectionResult {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::ChildTemplate {
                        reference: reference.with_root(derived_id),
                        occurrence_id,
                    },
                    span,
                )),
                changed: true,
                summary_delta: RuntimeFillSummaryDelta::default(),
            })
        }

        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            let injected_body = inject_runtime_slot_fill(
                body,
                injections,
                slot_wrappers,
                preserve_key,
                fill_paths,
                store,
                copy_state,
            )?;

            if !injected_body.changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
                    summary_delta: RuntimeFillSummaryDelta::default(),
                });
            }

            let mut summary_delta = RuntimeFillSummaryDelta::default();
            summary_delta.merge_structural_child(injected_body.summary_delta)?;
            copy_state.record_control_flow();
            Ok(RuntimeSlotInjectionResult {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Conditional {
                        selector,
                        selector_site_id,
                        body: injected_body.root,
                    },
                    span,
                )),
                changed: true,
                summary_delta,
            })
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
            ..
        } => {
            let injected_body = inject_runtime_slot_fill(
                body,
                injections,
                slot_wrappers,
                preserve_key,
                fill_paths,
                store,
                copy_state,
            )?;
            let mut changed = injected_body.changed;
            let mut summary_delta = RuntimeFillSummaryDelta::default();
            if injected_body.changed {
                summary_delta.merge_structural_child(injected_body.summary_delta)?;
            }
            let injected_aggregate = match aggregate_wrapper {
                Some(wrapper_id) => {
                    let injected_aggregate = inject_runtime_slot_fill(
                        wrapper_id,
                        injections,
                        slot_wrappers,
                        preserve_key,
                        fill_paths,
                        store,
                        copy_state,
                    )?;
                    changed |= injected_aggregate.changed;
                    if injected_aggregate.changed {
                        summary_delta.merge_structural_child(injected_aggregate.summary_delta)?;
                    }
                    Some(injected_aggregate.root)
                }
                None => None,
            };
            if !changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
                    summary_delta: RuntimeFillSummaryDelta::default(),
                });
            }

            copy_state.record_control_flow();
            Ok(RuntimeSlotInjectionResult {
                root: store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Loop {
                        header,
                        header_sites,
                        body: injected_body.root,
                        aggregate_wrapper: injected_aggregate,
                    },
                    span,
                )),
                changed: true,
                summary_delta,
            })
        }

        _ => Ok(RuntimeSlotInjectionResult {
            root: wrapper_root,
            changed: false,
            summary_delta: RuntimeFillSummaryDelta::default(),
        }),
    }
}

fn empty_render_root(
    store: &mut TemplateIrStore,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> TemplateIrNodeId {
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children: vec![] },
        span,
    ))
}

fn collapse_render_roots(
    roots: Vec<TemplateIrNodeId>,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    store: &mut TemplateIrStore,
) -> TemplateIrNodeId {
    match roots.len() {
        0 => empty_render_root(store, span),
        1 => roots[0],
        _ => store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: roots },
            span,
        )),
    }
}

#[cfg(test)]
#[path = "tests/sites_tests.rs"]
mod sites_tests;
