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

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplateSlotPlan, TirCopyState, TirSlotContributions, TirSlotSchema,
    convert_tir_tree_to_active_slot_plan, copy_tir_subtree_with_active_slot_plan,
    record_tir_copy_counters,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(in crate::compiler_frontend::ast::templates) use sources::tir_contributions_need_runtime;
pub(crate) use types::{RuntimeSlotContributionSourceId, RuntimeSlotSiteId};

/// Materializes a TIR-native runtime slot plan from routed TIR contributions.
///
/// WHAT: when the TIR-native head-chain composition detects that a wrapper's
///       fill content is non-const-evaluable (runtime), this function produces a
///       new TIR template entry whose `runtime_slot_plan` carries the
///       contribution sources and slot sites, starting from already-routed
///       TIR node IDs.
/// WHY: the HIR materializes runtime slot plans through the template's
///      `runtime_slot_plan` field. Without this path, TIR-native composition
///      would structurally expand runtime fills, flattening wrapper text and
///      fill content together — which breaks loop-control semantics (wrapper
///      text would render before `continue` is reached) and drops runtime
///      slot-site boundaries. Producing a runtime plan here ensures the HIR
///      sees the owned `RuntimeSlotSite` / contribution-source structure.
pub(in crate::compiler_frontend::ast::templates) fn materialize_tir_native_runtime_slot_plan(
    store: &mut TemplateIrStore,
    wrapper_template_id: crate::compiler_frontend::ast::templates::tir::TemplateIrId,
    schema: &TirSlotSchema,
    routed: &TirSlotContributions,
    string_table: &StringTable,
    location: &SourceLocation,
) -> Result<crate::compiler_frontend::ast::templates::tir::TemplateIrId, TemplateError> {
    let wrapper_root = store
        .get_template(wrapper_template_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR-native runtime slot plan: wrapper template ID was not present in the store.",
            )
        })?
        .root;

    let templates_before = store.template_count();
    let nodes_before = store.node_count();

    let mut copy_state = TirCopyState::new();
    let mut scratch_copy_state = TirCopyState::new();

    // Copy the wrapper's TIR root as a scratch tree. No active slot plan is
    // passed so Slot nodes stay as Slot nodes for site-draft collection, then
    // get converted to RuntimeSlotSite nodes after the site plan is built.
    let scratch_tir_root =
        copy_tir_subtree_with_active_slot_plan(wrapper_root, None, store, &mut scratch_copy_state)?;

    let slot_plan_id = store.reserve_slot_plan();

    let sources = sources::build_tir_native_contribution_sources(
        schema,
        routed,
        location,
        string_table,
        store,
        &mut copy_state,
    )?;

    let slot_sites = sites::build_runtime_wrapper_site_plan(
        scratch_tir_root,
        &sources,
        slot_plan_id,
        store,
        &mut copy_state,
    )?;

    let slot_plan = TemplateSlotPlan {
        location: location.clone(),
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

    Ok(template_id)
}
