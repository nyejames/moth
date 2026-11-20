//! Effective expression-site traversal over the paired TIR and expression owners.
//! Exact TIR identities and named structural transitions remain the authority for
//! overlays. Nested template values enter their independently retained view.

use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::collect_tir_slot_schema_with_context;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites, TirSlotPlaceholder,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirSlotResolutionKind, TirWrapperApplicationMode,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{TirView, TirViewIdentity};
use crate::compiler_frontend::ast::templates::tir::{TemplateIrNodeId, TemplateTirReference};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// One item emitted by the structural payload walk in authored order.
///
/// WHAT: an expression payload, or the generated-failure inputs of a loop header emitted
///       between the header's operands and the loop body.
/// WHY: failure classification must classify a building template's runtime loop guards exactly
///      where HIR lowers them. A second expression-only walk could not place them in order.
pub(crate) enum TemplatePayload<'expressions> {
    Expression(ExpressionView<'expressions>),
    LoopFailure(LoopFailureSite),
}

/// The generated-failure inputs of one structurally reachable loop header.
///
/// WHAT: copies the resolved header facts and effective expression IDs that decide whether the
///       loop lowers a zero-step, no-progress or ordinal failure, and where that failure is
///       labelled.
/// WHY: classification reads these from the same walk that resolves effective overlays, so the
///      pending failure facts of a building template match its later runtime handoff.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LoopFailureSite {
    Range {
        domain: NumericScalar,
        fast_math: bool,
        has_index_binding: bool,
        start: ExprId,
        step: Option<ExprId>,
    },

    Collection {
        /// Present exactly when the header authored a step.
        step: Option<ExprId>,
    },
}

/// Walks expression payloads only. Loop generated failures stay invisible to callers that own
/// no failure semantics, such as type validation and dead-span collection.
pub(crate) fn walk_tir_view_expression_payloads<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), InfrastructureFailure>,
) -> Result<(), InfrastructureFailure> {
    let mut payload_visitor = |payload| match payload {
        TemplatePayload::Expression(expression) => visitor(expression),
        TemplatePayload::LoopFailure(_) => Ok(()),
    };
    let mut state = PayloadWalkState::new(expressions, &mut payload_visitor, true);
    walk_view(&mut state, view, None, None)
}

/// Walks expression payloads and loop generated-failure sites in authored order.
pub(crate) fn walk_tir_view_failure_payloads<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
) -> Result<(), InfrastructureFailure> {
    let mut state = PayloadWalkState::new(expressions, visitor, false);
    walk_view(&mut state, view, None, None)
}

/// Expression payloads reached from independently retained nested template values.
///
/// WHAT: inspects one expression tree, deferring each nested template value to an exact
///       `TirView` walk, then follows the same effective render traversal for those views.
/// WHY: nested template values own their own view context. Reusing the shared view walk keeps
///      slot plans, inherited wrappers and slot resolutions in the same emitted order the
///      runtime handoff later renders.
pub(crate) fn walk_expression_payloads_with_nested_tir_views<'expressions>(
    expression: ExpressionView<'expressions>,
    store: &TemplateIrStore,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), InfrastructureFailure>,
) -> Result<(), InfrastructureFailure> {
    // The worklist and the view walk share one completed-view set, so a template reached
    // through both a TIR child and an expression operand is entered once.
    let pending = RefCell::new(Vec::new());
    inspect_expression(expression, visitor, &mut pending.borrow_mut())?;

    let mut inspect = |payload| match payload {
        TemplatePayload::Expression(expression) => {
            inspect_expression(expression, visitor, &mut pending.borrow_mut())
        }
        TemplatePayload::LoopFailure(_) => Ok(()),
    };
    let mut state = PayloadWalkState::new(expressions, &mut inspect, true);
    loop {
        let Some(reference) = pending.borrow_mut().pop() else {
            break;
        };
        let view = TirView::new(store, reference.root, reference.phase, reference.context)?;
        walk_view(&mut state, &view, None, None)?;
    }
    Ok(())
}

fn inspect_expression<'expressions>(
    expression: ExpressionView<'expressions>,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), InfrastructureFailure>,
    pending: &mut Vec<TemplateTirReference>,
) -> Result<(), InfrastructureFailure> {
    match expression.operation() {
        ExpressionOperationView::Template(template) => pending.push(template.tir_reference),
        ExpressionOperationView::Coerce { value, .. } => {
            inspect_expression(value, visitor, pending)?
        }
        ExpressionOperationView::NumericUnary { .. }
        | ExpressionOperationView::NumericBinary { .. }
        | ExpressionOperationView::LogicalNot { .. }
        | ExpressionOperationView::LogicalBinary { .. }
        | ExpressionOperationView::Compare { .. } => {
            let mut result = Ok(());
            expression.visit_children(|child| {
                if result.is_ok() {
                    result = inspect_expression(child, visitor, pending);
                }
            });
            result?;
        }
        _ => visitor(expression)?,
    }
    Ok(())
}

/// Shared state for the effective render walk.
struct PayloadWalkState<'expressions, V> {
    expressions: &'expressions AstExpressionBuilder,
    visitor: V,
    /// Exact views already entered by this walk.
    ///
    /// Shared template values can be reached through a structural child, a slot plan and an
    /// expression operand. Entering each exact view once keeps payload order stable and avoids
    /// repeated traversal of the same structural tree.
    completed_views: HashSet<TirViewIdentity>,
    /// Injected views currently being walked, independent of the completed-view set.
    ///
    /// Wrapper injections are occurrence-specific: the same view can receive different child
    /// content at the same slot. Skipping a later injected entry would drop that child at the
    /// unmatched fallback after the wrapper suffix, so injected views bypass `completed_views`
    /// and only use this set to stop recursive re-entry.
    active_injected_views: HashSet<TirViewIdentity>,
    /// Resolved loose-fill target per wrapper reference.
    ///
    /// One `$before`/`$after` wrapper set is applied to every direct child, and each application
    /// would otherwise rebuild the full slot layout for the same wrapper template.
    wrapper_fill_targets: HashMap<TemplateWrapperReference, Option<SlotKey>>,
    /// Whether an absent `$supplied` guard body is structurally reachable.
    ///
    /// Failure classification skips it because composition removes an unselected guard. Pure
    /// payload consumers such as type validation and dead-span collection still visit authored
    /// body code so validation and liveness remain structural.
    walk_absent_supplied_guard_body: bool,
}

impl<'expressions, V> PayloadWalkState<'expressions, V>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    fn new(
        expressions: &'expressions AstExpressionBuilder,
        visitor: V,
        walk_absent_supplied_guard_body: bool,
    ) -> Self {
        Self {
            expressions,
            visitor,
            completed_views: HashSet::new(),
            active_injected_views: HashSet::new(),
            wrapper_fill_targets: HashMap::new(),
            walk_absent_supplied_guard_body,
        }
    }
}

/// Inherited child content injected at a wrapper's loose-fill slot.
struct Injection<'key, 'expressions, V> {
    key: &'key SlotKey,
    walk: &'key mut dyn FnMut(
        &mut PayloadWalkState<'expressions, V>,
    ) -> Result<(), InfrastructureFailure>,
}

fn effective<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    site: super::ids::ExpressionSiteId,
    structural: ExprId,
) -> Result<ExpressionView<'expressions>, InfrastructureFailure> {
    match view.effective_expression_for_site(expressions, site)? {
        Some(expression) => Ok(expression),
        None => expressions.view(structural),
    }
}

/// Walks one template root through the effective render edges the runtime handoff materializes.
///
/// WHAT: follows a runtime slot plan's wrapper root, contribution sources and slot sites in
///       emitted order, then any template-level conditional wrappers.
/// WHY: generated failures inside plan-owned render roots are not structural children, so a
///      root-only walk would let HIR discover work the AST never classified.
fn walk_view<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    inherited_active_plan: Option<TemplateSlotPlanId>,
    injection: Option<&mut Injection<'_, 'expressions, V>>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    let identity = view.identity();
    let has_injection = injection.is_some();
    if has_injection {
        if !state.active_injected_views.insert(identity) {
            return Ok(());
        }
    } else if !state.completed_views.insert(identity) {
        return Ok(());
    }

    let result = (|| {
        let (root, runtime_slot_plan, conditional_child_wrapper_set) = {
            let template = view.root_template()?;
            (
                template.root,
                template.runtime_slot_plan,
                template.conditional_child_wrapper_set,
            )
        };

        if let Some(slot_plan_id) = runtime_slot_plan {
            walk_node(state, view, root, Some(slot_plan_id), injection)?;

            let slot_plan = view.store().get_slot_plan(slot_plan_id).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR payload walk referenced a missing runtime slot plan.",
                )
            })?;
            for source in &slot_plan.contribution_sources {
                walk_node(state, view, source.render_root, Some(slot_plan_id), None)?;
            }
            for site in &slot_plan.slot_sites {
                walk_node(state, view, site.render_root, Some(slot_plan_id), None)?;
            }
        } else {
            walk_node(state, view, root, inherited_active_plan, injection)?;
        }

        if let Some(wrapper_set_id) = conditional_child_wrapper_set {
            let wrapper_set = view
                .store()
                .get_wrapper_set(wrapper_set_id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "TIR payload walk referenced a missing conditional child wrapper set.",
                    )
                })?;
            if !wrapper_set.wrappers.is_empty() {
                let mut no_child = |_state: &mut PayloadWalkState<'expressions, V>| Ok(());
                walk_wrapper_layers(
                    state,
                    view,
                    &wrapper_set.wrappers,
                    &mut no_child,
                    inherited_active_plan,
                )?;
            }
        }

        Ok(())
    })();

    if has_injection {
        state.active_injected_views.remove(&identity);
    }
    result
}

/// Walks one effective node and its render children in emitted order.
fn walk_node<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    node: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    mut injection: Option<&mut Injection<'_, 'expressions, V>>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    match &view.effective_node(node)?.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for &child in children {
                walk_node(
                    state,
                    view,
                    child,
                    active_slot_plan,
                    injection.as_deref_mut(),
                )?;
            }
        }
        TemplateIrNodeKind::DynamicExpression {
            expression,
            site_id,
            ..
        } => {
            let expressions = state.expressions;
            visit_expression(view, expressions, *site_id, *expression, &mut state.visitor)?;
        }
        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            let expressions = state.expressions;
            visit_expression(
                view,
                expressions,
                *selector_site_id,
                selector.condition_expression(),
                &mut state.visitor,
            )?;
            walk_node(state, view, *body, active_slot_plan, injection)?;
        }
        TemplateIrNodeKind::SuppliedGuard { body, .. } => {
            // A guard that reaches the runtime handoff is absent: composition replaces a
            // selected guard with its body. Its body therefore emits nothing and owns no
            // executable failure at this position, but validation still walks authored code.
            if state.walk_absent_supplied_guard_body {
                walk_node(state, view, *body, active_slot_plan, injection)?;
            }
        }
        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
            ..
        } => {
            let expressions = state.expressions;
            if let Some(failure_site) =
                visit_loop_header(view, expressions, header, *header_sites, &mut state.visitor)?
            {
                (state.visitor)(TemplatePayload::LoopFailure(failure_site))?;
            }
            walk_node(
                state,
                view,
                *body,
                active_slot_plan,
                injection.as_deref_mut(),
            )?;
            if let Some(wrapper) = aggregate_wrapper {
                walk_node(
                    state,
                    view,
                    *wrapper,
                    active_slot_plan,
                    injection.as_deref_mut(),
                )?;
            }
        }
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let child_view = view.structural_child(*reference)?;
            walk_child_with_wrapper_context(
                state,
                view,
                *occurrence_id,
                &child_view,
                active_slot_plan,
                injection,
            )?;
        }
        TemplateIrNodeKind::InsertContribution { template } => {
            let helper_view = view.structural_helper(*template)?;
            walk_view(state, &helper_view, active_slot_plan, None)?;
        }
        TemplateIrNodeKind::Slot { placeholder } => {
            walk_slot(state, view, placeholder, active_slot_plan, injection)?;
        }
        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. }
        | TemplateIrNodeKind::RetainedNumericIntake(_) => {}
    }
    Ok(())
}

/// Walks one child template, applying its effective inherited wrapper context around it.
fn walk_child_with_wrapper_context<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    parent_view: &TirView<'_>,
    occurrence_id: ChildTemplateOccurrenceId,
    child_view: &TirView<'_>,
    active_slot_plan: Option<TemplateSlotPlanId>,
    mut injection: Option<&mut Injection<'_, 'expressions, V>>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    let Some(context) = parent_view.effective_wrapper_context(occurrence_id)? else {
        return walk_view(state, child_view, active_slot_plan, injection);
    };
    if context.skip_parent_child_wrappers {
        return walk_view(state, child_view, active_slot_plan, injection);
    }
    let Some(wrapper_set_id) = context.inherited_wrapper_set else {
        return walk_view(state, child_view, active_slot_plan, injection);
    };
    let wrapper_set = parent_view
        .store()
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR payload walk referenced a missing inherited wrapper set.",
            )
        })?;

    match context.application_mode {
        TirWrapperApplicationMode::Always => {
            let mut child = |state: &mut PayloadWalkState<'expressions, V>| {
                walk_view(
                    state,
                    child_view,
                    active_slot_plan,
                    injection.as_deref_mut(),
                )
            };
            walk_wrapper_layers(
                state,
                parent_view,
                &wrapper_set.wrappers,
                &mut child,
                active_slot_plan,
            )
        }
        TirWrapperApplicationMode::IfChildEmits => {
            walk_view(state, child_view, active_slot_plan, injection)?;
            let mut no_child = |_state: &mut PayloadWalkState<'expressions, V>| Ok(());
            walk_wrapper_layers(
                state,
                parent_view,
                &wrapper_set.wrappers,
                &mut no_child,
                active_slot_plan,
            )
        }
    }
}

/// Walks one slot occurrence: injected child first, then routed sources, then authored fallback.
fn walk_slot<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    placeholder: &TirSlotPlaceholder,
    active_slot_plan: Option<TemplateSlotPlanId>,
    injection: Option<&mut Injection<'_, 'expressions, V>>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    if placeholder.closed {
        // A closed occurrence keeps its authored fallback and accepts no routed content.
        return Ok(());
    }

    if let Some(injection) = injection
        && placeholder.key == *injection.key
    {
        walk_slot_injection(state, view, placeholder, injection.walk, active_slot_plan)?;
        return Ok(());
    }

    let Some(resolution) = view.effective_slot_resolution(placeholder.occurrence_id)? else {
        // An authored fallback is constant text, which owns no executable failure.
        return Ok(());
    };
    if let TirSlotResolutionKind::Resolved { sources } = &resolution.kind {
        for source in sources {
            let source_view = view.resolved_slot_source(*source)?;
            walk_view(state, &source_view, active_slot_plan, None)?;
        }
    }
    Ok(())
}

/// Applies a slot occurrence's wrapper sets around injected child content.
fn walk_slot_injection<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    placeholder: &TirSlotPlaceholder,
    child: &mut dyn FnMut(
        &mut PayloadWalkState<'expressions, V>,
    ) -> Result<(), InfrastructureFailure>,
    active_slot_plan: Option<TemplateSlotPlanId>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    let applied_child_wrapper_set = if placeholder.skip_parent_child_wrappers {
        None
    } else {
        placeholder.applied_child_wrapper_set
    };

    match (placeholder.child_wrapper_set, applied_child_wrapper_set) {
        (Some(inner), Some(outer)) => {
            let mut inner_child = |state: &mut PayloadWalkState<'expressions, V>| {
                walk_wrapper_set_injection(state, view, inner, &mut *child, active_slot_plan)
            };
            walk_wrapper_set_injection(state, view, outer, &mut inner_child, active_slot_plan)
        }
        (Some(wrapper_set_id), None) | (None, Some(wrapper_set_id)) => {
            walk_wrapper_set_injection(state, view, wrapper_set_id, child, active_slot_plan)
        }
        (None, None) => child(state),
    }
}

fn walk_wrapper_set_injection<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    wrapper_set_id: crate::compiler_frontend::ast::templates::tir::ids::TemplateWrapperSetId,
    child: &mut dyn FnMut(
        &mut PayloadWalkState<'expressions, V>,
    ) -> Result<(), InfrastructureFailure>,
    active_slot_plan: Option<TemplateSlotPlanId>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    let wrapper_set = view
        .store()
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR payload walk referenced a missing slot wrapper set.",
            )
        })?;
    walk_wrapper_layers(state, view, &wrapper_set.wrappers, child, active_slot_plan)
}

/// Walks wrapper templates outermost-first, nesting each inner layer at the fill slot.
///
/// WHAT: `wrappers` is stored innermost-first, so the last element wraps the rest; the walk
///       enters it first and injects the remaining layers at its loose-fill slot.
/// WHY: this mirrors slot injection around a child, preserving the emitted failure order.
fn walk_wrapper_layers<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    wrappers: &[TemplateWrapperReference],
    child: &mut dyn FnMut(
        &mut PayloadWalkState<'expressions, V>,
    ) -> Result<(), InfrastructureFailure>,
    active_slot_plan: Option<TemplateSlotPlanId>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    let Some((outer, inner)) = wrappers.split_last() else {
        return child(state);
    };

    let mut inner_child = |state: &mut PayloadWalkState<'expressions, V>| {
        walk_wrapper_layers(state, view, inner, &mut *child, active_slot_plan)
    };
    walk_single_wrapper(state, view, *outer, &mut inner_child, active_slot_plan)
}

/// Walks one wrapper template with inherited content injected at its loose-fill slot.
fn walk_single_wrapper<'expressions, V>(
    state: &mut PayloadWalkState<'expressions, V>,
    view: &TirView<'_>,
    reference: TemplateWrapperReference,
    child: &mut dyn FnMut(
        &mut PayloadWalkState<'expressions, V>,
    ) -> Result<(), InfrastructureFailure>,
    active_slot_plan: Option<TemplateSlotPlanId>,
) -> Result<(), InfrastructureFailure>
where
    V: FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
{
    let wrapper_view = view.wrapper(reference)?;
    let root = wrapper_view.root_template()?.root;
    let fill_target = match state.wrapper_fill_targets.get(&reference) {
        Some(fill_target) => fill_target.clone(),
        None => {
            let schema = collect_tir_slot_schema_with_context(
                view.store(),
                reference.root,
                reference.context,
            )?;
            let fill_target = schema.loose_fill_target_key();
            state
                .wrapper_fill_targets
                .insert(reference, fill_target.clone());
            fill_target
        }
    };

    match fill_target.as_ref() {
        Some(key) => {
            let mut matched = false;
            let mut inject = |state: &mut PayloadWalkState<'expressions, V>| {
                matched = true;
                child(state)
            };
            let mut injection = Injection {
                key,
                walk: &mut inject,
            };
            walk_node(
                state,
                &wrapper_view,
                root,
                active_slot_plan,
                Some(&mut injection),
            )?;
            if !matched {
                // The schema promised a loose fill target. Walk the injected content anyway so
                // a malformed wrapper cannot hide generated failures from classification.
                child(state)?;
            }
        }
        None => {
            // A slotless or named-only wrapper has no fill target, so the handoff appends the
            // child after the wrapper content.
            walk_node(state, &wrapper_view, root, active_slot_plan, None)?;
            child(state)?;
        }
    }
    Ok(())
}

/// Emits one expression payload and returns its effective ID, which loop failure sites use to
/// label the generated failure at the position HIR lowers it.
fn visit_expression<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    site: super::ids::ExpressionSiteId,
    structural: ExprId,
    visitor: &mut impl FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
) -> Result<ExprId, InfrastructureFailure> {
    let expression = effective(view, expressions, site, structural)?;
    let id = expression.id();
    visitor(TemplatePayload::Expression(expression))?;
    Ok(id)
}

/// Visits loop header operands in evaluation order and returns the generated-failure inputs the
/// loop owns once those operands are evaluated. Conditional loops own none.
fn visit_loop_header<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    header: &TemplateLoopHeader,
    sites: TemplateLoopHeaderExpressionSites,
    visitor: &mut impl FnMut(TemplatePayload<'expressions>) -> Result<(), InfrastructureFailure>,
) -> Result<Option<LoopFailureSite>, InfrastructureFailure> {
    match (header, sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site },
        ) => {
            visit_expression(view, expressions, site, *condition, visitor)?;
            Ok(None)
        }
        (
            TemplateLoopHeader::Collection { iterable, step, .. },
            TemplateLoopHeaderExpressionSites::Collection {
                iterable: site,
                step: step_site,
            },
        ) => {
            visit_expression(view, expressions, site, *iterable, visitor)?;
            let step = match (step, step_site) {
                (Some(step), Some(site)) => {
                    Some(visit_expression(view, expressions, site, *step, visitor)?)
                }
                (None, None) => None,
                _ => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "TIR expression walk found mismatched collection step site.",
                    ));
                }
            };
            Ok(Some(LoopFailureSite::Collection { step }))
        }
        (
            TemplateLoopHeader::Range { bindings, range },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            let start_expression =
                visit_expression(view, expressions, start, range.start, visitor)?;
            visit_expression(view, expressions, end, range.end, visitor)?;
            let step_expression = match (step, range.step) {
                (Some(site), Some(expression)) => Some(visit_expression(
                    view,
                    expressions,
                    site,
                    expression,
                    visitor,
                )?),
                (None, None) => None,
                _ => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "TIR expression walk found mismatched range step site.",
                    ));
                }
            };
            Ok(Some(LoopFailureSite::Range {
                domain: range.domain,
                fast_math: range.fast_math,
                has_index_binding: bindings.index.is_some(),
                start: start_expression,
                step: step_expression,
            }))
        }
        _ => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "TIR expression walk found mismatched loop-header expression sites.",
        )),
    }
}
