//! Function return-metadata flow analysis.
//!
//! WHAT: builds an initial per-function flow map from signatures, then runs a
//! fixed-point iteration that recomputes success-return metadata from function
//! bodies using the current flow map.
//! WHY: function return metadata must be derived from completed bodies because
//! signatures are resolved before bodies are parsed, and direct calls can
//! propagate metadata through helper chains.

use super::collector::metadata_for_expression;
use super::types::{
    FunctionTemplateFlow, ReactiveTemplateValueEnvironment, merge_optional_metadata,
    reference_path_for_place_expression,
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{
    ExpressionKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnChannel};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use rustc_hash::FxHashMap;

pub(super) fn initialize_function_template_flows(
    ast: &[AstNode],
) -> FxHashMap<InternedPath, FunctionTemplateFlow> {
    let mut flows = FxHashMap::default();
    collect_initial_function_flows_from_nodes(ast, &mut flows);
    flows
}

pub(super) fn refresh_function_template_flows(
    ast: &[AstNode],
    current_flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    next_flows: &mut FxHashMap<InternedPath, FunctionTemplateFlow>,
    store: &TemplateIrStore,
) -> Result<(), CompilerError> {
    for node in ast {
        refresh_function_template_flows_from_node(node, current_flows, next_flows, store)?;
    }

    Ok(())
}

fn collect_initial_function_flows_from_nodes(
    nodes: &[AstNode],
    flows: &mut FxHashMap<InternedPath, FunctionTemplateFlow>,
) {
    for node in nodes {
        collect_initial_function_flows_from_node(node, flows);
    }
}

fn collect_initial_function_flows_from_node(
    node: &AstNode,
    flows: &mut FxHashMap<InternedPath, FunctionTemplateFlow>,
) {
    match &node.kind {
        NodeKind::Function(path, signature, body) => {
            flows.insert(path.clone(), empty_flow_for_signature(signature));
            collect_initial_function_flows_from_nodes(body, flows);
        }

        NodeKind::VariableDeclaration(declaration) => {
            collect_initial_function_flows_from_declaration(declaration, flows);
        }

        NodeKind::If(_, then_body, else_body, _) => {
            collect_initial_function_flows_from_nodes(then_body, flows);
            if let Some(else_body) = else_body {
                collect_initial_function_flows_from_nodes(else_body, flows);
            }
        }

        NodeKind::Match { arms, default, .. } => {
            for arm in arms {
                collect_initial_function_flows_from_nodes(&arm.body, flows);
            }
            if let Some(default_body) = default {
                collect_initial_function_flows_from_nodes(default_body, flows);
            }
        }

        NodeKind::ScopedBlock { body }
        | NodeKind::RangeLoop { body, .. }
        | NodeKind::CollectionLoop { body, .. }
        | NodeKind::WhileLoop(_, body) => {
            collect_initial_function_flows_from_nodes(body, flows);
        }

        NodeKind::ThenValue(_)
        | NodeKind::Return(_)
        | NodeKind::ReturnError(_)
        | NodeKind::Assert { .. }
        | NodeKind::PushStartRuntimeFragment(_)
        | NodeKind::StructDefinition(_, _)
        | NodeKind::Assignment { .. }
        | NodeKind::MultiBind { .. }
        | NodeKind::ExpressionStatement(_)
        | NodeKind::Break
        | NodeKind::Continue => {}
    }
}

fn collect_initial_function_flows_from_declaration(
    declaration: &Declaration,
    flows: &mut FxHashMap<InternedPath, FunctionTemplateFlow>,
) {
    if let ExpressionKind::Function(signature) = &declaration.value.kind {
        flows.insert(declaration.id.clone(), empty_flow_for_signature(signature));
    }
}

fn empty_flow_for_signature(signature: &FunctionSignature) -> FunctionTemplateFlow {
    FunctionTemplateFlow {
        parameters: signature.parameters.clone(),
        success_returns: signature
            .returns
            .iter()
            .filter(|slot| slot.channel == ReturnChannel::Success)
            .map(|_| None)
            .collect(),
    }
}

fn refresh_function_template_flows_from_node(
    node: &AstNode,
    current_flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    next_flows: &mut FxHashMap<InternedPath, FunctionTemplateFlow>,
    store: &TemplateIrStore,
) -> Result<(), CompilerError> {
    match &node.kind {
        NodeKind::Function(path, signature, body) => {
            let returns = collect_return_metadata(body, signature, current_flows, store)?;
            if let Some(flow) = next_flows.get_mut(path) {
                flow.success_returns = returns;
            }
            refresh_function_template_flows(body, current_flows, next_flows, store)?;
        }

        NodeKind::VariableDeclaration(declaration) => {
            if let ExpressionKind::Function(signature) = &declaration.value.kind {
                next_flows
                    .entry(declaration.id.clone())
                    .or_insert_with(|| empty_flow_for_signature(signature));
            }
        }

        NodeKind::If(_, then_body, else_body, _) => {
            refresh_function_template_flows(then_body, current_flows, next_flows, store)?;
            if let Some(else_body) = else_body {
                refresh_function_template_flows(else_body, current_flows, next_flows, store)?;
            }
        }

        NodeKind::Match { arms, default, .. } => {
            for arm in arms {
                refresh_function_template_flows(&arm.body, current_flows, next_flows, store)?;
            }
            if let Some(default_body) = default {
                refresh_function_template_flows(default_body, current_flows, next_flows, store)?;
            }
        }

        NodeKind::ScopedBlock { body }
        | NodeKind::RangeLoop { body, .. }
        | NodeKind::CollectionLoop { body, .. }
        | NodeKind::WhileLoop(_, body) => {
            refresh_function_template_flows(body, current_flows, next_flows, store)?;
        }

        _ => {}
    }

    Ok(())
}

fn collect_return_metadata(
    body: &[AstNode],
    signature: &FunctionSignature,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    store: &TemplateIrStore,
) -> Result<Vec<Option<ReactiveTemplateMetadata>>, CompilerError> {
    let mut returns: Vec<Option<ReactiveTemplateMetadata>> = signature
        .returns
        .iter()
        .filter(|slot| slot.channel == ReturnChannel::Success)
        .map(|slot| slot.reactive_template.clone())
        .collect();

    let mut value_environment =
        ReactiveTemplateValueEnvironment::for_parameters(&signature.parameters);
    collect_return_metadata_from_nodes(body, flows, &mut returns, &mut value_environment, store)?;
    Ok(returns)
}

fn collect_return_metadata_from_nodes(
    nodes: &[AstNode],
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    returns: &mut [Option<ReactiveTemplateMetadata>],
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &TemplateIrStore,
) -> Result<(), CompilerError> {
    for node in nodes {
        collect_return_metadata_from_node(node, flows, returns, value_environment, store)?;
    }

    Ok(())
}

fn collect_return_metadata_from_node(
    node: &AstNode,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    returns: &mut [Option<ReactiveTemplateMetadata>],
    value_environment: &mut ReactiveTemplateValueEnvironment,
    store: &TemplateIrStore,
) -> Result<(), CompilerError> {
    match &node.kind {
        NodeKind::Return(values) => {
            for (index, value) in values.iter().enumerate() {
                let Some(slot) = returns.get_mut(index) else {
                    continue;
                };
                merge_optional_metadata(
                    slot,
                    metadata_for_expression(value, flows, value_environment, store)?,
                );
            }
        }

        NodeKind::VariableDeclaration(declaration) => {
            let mut resolved_declaration = declaration.clone();
            resolved_declaration.value.reactive_template =
                metadata_for_expression(&declaration.value, flows, value_environment, store)?;
            value_environment.record_declaration(&resolved_declaration);
        }

        NodeKind::Assignment { target, value } => {
            if let Some(target_path) = reference_path_for_place_expression(target) {
                let mut resolved_value = value.clone();
                resolved_value.reactive_template =
                    metadata_for_expression(value, flows, value_environment, store)?;
                value_environment.record_assignment(target_path, &resolved_value);
            }
        }

        NodeKind::If(_, then_body, else_body, _) => {
            let mut then_environment = value_environment.clone();
            collect_return_metadata_from_nodes(
                then_body,
                flows,
                returns,
                &mut then_environment,
                store,
            )?;
            if let Some(else_body) = else_body {
                let mut else_environment = value_environment.clone();
                collect_return_metadata_from_nodes(
                    else_body,
                    flows,
                    returns,
                    &mut else_environment,
                    store,
                )?;
            }
        }

        NodeKind::Match { arms, default, .. } => {
            for arm in arms {
                let mut arm_environment = value_environment.clone();
                collect_return_metadata_from_nodes(
                    &arm.body,
                    flows,
                    returns,
                    &mut arm_environment,
                    store,
                )?;
            }
            if let Some(default_body) = default {
                let mut default_environment = value_environment.clone();
                collect_return_metadata_from_nodes(
                    default_body,
                    flows,
                    returns,
                    &mut default_environment,
                    store,
                )?;
            }
        }

        NodeKind::ScopedBlock { body }
        | NodeKind::RangeLoop { body, .. }
        | NodeKind::CollectionLoop { body, .. }
        | NodeKind::WhileLoop(_, body) => {
            let mut body_environment = value_environment.clone();
            collect_return_metadata_from_nodes(body, flows, returns, &mut body_environment, store)?;
        }

        _ => {}
    }

    Ok(())
}
