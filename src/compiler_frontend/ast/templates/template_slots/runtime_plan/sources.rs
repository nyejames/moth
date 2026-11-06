//! Runtime contribution source planning.
//!
//! WHAT: Detects whether routed contributions require runtime lowering and
//! converts routed TIR nodes into deterministic source plans.
//!
//! WHY: Source plans describe authored contribution work that HIR should lower
//! exactly once. Wrapper-local child-wrapper and `$fresh` behavior belongs to
//! site planning so repeated placeholders can replay the same source safely.

use super::super::RuntimeSlotPlanningServices;
use super::types::{RuntimeSlotContributionSourceDraft, RuntimeSlotContributionSourceId};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::{
    ContributionShape, TemplateIrNode, TemplateIrNodeKind, TemplateIrStore,
    TemplateSlotContributionSourcePlan, TirCopyState, TirSlotContributions, TirSlotSchema,
    classify_tir_contribution_node, copy_tir_subtree_with_active_slot_plan,
    tir_node_is_const_evaluable_value, tir_node_is_control_flow_root,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
pub(in crate::compiler_frontend::ast::templates) fn tir_contributions_need_runtime(
    schema: &TirSlotSchema,
    contributions: &TirSlotContributions,
    string_table: &StringTable,
    store: &TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    external_package_registry: Option<&ExternalPackageRegistry>,
) -> Result<bool, TemplateError> {
    for target in schema.ordered_slot_keys(string_table) {
        let nodes = contributions.nodes_for_slot(&target);

        if nodes.is_empty() {
            continue;
        }

        for node_id in nodes {
            if !tir_node_is_const_evaluable_value(
                store,
                expressions,
                const_values,
                *node_id,
                string_table,
                external_package_registry,
            )? {
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
///       TIR node. A target that routing decided with empty content still gets
///       one neutral empty-sequence presence source, so site planning and named
///       marker propagation suppress the occurrence default instead of falling
///       back. Each node is deep-copied with no active slot plan so nested
///       child templates and insert contributions become independent render
///       roots that the HIR can materialize separately.
/// WHY: the TIR-native head-chain composition path starts from already-routed
///      TIR node IDs and produces the source plans consumed by runtime slot
///      sites. Keeping source construction here makes that responsibility
///      explicit without introducing a broad utility abstraction.
pub(in crate::compiler_frontend::ast::templates) fn build_tir_native_contribution_sources(
    schema: &TirSlotSchema,
    contributions: &TirSlotContributions,
    services: &RuntimeSlotPlanningServices<'_>,
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    copy_state: &mut TirCopyState,
) -> Result<Vec<RuntimeSlotContributionSourceDraft>, TemplateError> {
    let mut sources = Vec::new();

    for target in schema.ordered_slot_keys(services.string_table) {
        let target_nodes = contributions.nodes_for_slot(&target);

        // Routing presence is independent of content. An explicitly empty
        // contribution (`[$in title:]`) has no authored nodes, but the runtime
        // plan still needs one source for the target so site planning and named
        // marker propagation suppress the occurrence default instead of falling
        // back. An empty sequence is the neutral render root for that presence.
        if target_nodes.is_empty() {
            if contributions.slot_has_routed_presence(&target) {
                let render_root = store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Sequence {
                        children: Vec::new(),
                    },
                    None,
                ));
                let id = RuntimeSlotContributionSourceId(sources.len());
                sources.push(RuntimeSlotContributionSourceDraft {
                    source: TemplateSlotContributionSourcePlan {
                        source: id,
                        target: target.clone(),
                        render_root,
                        renders_wrapper_unconditionally: true,
                        span: None,
                    },
                    shape: ContributionShape::Other,
                    control_flow: false,
                });
            }
            continue;
        }

        for node_id in target_nodes {
            let id = RuntimeSlotContributionSourceId(sources.len());
            let source_span = store
                .get_node(*node_id)
                .map(|node| node.span)
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(CompilerStage::AstConstruction,
                        "TIR runtime slot planning: contribution node was not present in the store.",
                    )
                })?;

            // Deep-copy the contribution node so it becomes an independent
            // render root. No active slot plan is passed: the contribution's
            // own Slot nodes (if any) must survive as structural placeholders,
            // not be converted to RuntimeSlotSite nodes under the wrapper's plan.
            let render_root =
                copy_tir_subtree_with_active_slot_plan(*node_id, None, store, copy_state)?;

            let renders_wrapper_unconditionally = tir_node_is_const_evaluable_value(
                store,
                expressions,
                const_values,
                render_root,
                services.string_table,
                services.external_package_registry,
            )?;

            let shape = classify_tir_contribution_node(store, *node_id)?;
            let control_flow = tir_node_is_control_flow_root(store, render_root)?;

            sources.push(RuntimeSlotContributionSourceDraft {
                source: TemplateSlotContributionSourcePlan {
                    source: id,
                    target: target.clone(),
                    render_root,
                    renders_wrapper_unconditionally,
                    span: source_span,
                },
                shape,
                control_flow,
            });
        }
    }

    Ok(sources)
}

#[cfg(test)]
#[path = "tests/sources_tests.rs"]
mod sources_tests;
