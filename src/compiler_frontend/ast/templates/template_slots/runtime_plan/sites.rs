//! Runtime wrapper slot-site planning.
//!
//! WHAT: assigns stable IDs to concrete wrapper `$slot` occurrences and builds
//! one TIR render root per site from plan-qualified contribution markers.
//!
//! WHY: runtime applications must evaluate each source once while repeated slot
//! placeholders can still carry different `$children(..)` and `$fresh`
//! metadata. Ordinary contributions compose wrappers around the source marker;
//! control-flow contributions attach them conditionally to the marker's fresh
//! template root so a skipped source emits no wrapper. Named markers carry their
//! source's shape and control-flow class so wrapper-local named slots make the
//! same wrapper decisions as structural expansion.

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
    TemplateIrStore, TemplateSlotPlanId, TemplateSlotSitePlan, TemplateWrapperSetId, TirCopyState,
    TirSlotPlaceholder, TirSlotPlaceholderRef, TirSlotSchema,
    apply_slot_wrapper_sets_to_deferred_fill, attach_conditional_wrapper_set,
    classify_tir_contribution_node, collect_tir_slot_layout, collect_tir_slot_layout_from_root,
    conditional_wrapper_set_for_control_flow, copy_tir_subtree_with_active_slot_plan,
    fill_direct_child_wrapper_names_into, fill_named_slots_in_wrapper_template,
    push_runtime_slot_contribution_source, tir_node_is_control_flow_root,
    wrap_tir_node_in_wrappers_into,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use rustc_hash::FxHashMap;

struct RuntimeSlotSiteDraft {
    site: RuntimeSlotSiteId,
    placeholder: TirSlotPlaceholderRef,
}

pub(super) struct RuntimeWrapperSitePlanBuilder<'a> {
    pub(super) sources: &'a [RuntimeSlotContributionSourceDraft],
    pub(super) slot_plan_id: TemplateSlotPlanId,
    pub(super) store: &'a mut TemplateIrStore,
    pub(super) copy_state: &'a mut TirCopyState,
    pub(super) expressions: &'a AstExpressionBuilder,
    pub(super) const_values: &'a ConstValueStore,
    pub(super) string_table: &'a StringTable,
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
            let default_placeholder = TirSlotPlaceholder::with_wrapper_sets(
                placeholder.key.clone(),
                self.store.next_slot_occurrence_id(),
                Some(default),
                None,
                None,
                false,
            );
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
        // apply their own `$children(..)` metadata without re-evaluating the source.
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
    /// WHAT: injects the fill and the plan's routed named content into the wrapper
    ///       tree, then versions the original wrapper template around the injected
    ///       root and references that derived template through the named structural
    ///       or composed reference transition for this application.
    /// WHY: reducing a wrapper to a bare template or node ID would drop exact
    ///      reference authority and place the rewritten tree under the
    ///      surrounding application's unrelated view. Named content becomes a
    ///      source marker so the plan evaluates it once and replays it at every
    ///      wrapper occurrence.
    fn apply_wrapper_reference(
        &mut self,
        wrapper_ref: TemplateWrapperReference,
        fill_root: TemplateIrNodeId,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let (wrapper_root, wrapper_span) = {
            let wrapper_template = self.store.get_template(wrapper_ref.root).ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                    "Runtime slot site planning found a slot wrapper template ref {} that was not present in the TIR store.",
                    wrapper_ref
                ))
            })?;
            (wrapper_template.root, wrapper_template.span)
        };
        let layout = collect_tir_slot_layout(self.store, wrapper_ref.root)?;

        if !layout.schema.has_any_slots() {
            let wrapper_reference = wrapper_ref.into_structural_child_reference();
            let unchanged_wrapper = self.push_wrapper_child(wrapper_reference, wrapper_span);
            return Ok(collapse_render_roots(
                vec![unchanged_wrapper, fill_root],
                wrapper_span,
                self.store,
            ));
        }

        let Some(target_key) = layout.schema.loose_fill_target_key() else {
            return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
                "Runtime slot site planning could not apply a TIR slot wrapper to a loose contribution.",
            )
            .into());
        };

        let named_fill = self.named_markers_for_schema(&layout.schema)?;

        // Keep each named marker separate so the receiving placeholder can apply
        // that source's own shape, `$fresh` and conditional-emission decisions
        // before the occurrence is composed.
        let mut injections = Vec::with_capacity(1 + named_fill.len());
        injections.push(RuntimeSlotInjection {
            key: target_key,
            fills: vec![RuntimeSlotFill::SiteGated(fill_root)],
        });
        for (name, markers) in &named_fill {
            injections.push(RuntimeSlotInjection {
                key: SlotKey::Named(*name),
                fills: markers
                    .iter()
                    .copied()
                    .map(RuntimeSlotFill::Named)
                    .collect(),
            });
        }

        // The wrapper's own slot occurrences may declare `$children(..)` wrappers. A
        // site fill is already shape-gated by the site planner, but a named fill still
        // needs its authored facts, which its marker carries.
        let slot_fill_wrappers = RuntimeSlotFillWrappers {
            slot_plan_id: self.slot_plan_id,
            sources: self.sources,
            named_fill: &named_fill,
            expressions: self.expressions,
            const_values: self.const_values,
            string_table: self.string_table,
            resolved_wrapper_sets: FxHashMap::default(),
        };
        let injected = inject_runtime_slot_fill_with_slot_wrappers(
            wrapper_root,
            &injections,
            &mut Some(slot_fill_wrappers),
            self.store,
            self.copy_state,
        )?;

        // The wrapper's fixed direct children keep their occurrence-keyed overlay context,
        // so the same named markers must also reach the `$children(..)` sets stored there.
        // Reusing the markers keeps one plan source per named contribution while still
        // replaying it at every wrapper occurrence this application owns.
        let wrapper_ref = TemplateWrapperReference::new(
            wrapper_ref.root,
            wrapper_ref.phase,
            fill_direct_child_wrapper_names_into(
                self.store,
                self.expressions,
                self.const_values,
                wrapper_ref.context,
                &named_fill,
                self.string_table,
            )?,
        );

        self.push_composed_wrapper_child(wrapper_ref, injected.root, wrapper_span)
    }

    /// Builds one contribution-source marker per routed named slot the schema declares.
    ///
    /// WHAT: matches the plan's named contribution sources against the wrapper's own
    ///       named schema and returns freshly pushed markers carrying their authored
    ///       shape and control-flow class.
    /// WHY: a wrapper occurrence inside a runtime slot application must replay the
    ///       receiver's named content. Markers keep one evaluation per source while
    ///       still rendering the value at every occurrence, and the carried facts let
    ///       every later structural composition classify them exactly as the authored
    ///       contribution.
    fn named_markers_for_schema(
        &mut self,
        schema: &TirSlotSchema,
    ) -> Result<FxHashMap<StringId, Vec<TemplateIrNodeId>>, TemplateError> {
        named_markers_for_schema(self.store, self.slot_plan_id, self.sources, schema)
    }

    /// Derives each wrapper in a set with its own named slot markers filled.
    ///
    /// WHAT: builds markers per wrapper from that wrapper's declared names, fills that
    ///       wrapper's structural occurrences, slot-owned wrapper sets and wrapper-local
    ///       direct-child overlay, and returns the replacement set. Resolutions are
    ///       memoised in the caller's per-application cache.
    /// WHY: conditional wrapper application stores only wrapper references, and its
    ///       injection happens at runtime handoff after the store becomes read-only.
    ///       Keeping the fill per wrapper prevents two wrappers that declare the same name
    ///       from replaying each other's markers inside one occurrence. A wrapper's fixed
    ///       direct children carry nested `$children(..)` sets in its reference context,
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
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> Result<TemplateIrNodeId, TemplateError> {
        let derived_id = self.derive_wrapper_template(wrapper_ref.root, new_root)?;
        let reference = wrapper_ref.into_composed_child_reference(derived_id);
        Ok(self.push_wrapper_child(reference, span))
    }

    fn derive_wrapper_template(
        &mut self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
    ) -> Result<TemplateIrId, TemplateError> {
        Ok(self.store.push_structurally_derived_template(
            source,
            new_root,
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

/// One planned fill for a wrapper slot occurrence.
#[derive(Clone, Copy, Debug)]
enum RuntimeSlotFill {
    /// A site render root whose shape gating already happened while the site was
    /// built. Its receiving wrapper occurrence still owns the slot's `$children(..)`
    /// metadata, which applies unconditionally to this already-gated child.
    SiteGated(TemplateIrNodeId),
    /// A named contribution marker that still carries the authored contribution
    /// facts needed for gating against the receiving slot's wrapper sets.
    Named(TemplateIrNodeId),
}

/// One slot key and its ordered fills for a wrapper application.
struct RuntimeSlotInjection {
    key: SlotKey,
    fills: Vec<RuntimeSlotFill>,
}

struct RuntimeSlotInjectionResult {
    root: TemplateIrNodeId,
    changed: bool,
}

/// Services and named markers needed to apply wrappers stored on a wrapper's own slots.
///
/// WHAT: carries the plan identity, its sources, the plan's flattened named markers,
///       the expression, constant and string owners that wrapper composition needs,
///       and one named-marker fill cache for this receiving application.
/// WHY: the injected fill is a plan-qualified marker, so the slot's own
///      `$children(..)` wrappers cannot be inferred from the injected site root.
///      Named markers carry their own authored facts and are classified through the
///      shared TIR classifiers, while site-gated fills keep the existing
///      unconditional application contract because their site planning already
///      gated them. Repeated slot occurrences of the application replay the same
///      wrapper set against the same plan sources, so their resolutions are memoised
///      here rather than published again per occurrence.
struct RuntimeSlotFillWrappers<'a> {
    slot_plan_id: TemplateSlotPlanId,
    sources: &'a [RuntimeSlotContributionSourceDraft],
    named_fill: &'a FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    expressions: &'a AstExpressionBuilder,
    const_values: &'a ConstValueStore,
    string_table: &'a StringTable,
    resolved_wrapper_sets: FxHashMap<TemplateWrapperSetId, TemplateWrapperSetId>,
}

/// Builds one contribution-source marker per routed named slot the schema declares.
///
/// WHAT: matches the plan's named contribution sources against the receiver's named
///       schema and returns freshly pushed markers carrying their authored shape and
///       control-flow class.
/// WHY: a wrapper occurrence inside a runtime slot application must replay the
///       receiver's named content. Markers keep one evaluation per source while the
///       carried facts let every occurrence classify them exactly as the authored
///       contribution.
fn named_markers_for_schema(
    store: &mut TemplateIrStore,
    slot_plan_id: TemplateSlotPlanId,
    sources: &[RuntimeSlotContributionSourceDraft],
    schema: &TirSlotSchema,
) -> Result<FxHashMap<StringId, Vec<TemplateIrNodeId>>, TemplateError> {
    let mut fills: FxHashMap<StringId, Vec<TemplateIrNodeId>> = FxHashMap::default();
    if schema.named_slots.is_empty() {
        return Ok(fills);
    }

    for draft in sources {
        let SlotKey::Named(name) = draft.source.target else {
            continue;
        };
        if !schema.named_slots.contains(&name) {
            continue;
        }

        let marker = push_runtime_slot_contribution_source(
            store,
            slot_plan_id,
            draft.source.source,
            draft.shape,
            draft.control_flow,
            draft.source.span,
        );
        fills.entry(name).or_default().push(marker);
    }

    Ok(fills)
}

/// Derives each wrapper in a set with its own named slot markers filled.
///
/// WHAT: builds markers per wrapper from that wrapper's declared names, fills that
///       wrapper's structural occurrences, slot-owned wrapper sets and wrapper-local
///       direct-child overlay, and returns the replacement set. One resolution per
///       distinct set is memoised in the caller's receiving-application cache.
/// WHY: conditional wrapper application stores only wrapper references, and its
///       injection happens at runtime handoff after the store becomes read-only.
///       Keeping the fill per wrapper prevents two wrappers that declare the same name
///       from replaying each other's markers inside one occurrence. A wrapper's fixed
///       direct children carry nested `$children(..)` sets in its reference context,
///       and wrappers stored on its slots are applied only once a runtime child is
///       injected, so both paths need the markers before handoff begins. Sibling
///       sources and repeated slot occurrences replay one set against the same
///       sources, and every repeat fill would otherwise publish equivalent derived
///       templates and wrapper sets that the store cannot deduplicate by identity.
#[allow(
    clippy::too_many_arguments,
    reason = "resolving a set needs the store, plan sources, expression/constant/string owners, the application cache and the set ID"
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
    // Conditional wrappers are the common case and usually carry no named targets.
    // Skip the per-wrapper layout walk when the plan has no named source at all.
    if !sources
        .iter()
        .any(|draft| matches!(draft.source.target, SlotKey::Named(_)))
    {
        return Ok(wrapper_set_id);
    }

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
        let layout = collect_tir_slot_layout(store, wrapper.root)?;
        let named_fill = named_markers_for_schema(store, slot_plan_id, sources, &layout.schema)?;
        let (filled_root, filled_context) = if named_fill.is_empty() {
            (None, wrapper.context)
        } else {
            // Slots whose own wrappers are deferred to handoff still need the named
            // content composed now; handoff cannot derive templates.
            (
                fill_named_slots_in_wrapper_template(
                    store,
                    expressions,
                    const_values,
                    wrapper.root,
                    &named_fill,
                    string_table,
                )?,
                fill_direct_child_wrapper_names_into(
                    store,
                    expressions,
                    const_values,
                    wrapper.context,
                    &named_fill,
                    string_table,
                )?,
            )
        };

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

/// Applies a receiving placeholder's `$children(..)` and `$fresh` semantics to a
/// named marker.
///
/// WHAT: classifies the marker through the shared contribution classifiers, then
///       applies the placeholder's child and applied wrapper sets in
///       structural-expansion order. Control-flow sources receive the same wrappers
///       as conditional metadata so an empty source emits no wrapper.
/// WHY: the marker carries the authored shape and control-flow class, so named and
///      structural composition make the same decisions instead of wrapping every
///      fill unconditionally or losing skipped and conditional output.
fn apply_named_fill_wrapper_sets(
    store: &mut TemplateIrStore,
    slot_wrappers: &mut RuntimeSlotFillWrappers<'_>,
    child_wrapper_set: Option<TemplateWrapperSetId>,
    applied_child_wrapper_set: Option<TemplateWrapperSetId>,
    skip_parent_child_wrappers: bool,
    fill_root: TemplateIrNodeId,
) -> Result<TemplateIrNodeId, TemplateError> {
    let shape = classify_tir_contribution_node(store, fill_root)?;
    let control_flow = tir_node_is_control_flow_root(store, fill_root)?;

    if control_flow {
        let Some(wrapper_set_id) = conditional_wrapper_set_for_control_flow(
            store,
            child_wrapper_set,
            applied_child_wrapper_set,
            skip_parent_child_wrappers,
            shape,
        )?
        else {
            return Ok(fill_root);
        };

        let wrapper_set_id = fill_named_markers_in_wrapper_set(
            store,
            slot_wrappers.slot_plan_id,
            slot_wrappers.sources,
            slot_wrappers.expressions,
            slot_wrappers.const_values,
            slot_wrappers.string_table,
            &mut slot_wrappers.resolved_wrapper_sets,
            wrapper_set_id,
        )?;
        return attach_conditional_wrapper_set(store, fill_root, wrapper_set_id);
    }

    // Only child-template contributions receive ordinary external wrappers. Text,
    // expressions and other non-child values keep their authored position.
    if !shape.is_child_template_contribution() {
        return Ok(fill_root);
    }

    // `$fresh` suppresses the receiving placeholder's own child set. Wrapping a
    // child contribution always produces another child-shaped root, so the
    // receiver's already-applied set still follows in structural-expansion order.
    let mut current = fill_root;
    if !shape.skips_parent_child_wrappers()
        && let Some(wrapper_set_id) = child_wrapper_set
    {
        current = apply_named_fill_wrapper_set(store, &*slot_wrappers, current, wrapper_set_id)?;
    }

    if !skip_parent_child_wrappers && let Some(wrapper_set_id) = applied_child_wrapper_set {
        current = apply_named_fill_wrapper_set(store, &*slot_wrappers, current, wrapper_set_id)?;
    }

    Ok(current)
}

/// Applies one ordered wrapper set around a named contribution marker.
///
/// WHAT: resolves complete wrapper references from the wrapper-set side table and
///       iterates them forward (innermost-first) through the shared wrapper
///       composition primitive.
/// WHY: `TemplateWrapperSet::wrappers` is stored innermost-to-outermost, so forward
///      consumption yields the `outermost(innermost(fill))` nesting structural
///      expansion produces. The named fill map keeps nested wrapper declarations
///      replaying the same plan markers instead of evaluating the source again.
fn apply_named_fill_wrapper_set(
    store: &mut TemplateIrStore,
    slot_wrappers: &RuntimeSlotFillWrappers<'_>,
    fill_root: TemplateIrNodeId,
    wrapper_set_id: TemplateWrapperSetId,
) -> Result<TemplateIrNodeId, TemplateError> {
    let Some(wrapper_set) = store.get_wrapper_set(wrapper_set_id) else {
        return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
            "Runtime slot site planning found a slot wrapper-set ID that was not present in the TIR store.",
        )
        .into());
    };
    let wrappers = wrapper_set.wrappers.clone();
    add_ast_counter(AstCounter::TemplateWrapperApplications, wrappers.len());

    wrap_tir_node_in_wrappers_into(
        store,
        slot_wrappers.expressions,
        slot_wrappers.const_values,
        fill_root,
        &wrappers,
        slot_wrappers.named_fill,
        slot_wrappers.string_table,
    )
}

/// Slot-only injection used by the focused site tests.
///
/// Slot occurrences without an injection render empty. Fill subtrees are copied
/// independently and receive fresh expression and occurrence IDs. Versioning the
/// wrapper structure itself preserves IDs for semantic sites that still represent
/// the same occurrence.
#[cfg(test)]
fn inject_runtime_slot_fill(
    wrapper_root: TemplateIrNodeId,
    injections: &[(SlotKey, TemplateIrNodeId)],
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<RuntimeSlotInjectionResult, TemplateError> {
    let injections: Vec<RuntimeSlotInjection> = injections
        .iter()
        .map(|(key, root)| RuntimeSlotInjection {
            key: key.clone(),
            fills: vec![RuntimeSlotFill::SiteGated(*root)],
        })
        .collect();
    inject_runtime_slot_fill_with_slot_wrappers(
        wrapper_root,
        &injections,
        &mut None,
        store,
        copy_state,
    )
}

/// Injects slot fills and applies each receiving slot's own `$children(..)` wrappers.
///
/// WHAT: like the test-only slot injection, fills every matching occurrence, but applies
///       the wrappers stored on the matched placeholder before the surrounding wrapper
///       template renders the fill. Site-gated fills keep the existing unconditional
///       wrapper application; named fills classify each source from the facts its marker
///       carries.
/// WHY: a wrapper applied around a runtime contribution still owns its own
///      `$children(..)` metadata. Deferring that metadata to handoff would require
///      deriving templates from an immutable store, so the site tree must carry the
///      wrappers structurally, exactly as authored slot expansion does, including
///      `$fresh` and conditional-emission behavior.
fn inject_runtime_slot_fill_with_slot_wrappers(
    wrapper_root: TemplateIrNodeId,
    injections: &[RuntimeSlotInjection],
    slot_wrappers: &mut Option<RuntimeSlotFillWrappers<'_>>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<RuntimeSlotInjectionResult, TemplateError> {
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
                // Routing owns presence. An occurrence with no matching injection
                // keeps its own fallback for handoff materialization. A matching
                // injection, including an explicitly empty one, suppresses that
                // fallback by replacing the occurrence with its fill roots.
                if placeholder.default.is_some() {
                    return Ok(RuntimeSlotInjectionResult {
                        root: wrapper_root,
                        changed: false,
                    });
                }

                return Ok(RuntimeSlotInjectionResult {
                    root: empty_render_root(store, span),
                    changed: true,
                });
            };

            let mut roots = Vec::with_capacity(injection.fills.len());
            for fill in &injection.fills {
                let source_root = match fill {
                    RuntimeSlotFill::SiteGated(root) => *root,
                    RuntimeSlotFill::Named(marker) => *marker,
                };
                let copied_fill =
                    copy_tir_subtree_with_active_slot_plan(source_root, None, store, copy_state)?;
                let root = match fill {
                    RuntimeSlotFill::SiteGated(_) => match slot_wrappers.as_ref() {
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
                    },
                    RuntimeSlotFill::Named(_) => match slot_wrappers.as_mut() {
                        Some(slot_wrappers) => apply_named_fill_wrapper_sets(
                            store,
                            slot_wrappers,
                            placeholder.child_wrapper_set,
                            placeholder.applied_child_wrapper_set,
                            placeholder.skip_parent_child_wrappers,
                            copied_fill,
                        )?,
                        None => copied_fill,
                    },
                };
                roots.push(root);
            }

            Ok(RuntimeSlotInjectionResult {
                root: collapse_render_roots(roots, span, store),
                changed: true,
            })
        }

        TemplateIrNodeKind::Sequence { children } => {
            let mut injected_children = Vec::with_capacity(children.len());
            let mut changed = false;
            for child_id in children {
                let injected_child = inject_runtime_slot_fill_with_slot_wrappers(
                    child_id,
                    injections,
                    slot_wrappers,
                    store,
                    copy_state,
                )?;
                changed |= injected_child.changed;
                injected_children.push(injected_child.root);
            }

            if !changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
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
            })
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let child_root = store
                .get_template(reference.root)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                        "Runtime slot site planning found child template {} missing from the TIR store.",
                        reference.root
                    ))
                })?
                .root;

            let injected = inject_runtime_slot_fill_with_slot_wrappers(
                child_root,
                injections,
                slot_wrappers,
                store,
                copy_state,
            )?;
            if !injected.changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
                });
            }

            let derived_id = store.push_structurally_derived_template(
                reference.root,
                injected.root,
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
            })
        }

        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            let injected_body = inject_runtime_slot_fill_with_slot_wrappers(
                body,
                injections,
                slot_wrappers,
                store,
                copy_state,
            )?;

            if !injected_body.changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
                });
            }

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
            })
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
            ..
        } => {
            let injected_body = inject_runtime_slot_fill_with_slot_wrappers(
                body,
                injections,
                slot_wrappers,
                store,
                copy_state,
            )?;
            let mut changed = injected_body.changed;
            let injected_aggregate = match aggregate_wrapper {
                Some(wrapper_id) => {
                    let injected_aggregate = inject_runtime_slot_fill_with_slot_wrappers(
                        wrapper_id,
                        injections,
                        slot_wrappers,
                        store,
                        copy_state,
                    )?;
                    changed |= injected_aggregate.changed;
                    Some(injected_aggregate.root)
                }
                None => None,
            };
            if !changed {
                return Ok(RuntimeSlotInjectionResult {
                    root: wrapper_root,
                    changed: false,
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
            })
        }

        _ => Ok(RuntimeSlotInjectionResult {
            root: wrapper_root,
            changed: false,
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
