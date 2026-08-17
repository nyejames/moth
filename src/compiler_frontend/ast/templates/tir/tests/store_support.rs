//! Test-only store conveniences and a malformed-state editor.
//!
//! Production mutation goes through checked store APIs. This module is a
//! test-only child of `store` so it can reach private vectors without widening
//! production visibility.

use super::{SlotPlanSlot, TemplateIrStore};
use crate::compiler_frontend::ast::templates::tir::ids::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::slot_plan::{
    TemplateSlotPlan, TemplateSlotSitePlan,
};

impl TemplateIrStore {
    pub(crate) fn wrapper_set_count(&self) -> usize {
        self.wrapper_sets.len()
    }

    pub(crate) fn slot_plan_index_count(&self) -> usize {
        self.slot_plans.len()
    }

    pub(crate) fn push_slot_plan(&mut self, slot_plan: TemplateSlotPlan) -> TemplateSlotPlanId {
        let id = self.reserve_slot_plan();
        self.commit_slot_plan(id, slot_plan)
            .expect("a just-reserved slot plan can be committed once");
        id
    }
}

/// Test-only editor for deliberately malformed store state.
#[derive(Debug)]
pub(crate) struct MalformedTirStore<'a> {
    store: &'a mut TemplateIrStore,
}

impl<'a> MalformedTirStore<'a> {
    pub(crate) fn new(store: &'a mut TemplateIrStore) -> Self {
        Self { store }
    }

    pub(crate) fn set_runtime_slot_plan(
        &mut self,
        id: TemplateIrId,
        plan: Option<TemplateSlotPlanId>,
    ) {
        let Some(template) = self.store.templates.get_mut(id.index()) else {
            panic!("MalformedTirStore: missing template {id}");
        };
        template.runtime_slot_plan = plan;
    }

    pub(crate) fn set_conditional_child_wrapper_set(
        &mut self,
        id: TemplateIrId,
        wrapper_set: Option<TemplateWrapperSetId>,
    ) {
        let Some(template) = self.store.templates.get_mut(id.index()) else {
            panic!("MalformedTirStore: missing template {id}");
        };
        template.conditional_child_wrapper_set = wrapper_set;
    }

    pub(crate) fn replace_contribution_sources(
        &mut self,
        id: TemplateSlotPlanId,
        contribution_sources: Vec<
            crate::compiler_frontend::ast::templates::tir::slot_plan::TemplateSlotContributionSourcePlan,
        >,
    ) {
        match self.store.slot_plans.get_mut(id.index()) {
            Some(SlotPlanSlot::Committed(plan)) => plan.contribution_sources = contribution_sources,
            Some(SlotPlanSlot::Reserved) => {
                panic!("MalformedTirStore: slot plan {id} is still reserved")
            }
            None => panic!("MalformedTirStore: missing slot plan {id}"),
        }
    }

    pub(crate) fn replace_slot_sites(
        &mut self,
        id: TemplateSlotPlanId,
        slot_sites: Vec<TemplateSlotSitePlan>,
    ) {
        match self.store.slot_plans.get_mut(id.index()) {
            Some(SlotPlanSlot::Committed(plan)) => plan.slot_sites = slot_sites,
            Some(SlotPlanSlot::Reserved) => {
                panic!("MalformedTirStore: slot plan {id} is still reserved")
            }
            None => panic!("MalformedTirStore: missing slot plan {id}"),
        }
    }

    pub(crate) fn set_wrapper_reference_context(
        &mut self,
        wrapper_set_id: TemplateWrapperSetId,
        wrapper_index: usize,
        context: TemplateViewContext,
    ) {
        let Some(wrapper_set) = self.store.wrapper_sets.get_mut(wrapper_set_id.index()) else {
            panic!("MalformedTirStore: missing wrapper set {wrapper_set_id}");
        };
        let Some(wrapper) = wrapper_set.wrappers.get_mut(wrapper_index) else {
            panic!(
                "MalformedTirStore: wrapper set {wrapper_set_id} has no wrapper {wrapper_index}"
            );
        };
        wrapper.context = context;
    }

    pub(crate) fn mutate_wrapper(
        &mut self,
        wrapper_set_id: TemplateWrapperSetId,
        wrapper_index: usize,
        mutate: impl FnOnce(&mut TemplateWrapperReference),
    ) {
        let Some(wrapper_set) = self.store.wrapper_sets.get_mut(wrapper_set_id.index()) else {
            panic!("MalformedTirStore: missing wrapper set {wrapper_set_id}");
        };
        let Some(wrapper) = wrapper_set.wrappers.get_mut(wrapper_index) else {
            panic!(
                "MalformedTirStore: wrapper set {wrapper_set_id} has no wrapper {wrapper_index}"
            );
        };
        mutate(wrapper);
    }

    pub(crate) fn set_template_root(&mut self, id: TemplateIrId, root: TemplateIrNodeId) {
        let Some(template) = self.store.templates.get_mut(id.index()) else {
            panic!("MalformedTirStore: missing template {id}");
        };
        template.root = root;
    }

    pub(crate) fn set_node_kind(
        &mut self,
        node_id: TemplateIrNodeId,
        kind: crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind,
    ) {
        let Some(node) = self.store.nodes.get_mut(node_id.index()) else {
            panic!("MalformedTirStore: missing node {node_id}");
        };
        node.kind = kind;
    }

    pub(crate) fn truncate_reactive_side_table(&mut self) {
        self.store.node_reactive_subscriptions.pop();
    }

    pub(crate) fn clear_expression_overlays(&mut self) {
        self.store.expression_overlays.clear();
    }
}
