//! TIR render-unit construction helpers.
//!
//! WHAT: owns TIR aggregate-wrapper candidate root construction, branch/fallback
//! body candidate root construction, body-root formatting and loop-control
//! boundary trimming.
//!
//! WHY: localizes the link between AST aggregate placeholders and TIR-native
//! loop aggregate wrappers. Candidate roots are returned as `TemplateIrNodeId`
//! directly, avoiding scratch `TemplateIr` entries in the durable store.
//! Aggregate-wrapper construction consumes parser TIR and module-local child
//! references directly, keeping parser-emitted TIR as the only structural
//! authority during render-unit preparation.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Style;

use crate::compiler_frontend::ast::templates::tir::formatter_view::{
    TirFormatterResult, format_tir_body_node_root, format_tir_template,
};
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIrNode, TemplateIrNodeKind};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::slot_composition::compose_tir_head_chain_from_root;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

// ------------------------------
//  Aggregate-wrapper candidates
// ------------------------------

/// Builds a loop aggregate-wrapper candidate root node for head-chain
/// composition.
///
/// WHAT: reuses the owning template's already-materialized head-prefix TIR
///       nodes and appends a compiler-internal `AggregateOutput` node as the
///       body fill. Returns the root sequence node ID directly.
/// WHY: loop aggregate wrapping consumes the exact nodes that the parser
///      already emitted. Composing from the root node avoids a scratch
///      `TemplateIr` that would remain in the durable store without a
///      referencing identity.
pub(in crate::compiler_frontend::ast::templates) fn build_aggregate_wrapper_candidate_root_from_tir_nodes(
    head_prefix_nodes: &[TemplateIrNodeId],
    store: &mut TemplateIrStore,
) -> Result<TemplateIrNodeId, TemplateError> {
    let mut children = Vec::with_capacity(head_prefix_nodes.len() + 1);
    let root_location =
        head_prefix_node_location(store, head_prefix_nodes).map_err(TemplateError::from)?;

    for &node_id in head_prefix_nodes {
        children.push(node_id);
    }

    children.push(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::AggregateOutput,
        root_location.to_owned(),
    )));

    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children },
        root_location,
    )))
}

/// Builds a branch/fallback body-root candidate node for head-chain composition.
///
/// WHAT: reuses the owning template's parser-emitted head-prefix TIR nodes
///       and appends the already-materialized body-only children as the body
///       fill. Returns the root sequence node ID directly.
/// WHY: branch and fallback bodies carry the shared head prefix plus their own
///      body content. Composing from the root node avoids a scratch
///      `TemplateIr` that would remain in the durable store without a
///      referencing identity.
pub(in crate::compiler_frontend::ast::templates) fn build_branch_body_candidate_root_from_tir_nodes(
    head_prefix_nodes: &[TemplateIrNodeId],
    body_children: &[TemplateIrNodeId],
    store: &mut TemplateIrStore,
) -> Result<TemplateIrNodeId, TemplateError> {
    let mut children = Vec::with_capacity(head_prefix_nodes.len() + body_children.len());
    let root_location = branch_body_candidate_location(store, head_prefix_nodes, body_children)
        .map_err(TemplateError::from)?;

    // Reuse each parser-emitted head-prefix node directly. Parser template
    // values are structural nodes before render-unit preparation begins.
    children.extend_from_slice(head_prefix_nodes);

    // The body children are already materialized in this store, so append them
    // directly. They carry Body-origin metadata so head-chain composition
    // partitions them into the body partition and applies head-prefix wrappers
    // around them.
    children.extend_from_slice(body_children);

    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children },
        root_location,
    )))
}

/// Returns the source location for an aggregate-wrapper candidate root.
///
/// WHAT: uses the first head-prefix node's location so the
///       aggregate wrapper candidate root carries a meaningful source span.
fn head_prefix_node_location(
    store: &TemplateIrStore,
    head_prefix_nodes: &[TemplateIrNodeId],
) -> Result<SourceLocation, CompilerError> {
    // An empty candidate carries no provenance, so a default location is the
    // only honest span. A selected node that is missing is an internal
    // invariant failure.
    match head_prefix_nodes.first().copied() {
        None => Ok(SourceLocation::default()),
        Some(node_id) => store
            .get_node(node_id)
            .map(|node| node.location.clone())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR aggregate-wrapper candidate: selected head-prefix node {} was missing from the store.",
                    node_id
                ))
            }),
    }
}

/// Returns the source location for a branch/fallback body candidate root.
///
/// WHAT: prefers the first shared head-prefix node and otherwise uses the first
///       prepared body child.
/// WHY: branch templates without a head prefix still need a concrete body span
///      after their candidate composition root is built.
fn branch_body_candidate_location(
    store: &TemplateIrStore,
    head_prefix_nodes: &[TemplateIrNodeId],
    body_children: &[TemplateIrNodeId],
) -> Result<SourceLocation, CompilerError> {
    // An empty candidate (no head prefix and no body children) carries no
    // provenance, so a default location is the only honest span. A selected
    // node that is missing is an internal invariant failure.
    let selected = head_prefix_nodes
        .first()
        .or_else(|| body_children.first())
        .copied();
    match selected {
        None => Ok(SourceLocation::default()),
        Some(node_id) => store
            .get_node(node_id)
            .map(|node| node.location.clone())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR branch body candidate: selected node {} was missing from the store.",
                    node_id
                ))
            }),
    }
}

// ------------------------------
//  Composed aggregate-wrapper output
// ------------------------------

/// Composed aggregate-wrapper output for TIR consumption.
pub(in crate::compiler_frontend::ast::templates) struct PreparedLoopAggregateWrapper {
    /// TIR root of the composed aggregate-wrapper subtree.
    ///
    /// WHAT: carries the internal `AggregateOutput` marker at its composed
    ///       position inside the wrapper tree.
    /// WHY: render-unit preparation installs this authoritative root directly
    ///      onto the owning `Loop` node for finalization and runtime handoff.
    pub(in crate::compiler_frontend::ast::templates) tir_root: TemplateIrNodeId,
}

// ------------------------------
//  Formatter helpers
// ------------------------------

/// Converts TIR formatter diagnostic messages into a single `TemplateError`.
///
/// WHAT: scans the formatter output for hard errors and returns the first one
///       as a `TemplateError`; when no error exists, fabricates a generic
///       compiler-error so the caller never receives an unexplained failure.
/// WHY: the formatter emits structured diagnostics; this helper provides the
///      narrow bridge from formatter messages back to template-stage errors.
fn tir_formatter_messages_to_template_error(messages: CompilerMessages) -> TemplateError {
    for diagnostic in messages.into_diagnostics() {
        if diagnostic.severity == DiagnosticSeverity::Error {
            return diagnostic.into();
        }
    }

    CompilerError::compiler_error("TIR formatter failed without returning a compiler error.").into()
}

/// Runs the TIR formatter and forwards any warnings to the active diagnostic
/// context.
///
/// WHAT: thin wrapper around `format_tir_template` that returns formatter
///       warnings with the formatted TIR result.
/// WHY: warnings are user-visible formatter behavior and must reach the caller.
pub(in crate::compiler_frontend::ast::templates) fn run_tir_formatter_with_warnings(
    store: &mut TemplateIrStore,
    root: TemplateIrId,
    phase: TemplateTirPhase,
    context: TemplateViewContext,
    style: &Style,
    scope_context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<TirFormatterResult, TemplateError> {
    let result = format_tir_template(store, root, phase, context, style, string_table)
        .map_err(tir_formatter_messages_to_template_error)?;

    for warning in &result.warnings {
        scope_context.emit_warning(warning.clone());
    }

    Ok(result)
}

/// Formats a control-flow body TIR root using the template's style formatter.
///
/// WHAT: runs the TIR-native formatter directly on the body root node without
///       pushing a scratch template. Returns the formatted root node ID.
/// WHY: control-flow body roots are nodes, not top-level templates, but they
///      still need the same formatter treatment as linear template bodies.
///      Formatting the node root directly avoids a scratch `TemplateIr` that
///      would remain in the durable store without a referencing identity.
pub(in crate::compiler_frontend::ast::templates) fn format_tir_body_root(
    body_root: TemplateIrNodeId,
    style: &Style,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<TemplateIrNodeId, TemplateError> {
    let mut store = context.template_ir_store.borrow_mut();
    let empty_context = TemplateViewContext::default();
    let result = format_tir_body_node_root(
        &mut store,
        None,
        body_root,
        TemplateTirPhase::Parsed,
        empty_context,
        style,
        string_table,
    )
    .map_err(tir_formatter_messages_to_template_error)?;

    for warning in &result.warnings {
        context.emit_warning(warning.clone());
    }

    Ok(result.root)
}

// ------------------------------
//  Sequence and whitespace helpers
// ------------------------------

/// Returns the children of a `Sequence` node, or an internal `CompilerError`
/// when the node is missing or not a sequence.
///
/// WHAT: extracts the flat child list so it can be appended after head-prefix
///       nodes in a branch/fallback body candidate.
/// WHY: parser-emitted body roots are sealed under a `Sequence` node; flattening
///      that wrapper keeps the body nodes at the same level as the head-prefix
///      nodes so head-chain composition can partition them correctly.
pub(in crate::compiler_frontend::ast::templates) fn sequence_children(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Result<Vec<TemplateIrNodeId>, CompilerError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "TIR sequence-children lookup: node {} was missing from the store.",
            node_id
        ))
    })?;
    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => Ok(children.clone()),
        _ => Err(CompilerError::compiler_error(format!(
            "TIR sequence-children lookup: node {} was not a Sequence root.",
            node_id
        ))),
    }
}

/// Returns true when a TIR node is whitespace-only literal text.
///
/// WHAT: checks the interned text of a Text node and reports whether it
///       contains only whitespace.
/// WHY: loop-control boundary trimming needs the same whitespace test used
///      during construction without sharing construction-context internals.
fn tir_node_is_whitespace_only_text(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> Result<bool, CompilerError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "TIR loop-control trim: whitespace candidate node {} was missing from the store.",
            node_id
        ))
    })?;

    let TemplateIrNodeKind::Text { text, .. } = &node.kind else {
        return Ok(false);
    };

    Ok(string_table.resolve(*text).trim().is_empty())
}

/// Trims whitespace-only text nodes that sit immediately before a top-level
/// LoopControl node in a loop body root sequence.
///
/// WHAT: applies the parser-level cleanup that strips trailing whitespace
///       before `[break]`/`[continue]` markers directly to the TIR body root.
/// WHY: loop-control boundary whitespace trimming belongs to the authoritative
///      loop body root and must reject malformed child references.
pub(in crate::compiler_frontend::ast::templates) fn trim_whitespace_before_loop_control_boundary(
    body_root: TemplateIrNodeId,
    store: &mut TemplateIrStore,
    string_table: &StringTable,
) -> Result<TemplateIrNodeId, CompilerError> {
    let (children, location) = {
        let node = store.get_node(body_root).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR loop-control trim: body root node {} was missing from the store.",
                body_root
            ))
        })?;
        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => (children.clone(), node.location.clone()),
            _ => {
                return Err(CompilerError::compiler_error(format!(
                    "TIR loop-control trim: body root node {} was not a Sequence.",
                    body_root
                )));
            }
        }
    };

    let mut new_children = Vec::with_capacity(children.len());
    let original_children_count = children.len();

    for child_id in &children {
        let child_id = *child_id;
        let child = store.get_node(child_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR loop-control trim: child node {} was missing from the store.",
                child_id
            ))
        })?;
        let is_loop_control = matches!(child.kind, TemplateIrNodeKind::LoopControl { .. });

        if is_loop_control {
            // Drop whitespace-only text nodes that immediately precede this
            // loop-control marker, preserving any preceding non-whitespace output.
            while let Some(last) = new_children.last().copied() {
                if tir_node_is_whitespace_only_text(last, store, string_table)? {
                    new_children.pop();
                } else {
                    break;
                }
            }
        }

        new_children.push(child_id);
    }

    if new_children.len() == original_children_count {
        return Ok(body_root);
    }

    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: new_children,
        },
        location,
    )))
}

// ------------------------------
//  Loop aggregate-wrapper preparation
// ------------------------------

/// Prepares the composed aggregate-wrapper for a template `loop`.
///
/// WHAT: derives the head-prefix TIR nodes from the owning template's
///       parser-emitted root children, builds an aggregate-wrapper
///       candidate root, composes it through `compose_tir_head_chain_from_root`, and
///       returns its authoritative composed root.
/// WHY: loop aggregate wrapping should consume the exact parser-emitted TIR
///      nodes so one structural authority owns aggregate preparation.
pub(in crate::compiler_frontend::ast::templates) fn prepare_loop_aggregate_wrapper(
    root_children: &[TemplateIrNodeId],
    string_table: &StringTable,
    template_ir_store: &mut TemplateIrStore,
) -> Result<PreparedLoopAggregateWrapper, TemplateError> {
    // Derive the head-prefix TIR nodes from the owning template's parser-emitted
    // root children. Reusing those exact nodes preserves parser identity and
    // avoids rebuilding an equivalent head structure.
    let head_prefix_nodes =
        head_prefix_tir_nodes(template_ir_store, root_children).map_err(TemplateError::from)?;

    let aggregate_root = build_aggregate_wrapper_candidate_root_from_tir_nodes(
        &head_prefix_nodes,
        template_ir_store,
    )?;
    let composed_root =
        compose_tir_head_chain_from_root(template_ir_store, aggregate_root, string_table, true)?;
    Ok(PreparedLoopAggregateWrapper {
        tir_root: composed_root,
    })
}

/// Extracts the head-prefix TIR nodes from the owning template's parser-emitted
/// root children.
///
/// WHAT: returns all root children before the first control-flow node (`Loop`
///       or `BranchChain`). These are the same TIR nodes the parser
///       materialized from the shared head-prefix atoms, so reusing them
///       avoids rebuilding TIR from content.
/// WHY: loop aggregate wrappers and branch/fallback body roots both wrap the
///      shared head prefix around their respective body output. The
///      head-prefix nodes are structurally everything before the control-flow
///      node in the owning template's root sequence, so one extractor serves
///      both control-flow kinds.
pub(in crate::compiler_frontend::ast::templates) fn head_prefix_tir_nodes(
    store: &TemplateIrStore,
    root_children: &[TemplateIrNodeId],
) -> Result<Vec<TemplateIrNodeId>, CompilerError> {
    let mut prefix = Vec::new();
    for &node_id in root_children {
        let node = store.get_node(node_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR head-prefix extraction: root child node {} was missing from the store.",
                node_id
            ))
        })?;
        if matches!(
            node.kind,
            TemplateIrNodeKind::Loop { .. } | TemplateIrNodeKind::BranchChain { .. }
        ) {
            break;
        }
        prefix.push(node_id);
    }
    Ok(prefix)
}
