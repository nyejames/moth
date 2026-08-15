//! Validation entry points for structured template control flow.
//!
//! Runtime-capable templates are validated for escaped helper artifacts that
//! should have been composed or routed into AST-owned slot application plans.
//! Const-required templates are validated for full foldability while still
//! allowing slot/helper structure that a parent const template may compose
//! before folding.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::tir::{
    PreparedTemplate, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
    TemplatePreparationMode, TemplateTirPhase, TirView, TirViewIdentity, prepare_tir_view,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::collections::HashSet;

/// Validates structured template control flow in a const-required context.
///
/// Runtime-capable templates keep structured control flow for HIR lowering.
/// Const-required callers use this narrower entry point so supported forms can
/// fold now while unsupported const shapes produce source diagnostics instead of
/// leaking as infrastructure errors during finalization.
///
/// WHAT: validates through the template's module-store Composed-or-later
///       `TirView` so expression overlays and module-local children are authoritative.
/// WHY: every production const-required template has completed construction before
///      this entry runs. Missing view authority is therefore a compiler invariant,
///      not permission to fall back to a raw store walk.
pub(crate) fn validate_const_required_template_control_flow(
    template: &Template,
    tir_store: &TemplateIrStore,
) -> Result<PreparedTemplate, TemplateError> {
    let reference = &template.tir_reference;
    let view = TirView::with_minimum_phase(
        tir_store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )
    .map_err(TemplateError::from)?;

    prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)
}

/// Rejects slot composition artifacts that would otherwise reach runtime
/// control-flow lowering.
///
/// Compile-time-required callers use the const validator above because slots can
/// still be resolved or folded before runtime. This runtime-only check runs
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

#[derive(Clone, Copy)]
enum RuntimeControlFlowArtifact {
    UnresolvedSlot,
    EscapedInsert,
}

/// Validates every reachable runtime control-flow body through a module-store
/// `TirView`.
///
/// WHAT: walks the view's structural tree, checking `BranchChain` and `Loop`
///       bodies for unresolved slots and escaped `$insert(...)` contributions.
///       Slot occurrences are checked against the view's effective slot-resolution
///       overlay so resolved slots are not falsely reported as artifacts.
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
/// WHAT: walks the structural tree from `node_ref`. For each `BranchChain` and
///       `Loop` body, checks for unresolved slots and escaped `$insert(...)`
///       contributions. Recurses through `Sequence`, control-flow bodies,
///       aggregate wrappers and nested child views. Missing effective-node
///       authority propagates as an internal error.
fn validate_runtime_tir_view_node(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<(), TemplateError> {
    let node = view.effective_node(node_ref)?;
    match &node.kind {
        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let branches = branches.clone();
            let fallback = *fallback;
            let node_location = node.location.clone();

            for branch in branches {
                validate_runtime_tir_view_control_flow_body(view, branch.body, &branch.location)?;
                validate_runtime_tir_view_node(view, branch.body, visiting)?;
            }

            if let Some(fallback_id) = fallback {
                validate_runtime_tir_view_control_flow_body(view, fallback_id, &node_location)?;
                validate_runtime_tir_view_node(view, fallback_id, visiting)?;
            }
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let body = *body;
            let aggregate_wrapper = *aggregate_wrapper;
            let node_location = node.location.clone();

            validate_runtime_tir_view_control_flow_body(view, body, &node_location)?;
            validate_runtime_tir_view_node(view, body, visiting)?;

            if let Some(wrapper_id) = aggregate_wrapper {
                validate_runtime_tir_view_node(view, wrapper_id, visiting)?;
            }
        }

        TemplateIrNodeKind::Sequence { children } => {
            let children = children.clone();
            for child in children {
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
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. } => {}
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

/// Checks a control-flow body root for unresolved slots and escaped inserts.
///
/// WHAT: runs two independent artifact scans over the body subtree, each with a
///       fresh cycle set, so a child view checked for one artifact kind is still
///       checked for the other.
fn validate_runtime_tir_view_control_flow_body(
    view: &TirView<'_>,
    body_root: TemplateIrNodeId,
    location: &SourceLocation,
) -> Result<(), TemplateError> {
    let body_ref = body_root;
    let root_cycle_key = view.identity();
    let mut escaped_insert_visiting = HashSet::from([root_cycle_key]);

    if tir_view_subtree_contains_runtime_artifact(
        view,
        body_ref,
        RuntimeControlFlowArtifact::EscapedInsert,
        &mut escaped_insert_visiting,
    )? {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
            location.clone(),
        )
        .into());
    }

    let mut unresolved_slot_visiting = HashSet::from([root_cycle_key]);
    if tir_view_subtree_contains_runtime_artifact(
        view,
        body_ref,
        RuntimeControlFlowArtifact::UnresolvedSlot,
        &mut unresolved_slot_visiting,
    )? {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedSlot,
            location.clone(),
        )
        .into());
    }

    Ok(())
}

/// Returns true when the subtree rooted at `node_ref` contains the requested
/// runtime artifact.
///
/// WHAT: walks the structural tree through the view's effective nodes. For
///       `Slot` nodes, checks the effective slot-resolution overlay. For
///       `ChildTemplate` and `InsertContribution` nodes, descends through
///       module-store child views, preserving each child reference's exact
///       root, phase and overlay identity. Missing effective-node or child-view
///       authority propagates as an internal error.
fn tir_view_subtree_contains_runtime_artifact(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    artifact: RuntimeControlFlowArtifact,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<bool, TemplateError> {
    let node = view.effective_node(node_ref)?;
    match &node.kind {
        TemplateIrNodeKind::Slot { placeholder } => {
            let occurrence_id = placeholder.occurrence_id;

            if !matches!(artifact, RuntimeControlFlowArtifact::UnresolvedSlot) {
                return Ok(false);
            }

            let resolution = view.effective_slot_resolution(occurrence_id)?;
            let is_resolved = resolution.is_some_and(|r| !r.is_unresolved());

            Ok(!is_resolved)
        }

        TemplateIrNodeKind::Sequence { children } => {
            let children = children.clone();
            for child in children {
                if tir_view_subtree_contains_runtime_artifact(view, child, artifact, visiting)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let bodies: Vec<_> = branches.iter().map(|branch| branch.body).collect();
            let fallback = *fallback;

            for body in bodies {
                if tir_view_subtree_contains_runtime_artifact(view, body, artifact, visiting)? {
                    return Ok(true);
                }
            }

            if let Some(fallback) = fallback
                && tir_view_subtree_contains_runtime_artifact(view, fallback, artifact, visiting)?
            {
                return Ok(true);
            }

            Ok(false)
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let body = *body;
            let aggregate_wrapper = *aggregate_wrapper;

            if tir_view_subtree_contains_runtime_artifact(view, body, artifact, visiting)? {
                return Ok(true);
            }

            if let Some(wrapper_id) = aggregate_wrapper
                && tir_view_subtree_contains_runtime_artifact(view, wrapper_id, artifact, visiting)?
            {
                return Ok(true);
            }

            Ok(false)
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_view = view.structural_child(*reference)?;
            runtime_child_view_contains_artifact(child_view, artifact, visiting)
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let helper_view = view.structural_helper(*template)?;
            runtime_child_view_contains_artifact(helper_view, artifact, visiting)
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. } => Ok(false),
    }
}

/// Checks a module-store child view for the requested runtime artifact.
///
/// WHAT: receives a child `TirView` from the caller's named structural
///       transition. For `EscapedInsert`, a child template
///       whose kind is `SlotInsert` is itself an escaped insert. The child view's
///       subtree is then checked recursively. The cycle key prevents infinite
///       recursion through mutually-referencing child templates.
fn runtime_child_view_contains_artifact(
    child_view: TirView<'_>,
    artifact: RuntimeControlFlowArtifact,
    visiting: &mut HashSet<TirViewIdentity>,
) -> Result<bool, TemplateError> {
    let cycle_key = child_view.identity();
    if !visiting.insert(cycle_key) {
        return Ok(false);
    }

    if matches!(artifact, RuntimeControlFlowArtifact::EscapedInsert) {
        let child_template = child_view.root_template()?;
        if matches!(child_template.kind, TemplateType::SlotInsert(_)) {
            visiting.remove(&cycle_key);
            return Ok(true);
        }
    }

    let child_root_node = child_view.root_template()?.root;
    let result = tir_view_subtree_contains_runtime_artifact(
        &child_view,
        child_root_node,
        artifact,
        visiting,
    );

    visiting.remove(&cycle_key);
    result
}
