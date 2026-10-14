//! Narrow constness rules shared by TIR preparation and runtime slot planning.
//!
//! This module owns expression-kind constness, exact branch/loop overlay payload
//! selection and the small structural query needed by runtime contribution
//! planning. It does not classify complete template views, slots or insert overlays.

use std::collections::HashSet;

use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, ExpressionView,
};
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
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(crate) fn effective_branch_selector_for_view(
    view: &TirView<'_>,
    expressions: &AstExpressionBuilder,
    selector: &TemplateBranchSelector,
    site_id: ExpressionSiteId,
) -> Result<TemplateBranchSelector, CompilerError> {
    let Some(expression) = view.effective_expression_for_site(expressions, site_id)? else {
        return Ok(selector.clone());
    };

    Ok(match selector {
        TemplateBranchSelector::Bool(_) => TemplateBranchSelector::Bool(expression.id()),
        TemplateBranchSelector::OptionPresentCapture { pattern, .. } => {
            TemplateBranchSelector::OptionPresentCapture {
                scrutinee: expression.id(),
                pattern: pattern.clone(),
            }
        }
    })
}

pub(crate) fn effective_loop_header_for_view(
    view: &TirView<'_>,
    expressions: &AstExpressionBuilder,
    header: &TemplateLoopHeader,
    header_sites: TemplateLoopHeaderExpressionSites,
) -> Result<TemplateLoopHeader, CompilerError> {
    Ok(match (header, header_sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
        ) => TemplateLoopHeader::Conditional {
            condition: view
                .effective_expression_for_site(expressions, site_id)?
                .map(ExpressionView::id)
                .unwrap_or(*condition),
        },
        (
            TemplateLoopHeader::Range { bindings, range },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            let mut range = range.as_ref().clone();
            if let Some(expression) = view.effective_expression_for_site(expressions, start)? {
                range.start = expression.id();
            }
            if let Some(expression) = view.effective_expression_for_site(expressions, end)? {
                range.end = expression.id();
            }
            match (&range.step, step) {
                (None, None) => {}
                (Some(_), Some(step_site_id)) => {
                    if let Some(expression) =
                        view.effective_expression_for_site(expressions, step_site_id)?
                    {
                        range.step = Some(expression.id());
                    }
                }
                _ => {
                    return Err(CompilerError::compiler_error(
                        "TIR range-loop header/site step shape mismatch.",
                    ));
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
            iterable: view
                .effective_expression_for_site(expressions, site_id)?
                .map(ExpressionView::id)
                .unwrap_or(*iterable),
        },
        _ => {
            return Err(CompilerError::compiler_error(
                "TIR loop header shape does not match its expression sites.",
            ));
        }
    })
}

/// Classifies operation views without reconstructing an expression tree. All
/// children are checked even when an earlier child is runtime-dependent.
pub(crate) fn classify_expression_const_evaluable_with_nested_template(
    expression: ExpressionView<'_>,
    loop_binding_paths: &[PathId],
    nested_template: &mut impl FnMut(TemplateTirReference, &[PathId]) -> Result<bool, TemplateError>,
) -> Result<bool, TemplateError> {
    use ExpressionOperationView as V;
    match expression.operation() {
        V::Constant(_) => Ok(true),
        V::Reference(path) => Ok(loop_binding_paths.contains(&path)),
        V::Template(template) => nested_template(template.tir_reference, loop_binding_paths),
        V::Coerce { .. }
        | V::NumericUnary { .. }
        | V::NumericBinary { .. }
        | V::LogicalNot { .. }
        | V::LogicalBinary { .. }
        | V::Compare { .. }
        | V::Struct { .. }
        | V::Choice { .. }
        | V::Collection(_)
        | V::Range { .. } => {
            let mut evaluable = true;
            let mut result = Ok(());
            expression.visit_children(|child| {
                if result.is_ok() {
                    match classify_expression_const_evaluable_with_nested_template(
                        child,
                        loop_binding_paths,
                        nested_template,
                    ) {
                        Ok(value) => evaluable &= value,
                        Err(error) => result = Err(error),
                    }
                }
            });
            result?;
            Ok(evaluable)
        }
        _ => Ok(false),
    }
}

pub(crate) fn tir_node_is_const_evaluable_value(
    store: &TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    node_id: TemplateIrNodeId,
    _string_table: &StringTable,
) -> Result<bool, TemplateError> {
    let mut query = StructuralConstness {
        store,
        expressions,
        const_values,
        visiting_templates: HashSet::new(),
    };
    query.tree(node_id, &[], true)
}

/// Structural contribution planning deliberately refuses non-default nested
/// overlays. Preparation owns exact effective-view classification instead.
struct StructuralConstness<'a> {
    store: &'a TemplateIrStore,
    expressions: &'a AstExpressionBuilder,
    const_values: &'a ConstValueStore,
    visiting_templates: HashSet<TemplateIrId>,
}

impl StructuralConstness<'_> {
    fn tree(
        &mut self,
        node_id: TemplateIrNodeId,
        bindings: &[PathId],
        standalone: bool,
    ) -> Result<bool, TemplateError> {
        let node = self.store.get_node(node_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR runtime contribution constness referenced missing node {node_id}."
            ))
        })?;
        match &node.kind {
            TemplateIrNodeKind::Sequence { children } => {
                for &child in children {
                    if !self.tree(child, bindings, standalone)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            TemplateIrNodeKind::Text { .. }
            | TemplateIrNodeKind::Slot { .. }
            | TemplateIrNodeKind::AggregateOutput => Ok(true),
            TemplateIrNodeKind::DynamicExpression { expression, .. } => {
                let expression = self.expressions.view(*expression)?;
                if standalone {
                    let kind = expression.const_value_kind_with_template_classifier(
                        self.const_values,
                        &mut |template| {
                            let reference = template.tir_reference;
                            if reference.context != TemplateViewContext::default() {
                                return Ok(TemplateConstValueKind::NonConst);
                            }
                            let result = self.nested(reference.root, &[], true)?;
                            Ok(if result {
                                TemplateConstValueKind::RenderableString
                            } else {
                                TemplateConstValueKind::NonConst
                            })
                        },
                    )?;
                    Ok(kind.is_compile_time_value())
                } else {
                    self.expression(expression, bindings)
                }
            }
            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                self.nested(reference.root, &[], standalone)
            }
            TemplateIrNodeKind::InsertContribution { template } => {
                self.nested(*template, &[], standalone)
            }
            TemplateIrNodeKind::Conditional { selector, body, .. } => {
                let Some(body_bindings) = self.selector(selector, bindings)? else {
                    return Ok(false);
                };
                self.tree(*body, &body_bindings, false)
            }
            TemplateIrNodeKind::Loop {
                header,
                body,
                aggregate_wrapper,
                ..
            } => {
                if !self.header(header, bindings)? {
                    return Ok(false);
                }
                let body_bindings = loop_body_const_evaluation_bindings(header, bindings);
                if !self.tree(*body, &body_bindings, false)? {
                    return Ok(false);
                }
                if let Some(wrapper) = aggregate_wrapper {
                    return self.tree(*wrapper, if standalone { &[] } else { bindings }, false);
                }
                Ok(true)
            }
            TemplateIrNodeKind::RuntimeSlotSite { .. }
            | TemplateIrNodeKind::RuntimeSlotContributionSource { .. }
            | TemplateIrNodeKind::RetainedNumericIntake(_) => Ok(false),
        }
    }

    fn nested(
        &mut self,
        id: TemplateIrId,
        bindings: &[PathId],
        standalone: bool,
    ) -> Result<bool, TemplateError> {
        let root = self
            .store
            .get_template(id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "TIR runtime contribution constness referenced missing template {id}."
                ))
            })?
            .root;
        if !self.visiting_templates.insert(id) {
            return Ok(false);
        }
        let result = self.tree(root, bindings, standalone);
        self.visiting_templates.remove(&id);
        result
    }

    fn expression(
        &mut self,
        expression: ExpressionView<'_>,
        bindings: &[PathId],
    ) -> Result<bool, TemplateError> {
        classify_expression_const_evaluable_with_nested_template(
            expression,
            bindings,
            &mut |reference, nested_bindings| {
                if reference.context != TemplateViewContext::default() {
                    return Ok(false);
                }
                self.nested(reference.root, nested_bindings, false)
            },
        )
    }

    fn selector(
        &mut self,
        selector: &TemplateBranchSelector,
        bindings: &[PathId],
    ) -> Result<Option<Vec<PathId>>, TemplateError> {
        match selector {
            TemplateBranchSelector::Bool(id) => Ok(self
                .expression(self.expressions.view(*id)?, bindings)?
                .then(|| bindings.to_vec())),
            TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
                let expression = self.expressions.view(*scrutinee)?;
                let decidable = match expression.operation() {
                    ExpressionOperationView::Constant(id) => matches!(
                        self.const_values.value(id).map(|value| &value.payload),
                        Some(
                            ConstValuePayload::OptionNone
                                | ConstValuePayload::OptionSome(_)
                                | ConstValuePayload::Coerced(_)
                        )
                    ),
                    ExpressionOperationView::Reference(path) => bindings.contains(&path),
                    ExpressionOperationView::Coerce { value, .. } => {
                        self.expression(value, bindings)?
                    }
                    _ => false,
                };
                if !decidable {
                    return Ok(None);
                }
                let mut body_bindings = bindings.to_vec();
                collect_option_capture_binding_path(pattern, &mut body_bindings);
                Ok(Some(body_bindings))
            }
        }
    }

    fn header(
        &mut self,
        header: &TemplateLoopHeader,
        bindings: &[PathId],
    ) -> Result<bool, TemplateError> {
        match header {
            TemplateLoopHeader::Conditional { condition } => {
                Ok(const_conditional_loop_condition_value(
                    self.expressions.view(*condition)?,
                    self.const_values,
                )
                .is_some())
            }
            TemplateLoopHeader::Range { range, .. } => {
                if !self.expression(self.expressions.view(range.start)?, bindings)?
                    || !self.expression(self.expressions.view(range.end)?, bindings)?
                {
                    return Ok(false);
                }
                match range.step {
                    Some(step) => self.expression(self.expressions.view(step)?, bindings),
                    None => Ok(true),
                }
            }
            TemplateLoopHeader::Collection { iterable, .. } => {
                self.expression(self.expressions.view(*iterable)?, bindings)
            }
        }
    }
}

pub(crate) fn const_conditional_loop_condition_value(
    mut condition: ExpressionView<'_>,
    const_values: &ConstValueStore,
) -> Option<bool> {
    while let ExpressionOperationView::Coerce { value, .. } = condition.operation() {
        condition = value;
    }
    match condition.operation() {
        ExpressionOperationView::Constant(id) => match &const_values.value(id)?.payload {
            ConstValuePayload::Bool(value) => Some(*value),
            _ => None,
        },
        _ => None,
    }
}
