//! AST runtime slot application planning.
//!
//! WHAT: TIR-native runtime slot plan materialization that produces owned
//! handoff payloads for the AST/HIR boundary.
//!
//! WHY: HIR should only consume prepared source/site plans. The runtime slot
//! planner writes side-tables into the module-scoped TIR store, then returns
//! neutral owned handoff shapes defined in `runtime_handoff.rs`.

mod sites;
mod sources;
mod types;

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrNodeId, TemplateIrStore, TemplateSlotPlan, TemplateSlotPlanId,
    TemplateViewContext, TirCopyState, TirSlotContributions, TirSlotSchema,
    convert_tir_tree_to_active_slot_plan,
    copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context,
    fill_direct_child_wrapper_names_into, fill_wrapper_tree_named_contexts,
    push_runtime_slot_contribution_source, record_tir_copy_counters,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringId;

use rustc_hash::FxHashMap;

use self::types::RuntimeSlotContributionSourceDraft;
use super::RuntimeSlotPlanningServices;
pub(in crate::compiler_frontend::ast::templates) use sources::tir_contributions_need_runtime;
pub(crate) use types::{RuntimeSlotContributionSourceId, RuntimeSlotSiteId};

/// Materializes a TIR-native runtime slot plan from routed TIR contributions.
///
/// WHAT: when the TIR-native head-chain composition detects that a wrapper's
///       fill content is non-const-evaluable (runtime), this function produces a
///       new TIR template entry whose `runtime_slot_plan` carries the
///       contribution sources and slot sites, starting from already-routed
///       TIR node IDs. It rebases the receiver's value-carried view context onto
///       the copied root so occurrence-keyed overlays still name the copied
///       direct children, then resolves the plan's named markers into both the
///       receiver's own overlay and every reachable child-edge decorator overlay
///       inside the copied root. A nested child template that owns its own runtime
///       plan is a leaf: only that plan may resolve markers inside its tree.
/// WHY: AST handoff materialization consumes this plan to produce owned
///      contribution sources and slot sites. HIR evaluates each source once
///      and replays it at every site, preserving structural no-output for
///      skipped conditional or zero-iteration contributions.
#[allow(
    clippy::too_many_arguments,
    reason = "the plan needs the store, expression and constant owners, the receiver identity and its exact view context, the schema and routed fill, and the string/resource services; bundling them would duplicate the existing planning inputs"
)]
pub(in crate::compiler_frontend::ast::templates) fn materialize_tir_native_runtime_slot_plan(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_template_id: TemplateIrId,
    wrapper_view_context: TemplateViewContext,
    schema: &TirSlotSchema,
    routed: &TirSlotContributions,
    services: &RuntimeSlotPlanningServices<'_>,
) -> Result<(TemplateIrId, TemplateViewContext), TemplateError> {
    let (wrapper_root, retained_template_wrapper_set) = store
        .get_template(wrapper_template_id)
        .map(|template| (template.root, template.conditional_child_wrapper_set))
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR-native runtime slot plan: wrapper template ID was not present in the store.",
            )
        })?;

    let templates_before = store.template_count();
    let nodes_before = store.node_count();

    let mut copy_state = TirCopyState::new();
    let mut scratch_copy_state = TirCopyState::new();

    // Copy the wrapper's TIR root as a scratch tree. No active slot plan is
    // passed so Slot nodes stay as Slot nodes for site-draft collection, then
    // get converted to RuntimeSlotSite nodes after the site plan is built.
    // The receiver's view context is rebased in the same pass: its occurrence
    // keys name the source root's children, and the scratch root mints fresh
    // child occurrences for every copied child-template node. The receiver
    // template's conditional child wrapper set survives derived publication, so
    // its wrapper templates stay reachable for retained expression overrides.
    let (scratch_tir_root, scratch_view_context) =
        copy_tir_subtree_with_active_slot_plan_and_retained_wrapper_context(
            wrapper_root,
            None,
            wrapper_view_context,
            retained_template_wrapper_set,
            store,
            &mut scratch_copy_state,
        )?;

    let slot_plan_id = store.reserve_slot_plan();
    let sources = sources::build_tir_native_contribution_sources(
        schema,
        routed,
        services,
        store,
        expressions,
        const_values,
        &mut copy_state,
    )?;

    // One plan-qualified marker per named source is shared by every overlay that
    // replays this application's named content. Reusing the node IDs keeps the
    // plan's evaluate-once contract while matching the structural expansion path,
    // which also reuses one routed named node at every occurrence.
    let named_markers = plan_named_markers(store, slot_plan_id, &sources);

    // A fixed child inside the receiver root keeps its decorators in the child
    // reference's wrapper-context overlay rather than in the root's structural
    // nodes. The structural path fills those overlays while expanding slots; the
    // runtime handoff store is read-only, so resolve the same markers into the
    // copied root before site planning and publication. Without this, a named
    // contribution accepted by the context-aware receiver schema would be routed
    // but never reach the decorator that declares it. The fill stops at a nested
    // child that owns its own runtime plan, matching the schema boundary that keeps
    // that child's names out of this receiver's addressable set.
    let scratch_tir_root = if named_markers.is_empty() {
        scratch_tir_root
    } else {
        fill_wrapper_tree_named_contexts(
            store,
            expressions,
            const_values,
            scratch_tir_root,
            &named_markers,
            services.string_table,
        )?
        .unwrap_or(scratch_tir_root)
    };

    let slot_sites = sites::RuntimeWrapperSitePlanBuilder {
        sources: &sources,
        slot_plan_id,
        store,
        copy_state: &mut copy_state,
        expressions,
        const_values,
        string_table: services.string_table,
        resolved_wrappers: FxHashMap::default(),
    }
    .build_slot_sites(scratch_tir_root)?;

    // The receiver's own occurrence overlay sits outside the copied root, so its
    // inherited wrapper sets need the same markers directly.
    let scratch_view_context = fill_receiver_direct_child_names_with_markers(
        store,
        expressions,
        const_values,
        scratch_view_context,
        &named_markers,
        services,
    )?;

    let slot_plan = TemplateSlotPlan {
        contribution_sources: sources.into_iter().map(|source| source.source).collect(),
        slot_sites,
    };

    // Convert the scratch tree using the local site-plan slice before commit.
    copy_state.reset_runtime_slot_site_cursor(slot_plan_id);
    convert_tir_tree_to_active_slot_plan(
        scratch_tir_root,
        slot_plan_id,
        &slot_plan.slot_sites,
        store,
        &mut copy_state,
    )?;

    store.commit_slot_plan(slot_plan_id, slot_plan)?;

    let template_id = store.push_runtime_slot_derived_template(
        wrapper_template_id,
        scratch_tir_root,
        slot_plan_id,
        crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(),
    )?;
    record_tir_copy_counters(store, templates_before, nodes_before, &copy_state);
    add_ast_counter(AstCounter::RuntimeSlotHandoffsMaterialized, 1);

    Ok((template_id, scratch_view_context))
}

/// Builds one plan source marker per named routed contribution.
///
/// WHAT: emits a `RuntimeSlotContributionSource` node for every named source draft,
///       including a routed target whose content is explicitly empty, keyed by the
///       declared slot name in source order.
/// WHY: every receiver-owned and nested child-edge overlay replays the same plan
///      sources. One shared marker set keeps the evaluate-once contract and lets the
///      caller resolve it into every relevant overlay before the store becomes
///      read-only at handoff.
fn plan_named_markers(
    store: &mut TemplateIrStore,
    slot_plan_id: TemplateSlotPlanId,
    sources: &[RuntimeSlotContributionSourceDraft],
) -> FxHashMap<StringId, Vec<TemplateIrNodeId>> {
    let mut named_markers: FxHashMap<StringId, Vec<TemplateIrNodeId>> = FxHashMap::default();
    for source in sources {
        let SlotKey::Named(name) = source.source.target else {
            continue;
        };
        let marker = push_runtime_slot_contribution_source(
            store,
            slot_plan_id,
            source.source.source,
            source.shape,
            source.control_flow,
            source.source.span,
        );
        named_markers.entry(name).or_default().push(marker);
    }
    named_markers
}

/// Resolves named contribution markers into the receiver's direct-child wrapper sets.
///
/// WHAT: rewrites the receiver's occurrence-keyed wrapper overlay so every inherited
///       `$children(..)` wrapper replays the shared plan markers at its own declared
///       named slots.
/// WHY: inherited wrappers around fixed direct children are materialized outside the
///      plan's slot-site roots, so they are not reached by site planning. Markers keep
///      the plan's evaluate-once contract and its structural no-output bookkeeping
///      instead of embedding authored nodes (which may be runtime) in the wrapper.
fn fill_receiver_direct_child_names_with_markers(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    context: TemplateViewContext,
    named_markers: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    services: &RuntimeSlotPlanningServices<'_>,
) -> Result<TemplateViewContext, TemplateError> {
    if context.wrapper_context.is_none() || named_markers.is_empty() {
        return Ok(context);
    }

    fill_direct_child_wrapper_names_into(
        store,
        expressions,
        const_values,
        context,
        named_markers,
        services.string_table,
    )
}
