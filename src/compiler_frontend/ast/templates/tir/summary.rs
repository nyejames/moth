//! Cheap TIR shape and capacity metadata.
//!
//! `TemplateIrSummary` stores counts and flags that have measured callers:
//! output capacity, node counts, depth and structural feature presence.
//! It is not semantic proof. Preparation owns constness, unresolved slots
//! and helper classification.
//!
//! Unresolved structural slots (`slot_count`) are separate from runtime slot
//! sites (`runtime_slot_site_count`). `has_slots()` is derived from `slot_count`.

use std::collections::HashSet;

use crate::compiler_frontend::ast::templates::tir::ids::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;

// -------------------------
//  Template IR Summary
// -------------------------

/// Shape metadata for a single TIR template.
///
/// `max_depth` is 0 for a single-node root. `estimated_output_bytes` is a
/// conservative lower bound; runtime expressions contribute nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateIrSummary {
    /// Conservative estimate of the final folded output size in bytes.
    pub(crate) estimated_output_bytes: usize,

    pub(crate) text_node_count: u32,
    pub(crate) text_byte_count: usize,
    pub(crate) dynamic_expression_count: u32,
    pub(crate) child_template_count: u32,

    /// Head-origin nodes recorded before the first body-origin node.
    pub(crate) head_node_count: u32,

    /// Unresolved structural `Slot` placeholders.
    pub(crate) slot_count: u32,

    /// Converted runtime slot sites. Separate from `slot_count`.
    pub(crate) runtime_slot_site_count: u32,

    /// Plan-qualified contribution-source markers.
    pub(crate) runtime_slot_contribution_source_count: u32,

    pub(crate) insert_contribution_count: u32,
    pub(crate) wrapper_count: u32,
    pub(crate) max_depth: usize,
    pub(crate) has_control_flow: bool,
    pub(crate) has_reactivity: bool,
}

impl TemplateIrSummary {
    pub(crate) fn empty() -> Self {
        Self {
            estimated_output_bytes: 0,
            text_node_count: 0,
            text_byte_count: 0,
            dynamic_expression_count: 0,
            child_template_count: 0,
            head_node_count: 0,
            slot_count: 0,
            runtime_slot_site_count: 0,
            runtime_slot_contribution_source_count: 0,
            insert_contribution_count: 0,
            wrapper_count: 0,
            max_depth: 0,
            has_control_flow: false,
            has_reactivity: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn has_slots(&self) -> bool {
        self.slot_count > 0
    }

    pub(crate) fn has_control_flow(&self) -> bool {
        self.has_control_flow
    }

    pub(crate) fn set_head_node_count(&mut self, head_node_count: u32) {
        self.head_node_count = head_node_count;
    }

    pub(crate) fn record_text_node(&mut self, byte_len: usize) {
        self.text_node_count += 1;
        self.text_byte_count += byte_len;
        self.estimated_output_bytes += byte_len;
    }

    pub(crate) fn record_dynamic_expression(&mut self, has_reactive_subscription: bool) {
        self.dynamic_expression_count += 1;
        if has_reactive_subscription {
            self.has_reactivity = true;
        }
    }

    pub(crate) fn record_child_template(&mut self) {
        self.child_template_count += 1;
    }

    pub(crate) fn record_slot(&mut self) {
        self.slot_count += 1;
    }

    pub(crate) fn record_control_flow(&mut self) {
        self.has_control_flow = true;
    }

    pub(crate) fn record_runtime_slot_site(&mut self) {
        self.runtime_slot_site_count += 1;
    }

    pub(crate) fn record_runtime_slot_contribution_source(&mut self) {
        self.runtime_slot_contribution_source_count += 1;
    }

    pub(crate) fn record_insert_contribution(&mut self) {
        self.insert_contribution_count += 1;
    }

    pub(crate) fn record_reactivity(&mut self) {
        self.has_reactivity = true;
    }
}

impl Default for TemplateIrSummary {
    fn default() -> Self {
        Self::empty()
    }
}

// -------------------------
//  Existing-node summary
// -------------------------

/// Summarizes an existing node used directly as a template root.
pub(crate) fn summarize_existing_root(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
) -> Result<TemplateIrSummary, CompilerError> {
    let mut summary = TemplateIrSummary::empty();
    let mut visiting_nodes = HashSet::new();
    let mut visiting_templates = HashSet::new();
    let mut completed_templates = HashSet::new();
    accumulate_nodes(
        store,
        std::slice::from_ref(&root_node_id),
        0,
        &mut summary,
        &mut visiting_nodes,
        &mut visiting_templates,
        &mut completed_templates,
    )?;
    Ok(summary)
}

/// Summarizes a published runtime-slot template from its completed root and
/// committed plan-owned source and site trees.
pub(crate) fn summarize_runtime_slot_representation(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
    plan: TemplateSlotPlanId,
) -> Result<TemplateIrSummary, CompilerError> {
    let plan_roots = {
        let slot_plan = store.get_slot_plan(plan).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR summary cannot read missing or uncommitted slot plan {plan}."
            ))
        })?;
        slot_plan
            .contribution_sources
            .iter()
            .map(|source| source.render_root)
            .chain(slot_plan.slot_sites.iter().map(|site| site.render_root))
            .collect::<Vec<_>>()
    };

    let mut summary = TemplateIrSummary::empty();
    let mut visiting_templates = HashSet::new();
    let mut completed_templates = HashSet::new();
    let mut visiting_nodes = HashSet::new();
    accumulate_nodes(
        store,
        std::slice::from_ref(&root_node_id),
        0,
        &mut summary,
        &mut visiting_nodes,
        &mut visiting_templates,
        &mut completed_templates,
    )?;

    for plan_root in plan_roots {
        let mut visiting_nodes = HashSet::new();
        accumulate_nodes(
            store,
            std::slice::from_ref(&plan_root),
            0,
            &mut summary,
            &mut visiting_nodes,
            &mut visiting_templates,
            &mut completed_templates,
        )?;
    }

    Ok(summary)
}

fn accumulate_nodes(
    store: &TemplateIrStore,
    node_ids: &[TemplateIrNodeId],
    depth: usize,
    summary: &mut TemplateIrSummary,
    visiting_nodes: &mut HashSet<TemplateIrNodeId>,
    visiting_templates: &mut HashSet<TemplateIrId>,
    completed_templates: &mut HashSet<TemplateIrId>,
) -> Result<(), CompilerError> {
    for &node_id in node_ids {
        if !visiting_nodes.insert(node_id) {
            return Err(CompilerError::compiler_error(format!(
                "TIR summary encountered a node cycle at {node_id:?}"
            )));
        }

        let Some(node) = store.get_node(node_id) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR summary requested missing node {node_id}."
            )));
        };

        if depth > summary.max_depth {
            summary.max_depth = depth;
        }

        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                accumulate_nodes(
                    store,
                    children,
                    child_depth(depth)?,
                    summary,
                    visiting_nodes,
                    visiting_templates,
                    completed_templates,
                )?;
            }

            TemplateIrNodeKind::Text { byte_len, .. } => {
                summary.record_text_node(*byte_len);
                if store.node_reactive_subscription(node_id)?.is_some() {
                    summary.record_reactivity();
                }
            }

            TemplateIrNodeKind::DynamicExpression {
                reactive_subscription,
                ..
            } => {
                summary.record_dynamic_expression(reactive_subscription.is_some());
            }

            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                validate_child_template(
                    store,
                    reference.root,
                    depth,
                    visiting_templates,
                    completed_templates,
                )?;
                summary.record_child_template();
            }

            TemplateIrNodeKind::Slot { .. } => {
                summary.record_slot();
            }

            TemplateIrNodeKind::InsertContribution { .. } => {
                summary.record_insert_contribution();
            }

            TemplateIrNodeKind::BranchChain { branches, fallback } => {
                summary.record_control_flow();

                for branch in branches {
                    accumulate_nodes(
                        store,
                        std::slice::from_ref(&branch.body),
                        child_depth(depth)?,
                        summary,
                        visiting_nodes,
                        visiting_templates,
                        completed_templates,
                    )?;
                }
                if let Some(fallback_id) = fallback {
                    accumulate_nodes(
                        store,
                        std::slice::from_ref(fallback_id),
                        child_depth(depth)?,
                        summary,
                        visiting_nodes,
                        visiting_templates,
                        completed_templates,
                    )?;
                }
            }

            TemplateIrNodeKind::Loop {
                body,
                aggregate_wrapper,
                ..
            } => {
                summary.record_control_flow();

                accumulate_nodes(
                    store,
                    std::slice::from_ref(body),
                    child_depth(depth)?,
                    summary,
                    visiting_nodes,
                    visiting_templates,
                    completed_templates,
                )?;
                if let Some(wrapper_id) = aggregate_wrapper {
                    accumulate_nodes(
                        store,
                        std::slice::from_ref(wrapper_id),
                        child_depth(depth)?,
                        summary,
                        visiting_nodes,
                        visiting_templates,
                        completed_templates,
                    )?;
                }
            }

            TemplateIrNodeKind::LoopControl { .. } => {
                summary.record_control_flow();
            }

            TemplateIrNodeKind::RuntimeSlotSite { .. } => {
                summary.record_runtime_slot_site();
            }

            TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {
                summary.record_runtime_slot_contribution_source();
            }

            TemplateIrNodeKind::AggregateOutput => {}
        }

        visiting_nodes.remove(&node_id);
    }

    Ok(())
}

fn validate_child_template(
    store: &TemplateIrStore,
    child_template_id: TemplateIrId,
    depth: usize,
    visiting_templates: &mut HashSet<TemplateIrId>,
    completed_templates: &mut HashSet<TemplateIrId>,
) -> Result<(), CompilerError> {
    let Some(child_template) = store.get_template(child_template_id) else {
        return Err(CompilerError::compiler_error(format!(
            "TIR summary referenced missing child template {child_template_id}."
        )));
    };

    if store.get_node(child_template.root).is_none() {
        return Err(CompilerError::compiler_error(format!(
            "TIR summary found child template {child_template_id} with missing root node {}.",
            child_template.root
        )));
    }

    if completed_templates.contains(&child_template_id) {
        return Ok(());
    }

    if !visiting_templates.insert(child_template_id) {
        return Err(CompilerError::compiler_error(format!(
            "TIR summary encountered a template cycle at {child_template_id:?}"
        )));
    }

    let child_root = child_template.root;
    let mut child_nodes = HashSet::new();
    accumulate_nodes(
        store,
        std::slice::from_ref(&child_root),
        depth,
        &mut TemplateIrSummary::empty(),
        &mut child_nodes,
        visiting_templates,
        completed_templates,
    )?;
    visiting_templates.remove(&child_template_id);
    completed_templates.insert(child_template_id);
    Ok(())
}

fn child_depth(depth: usize) -> Result<usize, CompilerError> {
    depth.checked_add(1).ok_or_else(|| {
        CompilerError::compiler_error("TIR summary depth overflowed usize; this is a compiler bug.")
    })
}
