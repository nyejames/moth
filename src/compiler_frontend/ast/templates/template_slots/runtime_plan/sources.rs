//! Runtime contribution source planning.
//!
//! WHAT: Detects whether routed contributions require runtime lowering and
//! converts routed TIR nodes into deterministic source plans.
//!
//! WHY: Source plans describe authored contribution work that HIR should lower
//! exactly once. Wrapper-local `$children(..)` and `$fresh` behavior belongs to
//! site planning so repeated placeholders can replay the same source safely.

use super::types::{RuntimeSlotContributionSourceDraft, RuntimeSlotContributionSourceId};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplateSlotContributionSourcePlan, TirCopyState, TirSlotContributions,
    TirSlotSchema, classify_tir_contribution_node, copy_tir_subtree_with_active_slot_plan,
    tir_node_is_const_evaluable_value,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(in crate::compiler_frontend::ast::templates) fn tir_contributions_need_runtime(
    schema: &TirSlotSchema,
    contributions: &TirSlotContributions,
    string_table: &StringTable,
    store: &TemplateIrStore,
) -> Result<bool, TemplateError> {
    for target in schema.ordered_slot_keys(string_table) {
        let nodes = contributions.nodes_for_slot(&target);

        if nodes.is_empty() {
            continue;
        }

        for node_id in nodes {
            if !tir_node_is_const_evaluable_value(store, *node_id, string_table)? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Builds contribution source drafts from TIR-native routed contributions.
///
/// WHAT: iterates slot keys in schema order (default, positional ascending,
///       named by resolved spelling) and creates one source draft per routed
///       TIR node. Each node is deep-copied with no active slot plan so nested
///       child templates and insert contributions become independent render
///       roots that the HIR can materialize separately.
/// WHY: the TIR-native head-chain composition path starts from already-routed
///      TIR node IDs and produces the source plans consumed by runtime slot
///      sites. Keeping source construction here makes that responsibility
///      explicit without introducing a broad utility abstraction.
pub(in crate::compiler_frontend::ast::templates) fn build_tir_native_contribution_sources(
    schema: &TirSlotSchema,
    contributions: &TirSlotContributions,
    location: &SourceLocation,
    string_table: &StringTable,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<Vec<RuntimeSlotContributionSourceDraft>, TemplateError> {
    let mut sources = Vec::new();

    for target in schema.ordered_slot_keys(string_table) {
        for node_id in contributions.nodes_for_slot(&target) {
            let id = RuntimeSlotContributionSourceId(sources.len());

            // Deep-copy the contribution node so it becomes an independent
            // render root. No active slot plan is passed: the contribution's
            // own Slot nodes (if any) must survive as structural placeholders,
            // not be converted to RuntimeSlotSite nodes under the wrapper's plan.
            let render_root =
                copy_tir_subtree_with_active_slot_plan(*node_id, None, store, copy_state)?;

            let renders_wrapper_unconditionally =
                tir_node_is_const_evaluable_value(store, render_root, string_table)?;

            let shape = classify_tir_contribution_node(store, *node_id)?;

            sources.push(RuntimeSlotContributionSourceDraft {
                source: TemplateSlotContributionSourcePlan {
                    source: id,
                    target: target.clone(),
                    render_root,
                    renders_wrapper_unconditionally,
                    location: location.clone(),
                },
                shape,
            });
        }
    }

    Ok(sources)
}
