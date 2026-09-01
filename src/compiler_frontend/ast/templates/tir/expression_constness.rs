//! Narrow constness rules shared by TIR preparation and runtime slot planning.
//!
//! This module owns expression-kind constness, exact branch/loop overlay payload
//! selection and the small structural query needed by runtime contribution
//! planning. It does not classify complete template views, slots or insert overlays.

use std::collections::HashSet;

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader, collect_option_capture_binding_path,
    loop_body_const_evaluation_bindings,
};
use crate::compiler_frontend::ast::templates::tir::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ExpressionSiteId, TemplateIrId, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::TirView;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(crate) fn effective_branch_selector_for_view(
    view: &TirView<'_>,
    selector: &TemplateBranchSelector,
    site_id: ExpressionSiteId,
) -> Result<TemplateBranchSelector, TemplateError> {
    let Some(expression) = view.effective_expression_for_site(site_id)? else {
        return Ok(selector.clone());
    };

    Ok(match selector {
        TemplateBranchSelector::Bool(_) => TemplateBranchSelector::Bool(expression.clone()),
        TemplateBranchSelector::OptionPresentCapture { pattern, .. } => {
            TemplateBranchSelector::OptionPresentCapture {
                scrutinee: expression.clone(),
                pattern: pattern.clone(),
            }
        }
    })
}

pub(crate) fn effective_loop_header_for_view(
    view: &TirView<'_>,
    header: &TemplateLoopHeader,
    header_sites: TemplateLoopHeaderExpressionSites,
) -> Result<TemplateLoopHeader, TemplateError> {
    Ok(match (header, header_sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
        ) => TemplateLoopHeader::Conditional {
            condition: Box::new(
                view.effective_expression_for_site(site_id)?
                    .cloned()
                    .unwrap_or_else(|| condition.as_ref().clone()),
            ),
        },
        (
            TemplateLoopHeader::Range { bindings, range },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            let mut range = range.as_ref().clone();
            if let Some(expression) = view.effective_expression_for_site(start)? {
                range.start = expression.clone();
            }
            if let Some(expression) = view.effective_expression_for_site(end)? {
                range.end = expression.clone();
            }
            match (&range.step, step) {
                (None, None) => {}
                (Some(_), Some(step_site_id)) => {
                    if let Some(expression) = view.effective_expression_for_site(step_site_id)? {
                        range.step = Some(expression.clone());
                    }
                }
                _ => {
                    return Err(CompilerError::compiler_error(
                        "TIR preparation: loop range header/site step shape mismatch.",
                    )
                    .into());
                }
            }

            TemplateLoopHeader::Range {
                bindings: bindings.clone(),
                range: Box::new(range),
            }
        }
        (
            TemplateLoopHeader::Collection { bindings, iterable },
            TemplateLoopHeaderExpressionSites::Collection { iterable: site_id },
        ) => TemplateLoopHeader::Collection {
            bindings: bindings.clone(),
            iterable: Box::new(
                view.effective_expression_for_site(site_id)?
                    .cloned()
                    .unwrap_or_else(|| iterable.as_ref().clone()),
            ),
        },
        _ => {
            return Err(CompilerError::compiler_error(
                "TIR preparation: loop header shape does not match its expression sites.",
            )
            .into());
        }
    })
}

/// Applies the expression-kind constness rules while delegating nested TIR
/// values to the caller's exact-view traversal.
pub(crate) fn classify_expression_const_evaluable_with_nested_template(
    expression: &Expression,
    loop_binding_paths: &[InternedPath],
    nested_template: &mut impl FnMut(
        TemplateTirReference,
        &[InternedPath],
    ) -> Result<bool, TemplateError>,
) -> Result<bool, TemplateError> {
    match &expression.kind {
        ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::StructuralString { .. }
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_) => Ok(true),

        ExpressionKind::Reference(path) => Ok(loop_binding_paths.iter().any(|known| known == path)),

        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            classify_expression_const_evaluable_with_nested_template(
                value,
                loop_binding_paths,
                nested_template,
            )
        }

        ExpressionKind::Coerced { value, .. } => {
            classify_expression_const_evaluable_with_nested_template(
                value,
                loop_binding_paths,
                nested_template,
            )
        }

        ExpressionKind::Runtime(rpn) => {
            let mut const_evaluable = true;
            for item in &rpn.items {
                if let ExpressionRpnItem::Operand(operand) = item {
                    const_evaluable &= classify_expression_const_evaluable_with_nested_template(
                        operand,
                        loop_binding_paths,
                        nested_template,
                    )?;
                }
            }
            Ok(const_evaluable)
        }

        ExpressionKind::Template(template) => {
            nested_template(template.tir_reference, loop_binding_paths)
        }

        ExpressionKind::ChoiceConstruct { fields, .. }
        | ExpressionKind::StructInstance(fields)
        | ExpressionKind::AnonymousConstRecord { fields } => {
            let mut const_evaluable = true;
            for field in fields {
                const_evaluable &= classify_expression_const_evaluable_with_nested_template(
                    &field.value,
                    loop_binding_paths,
                    nested_template,
                )?;
            }
            Ok(const_evaluable)
        }

        ExpressionKind::Collection(items) => {
            let mut const_evaluable = true;
            for item in items {
                const_evaluable &= classify_expression_const_evaluable_with_nested_template(
                    item,
                    loop_binding_paths,
                    nested_template,
                )?;
            }
            Ok(const_evaluable)
        }

        ExpressionKind::Range(start, end) => {
            Ok(classify_expression_const_evaluable_with_nested_template(
                start,
                loop_binding_paths,
                nested_template,
            )? && classify_expression_const_evaluable_with_nested_template(
                end,
                loop_binding_paths,
                nested_template,
            )?)
        }

        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Copy(_)
        | ExpressionKind::Function(_)
        | ExpressionKind::FunctionCall { .. }
        | ExpressionKind::FieldAccess { .. }
        | ExpressionKind::MethodCall { .. }
        | ExpressionKind::CollectionBuiltinCall { .. }
        | ExpressionKind::MapBuiltinCall { .. }
        | ExpressionKind::HandledFallibleFunctionCall { .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { .. }
        | ExpressionKind::Cast(_)
        | ExpressionKind::HandledFallibleExpression { .. }
        | ExpressionKind::OptionPropagation { .. }
        | ExpressionKind::HostFunctionCall { .. }
        | ExpressionKind::RuntimeTemplateHandoff(_)
        | ExpressionKind::RuntimeSlotApplicationHandoff(_)
        | ExpressionKind::MapLiteral(_)
        | ExpressionKind::StructDefinition(_)
        | ExpressionKind::ValueBlock { .. } => Ok(false),
    }
}

pub(crate) fn tir_node_is_const_evaluable_value(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    string_table: &StringTable,
) -> Result<bool, TemplateError> {
    tir_tree_is_const_evaluable_standalone_value(store, node_id, string_table, &mut HashSet::new())
}

fn tir_tree_is_const_evaluable_standalone_value(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    string_table: &StringTable,
    visiting_templates: &mut HashSet<TemplateIrId>,
) -> Result<bool, TemplateError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "TIR runtime contribution constness referenced missing node {node_id}."
        ))
    })?;

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child in children {
                if !tir_tree_is_const_evaluable_standalone_value(
                    store,
                    *child,
                    string_table,
                    visiting_templates,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        TemplateIrNodeKind::Text { .. } => Ok(store.node_reactive_subscription(node_id)?.is_none()),
        TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. } => Ok(true),
        TemplateIrNodeKind::DynamicExpression { expression, .. } => {
            let kind =
                expression.const_value_kind_with_template_classifier(&mut |template| {
                    let nested = store.get_template(template.tir_reference.root).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "TIR runtime contribution constness referenced missing template {}.",
                        template.tir_reference.root
                    ))
                })?;
                    if template.tir_reference.context != TemplateViewContext::default()
                        || !visiting_templates.insert(template.tir_reference.root)
                    {
                        return Ok(TemplateConstValueKind::NonConst);
                    }
                    let result = tir_tree_is_const_evaluable_standalone_value(
                        store,
                        nested.root,
                        string_table,
                        visiting_templates,
                    )?;
                    visiting_templates.remove(&template.tir_reference.root);
                    Ok(if result {
                        TemplateConstValueKind::RenderableString
                    } else {
                        TemplateConstValueKind::NonConst
                    })
                })?;
            Ok(kind.is_compile_time_value())
        }
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template = store.get_template(reference.root).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR runtime contribution constness referenced missing template {}.",
                    reference.root
                ))
            })?;
            if !visiting_templates.insert(reference.root) {
                return Ok(false);
            }
            let result = tir_tree_is_const_evaluable_standalone_value(
                store,
                template.root,
                string_table,
                visiting_templates,
            );
            visiting_templates.remove(&reference.root);
            result
        }
        TemplateIrNodeKind::InsertContribution {
            template: template_id,
        } => {
            let template = store.get_template(*template_id).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR runtime contribution constness referenced missing template {}.",
                    template_id
                ))
            })?;
            if !visiting_templates.insert(*template_id) {
                return Ok(false);
            }
            let result = tir_tree_is_const_evaluable_standalone_value(
                store,
                template.root,
                string_table,
                visiting_templates,
            );
            visiting_templates.remove(template_id);
            result
        }
        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                let Some(bindings) = selector_is_const(
                    &branch.selector,
                    &[],
                    store,
                    string_table,
                    visiting_templates,
                )?
                else {
                    return Ok(false);
                };
                if !tir_tree_is_const_evaluable_value(
                    store,
                    branch.body,
                    &bindings,
                    string_table,
                    visiting_templates,
                )? {
                    return Ok(false);
                }
            }
            if let Some(fallback) = fallback {
                return tir_tree_is_const_evaluable_value(
                    store,
                    *fallback,
                    &[],
                    string_table,
                    visiting_templates,
                );
            }
            Ok(true)
        }
        TemplateIrNodeKind::Loop {
            header,
            body,
            aggregate_wrapper,
            ..
        } => {
            if !loop_header_is_const(header, store, string_table, visiting_templates)? {
                return Ok(false);
            }
            let bindings = loop_body_const_evaluation_bindings(header, &[]);
            if !tir_tree_is_const_evaluable_value(
                store,
                *body,
                &bindings,
                string_table,
                visiting_templates,
            )? {
                return Ok(false);
            }
            if let Some(wrapper) = aggregate_wrapper {
                return tir_tree_is_const_evaluable_value(
                    store,
                    *wrapper,
                    &[],
                    string_table,
                    visiting_templates,
                );
            }
            Ok(true)
        }
        TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(false),
    }
}

fn tir_tree_is_const_evaluable_value(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    loop_binding_paths: &[InternedPath],
    string_table: &StringTable,
    visiting_templates: &mut HashSet<TemplateIrId>,
) -> Result<bool, TemplateError> {
    let node = store.get_node(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "TIR runtime contribution constness referenced missing node {node_id}."
        ))
    })?;

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child in children {
                if !tir_tree_is_const_evaluable_value(
                    store,
                    *child,
                    loop_binding_paths,
                    string_table,
                    visiting_templates,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        TemplateIrNodeKind::Text { .. } => Ok(store.node_reactive_subscription(node_id)?.is_none()),
        TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. } => Ok(true),
        TemplateIrNodeKind::DynamicExpression { expression, .. } => expression_is_const_evaluable(
            expression,
            loop_binding_paths,
            store,
            string_table,
            visiting_templates,
        ),
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template = store.get_template(reference.root).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR runtime contribution constness referenced missing template {}.",
                    reference.root
                ))
            })?;
            if !visiting_templates.insert(reference.root) {
                return Ok(false);
            }
            let result = tir_tree_is_const_evaluable_value(
                store,
                template.root,
                &[],
                string_table,
                visiting_templates,
            );
            visiting_templates.remove(&reference.root);
            result
        }
        TemplateIrNodeKind::InsertContribution {
            template: template_id,
        } => {
            let template = store.get_template(*template_id).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR runtime contribution constness referenced missing template {}.",
                    template_id
                ))
            })?;
            if !visiting_templates.insert(*template_id) {
                return Ok(false);
            }
            let result = tir_tree_is_const_evaluable_value(
                store,
                template.root,
                &[],
                string_table,
                visiting_templates,
            );
            visiting_templates.remove(template_id);
            result
        }
        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                let Some(bindings) = selector_is_const(
                    &branch.selector,
                    loop_binding_paths,
                    store,
                    string_table,
                    visiting_templates,
                )?
                else {
                    return Ok(false);
                };
                if !tir_tree_is_const_evaluable_value(
                    store,
                    branch.body,
                    &bindings,
                    string_table,
                    visiting_templates,
                )? {
                    return Ok(false);
                }
            }
            if let Some(fallback) = fallback {
                return tir_tree_is_const_evaluable_value(
                    store,
                    *fallback,
                    loop_binding_paths,
                    string_table,
                    visiting_templates,
                );
            }
            Ok(true)
        }
        TemplateIrNodeKind::Loop {
            header,
            body,
            aggregate_wrapper,
            ..
        } => {
            if !loop_header_is_const(header, store, string_table, visiting_templates)? {
                return Ok(false);
            }
            let bindings = loop_body_const_evaluation_bindings(header, loop_binding_paths);
            if !tir_tree_is_const_evaluable_value(
                store,
                *body,
                &bindings,
                string_table,
                visiting_templates,
            )? {
                return Ok(false);
            }
            if let Some(wrapper) = aggregate_wrapper {
                return tir_tree_is_const_evaluable_value(
                    store,
                    *wrapper,
                    loop_binding_paths,
                    string_table,
                    visiting_templates,
                );
            }
            Ok(true)
        }
        TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(false),
    }
}

fn expression_is_const_evaluable(
    expression: &Expression,
    loop_binding_paths: &[InternedPath],
    store: &TemplateIrStore,
    string_table: &StringTable,
    visiting_templates: &mut HashSet<TemplateIrId>,
) -> Result<bool, TemplateError> {
    classify_expression_const_evaluable_with_nested_template(
        expression,
        loop_binding_paths,
        &mut |reference, bindings| {
            let template = store.get_template(reference.root).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR runtime contribution constness referenced missing template {}.",
                    reference.root
                ))
            })?;
            if reference.context != TemplateViewContext::default() {
                return Ok(false);
            }
            if !visiting_templates.insert(reference.root) {
                return Ok(false);
            }
            let result = tir_tree_is_const_evaluable_value(
                store,
                template.root,
                bindings,
                string_table,
                visiting_templates,
            )?;
            visiting_templates.remove(&reference.root);
            Ok(result)
        },
    )
}

fn selector_is_const(
    selector: &TemplateBranchSelector,
    loop_binding_paths: &[InternedPath],
    store: &TemplateIrStore,
    string_table: &StringTable,
    visiting_templates: &mut HashSet<TemplateIrId>,
) -> Result<Option<Vec<InternedPath>>, TemplateError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => Ok(expression_is_const_evaluable(
            condition,
            loop_binding_paths,
            store,
            string_table,
            visiting_templates,
        )?
        .then(|| loop_binding_paths.to_vec())),
        TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
            let decidable = match &scrutinee.kind {
                ExpressionKind::OptionNone => true,
                ExpressionKind::Reference(path) => {
                    loop_binding_paths.iter().any(|known| known == path)
                }
                ExpressionKind::Coerced { value, .. } => expression_is_const_evaluable(
                    value,
                    loop_binding_paths,
                    store,
                    string_table,
                    visiting_templates,
                )?,
                _ => false,
            };
            if !decidable {
                return Ok(None);
            }
            let mut bindings = loop_binding_paths.to_vec();
            collect_option_capture_binding_path(pattern, &mut bindings);
            Ok(Some(bindings))
        }
    }
}

fn loop_header_is_const(
    header: &TemplateLoopHeader,
    store: &TemplateIrStore,
    string_table: &StringTable,
    visiting_templates: &mut HashSet<TemplateIrId>,
) -> Result<bool, TemplateError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            expression_is_const_evaluable(condition, &[], store, string_table, visiting_templates)
        }
        TemplateLoopHeader::Range { range, .. } => {
            if !expression_is_const_evaluable(
                &range.start,
                &[],
                store,
                string_table,
                visiting_templates,
            )? || !expression_is_const_evaluable(
                &range.end,
                &[],
                store,
                string_table,
                visiting_templates,
            )? {
                return Ok(false);
            }
            if let Some(step) = &range.step {
                return expression_is_const_evaluable(
                    step,
                    &[],
                    store,
                    string_table,
                    visiting_templates,
                );
            }
            Ok(true)
        }
        TemplateLoopHeader::Collection { iterable, .. } => {
            expression_is_const_evaluable(iterable, &[], store, string_table, visiting_templates)
        }
    }
}
