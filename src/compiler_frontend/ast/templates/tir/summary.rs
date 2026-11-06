//! Cheap TIR shape and capacity metadata.
//!
//! `TemplateIrSummary` stores counts and flags that have measured callers:
//! output capacity, node counts, depth and structural feature presence.
//! It is not semantic proof. Preparation owns constness, unresolved slots
//! and helper classification.
//!
//! Unresolved structural slots (`slot_count`) are separate from runtime slot
//! sites (`runtime_slot_site_count`). `has_slots()` is derived from `slot_count`.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use rustc_hash::FxHashSet;

use crate::compiler_frontend::ast::templates::tir::ids::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;

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

    /// Root-sequence nodes recorded while parsing the authored head, including
    /// head-position insert contributions. The first `head_node_count` root
    /// children belong to the authored head; every later child is non-head.
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

    pub(crate) fn record_dynamic_expression(&mut self) {
        self.dynamic_expression_count += 1;
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

    /// Adds the shape facts of one fill root inserted into a template graph.
    ///
    /// WHAT: sums the additive counts and output estimate, admits the fill's own
    ///       unresolved slots and records control flow. `max_depth`, `head_node_count`
    ///       and `wrapper_count` stay out: depth depends on where the fill is
    ///       inserted, and the other two remain source-template facts.
    /// WHY: runtime wrapper publication versions an already summarized source
    ///      around a locally rebuilt fill path. Folding the fill's facts into the
    ///      source summary avoids re-walking the unchanged prefix and suffix
    ///      edges for every child.
    pub(crate) fn add_fill_facts(&mut self, fill: &TemplateIrSummary) {
        self.estimated_output_bytes += fill.estimated_output_bytes;
        self.text_node_count += fill.text_node_count;
        self.text_byte_count += fill.text_byte_count;
        self.dynamic_expression_count += fill.dynamic_expression_count;
        self.child_template_count += fill.child_template_count;
        self.slot_count += fill.slot_count;
        self.runtime_slot_site_count += fill.runtime_slot_site_count;
        self.runtime_slot_contribution_source_count += fill.runtime_slot_contribution_source_count;
        self.insert_contribution_count += fill.insert_contribution_count;
        self.has_control_flow |= fill.has_control_flow;
    }

    /// Applies one fill pass's slot replacements to this source summary.
    ///
    /// WHAT: removes every source slot the pass replaced or resolved to empty,
    ///       folds in the combined fill facts, and widens depth only when a fill
    ///       reaches deeper than the source's own graph.
    /// WHY: a prepared wrapper's unchanged edges are already validated and
    ///      summarized. Recomputing the summary from the rebuilt root would walk
    ///      them again for every child and make publication scale with the whole
    ///      decorator instead of the changed fill path.
    pub(crate) fn apply_fill_replacements(
        &mut self,
        removed_slots: u32,
        fill_facts: &TemplateIrSummary,
        introduced_depth: usize,
    ) -> Result<(), InfrastructureFailure> {
        self.slot_count = self.slot_count.checked_sub(removed_slots).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR summary replacement removed more slots than the source template summary records.",
            )
        })?;
        self.add_fill_facts(fill_facts);
        self.max_depth = self.max_depth.max(introduced_depth);
        Ok(())
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
) -> Result<TemplateIrSummary, InfrastructureFailure> {
    let mut summary = TemplateIrSummary::empty();
    let mut visiting_nodes = Vec::new();
    let mut visiting_templates = FxHashSet::default();
    let mut completed_templates = FxHashSet::default();
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
) -> Result<TemplateIrSummary, InfrastructureFailure> {
    let plan_roots = {
        let slot_plan = store.get_slot_plan(plan).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("TIR summary cannot read missing or uncommitted slot plan {plan}."),
            )
        })?;
        slot_plan
            .contribution_sources
            .iter()
            .map(|source| source.render_root)
            .chain(slot_plan.slot_sites.iter().map(|site| site.render_root))
            .collect::<Vec<_>>()
    };

    let mut summary = TemplateIrSummary::empty();
    let mut visiting_templates = FxHashSet::default();
    let mut completed_templates = FxHashSet::default();
    let mut visiting_nodes = Vec::new();
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
        let mut visiting_nodes = Vec::new();
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

// `visiting_nodes` is the active DFS path, so it is a Vec used as a stack: entry pushes, normal
// exit pops. Errors return through `?` without reaching the pop, which is safe because an error
// aborts the whole summary. The path holds only the nested sequence, conditional and loop bodies
// above this node: construction flattens body sequences and turns nested templates into child
// templates, which start a fresh path. A linear `contains` over that short path avoids a hash
// probe per node; a pathologically deep path would already be bounded by this walk's own
// recursion depth. The template sets stay hashed; none of these sets is iterated, so hash order
// never reaches the summary.
fn accumulate_nodes(
    store: &TemplateIrStore,
    node_ids: &[TemplateIrNodeId],
    depth: usize,
    summary: &mut TemplateIrSummary,
    visiting_nodes: &mut Vec<TemplateIrNodeId>,
    visiting_templates: &mut FxHashSet<TemplateIrId>,
    completed_templates: &mut FxHashSet<TemplateIrId>,
) -> Result<(), InfrastructureFailure> {
    for &node_id in node_ids {
        if visiting_nodes.contains(&node_id) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("TIR summary encountered a node cycle at {node_id:?}"),
            ));
        }
        visiting_nodes.push(node_id);

        let Some(node) = store.get_node(node_id) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("TIR summary requested missing node {node_id}."),
            ));
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
            }

            TemplateIrNodeKind::DynamicExpression { .. } => summary.record_dynamic_expression(),

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

            TemplateIrNodeKind::Conditional { body, .. } => {
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

            TemplateIrNodeKind::RuntimeSlotSite { .. } => {
                summary.record_runtime_slot_site();
            }

            TemplateIrNodeKind::RuntimeSlotContributionSource { control_flow, .. } => {
                summary.record_runtime_slot_contribution_source();
                // The marker replaces a control-flow contribution, so summaries of
                // wrapper templates containing it must report conditional emission
                // exactly as the authored root would.
                if *control_flow {
                    summary.record_control_flow();
                }
            }

            TemplateIrNodeKind::AggregateOutput => {}

            // A retained position completes into the shape its role names: an expression splice
            // counts as one dynamic expression, a retained loop condition as one control-flow node.
            TemplateIrNodeKind::RetainedNumericIntake(role) => {
                if role.completes_into_control_flow() {
                    summary.record_control_flow();
                } else {
                    summary.record_dynamic_expression();
                }
            }
        }

        visiting_nodes.pop();
    }

    Ok(())
}

fn validate_child_template(
    store: &TemplateIrStore,
    child_template_id: TemplateIrId,
    depth: usize,
    visiting_templates: &mut FxHashSet<TemplateIrId>,
    completed_templates: &mut FxHashSet<TemplateIrId>,
) -> Result<(), InfrastructureFailure> {
    let Some(child_template) = store.get_template(child_template_id) else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("TIR summary referenced missing child template {child_template_id}."),
        ));
    };

    if store.get_node(child_template.root).is_none() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "TIR summary found child template {child_template_id} with missing root node {}.",
                child_template.root
            ),
        ));
    }

    if completed_templates.contains(&child_template_id) {
        return Ok(());
    }

    if !visiting_templates.insert(child_template_id) {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("TIR summary encountered a template cycle at {child_template_id:?}"),
        ));
    }

    let child_root = child_template.root;
    let mut child_nodes = Vec::new();
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

fn child_depth(depth: usize) -> Result<usize, InfrastructureFailure> {
    depth.checked_add(1).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "TIR summary depth overflowed usize; this is a compiler bug.",
        )
    })
}
