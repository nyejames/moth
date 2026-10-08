//! Validation entry points for structured template control flow.
//!
//! Runtime-capable templates are validated for escaped helper artifacts that
//! should have been composed or routed into AST-owned slot application plans.
//!
//! Const-required foldability has no entry point here. Construction prepares the
//! const view once through `TemplatePreparationMode::ConstRequired` and carries
//! that proof into folding, so a second validation entry would be a second
//! classifier of the same view.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplatePreparationMode,
    TemplateTirPhase, TirView, TirViewIdentity, prepare_tir_view,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use std::collections::HashSet;

/// Rejects slot composition artifacts that would otherwise reach runtime
/// control-flow lowering.
///
/// Compile-time-required callers do not run this check, because slots can still
/// be resolved or folded before runtime; their proof is the const-mode
/// preparation construction already performed. This runtime-only check runs
/// after composition/formatting, when any remaining slot or insertion inside a
/// control-flow body would otherwise become a HIR invariant failure.
///
/// WHAT: constructs one required module-store `TirView` and validates every
///       reachable control-flow body through that view. Missing module store,
///       template, root, node or overlay authority propagates as an internal
///       error rather than a silent no-op.
pub(crate) fn validate_runtime_template_control_flow_slot_artifacts(
    template: &Template,
    tir_store: &TemplateIrStore,
) -> Result<(), TemplateError> {
    let view = runtime_tir_view_for_template(template, tir_store)?;
    validate_runtime_tir_view_control_flow_slot_artifacts(&view)
}

/// Constructs the required module-store `TirView` for runtime artifact
/// validation.
///
/// WHAT: validates the durable reference against the module store before
///       constructing the effective view. Runtime validation runs during
///       template construction, so any post-parse phase is sufficient; we do not
///       require `Finalized` here. Missing authority is an internal compiler
///       error, not permission to fall back to a raw store walk.
fn runtime_tir_view_for_template<'a>(
    template: &Template,
    tir_store: &'a TemplateIrStore,
) -> Result<TirView<'a>, TemplateError> {
    let reference = &template.tir_reference;

    TirView::new(
        tir_store,
        reference.root,
        reference.phase,
        reference.context,
    )
    .map_err(TemplateError::from)
}

/// Validates every reachable runtime control-flow body through a module-store
/// `TirView`.
///
/// WHAT: walks the view's structural tree, checking `Conditional` and `Loop`
///       bodies for escaped `$insert(...)` contributions. Receiver `$slot`
///       markers may remain until a later wrapper or parent routes them.
///       Nested child-template traversal descends through module-store child
///       views, preserving each child reference's exact root, phase and overlay
///       identity.
/// WHY: the `TirView` is the sole production read path for runtime artifact
///      validation; overlay resolution stays centralized and child authority
///      propagates as an internal error when missing.
fn validate_runtime_tir_view_control_flow_slot_artifacts(
    view: &TirView<'_>,
) -> Result<(), TemplateError> {
    // Render-unit validation can still receive parser-owned Parsed TIR. The
    // complete preparation proof begins at Composed, so retain the narrow
    // structural check for that earlier construction boundary.
    if view.phase().is_at_least(TemplateTirPhase::Composed) {
        let preparation = prepare_tir_view(view, TemplatePreparationMode::Value)?;
        if !preparation.facts.has_escaped_insert_helpers {
            return Ok(());
        }
    }

    let root_node_id = view.root_template()?.root;
    let mut visiting = HashSet::from([view.identity()]);

    validate_runtime_tir_view_node(view, root_node_id, &mut visiting)
}

/// Validates every reachable runtime control-flow body in a module-store view.
///
/// WHAT: walks the structural tree from `node_ref`. For each `Conditional` and
///       `Loop` body, checks for escaped `$insert(...)` contributions. Recurses
///       through `Sequence`, control-flow bodies,
///       aggregate wrappers and nested child views. Missing effective-node
///       authority propagates as an internal error.
fn validate_runtime_tir_view_node(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<(), TemplateError> {
    let node = view.effective_node(node_ref)?;
    match &node.kind {
        TemplateIrNodeKind::Conditional { body, .. } => {
            let body = *body;
            let node_span = node.span;

            validate_runtime_tir_view_control_flow_body(view, body, node_span)?;
            validate_runtime_tir_view_node(view, body, visiting)?;
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let body = *body;
            let aggregate_wrapper = *aggregate_wrapper;
            let node_span = node.span;

            validate_runtime_tir_view_control_flow_body(view, body, node_span)?;
            validate_runtime_tir_view_node(view, body, visiting)?;

            if let Some(wrapper_id) = aggregate_wrapper {
                validate_runtime_tir_view_node(view, wrapper_id, visiting)?;
            }
        }

        TemplateIrNodeKind::Sequence { children } => {
            for &child in children {
                validate_runtime_tir_view_node(view, child, visiting)?;
            }
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_view = view.structural_child(*reference)?;
            validate_runtime_qualified_child_view(child_view, visiting)?;
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let helper_view = view.structural_helper(*template)?;
            validate_runtime_qualified_child_view(helper_view, visiting)?;
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {}
    }

    Ok(())
}

/// Recurses into a module-store child view to validate nested control-flow
/// bodies.
///
/// WHAT: receives the exact child `TirView` produced by the caller's named
///       structural transition, then recurses into
///       [`validate_runtime_tir_view_node`]. The cycle key prevents infinite
///       recursion through mutually-referencing child templates.
fn validate_runtime_qualified_child_view(
    child_view: TirView<'_>,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<(), TemplateError> {
    let cycle_key = child_view.identity();
    if !visiting.insert(cycle_key) {
        return Ok(());
    }

    let child_root_node = child_view.root_template()?.root;
    let result = validate_runtime_tir_view_node(&child_view, child_root_node, visiting);

    visiting.remove(&cycle_key);
    result
}

/// Checks a control-flow body root for escaped inserts.
///
/// Receiver `$slot` markers may remain until later routing or wrapper fill
/// injection. Escaped `$insert(...)` helpers stay invalid.
fn validate_runtime_tir_view_control_flow_body(
    view: &TirView<'_>,
    body_root: TemplateIrNodeId,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Result<(), TemplateError> {
    let mut escaped_insert_visiting = HashSet::from([view.identity()]);

    if tir_view_subtree_contains_escaped_insert(view, body_root, &mut escaped_insert_visiting)? {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
            span,
        )
        .into());
    }

    Ok(())
}

/// Returns true when the subtree rooted at `node_ref` contains an escaped
/// `$insert(...)` helper.
///
/// WHAT: follows structural children through the effective view. Slot
///       placeholders remain owned by later slot routing, so their optional
///       resolution is outside this escaped-insert query. Child-template and
///       insert-contribution references are resolved through their exact
///       module-store child views. Missing node or child-view authority
///       propagates as an internal error.
fn tir_view_subtree_contains_escaped_insert(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<bool, TemplateError> {
    let node = view.effective_node(node_ref)?;
    match &node.kind {
        TemplateIrNodeKind::Slot { .. } => Ok(false),

        TemplateIrNodeKind::Sequence { children } => {
            for &child in children {
                if tir_view_subtree_contains_escaped_insert(view, child, visiting)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }

        TemplateIrNodeKind::Conditional { body, .. } => {
            tir_view_subtree_contains_escaped_insert(view, *body, visiting)
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let body = *body;
            let aggregate_wrapper = *aggregate_wrapper;

            if tir_view_subtree_contains_escaped_insert(view, body, visiting)? {
                return Ok(true);
            }

            if let Some(wrapper_id) = aggregate_wrapper
                && tir_view_subtree_contains_escaped_insert(view, wrapper_id, visiting)?
            {
                return Ok(true);
            }

            Ok(false)
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_view = view.structural_child(*reference)?;
            runtime_child_view_contains_escaped_insert(child_view, visiting)
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let helper_view = view.structural_helper(*template)?;
            runtime_child_view_contains_escaped_insert(helper_view, visiting)
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(false),
    }
}

/// Checks a module-store child view for an escaped insert.
///
/// WHAT: receives a child `TirView` from the caller's named structural
///       transition. A child template whose kind is `SlotInsert` is itself an
///       escaped insert. The child view's subtree is then checked recursively.
///       The cycle key prevents infinite recursion through mutually-referencing
///       child templates.
fn runtime_child_view_contains_escaped_insert(
    child_view: TirView<'_>,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<bool, TemplateError> {
    let cycle_key = child_view.identity();
    if !visiting.insert(cycle_key) {
        return Ok(false);
    }

    let child_template = child_view.root_template()?;
    if matches!(child_template.kind, TemplateType::SlotInsert(_)) {
        visiting.remove(&cycle_key);
        return Ok(true);
    }

    let child_root_node = child_template.root;
    let result = tir_view_subtree_contains_escaped_insert(&child_view, child_root_node, visiting);

    visiting.remove(&cycle_key);
    result
}
