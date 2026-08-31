//! Mutating annotation pass for reactive template metadata.
//!
//! WHAT: walks the finalized AST and attaches computed reactive template metadata
//! to expressions, declarations, assignments, and template structures.
//! WHY: separating the annotation traversal from flow analysis and metadata
//! collection keeps each phase focused on one responsibility.

use super::collector::metadata_for_expression;
use super::types::{
    FunctionTemplateFlow, ReactiveTemplateValueEnvironment, reference_path_for_place_expression,
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, LoopBindings, NodeKind};
use crate::compiler_frontend::ast::expressions::assertion_message_effects::assertion_condition_is_statically_true;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleHandling, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnChannel};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::reactive_template_metadata::{
    metadata_for_owned_runtime_slot_application_handoff,
    metadata_for_owned_runtime_template_handoff,
};
use crate::compiler_frontend::ast::templates::runtime_handoff;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, TemplateIrId, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
    TemplateLoopHeaderExpressionSites, TemplateSlotPlanId, TemplateTirPhase, TirView,
    TirViewIdentity, collect_effective_tir_expression_overlay_payloads,
    replace_expression_overlay_entries, runtime_slot_plan_roots,
    runtime_slot_plan_site_render_root,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use rustc_hash::FxHashMap;
use std::collections::HashSet;

pub(super) fn annotate_nodes(
    nodes: &mut [AstNode],
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    for node in nodes {
        annotate_node(node, flows, value_environment, store)?;
    }

    Ok(())
}

/// One expression payload collected with the environment that applies at its
/// TIR expression site.
///
/// WHAT: pairs a cloned expression, its `ExpressionSiteId`, and the
///       `ReactiveTemplateValueEnvironment` that is active at that site.
/// WHY: reactive annotation must respect branch-capture and loop-binding
///      environment boundaries when composing one root overlay, so each
///      payload carries its own environment rather than sharing a flattened scope.
struct EnvironmentAwarePayload {
    site_id: ExpressionSiteId,
    expression: Expression,
    environment: ReactiveTemplateValueEnvironment,
}

/// Collects every expression payload reachable from `root`, preserving the
/// `ReactiveTemplateValueEnvironment` boundary at each branch, fallback, and
/// loop body.
///
/// WHAT: walks the same-store TIR subtree below `root` and records one
///       `EnvironmentAwarePayload` per expression site (dynamic-expression
///       splices, branch selectors, loop-header expressions). Branch bodies
///       get a cloned environment, fallback bodies get a cloned base
///       environment, and loop bodies get a cloned environment with loop
///       bindings recorded.
/// WHY: one root overlay must carry annotated expressions from every
///      control-flow body, but each body's expressions must be annotated with
///      the environment active inside that body, not a flattened scope.
fn collect_environment_aware_tir_expression_payloads(
    root_view: TirView<'_>,
    base_environment: &ReactiveTemplateValueEnvironment,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
) -> Result<Vec<EnvironmentAwarePayload>, CompilerError> {
    let root = root_view.root_template()?.root;
    // This is a construction-time merge input used to annotate one complete
    // root overlay. It is not a durable read context; recursive reads remain
    // owned by the exact `TirView` below.
    let effective_expressions = collect_effective_tir_expression_overlay_payloads(&root_view)?
        .into_iter()
        .collect();
    let mut collector =
        EnvironmentAwarePayloadCollector::new(flows, root_view, effective_expressions);
    collector.collect_node(root, base_environment)?;
    Ok(collector.into_payloads())
}

/// Environment-aware TIR expression-payload collector.
///
/// WHAT: walks same-store TIR nodes, recording one `EnvironmentAwarePayload`
///       per expression site while cloning the `ReactiveTemplateValueEnvironment`
///       at branch, fallback, and loop boundaries.
/// WHY: keeps the structural traversal and environment-boundary logic in one
///      place so the annotation pass composes one authoritative root overlay
///      without flattening control-flow scopes.
struct EnvironmentAwarePayloadCollector<'store, 'flow> {
    flows: &'flow FxHashMap<InternedPath, FunctionTemplateFlow>,
    view: TirView<'store>,
    // Temporary normalization input for the root overlay being constructed.
    // Durable effective-expression reads belong to `TirView`, not this map.
    effective_expressions: FxHashMap<ExpressionSiteId, Expression>,
    payloads: FxHashMap<ExpressionSiteId, EnvironmentAwarePayload>,
    active_nodes: HashSet<(TemplateIrNodeId, TirViewIdentity)>,
    completed_nodes: HashSet<(TemplateIrNodeId, TirViewIdentity)>,
    active_templates: HashSet<TirViewIdentity>,
    completed_templates: HashSet<TirViewIdentity>,
    active_slot_plans: HashSet<(TemplateSlotPlanId, TirViewIdentity)>,
    completed_slot_plans: HashSet<(TemplateSlotPlanId, TirViewIdentity)>,
}

impl<'store, 'flow> EnvironmentAwarePayloadCollector<'store, 'flow> {
    fn new(
        flows: &'flow FxHashMap<InternedPath, FunctionTemplateFlow>,
        view: TirView<'store>,
        effective_expressions: FxHashMap<ExpressionSiteId, Expression>,
    ) -> Self {
        Self {
            flows,
            view,
            effective_expressions,
            payloads: FxHashMap::default(),
            active_nodes: HashSet::new(),
            completed_nodes: HashSet::new(),
            active_templates: HashSet::new(),
            completed_templates: HashSet::new(),
            active_slot_plans: HashSet::new(),
            completed_slot_plans: HashSet::new(),
        }
    }

    fn into_payloads(self) -> Vec<EnvironmentAwarePayload> {
        let mut payloads: Vec<_> = self.payloads.into_values().collect();
        payloads.sort_unstable_by_key(|payload| payload.site_id.index());
        payloads
    }

    fn record_payload(&mut self, payload: EnvironmentAwarePayload) {
        // Site IDs are module-unique; repeated reachability is shared structure,
        // so retain the first environment and sort only at the overlay boundary.
        self.payloads.entry(payload.site_id).or_insert(payload);
    }

    fn effective_expression(
        &self,
        site_id: ExpressionSiteId,
        structural_expression: &Expression,
    ) -> Result<Expression, CompilerError> {
        Ok(self
            .effective_expressions
            .get(&site_id)
            .cloned()
            .unwrap_or_else(|| structural_expression.clone()))
    }

    fn collect_template(
        &mut self,
        template_id: TemplateIrId,
        environment: &ReactiveTemplateValueEnvironment,
    ) -> Result<(), CompilerError> {
        let traversal_key = self.view.identity();
        if self.completed_templates.contains(&traversal_key) {
            return Ok(());
        }

        if !self.active_templates.insert(traversal_key) {
            return Err(CompilerError::compiler_error(
                "TIR environment-aware payload collection found a recursive child-template reference.",
            ));
        }

        let (root, runtime_slot_plan) = self
            .view
            .store()
            .get_template(template_id)
            .map(|template| (template.root, template.runtime_slot_plan))
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR environment-aware payload collection referenced missing child template {}",
                    template_id
                ))
            })?;

        let result = if let Some(slot_plan_id) = runtime_slot_plan {
            self.collect_runtime_slot_application(root, slot_plan_id, environment)
        } else {
            self.collect_node(root, environment)
        };

        self.active_templates.remove(&traversal_key);
        if result.is_ok() {
            self.completed_templates.insert(traversal_key);
        }
        result
    }

    fn collect_runtime_slot_application(
        &mut self,
        wrapper_root: TemplateIrNodeId,
        slot_plan_id: TemplateSlotPlanId,
        environment: &ReactiveTemplateValueEnvironment,
    ) -> Result<(), CompilerError> {
        let traversal_key = (slot_plan_id, self.view.identity());
        if self.completed_slot_plans.contains(&traversal_key) {
            return self.collect_node(wrapper_root, environment);
        }

        if !self.active_slot_plans.insert(traversal_key) {
            return Err(CompilerError::compiler_error(
                "TIR environment-aware payload collection found a recursive runtime slot plan.",
            ));
        }

        let (contribution_roots, site_render_roots) =
            runtime_slot_plan_roots(self.view.store(), slot_plan_id)?;

        let result = self.collect_node(wrapper_root, environment).and_then(|()| {
            for root in contribution_roots {
                self.collect_node(root, environment)?;
            }
            for root in site_render_roots {
                self.collect_node(root, environment)?;
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
        node_id: TemplateIrNodeId,
        environment: &ReactiveTemplateValueEnvironment,
    ) -> Result<(), CompilerError> {
        let traversal_key = (node_id, self.view.identity());
        if self.completed_nodes.contains(&traversal_key) {
            return Ok(());
        }

        if !self.active_nodes.insert(traversal_key) {
            return Err(CompilerError::compiler_error(
                "TIR environment-aware payload collection found a recursive node reference.",
            ));
        }

        let result = self.collect_node_payload_and_children(node_id, environment);

        self.active_nodes.remove(&traversal_key);
        if result.is_ok() {
            self.completed_nodes.insert(traversal_key);
        }
        result
    }

    fn collect_node_payload_and_children(
        &mut self,
        node_id: TemplateIrNodeId,
        environment: &ReactiveTemplateValueEnvironment,
    ) -> Result<(), CompilerError> {
        let Some(node) = self.view.store().get_node(node_id) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR environment-aware payload collection referenced missing node {}",
                node_id
            )));
        };

        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                for child in children.iter().copied() {
                    self.collect_node(child, environment)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::DynamicExpression {
                expression,
                site_id,
                ..
            } => {
                self.record_payload(EnvironmentAwarePayload {
                    site_id: *site_id,
                    expression: self.effective_expression(*site_id, expression)?,
                    environment: environment.clone(),
                });
                Ok(())
            }

            TemplateIrNodeKind::BranchChain { branches, fallback } => {
                for branch in branches {
                    let selector_expression = self.effective_expression(
                        branch.selector_site_id,
                        branch.condition_expression(),
                    )?;
                    let mut branch_environment = environment.clone();

                    self.record_payload(EnvironmentAwarePayload {
                        site_id: branch.selector_site_id,
                        expression: selector_expression.clone(),
                        environment: environment.clone(),
                    });

                    if let TemplateBranchSelector::OptionPresentCapture { pattern, .. } =
                        &branch.selector
                        && let MatchPattern::OptionPresentCapture { binding_path, .. } =
                            pattern.as_ref()
                    {
                        let captured_metadata = metadata_for_expression(
                            &selector_expression,
                            self.flows,
                            &branch_environment,
                            self.view.store(),
                        )?;
                        branch_environment.record_binding_metadata(binding_path, captured_metadata);
                    }

                    self.collect_node(branch.body, &branch_environment)?;
                }

                if let Some(fallback_id) = fallback {
                    let fallback_environment = environment.clone();
                    self.collect_node(*fallback_id, &fallback_environment)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body,
                aggregate_wrapper,
            } => {
                let mut loop_environment = environment.clone();
                record_loop_binding_declarations(header, &mut loop_environment);
                self.collect_loop_header_payloads(header, header_sites, &loop_environment)?;
                self.collect_node(*body, &loop_environment)?;
                if let Some(wrapper) = aggregate_wrapper {
                    self.collect_node(*wrapper, &loop_environment)?;
                }
                Ok(())
            }

            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                if reference.phase.is_at_least(TemplateTirPhase::Composed) {
                    let child_view = self.view.structural_child(*reference)?;
                    let parent_view = self.view.clone();
                    self.view = child_view;
                    let result = self.collect_template(reference.root, environment);
                    self.view = parent_view;
                    result?;
                }
                Ok(())
            }

            TemplateIrNodeKind::InsertContribution { template } => {
                let child_view = self.view.structural_helper(*template)?;
                let parent_view = self.view.clone();
                self.view = child_view;
                let result = self.collect_template(*template, environment);
                self.view = parent_view;
                result?;
                Ok(())
            }

            TemplateIrNodeKind::RuntimeSlotSite { plan, site } => {
                runtime_slot_plan_site_render_root(self.view.store(), *plan, *site)?;
                Ok(())
            }

            TemplateIrNodeKind::Text { .. }
            | TemplateIrNodeKind::Slot { .. }
            | TemplateIrNodeKind::AggregateOutput
            | TemplateIrNodeKind::LoopControl { .. }
            | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(()),
        }
    }

    fn collect_loop_header_payloads(
        &mut self,
        header: &TemplateLoopHeader,
        header_sites: &TemplateLoopHeaderExpressionSites,
        environment: &ReactiveTemplateValueEnvironment,
    ) -> Result<(), CompilerError> {
        match (header, header_sites) {
            (
                TemplateLoopHeader::Conditional { condition },
                TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
            ) => {
                self.record_payload(EnvironmentAwarePayload {
                    site_id: *site_id,
                    expression: self.effective_expression(*site_id, condition)?,
                    environment: environment.clone(),
                });
            }

            (
                TemplateLoopHeader::Range { range, .. },
                TemplateLoopHeaderExpressionSites::Range { start, end, step },
            ) => {
                self.record_payload(EnvironmentAwarePayload {
                    site_id: *start,
                    expression: self.effective_expression(*start, &range.start)?,
                    environment: environment.clone(),
                });
                self.record_payload(EnvironmentAwarePayload {
                    site_id: *end,
                    expression: self.effective_expression(*end, &range.end)?,
                    environment: environment.clone(),
                });

                match (step, &range.step) {
                    (Some(step_site_id), Some(step_expression)) => {
                        self.record_payload(EnvironmentAwarePayload {
                            site_id: *step_site_id,
                            expression: self
                                .effective_expression(*step_site_id, step_expression)?,
                            environment: environment.clone(),
                        });
                    }
                    (None, None) => {}
                    _ => {
                        return Err(CompilerError::compiler_error(
                            "TIR environment-aware payload collection found mismatched range loop step site.",
                        ));
                    }
                }
            }

            (
                TemplateLoopHeader::Collection { iterable, .. },
                TemplateLoopHeaderExpressionSites::Collection { iterable: site_id },
            ) => {
                self.record_payload(EnvironmentAwarePayload {
                    site_id: *site_id,
                    expression: self.effective_expression(*site_id, iterable)?,
                    environment: environment.clone(),
                });
            }

            _ => {
                return Err(CompilerError::compiler_error(
                    "TIR environment-aware payload collection found mismatched loop-header expression sites.",
                ));
            }
        }

        Ok(())
    }
}

/// Records loop binding declarations in the environment so binding names are
/// available when annotating expressions inside the loop body and aggregate
/// wrapper.
///
/// WHAT: calls `record_declaration` for each binding without annotating the
///       binding value expression, because the binding value is `NoValue`
///       and the TIR node header is shared read-only.
/// WHY: preserves the same environment semantics as the prior control-flow
///      annotation path without mutating shared TIR structure.
fn record_loop_binding_declarations(
    header: &TemplateLoopHeader,
    environment: &mut ReactiveTemplateValueEnvironment,
) {
    match header {
        TemplateLoopHeader::Conditional { .. } => {}

        TemplateLoopHeader::Range { bindings, .. }
        | TemplateLoopHeader::Collection { bindings, .. } => {
            if let Some(item) = &bindings.item {
                environment.record_declaration(item);
            }
            if let Some(index) = &bindings.index {
                environment.record_declaration(index);
            }
        }
    }
}

fn annotate_node(
    node: &mut AstNode,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match &mut node.kind {
        NodeKind::Function(path, signature, body) => {
            let mut function_environment =
                ReactiveTemplateValueEnvironment::for_parameters(&signature.parameters);
            annotate_nodes(body, flows, &mut function_environment, store)?;
            apply_flow_to_signature(path, signature, flows);
        }

        NodeKind::VariableDeclaration(declaration) => {
            annotate_declaration(declaration, flows, value_environment, store)?;
        }

        NodeKind::Return(values) => {
            annotate_expressions(values, flows, value_environment, store)?;
        }

        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value) => {
            annotate_expression(value, flows, value_environment, store)?;
        }

        NodeKind::ThenValue(produced_values) => {
            annotate_expressions(
                &mut produced_values.expressions,
                flows,
                value_environment,
                store,
            )?;
        }

        NodeKind::If(condition, then_body, else_body, _) => {
            annotate_expression(condition, flows, value_environment, store)?;
            let mut then_environment = value_environment.clone();
            annotate_nodes(then_body, flows, &mut then_environment, store)?;
            if let Some(else_body) = else_body {
                let mut else_environment = value_environment.clone();
                annotate_nodes(else_body, flows, &mut else_environment, store)?;
            }
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            annotate_expression(scrutinee, flows, value_environment, store)?;
            for arm in arms {
                let mut arm_environment = value_environment.clone();
                annotate_match_pattern(&mut arm.pattern, flows, &mut arm_environment, store)?;
                if let Some(guard) = &mut arm.guard {
                    annotate_expression(guard, flows, &mut arm_environment, store)?;
                }
                annotate_nodes(&mut arm.body, flows, &mut arm_environment, store)?;
            }
            if let Some(default_body) = default {
                let mut default_environment = value_environment.clone();
                annotate_nodes(default_body, flows, &mut default_environment, store)?;
            }
        }

        NodeKind::LexicalScope { body } => {
            let mut body_environment = value_environment.clone();
            annotate_nodes(body, flows, &mut body_environment, store)?;
        }

        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            let mut loop_environment = value_environment.clone();
            annotate_loop_bindings(bindings, flows, &mut loop_environment, store)?;
            annotate_expression(&mut range.start, flows, &mut loop_environment, store)?;
            annotate_expression(&mut range.end, flows, &mut loop_environment, store)?;
            if let Some(step) = &mut range.step {
                annotate_expression(step, flows, &mut loop_environment, store)?;
            }
            annotate_nodes(body, flows, &mut loop_environment, store)?;
        }

        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            let mut loop_environment = value_environment.clone();
            annotate_loop_bindings(bindings, flows, &mut loop_environment, store)?;
            annotate_expression(iterable, flows, &mut loop_environment, store)?;
            annotate_nodes(body, flows, &mut loop_environment, store)?;
        }

        NodeKind::WhileLoop(condition, body) => {
            annotate_expression(condition, flows, value_environment, store)?;
            let mut body_environment = value_environment.clone();
            annotate_nodes(body, flows, &mut body_environment, store)?;
        }

        NodeKind::Assert { condition, message } => {
            annotate_expression(condition, flows, value_environment, store)?;
            if !assertion_condition_is_statically_true(condition) {
                annotate_expression(message, flows, value_environment, store)?;
            }
        }

        NodeKind::StructDefinition(_, fields) => {
            for field in fields {
                annotate_declaration(field, flows, value_environment, store)?;
            }
        }

        NodeKind::Assignment { target, value } => {
            annotate_expression(value, flows, value_environment, store)?;
            if let Some(target_path) = reference_path_for_place_expression(target) {
                value_environment.record_assignment(target_path, value);
            }
        }

        NodeKind::MultiBind { value, .. } => {
            annotate_expression(value, flows, value_environment, store)?;
        }

        NodeKind::Break | NodeKind::Continue => {}
    }

    Ok(())
}

fn apply_flow_to_signature(
    path: &InternedPath,
    signature: &mut FunctionSignature,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
) {
    let Some(flow) = flows.get(path) else {
        return;
    };

    let mut success_index = 0;
    for slot in &mut signature.returns {
        if slot.channel != ReturnChannel::Success {
            continue;
        }

        slot.reactive_template = flow
            .success_returns
            .get(success_index)
            .cloned()
            .unwrap_or(None);
        success_index += 1;
    }
}

fn annotate_declaration(
    declaration: &mut Declaration,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    if let ExpressionKind::Function(signature) = &mut declaration.value.kind {
        apply_flow_to_signature(&declaration.id, signature, flows);
        declaration.value.reactive_template =
            metadata_for_expression(&declaration.value, flows, value_environment, store)?;
        value_environment.record_declaration(declaration);
        return Ok(());
    }

    annotate_expression(&mut declaration.value, flows, value_environment, store)?;
    value_environment.record_declaration(declaration);

    Ok(())
}

fn annotate_expressions(
    expressions: &mut [Expression],
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    for expression in expressions {
        annotate_expression(expression, flows, value_environment, store)?;
    }

    Ok(())
}

fn annotate_place_expression(place: &mut PlaceExpression) {
    match &mut place.kind {
        PlaceExpressionKind::Local(_) => {}
        PlaceExpressionKind::Field { base, .. } => annotate_place_expression(base),
    }
}

fn annotate_expression(
    expression: &mut Expression,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match &mut expression.kind {
        ExpressionKind::Template(template) => {
            annotate_template(template, flows, value_environment, store)?;
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            let handoff_metadata =
                annotate_owned_runtime_template_handoff(handoff, flows, value_environment, store)?;
            expression.reactive_template = Some(handoff_metadata);
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            let handoff_metadata =
                annotate_runtime_slot_handoff(handoff, flows, value_environment, store)?;
            expression.reactive_template = Some(handoff_metadata);
        }

        ExpressionKind::Function(_) => {}

        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. } => {
            annotate_call_arguments(args, flows, value_environment, store)?;
        }

        ExpressionKind::FieldAccess { base, .. } => {
            annotate_expression(base, flows, value_environment, store)?;
        }

        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            annotate_expression(receiver, flows, value_environment, store)?;
            annotate_call_arguments(args, flows, value_environment, store)?;
        }

        ExpressionKind::HandledFallibleFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
            annotate_call_arguments(args, flows, value_environment, store)?;
        }

        ExpressionKind::Copy(place) => {
            annotate_place_expression(place);
        }

        ExpressionKind::Runtime(rpn) => {
            for item in &mut rpn.items {
                match item {
                    ExpressionRpnItem::Operand(expression) => {
                        annotate_expression(expression, flows, value_environment, store)?;
                    }
                    ExpressionRpnItem::Operator { .. } => {}
                }
            }
        }

        ExpressionKind::Collection(items) => {
            annotate_expressions(items, flows, value_environment, store)?
        }

        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                annotate_expression(&mut entry.key, flows, value_environment, store)?;
                annotate_expression(&mut entry.value, flows, value_environment, store)?;
            }
        }

        ExpressionKind::StructInstance(fields)
        | ExpressionKind::StructDefinition(fields)
        | ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                annotate_declaration(field, flows, value_environment, store)?;
            }
        }

        ExpressionKind::Range(start, end) => {
            annotate_expression(start, flows, value_environment, store)?;
            annotate_expression(end, flows, value_environment, store)?;
        }

        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            annotate_expression(value, flows, value_environment, store)?;
        }

        ExpressionKind::OptionPropagation { value } | ExpressionKind::Coerced { value, .. } => {
            annotate_expression(value, flows, value_environment, store)?;
        }

        ExpressionKind::HandledFallibleExpression { value, .. } => {
            annotate_expression(value, flows, value_environment, store)?;
        }

        ExpressionKind::Cast(cast) => {
            annotate_expression(&mut cast.source, flows, value_environment, store)?;
        }

        ExpressionKind::ValueBlock { block } => {
            annotate_value_block(block, flows, value_environment, store)?
        }

        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::StructuralString { .. }
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_) => {}
    }

    expression.reactive_template =
        metadata_for_expression(expression, flows, value_environment, store)?;

    Ok(())
}

fn annotate_template(
    template: &mut Template,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    annotate_template_tir_root(template, flows, value_environment, store)?;

    // `$children(..)` wrappers are exact TIR references by this
    // stage. Their reactive payloads are annotated through effective TIR views
    // and overlays, so there is no recursive AST wrapper tree to walk here.

    Ok(())
}

fn annotate_branch_selector(
    selector: &mut TemplateBranchSelector,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            annotate_expression(condition, flows, value_environment, store)?
        }
        TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
            annotate_expression(scrutinee, flows, value_environment, store)?;
            annotate_match_pattern(pattern, flows, value_environment, store)?;
        }
    }

    Ok(())
}

fn annotate_loop_header(
    header: &mut TemplateLoopHeader,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            annotate_expression(condition, flows, value_environment, store)?
        }
        TemplateLoopHeader::Range { bindings, range } => {
            annotate_loop_bindings(bindings, flows, value_environment, store)?;
            annotate_expression(&mut range.start, flows, value_environment, store)?;
            annotate_expression(&mut range.end, flows, value_environment, store)?;
            if let Some(step) = &mut range.step {
                annotate_expression(step, flows, value_environment, store)?;
            }
        }
        TemplateLoopHeader::Collection { bindings, iterable } => {
            annotate_loop_bindings(bindings, flows, value_environment, store)?;
            annotate_expression(iterable, flows, value_environment, store)?;
        }
    }

    Ok(())
}

fn annotate_runtime_slot_handoff(
    handoff: &mut OwnedRuntimeSlotApplicationHandoff,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<ReactiveTemplateMetadata, CompilerError> {
    runtime_handoff::walk_owned_runtime_slot_application_handoff_mut(
        handoff,
        &mut |event| -> Result<(), CompilerError> {
            annotate_owned_runtime_template_node(event, flows, value_environment, store)?;
            Ok(())
        },
    )?;

    // After annotating nested expression payloads, compute the handoff's own
    // reactive template metadata from its structural shape so HIR can bind the
    // runtime slot application's reactive dependencies.
    metadata_for_owned_runtime_slot_application_handoff(handoff, &mut |expression| {
        Ok(expression.reactive_template.clone())
    })
}

fn annotate_owned_runtime_template_handoff(
    handoff: &mut OwnedRuntimeTemplateHandoff,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<ReactiveTemplateMetadata, CompilerError> {
    runtime_handoff::walk_owned_runtime_template_handoff_mut(handoff, &mut |event| -> Result<
        (),
        CompilerError,
    > {
        annotate_owned_runtime_template_node(event, flows, value_environment, store)?;
        Ok(())
    })?;

    // After annotating nested expression payloads, compute the handoff's own
    // reactive template metadata from its structural shape so HIR can bind the
    // runtime template's reactive dependencies.
    metadata_for_owned_runtime_template_handoff(handoff, &mut |expression| {
        Ok(expression.reactive_template.clone())
    })
}

fn annotate_owned_runtime_template_node(
    node: &mut OwnedRuntimeTemplateNode,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            annotate_expression(expression, flows, value_environment, store)?;
        }

        OwnedRuntimeTemplateNode::BranchChain { branches, .. } => {
            for branch in branches {
                annotate_branch_selector(&mut branch.selector, flows, value_environment, store)?;
            }
        }

        OwnedRuntimeTemplateNode::Loop { header, .. } => {
            annotate_loop_header(header, flows, value_environment, store)?;
        }

        OwnedRuntimeTemplateNode::Sequence { .. }
        | OwnedRuntimeTemplateNode::ChildTemplate { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }

    Ok(())
}

fn annotate_loop_bindings(
    bindings: &mut LoopBindings,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    if let Some(item) = &mut bindings.item {
        annotate_declaration(item, flows, value_environment, store)?;
    }
    if let Some(index) = &mut bindings.index {
        annotate_declaration(index, flows, value_environment, store)?;
    }

    Ok(())
}

fn annotate_call_arguments(
    arguments: &mut [CallArgument],
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    for argument in arguments {
        annotate_expression(&mut argument.value, flows, value_environment, store)?;
    }

    Ok(())
}

fn annotate_fallible_handling(
    handling: &mut FallibleHandling,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match handling {
        FallibleHandling::Propagate => {}
        FallibleHandling::Handler { body, .. } => {
            let mut handler_environment = value_environment.clone();
            annotate_nodes(body, flows, &mut handler_environment, store)?;
        }
    }

    Ok(())
}

fn annotate_match_pattern(
    pattern: &mut MatchPattern,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match pattern {
        MatchPattern::Literal(value)
        | MatchPattern::OptionValue { value, .. }
        | MatchPattern::Relational { value, .. } => {
            annotate_expression(value, flows, value_environment, store)?
        }

        MatchPattern::ChoiceVariant { .. }
        | MatchPattern::OptionNone { .. }
        | MatchPattern::OptionPresentCapture { .. } => {}
    }

    Ok(())
}

fn annotate_value_block(
    block: &mut Box<ValueBlock>,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    match block.as_mut() {
        ValueBlock::If(value_if) => {
            annotate_expression(&mut value_if.condition, flows, value_environment, store)?;
            let mut then_environment = value_environment.clone();
            annotate_nodes(&mut value_if.then_body, flows, &mut then_environment, store)?;
            let mut else_environment = value_environment.clone();
            annotate_nodes(&mut value_if.else_body, flows, &mut else_environment, store)?;
        }
        ValueBlock::LexicalScope(value_lexical_scope) => {
            let mut lexical_environment = value_environment.clone();
            annotate_nodes(
                &mut value_lexical_scope.body,
                flows,
                &mut lexical_environment,
                store,
            )?;
        }
        ValueBlock::Match(value_match) => {
            annotate_expression(&mut value_match.scrutinee, flows, value_environment, store)?;
            for arm in &mut value_match.arms {
                let mut arm_environment = value_environment.clone();
                annotate_match_pattern(&mut arm.pattern, flows, &mut arm_environment, store)?;
                if let Some(guard) = &mut arm.guard {
                    annotate_expression(guard, flows, &mut arm_environment, store)?;
                }
                annotate_nodes(&mut arm.body, flows, &mut arm_environment, store)?;
            }
            if let Some(default_body) = &mut value_match.default {
                let mut default_environment = value_environment.clone();
                annotate_nodes(default_body, flows, &mut default_environment, store)?;
            }
        }
        ValueBlock::Catch(value_catch) => {
            annotate_expression(
                &mut value_catch.handled_value,
                flows,
                value_environment,
                store,
            )?;
            annotate_fallible_handling(&mut value_catch.handler, flows, value_environment, store)?;
        }
    }

    Ok(())
}

/// Annotates a template's same-store TIR root with one composed expression
/// overlay.
///
/// WHAT: for templates with a same-store TIR root at `Composed` phase or later,
///       collects every expression payload reachable from the root while
///       preserving branch-capture and loop-binding environment boundaries,
///       annotates each expression, and composes the resulting overrides into
///       one root overlay attached to `template.tir_reference`.
/// WHY: the TIR root is the sole template-structure authority. Branch chains,
///      loops, selectors, headers, bodies, and aggregate wrappers are all
///      reachable from the root, so one environment-aware traversal and one
///      composed overlay replaces the prior durable control-flow and per-body
///      overlay paths. Pre-Composed references remain semantic non-participants;
///      required module-store authority failures propagate as compiler errors.
fn annotate_template_tir_root(
    template: &mut Template,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    let reference = &mut template.tir_reference;

    if !reference.phase.is_at_least(TemplateTirPhase::Composed) {
        return Ok(());
    }

    let root_view = TirView::new(store, reference.root, reference.phase, reference.context)?;
    let environment_aware_payloads =
        collect_environment_aware_tir_expression_payloads(root_view, value_environment, flows)?;

    if environment_aware_payloads.is_empty() {
        // The reference is already at least Composed, and the collector has
        // validated the root and its expression-overlay authority.
        return Ok(());
    }

    let mut annotated_overrides = Vec::with_capacity(environment_aware_payloads.len());
    for mut payload in environment_aware_payloads {
        annotate_expression(
            &mut payload.expression,
            flows,
            &mut payload.environment,
            store,
        )?;
        annotated_overrides.push((payload.site_id, Box::new(payload.expression)));
    }

    let new_context =
        replace_expression_overlay_entries(store, reference.context, annotated_overrides)?;

    reference.context = new_context;

    Ok(())
}
