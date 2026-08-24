//! Flow-aware reactive template metadata collection.
//!
//! WHAT: computes reactive template metadata for expressions and function calls
//! using the current function flow map and value environment. Template structure
//! is traversed by the template-owned helper in `templates::reactive_template_metadata`;
//! this module supplies a flow-aware expression resolver.
//! WHY: this lookup is read-only with respect to the AST; the separate
//! `annotation` phase applies the collected metadata back to the tree.

use super::types::{FunctionTemplateFlow, ReactiveTemplateValueEnvironment};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::templates::reactive_template_metadata;
use crate::compiler_frontend::ast::templates::reactive_template_metadata::{
    metadata_for_owned_runtime_slot_application_handoff,
    metadata_for_owned_runtime_template_handoff,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{TemplateIrStore, TemplateTirPhase, TirView};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use rustc_hash::FxHashMap;

pub(super) fn metadata_for_expression(
    expression: &Expression,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &ReactiveTemplateValueEnvironment,
    store: &TemplateIrStore,
) -> Result<Option<ReactiveTemplateMetadata>, CompilerError> {
    match &expression.kind {
        ExpressionKind::Template(template) => {
            metadata_for_template(template, flows, value_environment, store)
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            metadata_for_owned_runtime_template_handoff(handoff, &mut |nested| {
                metadata_for_expression(nested, flows, value_environment, store)
            })
            .map(Some)
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            metadata_for_owned_runtime_slot_application_handoff(handoff, &mut |nested| {
                metadata_for_expression(nested, flows, value_environment, store)
            })
            .map(Some)
        }

        ExpressionKind::FunctionCall { name, args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { name, args, .. } => {
            metadata_for_function_call(name, args, flows, value_environment, store)
        }

        ExpressionKind::Coerced { value, .. } => {
            Ok(
                metadata_for_expression(value, flows, value_environment, store)?
                    .or_else(|| expression.reactive_template.clone()),
            )
        }

        ExpressionKind::Reference(path) => Ok(value_environment
            .metadata_for_path(path)
            .or_else(|| expression.reactive_template.clone())),

        _ => Ok(expression.reactive_template.clone()),
    }
}

fn metadata_for_function_call(
    name: &InternedPath,
    arguments: &[CallArgument],
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &ReactiveTemplateValueEnvironment,
    store: &TemplateIrStore,
) -> Result<Option<ReactiveTemplateMetadata>, CompilerError> {
    let Some(flow) = flows.get(name) else {
        return Ok(None);
    };
    let Some(metadata) = flow.success_returns.first().and_then(Option::as_ref) else {
        return Ok(None);
    };
    let resolved_arguments = arguments
        .iter()
        .map(|argument| -> Result<CallArgument, CompilerError> {
            let mut resolved_argument = argument.clone();
            resolved_argument.value.reactive_template =
                metadata_for_expression(&argument.value, flows, value_environment, store)?;
            Ok(resolved_argument)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(metadata.instantiate_for_call(&flow.parameters, &resolved_arguments))
}

fn metadata_for_template(
    template: &Template,
    flows: &FxHashMap<InternedPath, FunctionTemplateFlow>,
    value_environment: &ReactiveTemplateValueEnvironment,
    store: &TemplateIrStore,
) -> Result<Option<ReactiveTemplateMetadata>, CompilerError> {
    let mut metadata = ReactiveTemplateMetadata::template_backed();
    let reference = template.tir_reference;
    let view = TirView::with_minimum_phase(
        store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;

    reactive_template_metadata::merge_reactive_template_metadata(
        &view,
        &mut metadata,
        &mut |expression| metadata_for_expression(expression, flows, value_environment, store),
    )?;

    Ok(Some(metadata))
}
