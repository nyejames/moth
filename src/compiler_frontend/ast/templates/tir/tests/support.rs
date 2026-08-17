//! Test-only TIR fixture constructors and inspection helpers.
//!
//! Production TIR files keep parser and reducer APIs. Fixture constructors,
//! view location lookups and mutation walkers live here so test builds do not
//! grow extra semantic states or convenience methods on the compiler path.

use super::super::slot_composition::child_wrappers::wrap_tir_node_in_wrappers_into;
use super::super::slot_composition::schema::expand_tir_slot_placeholders_into;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId,
    TemplateSlotPlanId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNode, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites, TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirWrapperApplicationMode, TirWrapperContext,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::slot_plan::runtime_slot_plan_roots;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::TirView;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use std::collections::HashSet;

impl TirSlotPlaceholder {
    pub(crate) fn new(
        key: SlotKey,
        occurrence_id: SlotOccurrenceId,
        location: SourceLocation,
    ) -> Self {
        Self::with_wrapper_sets(key, occurrence_id, location, None, None, false)
    }
}

impl TirWrapperContext {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn inherited(wrapper_set: TemplateWrapperSetId) -> Self {
        Self {
            inherited_wrapper_set: Some(wrapper_set),
            skip_parent_child_wrappers: false,
            application_mode: TirWrapperApplicationMode::Always,
        }
    }
}

// -------------------------
//  View inspection
// -------------------------

impl<'a> TirView<'a> {
    pub(crate) fn root_node(&self) -> Result<&'a TemplateIrNode, CompilerError> {
        let root_node_id = self.root_template()?.root;
        self.effective_node(root_node_id)
    }

    pub(crate) fn effective_expression_for_node(
        &self,
        node_ref: TemplateIrNodeId,
    ) -> Result<Option<&'a Expression>, CompilerError> {
        let site_id = {
            let node = self.effective_node(node_ref)?;
            match &node.kind {
                TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
                _ => return Ok(None),
            }
        };

        self.effective_expression_for_site(site_id)
    }

    pub(crate) fn source_location_for_slot_occurrence(
        &self,
        occurrence_id: SlotOccurrenceId,
    ) -> Result<Option<SourceLocation>, CompilerError> {
        let root_node_ref = self.root_template()?.root;
        find_location_in_subtree(self, root_node_ref, &|kind, location| match kind {
            TemplateIrNodeKind::Slot { placeholder }
                if placeholder.occurrence_id == occurrence_id =>
            {
                Some(location.clone())
            }
            _ => None,
        })
    }

    pub(crate) fn source_location_for_child_template_occurrence(
        &self,
        occurrence_id: ChildTemplateOccurrenceId,
    ) -> Result<Option<SourceLocation>, CompilerError> {
        let root_node_ref = self.root_template()?.root;
        find_location_in_subtree(self, root_node_ref, &|kind, location| match kind {
            TemplateIrNodeKind::ChildTemplate {
                occurrence_id: child_id,
                ..
            } if *child_id == occurrence_id => Some(location.clone()),
            _ => None,
        })
    }

    pub(crate) fn source_location_for_expression_site(
        &self,
        site_id: ExpressionSiteId,
    ) -> Result<Option<SourceLocation>, CompilerError> {
        let root_node_ref = self.root_template()?.root;
        find_location_in_subtree(self, root_node_ref, &|kind, location| match kind {
            TemplateIrNodeKind::DynamicExpression {
                site_id: expr_site_id,
                ..
            } if *expr_site_id == site_id => Some(location.clone()),

            TemplateIrNodeKind::BranchChain { branches, .. } => branches
                .iter()
                .find(|branch| branch.selector_site_id == site_id)
                .map(|branch| branch.location.clone()),

            TemplateIrNodeKind::Loop { header_sites, .. }
                if expression_site_in_header(header_sites, site_id) =>
            {
                Some(location.clone())
            }

            _ => None,
        })
    }
}

fn find_location_in_subtree(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    matches: &impl Fn(&TemplateIrNodeKind, &SourceLocation) -> Option<SourceLocation>,
) -> Result<Option<SourceLocation>, CompilerError> {
    let (found, children) = {
        let node = view.effective_node(node_ref)?;
        let found = matches(&node.kind, &node.location);
        let children = child_node_ids(&node.kind);
        (found, children)
    };

    if let Some(location) = found {
        return Ok(Some(location));
    }

    for child_node_id in children {
        if let Some(location) = find_location_in_subtree(view, child_node_id, matches)? {
            return Ok(Some(location));
        }
    }

    Ok(None)
}

fn child_node_ids(kind: &TemplateIrNodeKind) -> Vec<TemplateIrNodeId> {
    match kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let mut ids: Vec<TemplateIrNodeId> =
                branches.iter().map(|branch| branch.body).collect();
            if let Some(fallback) = fallback {
                ids.push(*fallback);
            }
            ids
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let mut ids = vec![*body];
            if let Some(aggregate) = aggregate_wrapper {
                ids.push(*aggregate);
            }
            ids
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::ChildTemplate { .. }
        | TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::InsertContribution { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Vec::new(),
    }
}

fn expression_site_in_header(
    header_sites: &TemplateLoopHeaderExpressionSites,
    site_id: ExpressionSiteId,
) -> bool {
    match header_sites {
        TemplateLoopHeaderExpressionSites::Conditional { condition } => *condition == site_id,
        TemplateLoopHeaderExpressionSites::Range { start, end, step } => {
            *start == site_id || *end == site_id || *step == Some(site_id)
        }
        TemplateLoopHeaderExpressionSites::Collection { iterable } => *iterable == site_id,
    }
}

// -------------------------
//  Slot composition fixtures
// -------------------------

pub(crate) fn expand_tir_slot_placeholders(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    routed_contributions: &super::super::slot_composition::TirSlotContributions,
    string_table: &crate::compiler_frontend::symbols::string_interning::StringTable,
) -> Result<TemplateIrNodeId, TemplateError> {
    expand_tir_slot_placeholders_into(
        store,
        wrapper_template_id,
        routed_contributions,
        string_table,
    )
}

pub(crate) fn wrap_tir_node_in_wrappers(
    store: &mut TemplateIrStore,
    child_node_id: TemplateIrNodeId,
    wrapper_template_ids: &[TemplateIrId],
    string_table: &crate::compiler_frontend::symbols::string_interning::StringTable,
) -> Result<TemplateIrNodeId, TemplateError> {
    let wrapper_references = wrapper_template_ids
        .iter()
        .map(|template_id| {
            TemplateWrapperReference::new(
                *template_id,
                super::super::view::TemplateTirPhase::Parsed,
                super::super::overlays::TemplateViewContext::default(),
            )
        })
        .collect::<Vec<_>>();
    wrap_tir_node_in_wrappers_into(store, child_node_id, &wrapper_references, string_table)
}

pub(crate) fn wrap_tir_node_in_wrapper_references(
    store: &mut TemplateIrStore,
    child_node_id: TemplateIrNodeId,
    wrapper_references: &[TemplateWrapperReference],
    string_table: &crate::compiler_frontend::symbols::string_interning::StringTable,
) -> Result<TemplateIrNodeId, TemplateError> {
    wrap_tir_node_in_wrappers_into(store, child_node_id, wrapper_references, string_table)
}

// -------------------------
//  Overlay fixtures
// -------------------------

// -------------------------
//  Expression mutation walker
// -------------------------

pub(crate) trait TirExpressionPayloadMutator {
    fn mutate_expression_payload(
        &mut self,
        expression: &mut Expression,
    ) -> Result<(), CompilerError>;
}

pub(crate) fn mutate_finalized_tir_body_root_expression_payloads<M>(
    store: &mut TemplateIrStore,
    root: TemplateIrNodeId,
    mutator: &mut M,
) -> Result<(), CompilerError>
where
    M: TirExpressionPayloadMutator,
{
    let mut walker = FinalizedBodyRootExpressionMutator::new(store, mutator);
    walker.walk_node(root)
}

struct FinalizedBodyRootExpressionMutator<'store, 'visitor, M>
where
    M: TirExpressionPayloadMutator,
{
    store: &'store mut TemplateIrStore,
    mutator: &'visitor mut M,
    active_nodes: HashSet<TemplateIrNodeId>,
    completed_nodes: HashSet<TemplateIrNodeId>,
    active_templates: HashSet<TemplateIrId>,
    completed_templates: HashSet<TemplateIrId>,
    active_slot_plans: HashSet<TemplateSlotPlanId>,
    completed_slot_plans: HashSet<TemplateSlotPlanId>,
}

enum TirExpressionWalkChild {
    Node(TemplateIrNodeId),
    Template(TemplateIrId),
}

impl<'store, 'visitor, M> FinalizedBodyRootExpressionMutator<'store, 'visitor, M>
where
    M: TirExpressionPayloadMutator,
{
    fn new(store: &'store mut TemplateIrStore, mutator: &'visitor mut M) -> Self {
        Self {
            store,
            mutator,
            active_nodes: HashSet::new(),
            completed_nodes: HashSet::new(),
            active_templates: HashSet::new(),
            completed_templates: HashSet::new(),
            active_slot_plans: HashSet::new(),
            completed_slot_plans: HashSet::new(),
        }
    }

    fn walk_template(&mut self, template_id: TemplateIrId) -> Result<(), CompilerError> {
        if self.completed_templates.contains(&template_id) {
            return Ok(());
        }

        if !self.active_templates.insert(template_id) {
            return Err(CompilerError::compiler_error(
                "TIR expression mutation found a recursive child-template reference.",
            ));
        }

        let (root, runtime_slot_plan) = {
            let template = self.store.get_template(template_id).ok_or_else(|| {
                CompilerError::compiler_error(
                    "TIR expression mutation referenced a missing child template.",
                )
            })?;
            (template.root, template.runtime_slot_plan)
        };

        let result = if let Some(slot_plan_id) = runtime_slot_plan {
            self.walk_runtime_slot_application(root, slot_plan_id)
        } else {
            self.walk_node(root)
        };

        self.active_templates.remove(&template_id);
        if result.is_ok() {
            self.completed_templates.insert(template_id);
        }
        result
    }

    fn walk_runtime_slot_application(
        &mut self,
        wrapper_root: TemplateIrNodeId,
        slot_plan_id: TemplateSlotPlanId,
    ) -> Result<(), CompilerError> {
        if self.completed_slot_plans.contains(&slot_plan_id) {
            return self.walk_node(wrapper_root);
        }

        if !self.active_slot_plans.insert(slot_plan_id) {
            return Err(CompilerError::compiler_error(
                "TIR expression mutation found a recursive runtime slot plan.",
            ));
        }

        let (contribution_roots, site_render_roots) =
            runtime_slot_plan_roots(self.store, slot_plan_id)?;

        let result = self.walk_node(wrapper_root).and_then(|()| {
            for root in contribution_roots {
                self.walk_node(root)?;
            }

            for root in site_render_roots {
                self.walk_node(root)?;
            }

            Ok(())
        });

        self.active_slot_plans.remove(&slot_plan_id);
        if result.is_ok() {
            self.completed_slot_plans.insert(slot_plan_id);
        }
        result
    }

    fn walk_node(&mut self, node_id: TemplateIrNodeId) -> Result<(), CompilerError> {
        if self.completed_nodes.contains(&node_id) {
            return Ok(());
        }

        if !self.active_nodes.insert(node_id) {
            return Err(CompilerError::compiler_error(
                "TIR expression mutation found a recursive node reference.",
            ));
        }

        let children = self.mutate_node_and_collect_children(node_id);
        let result = match children {
            Ok(children) => self.walk_children(children),
            Err(error) => Err(error),
        };

        self.active_nodes.remove(&node_id);
        if result.is_ok() {
            self.completed_nodes.insert(node_id);
        }
        result
    }

    fn validate_runtime_slot_site(
        &self,
        plan: TemplateSlotPlanId,
        site: RuntimeSlotSiteId,
    ) -> Result<Vec<TirExpressionWalkChild>, CompilerError> {
        let Some(slot_plan) = self.store.get_slot_plan(plan) else {
            return Err(CompilerError::compiler_error(
                "TIR expression mutation referenced a missing runtime slot plan.",
            ));
        };

        let Some(indexed_site) = slot_plan.slot_sites.get(site.0) else {
            return Err(CompilerError::compiler_error(
                "TIR expression mutation referenced a missing runtime slot site.",
            ));
        };

        if indexed_site.site != site {
            return Err(CompilerError::compiler_error(
                "TIR expression mutation found a runtime slot site index mismatch.",
            ));
        }

        Ok(Vec::new())
    }

    fn mutate_node_and_collect_children(
        &mut self,
        node_id: TemplateIrNodeId,
    ) -> Result<Vec<TirExpressionWalkChild>, CompilerError> {
        if let TemplateIrNodeKind::RuntimeSlotSite { plan, site } = &self
            .store
            .get_node(node_id)
            .ok_or_else(|| {
                CompilerError::compiler_error("TIR expression mutation referenced a missing node.")
            })?
            .kind
        {
            return self.validate_runtime_slot_site(*plan, *site);
        }

        let node = self.store.node_mut(node_id)?;

        match &mut node.kind {
            TemplateIrNodeKind::Sequence { children } => Ok(children
                .iter()
                .copied()
                .map(TirExpressionWalkChild::Node)
                .collect()),

            TemplateIrNodeKind::DynamicExpression { expression, .. } => {
                self.mutator.mutate_expression_payload(expression)?;
                Ok(Vec::new())
            }

            TemplateIrNodeKind::BranchChain { branches, fallback } => {
                let mut children =
                    Vec::with_capacity(branches.len() + usize::from(fallback.is_some()));
                for branch in branches.iter_mut() {
                    mutate_branch_selector_expression(&mut branch.selector, self.mutator)?;
                    children.push(TirExpressionWalkChild::Node(branch.body));
                }

                if let Some(fallback_id) = fallback {
                    children.push(TirExpressionWalkChild::Node(*fallback_id));
                }

                Ok(children)
            }

            TemplateIrNodeKind::Loop {
                header,
                body,
                aggregate_wrapper,
                ..
            } => {
                mutate_loop_header_expressions(header, self.mutator)?;

                let mut children = Vec::with_capacity(1 + usize::from(aggregate_wrapper.is_some()));
                children.push(TirExpressionWalkChild::Node(*body));

                if let Some(wrapper_id) = aggregate_wrapper {
                    children.push(TirExpressionWalkChild::Node(*wrapper_id));
                }

                Ok(children)
            }

            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                Ok(vec![TirExpressionWalkChild::Template(reference.root)])
            }

            TemplateIrNodeKind::InsertContribution { template } => {
                Ok(vec![TirExpressionWalkChild::Template(*template)])
            }

            TemplateIrNodeKind::RuntimeSlotSite { .. } => Ok(Vec::new()),

            TemplateIrNodeKind::Text { .. }
            | TemplateIrNodeKind::Slot { .. }
            | TemplateIrNodeKind::AggregateOutput
            | TemplateIrNodeKind::LoopControl { .. }
            | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(Vec::new()),
        }
    }

    fn walk_children(
        &mut self,
        children: Vec<TirExpressionWalkChild>,
    ) -> Result<(), CompilerError> {
        for child in children {
            match child {
                TirExpressionWalkChild::Node(node_id) => self.walk_node(node_id)?,
                TirExpressionWalkChild::Template(template_id) => self.walk_template(template_id)?,
            }
        }

        Ok(())
    }
}

fn mutate_branch_selector_expression<M>(
    selector: &mut TemplateBranchSelector,
    mutator: &mut M,
) -> Result<(), CompilerError>
where
    M: TirExpressionPayloadMutator,
{
    match selector {
        TemplateBranchSelector::Bool(condition) => mutator.mutate_expression_payload(condition),
        TemplateBranchSelector::OptionPresentCapture { scrutinee, .. } => {
            mutator.mutate_expression_payload(scrutinee)
        }
    }
}

fn mutate_loop_header_expressions<M>(
    header: &mut TemplateLoopHeader,
    mutator: &mut M,
) -> Result<(), CompilerError>
where
    M: TirExpressionPayloadMutator,
{
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            mutator.mutate_expression_payload(condition)
        }
        TemplateLoopHeader::Range { range, .. } => {
            mutator.mutate_expression_payload(&mut range.start)?;
            mutator.mutate_expression_payload(&mut range.end)?;
            if let Some(step) = &mut range.step {
                mutator.mutate_expression_payload(step)?;
            }
            Ok(())
        }
        TemplateLoopHeader::Collection { iterable, .. } => {
            mutator.mutate_expression_payload(iterable)
        }
    }
}
