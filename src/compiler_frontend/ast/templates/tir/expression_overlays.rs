//! TIR expression-overlay collection and precedence.
//!
//! WHAT: collects structural and effective expression payloads into the keyed
//!       overlay input used by AST finalization.
//! WHY: overlay normalization has different ownership from read-only
//!      expression-site traversal. This module owns its temporary authority
//!      layers, precedence rules and cycle-guarded structural collection while
//!      `TirView` remains the production read authority.

use std::collections::{HashSet, hash_map::Entry};

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::ids::ExpressionSiteId;
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirExpressionOverlay, TirExpressionOverlayId,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::slot_plan::runtime_slot_plan_roots;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{TirView, TirViewIdentity};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId, TemplateViewContext,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use rustc_hash::{FxHashMap, FxHashSet};

/// Collects cloned structural expression payloads reachable from a template.
///
/// WHAT: traverses template structure below `template_id`, including runtime
///       slot-plan roots owned by that template, records dynamic-expression
///       payloads, branch selectors and loop-header expressions keyed by their
///       `ExpressionSiteId`.
/// WHY: focused walker tests compare raw structural coverage with the effective
///      production collector without exposing collector internals.
/// This test-only entry intentionally remains raw structural collection: it
/// does not manufacture a durable view or import overlay authority.
#[cfg(test)]
pub(crate) fn collect_tir_expression_overlay_payloads<'store>(
    store: &'store TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<Vec<(ExpressionSiteId, Expression)>, CompilerError> {
    let mut collector = ExpressionOverlayPayloadCollector::<'store>::new(None);
    collector.collect_template(store, template_id)?;
    Ok(collector.into_payloads())
}

/// Collects effective expression payloads from an exact root view.
///
/// The caller supplies the durable view so collection cannot reconstruct a
/// root, phase and context triple independently from the view authority.
pub(crate) fn collect_effective_tir_expression_overlay_payloads<'store>(
    root_view: &TirView<'store>,
) -> Result<Vec<(ExpressionSiteId, Expression)>, CompilerError> {
    let mut collector = ExpressionOverlayPayloadCollector::new(Some(root_view.clone()));
    collector.collect_template(root_view.store(), root_view.root_ref())?;
    Ok(collector.into_payloads())
}

/// Replaces selected expression entries while preserving the other view
/// dimensions and every untouched expression override.
pub(crate) fn replace_expression_overlay_entries(
    store: &mut TemplateIrStore,
    base: TemplateViewContext,
    replacements: impl IntoIterator<Item = (ExpressionSiteId, Box<Expression>)>,
) -> Result<TemplateViewContext, CompilerError> {
    let mut replacement_entries = Vec::new();
    let mut replacement_site_ids = FxHashSet::default();
    for (site_id, expression) in replacements {
        if !replacement_site_ids.insert(site_id) {
            return Err(CompilerError::compiler_error(format!(
                "TIR expression overlay replacement received duplicate expression site {site_id}"
            )));
        }
        replacement_entries.push((site_id, expression));
    }

    let existing_overrides = if let Some(overlay_id) = base.expression_overlay {
        store
            .expression_overlay(overlay_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR expression overlay replacement referenced missing expression overlay {overlay_id}"
                ))
            })?
            .overrides
            .iter()
            .filter(|(site_id, _)| !replacement_site_ids.contains(site_id))
            .map(|(site_id, expression)| (*site_id, expression.clone()))
            .collect()
    } else {
        Vec::new()
    };

    if replacement_entries.is_empty() {
        return Ok(base);
    }

    let mut overrides = existing_overrides;
    overrides.extend(replacement_entries);
    overrides.sort_unstable_by_key(|(site_id, _)| site_id.index());

    let overlay_id = store.allocate_expression_overlay(TirExpressionOverlay { overrides })?;
    Ok(TemplateViewContext {
        expression_overlay: Some(overlay_id),
        ..base
    })
}

/// Descendant expression overlays are temporary authority layers. The first
/// layer containing a site wins, so an outer structural context remains more
/// authoritative than an overlay introduced by a nested child reference.
#[derive(Default)]
struct ExpressionOverlayAuthority {
    layers: Vec<TirExpressionOverlayId>,
}

impl ExpressionOverlayAuthority {
    fn push_context(
        &mut self,
        store: &TemplateIrStore,
        context: TemplateViewContext,
    ) -> Result<bool, CompilerError> {
        let Some(overlay_id) = context.expression_overlay else {
            return Ok(false);
        };

        if store.expression_overlay(overlay_id).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "TIR expression overlay collection referenced missing expression overlay {overlay_id}"
            )));
        }

        self.layers.push(overlay_id);
        Ok(true)
    }

    fn pop_layer(&mut self) {
        let _ = self.layers.pop();
    }

    fn expression_for_site<'store>(
        &self,
        store: &'store TemplateIrStore,
        site_id: ExpressionSiteId,
    ) -> Result<Option<(&'store Expression, usize)>, CompilerError> {
        for (layer_index, overlay_id) in self.layers.iter().copied().enumerate() {
            let overlay = store.expression_overlay(overlay_id).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR expression overlay collection referenced missing expression overlay {overlay_id}"
                ))
            })?;

            if let Some(expression) = overlay.expression_for_site(site_id) {
                return Ok(Some((expression, layer_index + 1)));
            }
        }

        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ExpressionOverlayTemplateTraversalKey {
    template_id: TemplateIrId,
    view: Option<TirViewIdentity>,
}

struct ExpressionOverlayPayloadCollector<'store> {
    // Temporary exact view used to collect a normalized root overlay. It is
    // restored after each named transition and is never retained by a caller.
    effective_view: Option<TirView<'store>>,
    authority: ExpressionOverlayAuthority,
    payloads: FxHashMap<ExpressionSiteId, (Expression, Option<usize>)>,
    active_nodes: HashSet<(TemplateIrNodeId, Option<TirViewIdentity>)>,
    completed_nodes: HashSet<(TemplateIrNodeId, Option<TirViewIdentity>)>,
    active_templates: HashSet<ExpressionOverlayTemplateTraversalKey>,
    completed_templates: HashSet<ExpressionOverlayTemplateTraversalKey>,
    active_slot_plans: HashSet<(TemplateSlotPlanId, Option<TirViewIdentity>)>,
    completed_slot_plans: HashSet<(TemplateSlotPlanId, Option<TirViewIdentity>)>,
}

impl<'store> ExpressionOverlayPayloadCollector<'store> {
    fn new(effective_view: Option<TirView<'store>>) -> Self {
        Self {
            effective_view,
            authority: ExpressionOverlayAuthority::default(),
            payloads: FxHashMap::default(),
            active_nodes: HashSet::new(),
            completed_nodes: HashSet::new(),
            active_templates: HashSet::new(),
            completed_templates: HashSet::new(),
            active_slot_plans: HashSet::new(),
            completed_slot_plans: HashSet::new(),
        }
    }

    fn into_payloads(self) -> Vec<(ExpressionSiteId, Expression)> {
        let mut payloads: Vec<_> = self
            .payloads
            .into_iter()
            .map(|(site_id, (expression, _))| (site_id, expression))
            .collect();
        payloads.sort_unstable_by_key(|(site_id, _)| site_id.index());
        payloads
    }

    fn effective_expression(
        &self,
        store: &'store TemplateIrStore,
        site_id: ExpressionSiteId,
        structural_expression: &Expression,
    ) -> Result<(Expression, Option<usize>), CompilerError> {
        if let Some(view) = &self.effective_view
            && let Some(expression) = view.effective_expression_for_site(site_id)?
        {
            return Ok((expression.clone(), Some(0)));
        }

        if let Some((expression, precedence)) =
            self.authority.expression_for_site(store, site_id)?
        {
            return Ok((expression.clone(), Some(precedence)));
        }

        Ok((structural_expression.clone(), None))
    }

    fn record_payload(
        &mut self,
        site_id: ExpressionSiteId,
        expression: Expression,
        precedence: Option<usize>,
    ) {
        match self.payloads.entry(site_id) {
            Entry::Vacant(entry) => {
                entry.insert((expression, precedence));
            }
            Entry::Occupied(mut entry) => {
                let existing_precedence = entry.get().1;
                let should_replace = match (precedence, existing_precedence) {
                    (Some(candidate), Some(existing)) => candidate < existing,
                    (Some(_), None) => true,
                    (None, _) => false,
                };

                if should_replace {
                    entry.insert((expression, precedence));
                }
            }
        }
    }

    fn collect_template(
        &mut self,
        store: &'store TemplateIrStore,
        template_id: TemplateIrId,
    ) -> Result<(), CompilerError> {
        let traversal_key = ExpressionOverlayTemplateTraversalKey {
            template_id,
            view: self.current_view_identity(),
        };
        if self.effective_view.is_none() && self.completed_templates.contains(&traversal_key) {
            return Ok(());
        }

        if !self.active_templates.insert(traversal_key) {
            return Err(CompilerError::compiler_error(
                "TIR expression overlay collection found a recursive child-template reference.",
            ));
        }

        let (root, runtime_slot_plan) = store
            .get_template(template_id)
            .map(|template| (template.root, template.runtime_slot_plan))
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "TIR expression overlay collection referenced a missing child template.",
                )
            })?;
        let result = if let Some(slot_plan_id) = runtime_slot_plan {
            self.collect_runtime_slot_application(store, root, slot_plan_id)
        } else {
            self.collect_node(store, root)
        };

        self.active_templates.remove(&traversal_key);
        if result.is_ok() {
            self.completed_templates.insert(traversal_key);
        }

        result
    }

    fn collect_runtime_slot_application(
        &mut self,
        store: &'store TemplateIrStore,
        wrapper_root: TemplateIrNodeId,
        slot_plan_id: TemplateSlotPlanId,
    ) -> Result<(), CompilerError> {
        let traversal_key = (slot_plan_id, self.current_view_identity());
        if self.effective_view.is_none() && self.completed_slot_plans.contains(&traversal_key) {
            return self.collect_node(store, wrapper_root);
        }

        if !self.active_slot_plans.insert(traversal_key) {
            return Err(CompilerError::compiler_error(
                "TIR expression overlay collection found a recursive runtime slot plan.",
            ));
        }

        let (contribution_roots, site_render_roots) = runtime_slot_plan_roots(store, slot_plan_id)?;

        let result = self.collect_node(store, wrapper_root).and_then(|()| {
            for root in contribution_roots {
                self.collect_node(store, root)?;
            }

            for root in site_render_roots {
                self.collect_node(store, root)?;
            }

            Ok(())
        });

        self.active_slot_plans.remove(&traversal_key);
        if result.is_ok() {
            self.completed_slot_plans.insert(traversal_key);
        }

        result
    }

    fn collect_node(
        &mut self,
        store: &'store TemplateIrStore,
        node_id: TemplateIrNodeId,
    ) -> Result<(), CompilerError> {
        let traversal_key = (node_id, self.current_view_identity());
        if self.effective_view.is_none() && self.completed_nodes.contains(&traversal_key) {
            return Ok(());
        }

        if !self.active_nodes.insert(traversal_key) {
            return Err(CompilerError::compiler_error(
                "TIR expression overlay collection found a recursive node reference.",
            ));
        }

        let result = self.collect_node_contents(store, node_id);

        self.active_nodes.remove(&traversal_key);
        if result.is_ok() {
            self.completed_nodes.insert(traversal_key);
        }

        result
    }

    fn collect_node_contents(
        &mut self,
        store: &'store TemplateIrStore,
        node_id: TemplateIrNodeId,
    ) -> Result<(), CompilerError> {
        let node = store.get_node(node_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR expression overlay collection referenced a missing node.",
            )
        })?;

        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                for &child in children {
                    self.collect_node(store, child)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::DynamicExpression {
                expression,
                site_id,
                ..
            } => {
                let (expression, precedence) =
                    self.effective_expression(store, *site_id, expression)?;
                self.record_payload(*site_id, expression, precedence);
                Ok(())
            }

            TemplateIrNodeKind::BranchChain { branches, fallback } => {
                for branch in branches {
                    let (expression, precedence) = self.effective_expression(
                        store,
                        branch.selector_site_id,
                        branch.condition_expression(),
                    )?;
                    self.record_payload(branch.selector_site_id, expression, precedence);
                    self.collect_node(store, branch.body)?;
                }

                if let Some(fallback) = fallback {
                    self.collect_node(store, *fallback)?;
                }

                Ok(())
            }

            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper,
                ..
            } => {
                self.collect_loop_header_payloads(store, header, header_sites)?;
                self.collect_node(store, *body)?;

                if let Some(wrapper) = aggregate_wrapper {
                    self.collect_node(store, *wrapper)?;
                }

                Ok(())
            }

            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                self.collect_structural_child(store, *reference)
            }

            TemplateIrNodeKind::InsertContribution { template } => self.collect_in_transition(
                store,
                *template,
                Some(|view: &TirView<'store>| view.structural_helper(*template)),
            ),

            TemplateIrNodeKind::Text { .. }
            | TemplateIrNodeKind::Slot { .. }
            | TemplateIrNodeKind::AggregateOutput
            | TemplateIrNodeKind::LoopControl { .. }
            | TemplateIrNodeKind::RuntimeSlotSite { .. }
            | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(()),
        }
    }

    fn collect_loop_header_payloads(
        &mut self,
        store: &'store TemplateIrStore,
        header: &TemplateLoopHeader,
        header_sites: &TemplateLoopHeaderExpressionSites,
    ) -> Result<(), CompilerError> {
        match (header, header_sites) {
            (
                TemplateLoopHeader::Conditional { condition },
                TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
            ) => {
                let (expression, precedence) =
                    self.effective_expression(store, *site_id, condition)?;
                self.record_payload(*site_id, expression, precedence);
            }

            (
                TemplateLoopHeader::Range { range, .. },
                TemplateLoopHeaderExpressionSites::Range { start, end, step },
            ) => {
                let (expression, precedence) =
                    self.effective_expression(store, *start, &range.start)?;
                self.record_payload(*start, expression, precedence);

                let (expression, precedence) =
                    self.effective_expression(store, *end, &range.end)?;
                self.record_payload(*end, expression, precedence);

                match (step, &range.step) {
                    (Some(step_site_id), Some(step_expression)) => {
                        let (expression, precedence) =
                            self.effective_expression(store, *step_site_id, step_expression)?;
                        self.record_payload(*step_site_id, expression, precedence);
                    }
                    (None, None) => {}
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "TIR expression overlay collection found mismatched range loop step site.",
                        ));
                    }
                }
            }

            (
                TemplateLoopHeader::Collection { iterable, .. },
                TemplateLoopHeaderExpressionSites::Collection { iterable: site_id },
            ) => {
                let (expression, precedence) =
                    self.effective_expression(store, *site_id, iterable)?;
                self.record_payload(*site_id, expression, precedence);
            }

            _ => {
                return Err(CompilerError::compiler_error(
                    "TIR expression overlay collection found mismatched loop-header expression sites.",
                ));
            }
        }

        Ok(())
    }

    fn collect_structural_child(
        &mut self,
        store: &'store TemplateIrStore,
        reference: TemplateTirChildReference,
    ) -> Result<(), CompilerError> {
        let added_layer = if self.effective_view.is_some() {
            self.authority.push_context(store, reference.context)?
        } else {
            false
        };

        let result = self.collect_in_transition(
            store,
            reference.root,
            Some(|view: &TirView<'store>| view.structural_child(reference)),
        );

        if added_layer {
            self.authority.pop_layer();
        }

        result
    }

    fn current_view_identity(&self) -> Option<TirViewIdentity> {
        self.effective_view.as_ref().map(TirView::identity)
    }

    fn collect_in_transition(
        &mut self,
        store: &'store TemplateIrStore,
        template_id: TemplateIrId,
        transition: Option<impl FnOnce(&TirView<'store>) -> Result<TirView<'store>, CompilerError>>,
    ) -> Result<(), CompilerError> {
        let parent_view = self.effective_view.take();
        let child_view = match (parent_view.as_ref(), transition) {
            (Some(parent_view), Some(transition)) => match transition(parent_view) {
                Ok(child_view) => Some(child_view),
                Err(error) => {
                    self.effective_view = Some(parent_view.clone());
                    return Err(error);
                }
            },
            _ => None,
        };

        self.effective_view = child_view;
        let result = self.collect_template(store, template_id);
        self.effective_view = parent_view;
        result
    }
}
