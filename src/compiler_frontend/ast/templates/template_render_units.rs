//! Template render-unit preparation for linear and control-flow templates.
//!
//! WHAT: Prepares control-flow body roots in TIR and installs the formatted
//! TIR reference for linear templates. Linear templates format directly from
//! a TIR view, making TIR formatting the sole production authority.
//!
//! WHY: Normal templates, template `if` branches, and template `loop` bodies
//! all need the same composition and formatting rules. Keeping the render-unit
//! shaping here prevents control-flow support from growing a parallel template
//! pipeline.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Style;
use crate::compiler_frontend::ast::templates::tir::{
    ControlFlowBodyKind, DerivedTemplateMetadata, TemplateConstructionContext, TemplateIrNodeId,
    TemplateIrNodeKind, TemplateTirPhase, TemplateTirReference,
    build_branch_body_candidate_root_from_tir_nodes, compose_tir_head_chain_from_root,
    format_tir_body_root, head_prefix_tir_nodes, prepare_loop_aggregate_wrapper,
    run_tir_formatter_with_warnings, sequence_children,
    trim_whitespace_before_loop_control_boundary,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Installs formatter output as the current module-local TIR reference.
///
/// WHAT: runs the TIR formatter adapter over the template's current referenced
///       root and stores the append-only formatted root as a new TIR template.
/// WHY: linear templates now carry their formatted root directly in TIR.
///      TIR formatting is the production authority for linear bodies.
pub(in crate::compiler_frontend::ast::templates) fn install_formatted_tir_reference_for_linear_template(
    tir_reference: &mut TemplateTirReference,
    has_control_flow: bool,
    style: &Style,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    if has_control_flow {
        return Ok(());
    }

    let reference = *tir_reference;

    if reference.phase.is_at_least(TemplateTirPhase::Formatted) {
        return Ok(());
    }

    let mut store = context.template_ir_store.borrow_mut();
    let formatter_result = run_tir_formatter_with_warnings(
        &mut store,
        reference.root,
        reference.phase,
        reference.context,
        style,
        context,
        string_table,
    )?;
    let formatted_template_id = store.push_structurally_derived_template(
        reference.root,
        formatter_result.root,
        DerivedTemplateMetadata::preserve_source(),
    )?;

    *tir_reference = TemplateTirReference {
        root: formatted_template_id,
        phase: TemplateTirPhase::Formatted,
        context: reference.context,
    };

    Ok(())
}

struct TirBodyRootInput<'a> {
    root_children: &'a [TemplateIrNodeId],
    style: &'a Style,
    body_root: TemplateIrNodeId,
    body_kind: ControlFlowBodyKind,
    control_flow_node_id: TemplateIrNodeId,
}

/// Prepares a branch/fallback body TIR root from parser-emitted head-prefix
/// nodes plus the parsed body root.
///
/// WHAT: reuses the owning template's parser-emitted head-prefix TIR nodes,
///       formats the parsed body root, builds a candidate root node combining
///       head prefix and body, and composes it so head-chain wrappers apply to
///       the body. Inherited `$children(..)` wrappers stay on the wrapper-context
///       overlay attached after composition.
/// WHY: with control-flow bodies emitted directly into TIR, body-root
///      preparation reuses the parser-emitted body sequence and the owning
///      template's head-prefix nodes. The prepared root is installed directly
///      onto the owning TIR control-flow node; a missing store/root or
///      impossible replacement is an internal `CompilerError`, not a silent
///      fallback.
fn prepare_branch_body_tir_root(
    input: TirBodyRootInput<'_>,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    let TirBodyRootInput {
        root_children,
        style,
        body_root,
        body_kind,
        control_flow_node_id,
    } = input;

    // Derive the head-prefix TIR nodes from the owning template's parser-emitted
    // root children. These are the same nodes the parser materialized from the
    // shared head-prefix atoms, so reusing them avoids rebuilding TIR from
    // the formatted TIR root.
    let head_prefix_nodes = {
        let store = context.template_ir_store.borrow();
        head_prefix_tir_nodes(&store, root_children).map_err(TemplateError::from)?
    };

    // The body is already a parser-emitted TIR sequence node. Formatting and
    // head-chain composition operate on this root directly without reconstructing
    // a second template representation.

    // Format the body root before head-chain composition so the final body tree
    // carries formatted text. The store borrow is released around this call
    // because the TIR formatter mutates the shared module store through `TirView`.
    let body_root = format_tir_body_root(body_root, style, context, string_table)?;

    let mut store = context.template_ir_store.borrow_mut();
    let body_children = sequence_children(&store, body_root).map_err(TemplateError::from)?;

    // Build a candidate root node combining the head-prefix nodes with the body
    // children, then compose so head-chain wrappers apply to the body.
    let candidate_root = build_branch_body_candidate_root_from_tir_nodes(
        &head_prefix_nodes,
        &body_children,
        &mut store,
    )?;
    let composed_root =
        compose_tir_head_chain_from_root(&mut store, candidate_root, string_table, true)?;

    store
        .replace_control_flow_body(control_flow_node_id, body_kind, composed_root)
        .map_err(TemplateError::from)
}

/// Shared inputs for preparing one branch, fallback or loop body.
struct ControlFlowBodyPreparationContext<'a> {
    construction_context: &'a TemplateConstructionContext,
    style: &'a Style,
    context: &'a ScopeContext,
    string_table: &'a mut StringTable,
}

/// Prepares one branch or fallback body.
///
/// WHAT: reads the parsed body root node ID from the owning TIR control-flow
///       node, derives the prepared TIR root from parser-emitted head-prefix
///       nodes plus that body root, formats it via the TIR-native formatter,
///       then applies head-chain composition. The prepared root is installed
///       directly onto the TIR control-flow node.
/// WHY: branch and fallback bodies share the same head-prefix + body shape, so
///      one preparation owner keeps TIR formatting and root installation in
///      sync without duplicating the flow per arm.
fn prepare_branch_or_fallback_body(
    ctx: ControlFlowBodyPreparationContext<'_>,
    control_flow_node_id: TemplateIrNodeId,
    body_root: TemplateIrNodeId,
    body_kind: ControlFlowBodyKind,
) -> Result<(), TemplateError> {
    let ControlFlowBodyPreparationContext {
        construction_context,
        style,
        context,
        string_table,
    } = ctx;

    // Collect parser-emitted root children before the mutable store borrow so
    // the TIR-derived path can reuse module-local head-prefix nodes.
    let root_children = construction_context.root_children().to_vec();

    prepare_branch_body_tir_root(
        TirBodyRootInput {
            root_children: &root_children,
            style,
            body_root,
            body_kind,
            control_flow_node_id,
        },
        context,
        string_table,
    )
}

/// Prepares a loop body TIR root from the parsed body root.
///
/// WHAT: formats the parsed TIR body root, trims whitespace-only text nodes
///       before any top-level loop-control marker, and installs the result as
///       the loop's body root. Inherited `$children(..)` wrappers stay on the
///       wrapper-context overlay attached after composition.
/// WHY: loop bodies do not carry the owning template's shared head prefix (that
///      wraps the aggregate output), so they can skip head-chain composition.
///      Loop-control boundary whitespace trimming is applied as a TIR-local
///      transform so the loop body root owns the behavior. The prepared root is
///      installed directly onto the TIR `Loop` node; a missing store/root or
///      impossible replacement is an internal `CompilerError`, not a silent
///      fallback.
fn prepare_loop_body_tir_root(
    control_flow_node_id: TemplateIrNodeId,
    style: &Style,
    body_root: TemplateIrNodeId,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    // The loop body is already a parser-emitted TIR sequence node; formatting
    // and loop-control boundary trimming operate on it directly without
    // reconstructing a second template representation.

    // Release the store borrow around the TIR formatter call; the formatter
    // authority mutates the shared module store through `TirView`.
    let body_root = format_tir_body_root(body_root, style, context, string_table)?;

    let mut store = context.template_ir_store.borrow_mut();
    let body_root =
        trim_whitespace_before_loop_control_boundary(body_root, &mut store, string_table)
            .map_err(TemplateError::from)?;

    store
        .replace_control_flow_body(
            control_flow_node_id,
            ControlFlowBodyKind::LoopBody,
            body_root,
        )
        .map_err(TemplateError::from)
}

/// Prepares a template `loop` body.
///
/// WHAT: reads the parsed body root node ID from the owning TIR `Loop` node and
///       formats that root via the TIR-native formatter. Loop bodies
///       intentionally skip head-prefix composition because the owning head
///       wraps the aggregate output once, not each iteration. Loop-control
///       boundary whitespace trimming is applied as a TIR-local transform.
/// WHY: the loop body root owns loop-control boundary whitespace trimming and
///      formatting natively in TIR.
fn prepare_loop_body(
    ctx: ControlFlowBodyPreparationContext<'_>,
    control_flow_node_id: TemplateIrNodeId,
    body_root: TemplateIrNodeId,
) -> Result<(), TemplateError> {
    let ControlFlowBodyPreparationContext {
        style,
        context,
        string_table,
        ..
    } = ctx;

    prepare_loop_body_tir_root(
        control_flow_node_id,
        style,
        body_root,
        context,
        string_table,
    )
}

/// Applies composition and formatting to a structured control-flow template.
///
/// For `if`, each branch is a complete TIR render unit that includes the shared
/// head prefix. For `loop`, the per-iteration body is finalized independently
/// and the parser-emitted head prefix becomes an aggregate-wrapper TIR subtree,
/// so later folding and lowering apply it once around the aggregate.
///
/// After each body is formatted, this installs the prepared body root directly
/// onto the owning parser TIR control-flow node. The control-flow node and its
/// body node IDs are read from the construction context and TIR store, not from
/// a durable AST carrier.
pub(in crate::compiler_frontend::ast::templates) struct ControlFlowRenderUnitRequest<'a> {
    pub(in crate::compiler_frontend::ast::templates) style: &'a Style,
    pub(in crate::compiler_frontend::ast::templates) context: &'a ScopeContext,
    pub(in crate::compiler_frontend::ast::templates) string_table: &'a mut StringTable,
}

pub(in crate::compiler_frontend::ast::templates) fn prepare_control_flow_render_units(
    construction_context: &mut TemplateConstructionContext,
    request: ControlFlowRenderUnitRequest<'_>,
) -> Result<(), TemplateError> {
    let ControlFlowRenderUnitRequest {
        style,
        context,
        string_table,
    } = request;

    // Locate the owning TIR control-flow node through the construction context.
    // The body parser already constructed the BranchChain/Loop node and its
    // body node IDs in the TIR store; render-unit preparation reads and
    // updates them directly.
    let control_flow_node_id = construction_context.control_flow_node_id().ok_or_else(|| {
        CompilerError::compiler_error(
            "prepare_control_flow_render_units called on template without a TIR control-flow node",
        )
    })?;

    // Extract only the body node IDs needed for preparation. The RefCell
    // borrow must end before the mutable store operations below, so the
    // IDs are copied rather than holding a borrowed reference to the kind.
    let (branch_bodies, fallback_body, loop_body) = {
        let store = context.template_ir_store.borrow();
        let node = store.get_node(control_flow_node_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "Control-flow node disappeared from the TIR store during render-unit preparation.",
            )
        })?;
        match &node.kind {
            TemplateIrNodeKind::BranchChain { branches, fallback } => {
                let bodies: Vec<_> = branches.iter().map(|b| b.body).collect();
                (Some(bodies), *fallback, None)
            }
            TemplateIrNodeKind::Loop { body, .. } => (None, None, Some(*body)),
            _ => (None, None, None),
        }
    };

    match (branch_bodies, fallback_body, loop_body) {
        (Some(branch_bodies), fallback, _) => {
            prepare_branch_chain_render_units(
                control_flow_node_id,
                &branch_bodies,
                fallback,
                ControlFlowBodyPreparationContext {
                    construction_context,
                    style,
                    context,
                    string_table,
                },
            )?;
        }

        (_, _, Some(body)) => {
            prepare_loop_render_units(
                control_flow_node_id,
                body,
                construction_context,
                style,
                context,
                string_table,
            )?;
        }

        _ => {
            return Err(CompilerError::compiler_error(
                "Control-flow node was neither a BranchChain nor a Loop during render-unit preparation.",
            )
            .into());
        }
    }

    Ok(())
}

/// Prepares every branch and fallback body in a branch chain.
///
/// WHAT: reads branch and fallback body node IDs from the TIR `BranchChain`
///       node, prepares each body, and installs the prepared root directly onto
///       the TIR node. Body node IDs are read before preparation starts so the
///       store can be mutated during each body's format/compose/install cycle
///       without holding a borrow across the mutable phase.
fn prepare_branch_chain_render_units(
    control_flow_node_id: TemplateIrNodeId,
    branch_bodies: &[TemplateIrNodeId],
    fallback: Option<TemplateIrNodeId>,
    ctx: ControlFlowBodyPreparationContext<'_>,
) -> Result<(), TemplateError> {
    let ControlFlowBodyPreparationContext {
        construction_context,
        style,
        context,
        string_table,
    } = ctx;

    for (index, &body) in branch_bodies.iter().enumerate() {
        prepare_branch_or_fallback_body(
            ControlFlowBodyPreparationContext {
                construction_context,
                style,
                context,
                string_table: &mut *string_table,
            },
            control_flow_node_id,
            body,
            ControlFlowBodyKind::Branch { index },
        )?;
    }

    if let Some(fallback_body) = fallback {
        prepare_branch_or_fallback_body(
            ControlFlowBodyPreparationContext {
                construction_context,
                style,
                context,
                string_table: &mut *string_table,
            },
            control_flow_node_id,
            fallback_body,
            ControlFlowBodyKind::Fallback,
        )?;
    }

    Ok(())
}

/// Prepares the loop body and installs the aggregate-wrapper subtree.
///
/// WHAT: reads the loop body node ID from the TIR `Loop` node, prepares the
///       body, then builds and installs the aggregate wrapper directly onto the
///       TIR `Loop` node. The aggregate wrapper root no longer needs to be
///       cached on a durable carrier because the TIR node owns it and reactive
///       metadata walks the TIR root directly.
fn prepare_loop_render_units(
    control_flow_node_id: TemplateIrNodeId,
    body: TemplateIrNodeId,
    construction_context: &mut TemplateConstructionContext,
    style: &Style,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    // Prepare the loop body with the same format-once + TIR pattern used
    // by branch/fallback bodies. Loop bodies skip the shared head prefix
    // because the owning head wraps the aggregate output once, not each
    // iteration.
    prepare_loop_body(
        ControlFlowBodyPreparationContext {
            construction_context,
            style,
            context,
            string_table,
        },
        control_flow_node_id,
        body,
    )?;

    // Collect the parser-emitted root children before the mutable store
    // borrow so the aggregate wrapper can reuse existing module-local TIR
    // head-prefix nodes instead of rebuilding from content atoms.
    let root_children = construction_context.root_children().to_vec();

    let mut template_ir_store = context.template_ir_store.borrow_mut();
    let aggregate_wrapper =
        prepare_loop_aggregate_wrapper(&root_children, string_table, &mut template_ir_store)?;

    // Install the composed TIR aggregate-wrapper subtree onto the owning
    // `Loop` node.
    template_ir_store
        .replace_loop_aggregate_wrapper(control_flow_node_id, aggregate_wrapper.tir_root)?;

    Ok(())
}
