//! Effective expression-site traversal over the paired TIR and expression owners.
//! Exact TIR identities and named structural transitions remain the authority for
//! overlays. Nested template values enter their independently retained view.

use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{TirView, TirViewIdentity};
use crate::compiler_frontend::ast::templates::tir::{TemplateIrNodeId, TemplateTirReference};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::HashSet;

pub(crate) fn walk_tir_view_expression_payloads<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    walk_view(view, expressions, visitor, &mut HashSet::new())
}

fn walk_view<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), CompilerError>,
    visited: &mut HashSet<TirViewIdentity>,
) -> Result<(), CompilerError> {
    if !visited.insert(view.identity()) {
        return Ok(());
    }
    walk_node(
        view,
        view.root_template()?.root,
        expressions,
        visitor,
        visited,
    )
}

/// Predicate-oriented nested traversal uses a worklist so expression children
/// cannot retain a mutable borrow of the TIR visited set during re-entry.
pub(crate) fn walk_expression_payloads_with_nested_tir_views<'expressions>(
    expression: ExpressionView<'expressions>,
    store: &TemplateIrStore,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    let mut visited = HashSet::new();
    let mut pending = Vec::new();
    inspect_expression(expression, visitor, &mut pending)?;
    while let Some(reference) = pending.pop() {
        let view = TirView::new(store, reference.root, reference.phase, reference.context)?;
        let mut inspect = |expression| inspect_expression(expression, visitor, &mut pending);
        walk_view(&view, expressions, &mut inspect, &mut visited)?;
    }
    Ok(())
}

fn inspect_expression<'expressions>(
    expression: ExpressionView<'expressions>,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), CompilerError>,
    pending: &mut Vec<TemplateTirReference>,
) -> Result<(), CompilerError> {
    match expression.operation() {
        ExpressionOperationView::Template(template) => pending.push(template.tir_reference),
        ExpressionOperationView::Coerce { value, .. } => {
            inspect_expression(value, visitor, pending)?
        }
        ExpressionOperationView::NumericUnary { .. }
        | ExpressionOperationView::NumericBinary { .. }
        | ExpressionOperationView::LogicalNot { .. }
        | ExpressionOperationView::LogicalBinary { .. }
        | ExpressionOperationView::Compare { .. } => {
            let mut result = Ok(());
            expression.visit_children(|child| {
                if result.is_ok() {
                    result = inspect_expression(child, visitor, pending);
                }
            });
            result?;
        }
        _ => visitor(expression)?,
    }
    Ok(())
}

fn effective<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    site: super::ids::ExpressionSiteId,
    structural: ExprId,
) -> Result<ExpressionView<'expressions>, CompilerError> {
    match view.effective_expression_for_site(expressions, site)? {
        Some(expression) => Ok(expression),
        None => expressions.view(structural),
    }
}

fn walk_node<'expressions>(
    view: &TirView<'_>,
    node: TemplateIrNodeId,
    expressions: &'expressions AstExpressionBuilder,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), CompilerError>,
    visited: &mut HashSet<TirViewIdentity>,
) -> Result<(), CompilerError> {
    match &view.effective_node(node)?.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for &child in children {
                walk_node(view, child, expressions, visitor, visited)?;
            }
        }
        TemplateIrNodeKind::DynamicExpression {
            expression,
            site_id,
            ..
        } => {
            visitor(effective(view, expressions, *site_id, *expression)?)?;
        }
        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            visitor(effective(
                view,
                expressions,
                *selector_site_id,
                selector.condition_expression(),
            )?)?;
            walk_node(view, *body, expressions, visitor, visited)?;
        }
        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
            ..
        } => {
            visit_loop_header(view, expressions, header, *header_sites, visitor)?;
            walk_node(view, *body, expressions, visitor, visited)?;
            if let Some(wrapper) = aggregate_wrapper {
                walk_node(view, *wrapper, expressions, visitor, visited)?;
            }
        }
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            walk_view(
                &view.structural_child(*reference)?,
                expressions,
                visitor,
                visited,
            )?;
        }
        TemplateIrNodeKind::InsertContribution { template } => {
            walk_view(
                &view.structural_helper(*template)?,
                expressions,
                visitor,
                visited,
            )?;
        }
        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. }
        | TemplateIrNodeKind::RetainedNumericIntake(_) => {}
    }
    Ok(())
}

fn visit_loop_header<'expressions>(
    view: &TirView<'_>,
    expressions: &'expressions AstExpressionBuilder,
    header: &TemplateLoopHeader,
    sites: TemplateLoopHeaderExpressionSites,
    visitor: &mut impl FnMut(ExpressionView<'expressions>) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    match (header, sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site },
        ) => {
            visitor(effective(view, expressions, site, *condition)?)?;
        }
        (
            TemplateLoopHeader::Collection { iterable, .. },
            TemplateLoopHeaderExpressionSites::Collection { iterable: site },
        ) => {
            visitor(effective(view, expressions, site, *iterable)?)?;
        }
        (
            TemplateLoopHeader::Range { range, .. },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            visitor(effective(view, expressions, start, range.start)?)?;
            visitor(effective(view, expressions, end, range.end)?)?;
            match (step, range.step) {
                (Some(site), Some(expression)) => {
                    visitor(effective(view, expressions, site, expression)?)?
                }
                (None, None) => {}
                _ => {
                    return Err(CompilerError::compiler_error(
                        "TIR expression walk found mismatched range step site.",
                    ));
                }
            }
        }
        _ => {
            return Err(CompilerError::compiler_error(
                "TIR expression walk found mismatched loop-header expression sites.",
            ));
        }
    }
    Ok(())
}
