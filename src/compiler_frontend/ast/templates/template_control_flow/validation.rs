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
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TirView, TirViewIdentity,
};
use crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use std::collections::HashSet;

/// One escaped `$insert(...)` contribution discovered in a composed view.
///
/// WHAT: pairs the authored insertion site and constant name with the named slot the helper
///       targets.
/// WHY: a const-required template has no parent template to receive an escaped contribution, so
///      the refusal names the authored insert and its target slot instead of a generic template
///      shape.
pub(crate) struct EscapedInsertSite {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) slot_name: Option<StringId>,
    pub(crate) insert_name: Option<StringId>,
}

/// Returns the first escaped `$insert(...)` contribution in a composed view.
///
/// WHAT: walks the structural tree in authored order and returns the first contribution that no
///       wrapper consumed, together with its authored span and target slot name.
/// WHY: const-required template construction rejects this shape, and only the exact authored
///      insertion can explain which insert escaped and which slot its parent would need.
pub(crate) fn first_escaped_insert_site(
    view: &TirView<'_>,
) -> Result<Option<EscapedInsertSite>, TemplateError> {
    let root_node_id = view.root_template()?.root;
    let mut visiting = HashSet::from([view.identity()]);
    tir_view_subtree_first_escaped_insert(view, root_node_id, &mut visiting)
}

/// Rejects slot composition artifacts that would otherwise reach runtime
/// control-flow lowering.
///
/// Compile-time-required callers do not run this check because their prepared
/// view is validated and folded through the const-mode path. Runtime
/// construction gates the walk on preparation's escaped-insert fact so it can
/// locate any helper that remains inside a control-flow body.
///
/// WHAT: constructs the exact module-store `TirView` from the template's
///       durable reference and checks every reachable control-flow body.
///       Missing template, root, node or overlay authority propagates as an
///       internal error rather than a silent no-op.
pub(crate) fn validate_runtime_template_control_flow_slot_artifacts(
    template: &Template,
    tir_store: &TemplateIrStore,
) -> Result<(), TemplateError> {
    let reference = &template.tir_reference;
    let view = TirView::new(
        tir_store,
        reference.root,
        reference.phase,
        reference.context,
    )
    .map_err(TemplateError::from)?;
    validate_runtime_tir_view_control_flow_slot_artifacts(&view)
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
        // A retained position is pending compiler construction data with no payload and no
        // children: body-root completion replaces it before the runtime template is published.
        TemplateIrNodeKind::RetainedNumericIntake(_) => {}
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
    span: Option<SourceSpan>,
) -> Result<(), TemplateError> {
    let mut escaped_insert_visiting = HashSet::from([view.identity()]);

    if tir_view_subtree_first_escaped_insert(view, body_root, &mut escaped_insert_visiting)?
        .is_some()
    {
        return Err(templates::invalid_template_structure(
            InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
            span,
        )
        .into());
    }

    Ok(())
}

/// Returns the first escaped `$insert(...)` helper in the subtree rooted at `node_ref`.
///
/// WHAT: follows structural children through the effective view. Slot
///       placeholders remain owned by later slot routing, so their optional
///       resolution is outside this escaped-insert query. Child-template and
///       insert-contribution references are resolved through their exact
///       module-store child views. Missing node or child-view authority
///       propagates as an internal error.
/// WHY: returning the site itself lets a const-required refusal name the
///      authored insertion while the runtime check only needs its presence.
fn tir_view_subtree_first_escaped_insert(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<Option<EscapedInsertSite>, TemplateError> {
    let node = view.effective_node(node_ref)?;
    let node_span = node.span;
    match &node.kind {
        TemplateIrNodeKind::Slot { .. } => Ok(None),

        TemplateIrNodeKind::Sequence { children } => {
            for &child in children {
                if let Some(site) = tir_view_subtree_first_escaped_insert(view, child, visiting)? {
                    return Ok(Some(site));
                }
            }
            Ok(None)
        }

        TemplateIrNodeKind::Conditional { body, .. } => {
            tir_view_subtree_first_escaped_insert(view, *body, visiting)
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let body = *body;
            let aggregate_wrapper = *aggregate_wrapper;

            if let Some(site) = tir_view_subtree_first_escaped_insert(view, body, visiting)? {
                return Ok(Some(site));
            }

            match aggregate_wrapper {
                Some(wrapper_id) => {
                    tir_view_subtree_first_escaped_insert(view, wrapper_id, visiting)
                }
                None => Ok(None),
            }
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_view = view.structural_child(*reference)?;
            runtime_child_view_first_escaped_insert(child_view, node_span, None, visiting)
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let helper_view = view.structural_helper(*template)?;
            let insert_name = view.store().insert_contribution_name(node_ref);
            runtime_child_view_first_escaped_insert(helper_view, node_span, insert_name, visiting)
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(None),
        // A retained position holds no insert helper: its value is installed by body-root
        // completion, and this scan runs on published runtime templates.
        TemplateIrNodeKind::RetainedNumericIntake(_) => Ok(None),
    }
}

/// Checks a module-store child view for an escaped insert.
///
/// WHAT: receives a child `TirView` from the caller's named structural
///       transition. A child template whose kind is `SlotInsert` is itself an
///       escaped insert, reported at the caller's authored node span. The child
///       view's subtree is then checked recursively. The cycle key prevents
///       infinite recursion through mutually-referencing child templates.
fn runtime_child_view_first_escaped_insert(
    child_view: TirView<'_>,
    authored_span: Option<SourceSpan>,
    insert_name: Option<StringId>,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<Option<EscapedInsertSite>, TemplateError> {
    let cycle_key = child_view.identity();
    if !visiting.insert(cycle_key) {
        return Ok(None);
    }

    let child_template = child_view.root_template()?;
    let slot_name = match &child_template.kind {
        TemplateType::SlotInsert(SlotKey::Named(name)) => Some(Some(*name)),
        TemplateType::SlotInsert(_) => Some(None),
        _ => None,
    };
    if let Some(slot_name) = slot_name {
        visiting.remove(&cycle_key);
        return Ok(Some(EscapedInsertSite {
            span: authored_span,
            slot_name,
            insert_name,
        }));
    }

    let child_root_node = child_template.root;
    let result = tir_view_subtree_first_escaped_insert(&child_view, child_root_node, visiting);

    visiting.remove(&cycle_key);
    result
}
