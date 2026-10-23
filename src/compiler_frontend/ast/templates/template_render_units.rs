//! Template render-unit preparation for linear and control-flow templates.
//!
//! WHAT: Prepares control-flow body roots in TIR and installs the formatted
//! TIR reference for linear templates. Linear templates format directly from
//! a TIR view, making TIR formatting the sole production authority.
//!
//! WHY: Normal templates, template `if` bodies, and template `loop` bodies
//! all need the same composition and formatting rules. Keeping the render-unit
//! shaping here prevents control-flow support from growing a parallel template
//! pipeline.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Style;
use crate::compiler_frontend::ast::templates::tir::{
    ControlFlowBodyKind, DerivedTemplateMetadata, RetainedControlFlowRenderUnit,
    RetainedNumericIntakeRole, TemplateConstructionContext, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateTirPhase, TemplateTirReference, build_conditional_body_candidate_root_from_tir_nodes,
    compose_tir_head_chain_from_root, format_tir_body_root, head_prefix_tir_nodes,
    prepare_loop_aggregate_wrapper, run_tir_formatter_with_warnings, sequence_children,
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
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
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
    let formatted_root = run_tir_formatter_with_warnings(
        &mut store,
        expression_builder,
        const_values,
        reference.root,
        reference.phase,
        reference.context,
        style,
        context,
        string_table,
    )?;
    let formatted_template_id = store.push_structurally_derived_template(
        reference.root,
        formatted_root,
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
    control_flow_node_id: TemplateIrNodeId,
}

/// Builds a conditional body render unit from parser-emitted head-prefix nodes
/// plus the parsed body root.
///
/// WHAT: formats the body, combines it with the owning template's head prefix,
///       and composes the shared head chain. Inherited `$children(..)` wrappers
///       stay on the wrapper-context overlay attached after composition.
/// WHY: concrete and retained conditional positions use the same TIR composition
///      authority without rebuilding a second template representation.
fn compose_conditional_body_tir_root(
    root_children: &[TemplateIrNodeId],
    style: &Style,
    body_root: TemplateIrNodeId,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<TemplateIrNodeId, TemplateError> {
    // Derive the head-prefix TIR nodes from the owning template's parser-emitted
    // root children. Reusing them avoids rebuilding TIR from formatted output.
    let head_prefix_nodes = {
        let store = context.template_ir_store.borrow();
        head_prefix_tir_nodes(&store, root_children).map_err(TemplateError::from)?
    };

    // The parser-emitted body is formatted before head-chain composition so the
    // final body tree carries formatted text.
    let body_root = format_tir_body_root(
        body_root,
        style,
        context,
        expression_builder,
        const_values,
        string_table,
    )?;

    let mut store = context.template_ir_store.borrow_mut();
    let body_children = sequence_children(&store, body_root).map_err(TemplateError::from)?;
    let candidate_root = build_conditional_body_candidate_root_from_tir_nodes(
        &head_prefix_nodes,
        &body_children,
        &mut store,
    )?;

    compose_tir_head_chain_from_root(
        &mut store,
        expression_builder,
        const_values,
        candidate_root,
        string_table,
        true,
        Some(context.external_package_registry.as_ref()),
    )
}

/// Installs the shared conditional render unit on a concrete control-flow node.
fn prepare_conditional_body_tir_root(
    input: TirBodyRootInput<'_>,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    let TirBodyRootInput {
        root_children,
        style,
        body_root,
        control_flow_node_id,
    } = input;

    let composed_root = compose_conditional_body_tir_root(
        root_children,
        style,
        body_root,
        context,
        expression_builder,
        const_values,
        string_table,
    )?;

    context
        .template_ir_store
        .borrow_mut()
        .replace_control_flow_body(
            control_flow_node_id,
            ControlFlowBodyKind::ConditionalBody,
            composed_root,
        )
        .map_err(TemplateError::from)
}

/// Prepared loop body and wrapper roots produced by the shared render-unit authority.
struct PreparedLoopRenderUnit {
    body: TemplateIrNodeId,
    aggregate_wrapper: TemplateIrNodeId,
}

/// Builds the formatted loop body and aggregate wrapper from parser-emitted TIR roots.
///
/// Loop bodies skip the shared head prefix because it wraps the aggregate output
/// once, not each iteration.
fn prepare_loop_render_unit_roots(
    body: TemplateIrNodeId,
    construction_context: &TemplateConstructionContext,
    style: &Style,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<PreparedLoopRenderUnit, TemplateError> {
    let body = format_tir_body_root(
        body,
        style,
        context,
        expression_builder,
        const_values,
        string_table,
    )?;
    let mut store = context.template_ir_store.borrow_mut();
    let aggregate_wrapper = prepare_loop_aggregate_wrapper(
        construction_context.root_children(),
        string_table,
        &mut store,
        expression_builder,
        const_values,
        Some(context.external_package_registry.as_ref()),
    )?;
    Ok(PreparedLoopRenderUnit {
        body,
        aggregate_wrapper,
    })
}

/// Inputs required by shared control-flow render-unit preparation.
pub(in crate::compiler_frontend::ast::templates) struct ControlFlowRenderUnitRequest<'a> {
    pub(in crate::compiler_frontend::ast::templates) style: &'a Style,
    pub(in crate::compiler_frontend::ast::templates) context: &'a ScopeContext,
    pub(in crate::compiler_frontend::ast::templates) expression_builder:
        &'a mut AstExpressionBuilder,
    pub(in crate::compiler_frontend::ast::templates) const_values: &'a mut ConstValueStore,
    pub(in crate::compiler_frontend::ast::templates) string_table: &'a mut StringTable,
}

enum ControlFlowRenderTarget {
    Concrete {
        body_kind: ControlFlowBodyKind,
        body_root: TemplateIrNodeId,
    },
    Retained {
        body_kind: ControlFlowBodyKind,
        body_root: TemplateIrNodeId,
    },
}

/// Prepares the first registered control-flow node through the shared TIR render-unit owner.
pub(in crate::compiler_frontend::ast::templates) fn prepare_control_flow_render_units(
    construction_context: &TemplateConstructionContext,
    request: ControlFlowRenderUnitRequest<'_>,
) -> Result<(), TemplateError> {
    let control_flow_node_id = construction_context.control_flow_node_id().ok_or_else(|| {
        CompilerError::compiler_error(
            "prepare_control_flow_render_units called on template without a TIR control-flow node",
        )
    })?;

    // Copy the target shape before formatting mutates the shared store.
    let target = {
        let store = request.context.template_ir_store.borrow();
        let node = store.get_node(control_flow_node_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "Control-flow node disappeared from the TIR store during render-unit preparation.",
            )
        })?;
        match &node.kind {
            TemplateIrNodeKind::Conditional { body, .. } => ControlFlowRenderTarget::Concrete {
                body_kind: ControlFlowBodyKind::ConditionalBody,
                body_root: *body,
            },
            TemplateIrNodeKind::Loop { body, .. } => ControlFlowRenderTarget::Concrete {
                body_kind: ControlFlowBodyKind::LoopBody,
                body_root: *body,
            },
            TemplateIrNodeKind::RetainedNumericIntake(role)
                if role.completes_into_control_flow() =>
            {
                let role = *role;
                let (body_kind, body_root) = match role {
                    RetainedNumericIntakeRole::ConditionalLoopCondition { body, .. }
                    | RetainedNumericIntakeRole::LoopRange { body, .. }
                    | RetainedNumericIntakeRole::LoopCollection { body, .. } => {
                        (ControlFlowBodyKind::LoopBody, body)
                    }
                    RetainedNumericIntakeRole::ConditionalSelector { body, .. } => {
                        (ControlFlowBodyKind::ConditionalBody, body)
                    }
                    RetainedNumericIntakeRole::HeadExpression { .. } => {
                        return Err(CompilerError::compiler_error(
                            "head-expression intake was reported as a control-flow position.",
                        )
                        .into());
                    }
                };
                ControlFlowRenderTarget::Retained {
                    body_kind,
                    body_root,
                }
            }
            _ => {
                return Err(CompilerError::compiler_error(
                    "Control-flow node was neither a Conditional, Loop, nor retained control-flow position during render-unit preparation.",
                )
                .into());
            }
        }
    };

    match target {
        ControlFlowRenderTarget::Concrete {
            body_kind,
            body_root,
        } => match body_kind {
            ControlFlowBodyKind::ConditionalBody => {
                prepare_conditional_body_tir_root(
                    TirBodyRootInput {
                        root_children: construction_context.root_children(),
                        style: request.style,
                        body_root,
                        control_flow_node_id,
                    },
                    request.context,
                    request.expression_builder,
                    request.const_values,
                    request.string_table,
                )?;
            }
            ControlFlowBodyKind::LoopBody => {
                prepare_loop_render_units(
                    control_flow_node_id,
                    body_root,
                    construction_context,
                    request,
                )?;
            }
        },
        ControlFlowRenderTarget::Retained {
            body_kind,
            body_root,
        } => {
            let (body, render_unit) = match body_kind {
                ControlFlowBodyKind::ConditionalBody => (
                    compose_conditional_body_tir_root(
                        construction_context.root_children(),
                        request.style,
                        body_root,
                        request.context,
                        request.expression_builder,
                        request.const_values,
                        request.string_table,
                    )?,
                    RetainedControlFlowRenderUnit::Conditional,
                ),
                ControlFlowBodyKind::LoopBody => {
                    let prepared = prepare_loop_render_unit_roots(
                        body_root,
                        construction_context,
                        request.style,
                        request.context,
                        request.expression_builder,
                        request.const_values,
                        request.string_table,
                    )?;
                    (
                        prepared.body,
                        RetainedControlFlowRenderUnit::Loop {
                            aggregate_wrapper: prepared.aggregate_wrapper,
                        },
                    )
                }
            };

            request
                .context
                .template_ir_store
                .borrow_mut()
                .record_retained_control_flow_render_unit(control_flow_node_id, body, render_unit)
                .map_err(TemplateError::from)?;
        }
    }

    Ok(())
}

/// Prepares the loop body and installs its complete render unit on a concrete `Loop`.
fn prepare_loop_render_units(
    control_flow_node_id: TemplateIrNodeId,
    body: TemplateIrNodeId,
    construction_context: &TemplateConstructionContext,
    request: ControlFlowRenderUnitRequest<'_>,
) -> Result<(), TemplateError> {
    let prepared = prepare_loop_render_unit_roots(
        body,
        construction_context,
        request.style,
        request.context,
        request.expression_builder,
        request.const_values,
        request.string_table,
    )?;

    let mut store = request.context.template_ir_store.borrow_mut();

    store.replace_control_flow_body(
        control_flow_node_id,
        ControlFlowBodyKind::LoopBody,
        prepared.body,
    )?;
    store.replace_loop_aggregate_wrapper(control_flow_node_id, prepared.aggregate_wrapper)?;
    Ok(())
}
