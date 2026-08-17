//! Central TIR storage.
//!
//! `TemplateIrStore` owns every TIR template, node, wrapper set, overlay and
//! slot-plan entry in contiguous private vectors. Consumers obtain cheap `Copy`
//! IDs and use checked lookup or named mutation APIs.
//!
//! The store is AST-local. It is not shared with HIR, backends or the public
//! API. Each module AST construction may create its own store; the store is
//! dropped when the AST stage finishes template processing for that module.
//!
//! Low-level `push_template` and `push_node` append records. They do not prove
//! that every stored ID is reachable or that the reachable graph is well
//! formed. Preparation remains the exhaustive authority validator.
//!
//! Concept-specific mutation lives in sibling modules:
//! `control_flow`, `slot_plans` and `overlays`.

mod control_flow;
mod overlays;
mod slot_plans;

#[cfg(test)]
#[path = "tests/store_support.rs"]
mod store_support;

use crate::compiler_frontend::arena::capacity::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotPlaceholder, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
    TemplateSlotPlanId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
    TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirExpressionOverlay, TirSlotResolutionOverlay, TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::summary::{
    TemplateIrSummary, summarize_existing_root, summarize_runtime_slot_representation,
};
use crate::compiler_frontend::ast::templates::tir::wrapper_sets::wrapper_sets_are_equivalent;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

pub(crate) use control_flow::ControlFlowBodyKind;
#[cfg(test)]
pub(crate) use store_support::MalformedTirStore;

use slot_plans::SlotPlanSlot;

// -------------------------
//  Side-table types
// -------------------------

/// A reusable set of `$children(..)` wrapper template refs.
///
/// Wrapper sets store effective wrapper references (root, phase and
/// value-carried context). A wrapper's effective identity is not only its
/// structural root.
#[derive(Clone, Debug)]
pub(crate) struct TemplateWrapperSet {
    /// Effective wrapper refs, innermost to outermost as stored on the AST
    /// `Template`.
    pub(crate) wrappers: Vec<TemplateWrapperReference>,
}

// -------------------------
//  Template IR Store
// -------------------------

/// Central owned storage for all TIR data within one module's template subsystem.
///
/// Invariants that checked APIs maintain:
/// - every issued ID indexes an entry in its collection
/// - `get_slot_plan` returns only committed plans
/// - derived publication copies complete source identity
///
/// Append APIs do not prove that the reachable graph is complete. Preparation
/// is the exhaustive validator for reachable authority.
#[derive(Debug)]
pub(crate) struct TemplateIrStore {
    next_slot_occurrence: u32,
    next_child_template_occurrence: u32,
    next_expression_site: u32,

    templates: Vec<TemplateIr>,
    nodes: Vec<TemplateIrNode>,
    wrapper_sets: Vec<TemplateWrapperSet>,
    slot_plans: Vec<SlotPlanSlot>,
    expression_overlays: Vec<TirExpressionOverlay>,
    slot_resolution_overlays: Vec<TirSlotResolutionOverlay>,
    wrapper_context_overlays: Vec<TirWrapperContextOverlay>,

    /// Reactive `$(source)` subscription metadata attached to text nodes.
    /// Indexed by `TemplateIrNodeId`; `None` means the node carries no
    /// reactive dependency.
    node_reactive_subscriptions: Vec<Option<ReactiveSubscription>>,
}

impl TemplateIrStore {
    pub(crate) fn new() -> Self {
        Self {
            next_slot_occurrence: 0,
            next_child_template_occurrence: 0,
            next_expression_site: 0,
            templates: Vec::new(),
            nodes: Vec::new(),
            wrapper_sets: Vec::new(),
            slot_plans: Vec::new(),
            expression_overlays: Vec::new(),
            slot_resolution_overlays: Vec::new(),
            wrapper_context_overlays: Vec::new(),
            node_reactive_subscriptions: Vec::new(),
        }
    }

    /// Creates a store pre-sized from a module-level capacity estimate.
    ///
    /// The estimate is policy-only and does not affect correctness.
    pub(crate) fn with_capacity_estimate(estimate: FrontendArenaCapacityEstimate) -> Self {
        let template_capacity = estimate.templates;
        let node_capacity = estimate.template_atoms;
        let side_capacity = template_capacity;

        Self {
            next_slot_occurrence: 0,
            next_child_template_occurrence: 0,
            next_expression_site: 0,
            templates: Vec::with_capacity(template_capacity),
            nodes: Vec::with_capacity(node_capacity),
            wrapper_sets: Vec::with_capacity(side_capacity),
            slot_plans: Vec::with_capacity(side_capacity),
            expression_overlays: Vec::with_capacity(side_capacity),
            slot_resolution_overlays: Vec::with_capacity(side_capacity),
            wrapper_context_overlays: Vec::with_capacity(side_capacity),
            node_reactive_subscriptions: Vec::with_capacity(node_capacity),
        }
    }

    pub(crate) fn next_slot_occurrence_id(&mut self) -> SlotOccurrenceId {
        let id = SlotOccurrenceId::new(self.next_slot_occurrence as usize);
        self.next_slot_occurrence = self
            .next_slot_occurrence
            .checked_add(1)
            .expect("slot occurrence counter overflow; this is a compiler bug");
        id
    }

    pub(crate) fn next_child_template_occurrence_id(&mut self) -> ChildTemplateOccurrenceId {
        let id = ChildTemplateOccurrenceId::new(self.next_child_template_occurrence as usize);
        self.next_child_template_occurrence = self
            .next_child_template_occurrence
            .checked_add(1)
            .expect("child-template occurrence counter overflow; this is a compiler bug");
        id
    }

    pub(crate) fn next_expression_site_id(&mut self) -> ExpressionSiteId {
        let id = ExpressionSiteId::new(self.next_expression_site as usize);
        self.next_expression_site = self
            .next_expression_site
            .checked_add(1)
            .expect("expression site counter overflow; this is a compiler bug");
        id
    }

    /// Allocates expression-site IDs for every expression-bearing position in a
    /// loop header, drawing from the same document-order counter as
    /// `DynamicExpression` and branch-selector sites.
    pub(crate) fn allocate_loop_header_expression_sites(
        &mut self,
        header: &TemplateLoopHeader,
    ) -> TemplateLoopHeaderExpressionSites {
        match header {
            TemplateLoopHeader::Conditional { .. } => {
                TemplateLoopHeaderExpressionSites::Conditional {
                    condition: self.next_expression_site_id(),
                }
            }
            TemplateLoopHeader::Range { range, .. } => TemplateLoopHeaderExpressionSites::Range {
                start: self.next_expression_site_id(),
                end: self.next_expression_site_id(),
                step: range.step.as_ref().map(|_| self.next_expression_site_id()),
            },
            TemplateLoopHeader::Collection { .. } => {
                TemplateLoopHeaderExpressionSites::Collection {
                    iterable: self.next_expression_site_id(),
                }
            }
        }
    }

    pub(crate) fn template_count(&self) -> usize {
        self.templates.len()
    }

    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn allocated_child_template_occurrence_count(&self) -> usize {
        self.next_child_template_occurrence as usize
    }

    fn allocated_expression_site_count(&self) -> usize {
        self.next_expression_site as usize
    }

    /// Appends a newly constructed template. Does not prove reachable-graph
    /// completeness; derived versions must use a named publication path.
    pub(crate) fn push_template(&mut self, template: TemplateIr) -> TemplateIrId {
        let id = TemplateIrId::new(self.templates.len());
        self.templates.push(template);
        id
    }

    /// Versions an existing store-owned template around a new structural root.
    ///
    /// Recomputes structural summary facts from `new_root`. Style, kind,
    /// location, wrapper set and committed runtime-plan identity stay with the
    /// source. Head-node and wrapper counts are preserved or replaced only
    /// through [`DerivedTemplateMetadata`].
    pub(crate) fn push_structurally_derived_template(
        &mut self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
        metadata: DerivedTemplateMetadata,
    ) -> Result<TemplateIrId, CompilerError> {
        self.validate_derived_publication(source, new_root)?;
        let summary = summarize_existing_root(self, new_root)?;
        let derived = self.derived_template(source, new_root, summary, metadata)?;
        Ok(self.push_template(derived))
    }

    /// Versions an existing template as a runtime-slot application.
    ///
    /// Computes one summary from the completed wrapper root and the committed
    /// plan's source and site trees, then publishes the derived template with
    /// that plan attached.
    pub(crate) fn push_runtime_slot_derived_template(
        &mut self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
        plan: TemplateSlotPlanId,
        metadata: DerivedTemplateMetadata,
    ) -> Result<TemplateIrId, CompilerError> {
        self.validate_derived_publication(source, new_root)?;
        if self.get_slot_plan(plan).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot publish runtime-slot template {source} with missing or uncommitted slot plan {plan}."
            )));
        }

        let summary = summarize_runtime_slot_representation(self, new_root, plan)?;
        let mut derived = self.derived_template(source, new_root, summary, metadata)?;
        derived.runtime_slot_plan = None;
        let template_id = self.push_template(derived);
        self.attach_runtime_slot_plan(template_id, plan)?;
        Ok(template_id)
    }

    fn validate_derived_publication(
        &self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
    ) -> Result<(), CompilerError> {
        let Some(source_template) = self.get_template(source) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot derive from unknown source template {source}."
            )));
        };

        if self.get_node(new_root).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot version template {source} around missing root node {new_root}."
            )));
        }

        if let Some(plan) = source_template.runtime_slot_plan
            && self.get_slot_plan(plan).is_none()
        {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot preserve missing or uncommitted slot plan {plan} while deriving template {source}."
            )));
        }

        Ok(())
    }

    fn derived_template(
        &self,
        source: TemplateIrId,
        new_root: TemplateIrNodeId,
        mut summary: TemplateIrSummary,
        metadata: DerivedTemplateMetadata,
    ) -> Result<TemplateIr, CompilerError> {
        let source_template = self.get_template(source).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR store cannot derive from unknown source template {source}."
            ))
        })?;
        apply_derived_metadata(&mut summary, source_template, metadata);

        Ok(TemplateIr {
            root: new_root,
            style: source_template.style.clone(),
            kind: source_template.kind.clone(),
            summary,
            location: source_template.location.clone(),
            conditional_child_wrapper_set: source_template.conditional_child_wrapper_set,
            runtime_slot_plan: source_template.runtime_slot_plan,
        })
    }

    pub(crate) fn set_template_kind(
        &mut self,
        id: TemplateIrId,
        kind: TemplateType,
    ) -> Result<(), CompilerError> {
        let template = self.template_mut(id)?;
        template.kind = kind;
        Ok(())
    }

    pub(crate) fn attach_runtime_slot_plan(
        &mut self,
        id: TemplateIrId,
        plan: TemplateSlotPlanId,
    ) -> Result<(), CompilerError> {
        if self.get_slot_plan(plan).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot attach missing or uncommitted slot plan {plan} to template {id}."
            )));
        }

        let template = self.template_mut(id)?;
        template.runtime_slot_plan = Some(plan);
        Ok(())
    }

    pub(crate) fn set_conditional_child_wrapper_set(
        &mut self,
        id: TemplateIrId,
        wrapper_set: TemplateWrapperSetId,
    ) -> Result<(), CompilerError> {
        if self.get_wrapper_set(wrapper_set).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot attach missing wrapper set {wrapper_set} to template {id}."
            )));
        }

        let template = self.template_mut(id)?;
        template.conditional_child_wrapper_set = Some(wrapper_set);
        Ok(())
    }

    pub(crate) fn push_node(&mut self, node: TemplateIrNode) -> TemplateIrNodeId {
        let id = TemplateIrNodeId::new(self.nodes.len());
        self.nodes.push(node);
        self.node_reactive_subscriptions.push(None);
        id
    }

    /// Reads aligned text-node subscription metadata without hiding store corruption.
    pub(crate) fn node_reactive_subscription(
        &self,
        node_id: TemplateIrNodeId,
    ) -> Result<Option<&ReactiveSubscription>, CompilerError> {
        if self.get_node(node_id).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TIR store reactive subscription lookup referenced missing node {node_id}."
            )));
        }

        self.node_reactive_subscriptions
            .get(node_id.index())
            .map(|subscription| subscription.as_ref())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR store reactive side table is missing node {node_id}."
                ))
            })
    }

    /// Attaches a reactive subscription to an existing text node.
    pub(crate) fn set_node_reactive_subscription(
        &mut self,
        node_id: TemplateIrNodeId,
        subscription: ReactiveSubscription,
    ) -> Result<(), CompilerError> {
        let Some(node) = self.get_node(node_id) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot attach a reactive subscription to missing node {node_id}."
            )));
        };

        if !matches!(node.kind, TemplateIrNodeKind::Text { .. }) {
            return Err(CompilerError::compiler_error(format!(
                "TIR store cannot attach a reactive subscription to non-text node {node_id}."
            )));
        }

        let Some(entry) = self.node_reactive_subscriptions.get_mut(node_id.index()) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR store reactive side table is missing node {node_id}."
            )));
        };

        *entry = Some(subscription);
        Ok(())
    }

    pub(crate) fn push_wrapper_set(
        &mut self,
        wrapper_set: TemplateWrapperSet,
    ) -> TemplateWrapperSetId {
        let id = TemplateWrapperSetId::new(self.wrapper_sets.len());
        self.wrapper_sets.push(wrapper_set);
        id
    }

    pub(crate) fn push_or_reuse_wrapper_set(
        &mut self,
        wrappers: Vec<TemplateWrapperReference>,
    ) -> TemplateWrapperSetId {
        for (index, existing) in self.wrapper_sets.iter().enumerate() {
            if wrapper_sets_are_equivalent(&existing.wrappers, &wrappers) {
                increment_ast_counter(AstCounter::TirWrapperSetReuseHits);
                return TemplateWrapperSetId::new(index);
            }
        }

        increment_ast_counter(AstCounter::TirWrapperSetsCreated);
        self.push_wrapper_set(TemplateWrapperSet { wrappers })
    }

    fn push_or_reuse_optional_wrapper_set(
        &mut self,
        wrappers: &[TemplateWrapperReference],
    ) -> Option<TemplateWrapperSetId> {
        if wrappers.is_empty() {
            return None;
        }

        Some(self.push_or_reuse_wrapper_set(wrappers.to_vec()))
    }

    pub(crate) fn tir_slot_placeholder_from_ast(
        &mut self,
        placeholder: &SlotPlaceholder,
        location: SourceLocation,
    ) -> Result<TirSlotPlaceholder, TemplateError> {
        let occurrence_id = self.next_slot_occurrence_id();
        let applied_child_wrapper_set =
            self.push_or_reuse_optional_wrapper_set(&placeholder.applied_child_wrappers);
        let child_wrapper_set =
            self.push_or_reuse_optional_wrapper_set(&placeholder.child_wrappers);

        Ok(TirSlotPlaceholder::with_wrapper_sets(
            placeholder.key.to_owned(),
            occurrence_id,
            location,
            applied_child_wrapper_set,
            child_wrapper_set,
            placeholder.skip_parent_child_wrappers,
        ))
    }

    pub(crate) fn get_template(&self, id: TemplateIrId) -> Option<&TemplateIr> {
        self.templates.get(id.index())
    }

    pub(super) fn template_mut(
        &mut self,
        id: TemplateIrId,
    ) -> Result<&mut TemplateIr, CompilerError> {
        self.templates.get_mut(id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!("TIR store has no template {id}."))
        })
    }

    pub(crate) fn get_node(&self, id: TemplateIrNodeId) -> Option<&TemplateIrNode> {
        self.nodes.get(id.index())
    }

    pub(crate) fn replace_child_template_reference(
        &mut self,
        node_id: TemplateIrNodeId,
        template_id: TemplateIrId,
    ) -> Result<(), CompilerError> {
        let node = self.node_mut(node_id)?;
        let TemplateIrNodeKind::ChildTemplate { reference, .. } = &mut node.kind else {
            return Err(CompilerError::compiler_error(
                "TIR store cannot replace a non-child-template reference.",
            ));
        };
        reference.root = template_id;
        Ok(())
    }

    pub(super) fn node_mut(
        &mut self,
        id: TemplateIrNodeId,
    ) -> Result<&mut TemplateIrNode, CompilerError> {
        self.nodes
            .get_mut(id.index())
            .ok_or_else(|| CompilerError::compiler_error(format!("TIR store has no node {id}.")))
    }

    pub(crate) fn get_wrapper_set(&self, id: TemplateWrapperSetId) -> Option<&TemplateWrapperSet> {
        self.wrapper_sets.get(id.index())
    }

    /// Looks up the unique module-local slot placeholder for an occurrence ID.
    ///
    /// Public const-template projection uses this lookup to resolve the
    /// donor-local slot metadata before the TIR store is dropped.
    pub(crate) fn slot_placeholder(
        &self,
        occurrence: SlotOccurrenceId,
    ) -> Option<&TirSlotPlaceholder> {
        self.nodes.iter().find_map(|node| match &node.kind {
            TemplateIrNodeKind::Slot { placeholder } if placeholder.occurrence_id == occurrence => {
                Some(placeholder)
            }
            _ => None,
        })
    }
}

impl Default for TemplateIrStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Non-structural facts that a derived publication may preserve or replace.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DerivedTemplateMetadata {
    pub(crate) head_node_count: DerivedCount,
    pub(crate) wrapper_count: DerivedCount,
}

impl DerivedTemplateMetadata {
    pub(crate) fn preserve_source() -> Self {
        Self {
            head_node_count: DerivedCount::PreserveSource,
            wrapper_count: DerivedCount::PreserveSource,
        }
    }
}

/// Whether a non-structural count should stay with the source or be replaced.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DerivedCount {
    PreserveSource,
    Replace(u32),
}

fn apply_derived_metadata(
    summary: &mut TemplateIrSummary,
    source: &TemplateIr,
    metadata: DerivedTemplateMetadata,
) {
    summary.head_node_count = match metadata.head_node_count {
        DerivedCount::PreserveSource => source.summary.head_node_count,
        DerivedCount::Replace(count) => count,
    };
    summary.wrapper_count = match metadata.wrapper_count {
        DerivedCount::PreserveSource => source.summary.wrapper_count,
        DerivedCount::Replace(count) => count,
    };
}
