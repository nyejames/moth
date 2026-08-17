//! TIR expression-site walkers.
//!
//! WHAT: provides read-only effective-view traversals over every expression
//!       payload reachable from a finalized `TirView`, plus the nested
//!       expression-and-TIR-view walker used by the head parser.
//! WHY: final type-boundary validation and debug TypeId validation both need to
//!      inspect the same expression-bearing TIR nodes; centralizing the walks in
//!      TIR keeps the traversal authoritative and removes near-duplicate local
//!      helpers from AST finalization. The `TirView` walk reads effective
//!      expression overlays for dynamic-expression splices, branch selectors
//!      and loop headers, and recurses into child-template and
//!      insert-contribution views through one shared visited set. The
//!      nested-expression walker additionally recurses into `ExpressionKind`
//!      internals and re-enters TIR views for template-valued expressions.

use std::collections::HashSet;

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{TirView, TirViewIdentity};
use crate::compiler_frontend::ast::templates::tir::{TemplateIrNodeId, TemplateTirReference};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// Walks every expression payload reachable from `view`, reading effective
/// expression overlays for dynamic-expression nodes, branch selectors, and
/// loop headers.
///
/// WHAT: recursively traverses the structural root of `view` and its
///       module-local child-template and insert-contribution descendants.
///       Dynamic-expression splices, branch selectors and loop-header
///       expressions prefer the override expression provided by each effective
///       view, falling back to the stored structural expression. Insert
///       contributions recurse through a child `TirView` that inherits the
///       parent phase and view context, so every reachable payload is read
///       through the same effective-view authority.
/// WHY: centralizes the view-based expression-payload traversal used by debug
///      TypeId validation and final type-boundary validation without
///      duplicating overlay-resolution logic in finalization.
pub(crate) fn walk_tir_view_expression_payloads(
    view: &TirView<'_>,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    let mut visited_templates = HashSet::new();
    walk_tir_view_expression_payloads_with_visited(view, visitor, &mut visited_templates)
}

/// Walks every expression payload reachable from `view`, sharing `visited_templates`.
///
/// WHAT: same structural coverage as [`walk_tir_view_expression_payloads`], but
///       accepts an external visited set so callers that also enter TIR views
///       through nested `ExpressionKind` paths can share one cycle-prevention
///       set keyed by exact `TirViewIdentity` values.
/// WHY: the nested-expression walker needs a single visited set across both
///      TIR-view child-template references and `ExpressionKind::Template`
///      re-entries; extracting this entry point avoids duplicating the
///      view-walk logic while keeping the standalone API unchanged for
///      type-boundary and debug-TypeId validation.
fn walk_tir_view_expression_payloads_with_visited(
    view: &TirView<'_>,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
    visited_templates: &mut HashSet<TirViewIdentity>,
) -> Result<(), CompilerError> {
    let identity = view.identity();
    if !visited_templates.insert(identity) {
        return Ok(());
    }

    let root_node_id = {
        let root_template = view.root_template()?;
        root_template.root
    };
    let root_node_ref = root_node_id;

    walk_tir_view_expression_payload_node(view, root_node_ref, visitor, visited_templates)
}

/// Walks every expression payload reachable from `expression`, including nested
/// `ExpressionKind` internals and template-valued TIR views, using one shared
/// visited set keyed by exact `TirViewIdentity` values.
///
/// WHAT: starts from an AST expression, recursively inspects `ExpressionKind`
///       internals (`Runtime` operands, `Coerced` values), and enters the
///       effective TIR view for each `ExpressionKind::Template` encountered.
///       TIR view expression payloads are likewise inspected for nested
///       template-valued expressions. One visited set prevents infinite
///       recursion across both expression-kind and TIR-view paths. The visitor
///       receives every expression that is not a `Template`, `Runtime`, or
///       `Coerced` wrapper, including `RuntimeSlotApplicationHandoff` payloads.
/// WHY: centralizes the store-aware predicate traversal so the head parser
///      does not duplicate `ExpressionKind` recursion or maintain its own
///      effective-template visited set.
pub(crate) fn walk_expression_payloads_with_nested_tir_views(
    expression: &Expression,
    store: &TemplateIrStore,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    let mut visited_templates = HashSet::new();
    let mut pending_template_views: Vec<TemplateTirReference> = Vec::new();

    inspect_nested_expression_kind(expression, visitor, &mut pending_template_views)?;

    drain_pending_template_views(
        store,
        visitor,
        &mut visited_templates,
        &mut pending_template_views,
    )
}

/// Processes each template reference discovered in nested expression kinds.
///
/// WHAT: for each pending `TemplateTirReference`, checks the shared visited
///       set, validates the module-local reference, creates a `TirView`, and walks
///       its expression payloads while collecting further nested template
///       references.
/// WHY: using a worklist instead of immediate re-entry avoids borrow conflicts
///      between the TIR view walker (which holds `&mut visited_templates`) and
///      the nested-expression inspector (which needs to push new pending
///      references). Traversal order is not part of this predicate-oriented
///      API, while coverage and one-set cycle semantics match the original
///      head-parser recursion.
fn drain_pending_template_views(
    store: &TemplateIrStore,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
    visited_templates: &mut HashSet<TirViewIdentity>,
    pending_template_views: &mut Vec<TemplateTirReference>,
) -> Result<(), CompilerError> {
    while let Some(reference) = pending_template_views.pop() {
        let identity = TirViewIdentity {
            root: reference.root,
            phase: reference.phase,
            context: reference.context,
        };
        if visited_templates.contains(&identity) {
            continue;
        }

        // This is a worklist entry from an independently owned nested AST
        // value, so its durable reference supplies the complete root context.
        // Structural descendants below that entry still use named transitions.
        let view = TirView::new(store, reference.root, reference.phase, reference.context)?;

        let mut expression_visitor = |expression: &Expression| {
            inspect_nested_expression_kind(expression, visitor, pending_template_views)
        };
        walk_tir_view_expression_payloads_with_visited(
            &view,
            &mut expression_visitor,
            visited_templates,
        )?;
    }

    Ok(())
}

/// Recursively inspects `ExpressionKind` internals, collecting template
/// references and calling `visitor` for all other expression kinds.
///
/// WHAT: descends into `Runtime` operands and `Coerced` values, pushes
///       `ExpressionKind::Template` references to the pending list, and passes
///       every other kind (including `RuntimeSlotApplicationHandoff`) to the
///       visitor. Does not access the visited set; cycle prevention is handled
///       by the caller when draining pending references.
/// WHY: matches the `ExpressionKind` recursion previously duplicated in the
///      head parser so the central walker owns both TIR structural traversal
///      and nested expression inspection.
fn inspect_nested_expression_kind(
    expression: &Expression,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
    pending_template_views: &mut Vec<TemplateTirReference>,
) -> Result<(), CompilerError> {
    match &expression.kind {
        ExpressionKind::Template(template) => {
            pending_template_views.push(template.tir_reference);
            Ok(())
        }

        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                if let ExpressionRpnItem::Operand(operand) = item {
                    inspect_nested_expression_kind(operand, visitor, pending_template_views)?;
                }
            }
            Ok(())
        }

        ExpressionKind::Coerced { value, .. } => {
            inspect_nested_expression_kind(value, visitor, pending_template_views)
        }

        _ => visitor(expression),
    }
}

fn walk_tir_view_expression_payload_node(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
    visited_templates: &mut HashSet<TirViewIdentity>,
) -> Result<(), CompilerError> {
    let node = view.effective_node(node_ref)?;

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            let children = children.clone();
            for child in children {
                walk_tir_view_expression_payload_node(view, child, visitor, visited_templates)?;
            }
        }

        TemplateIrNodeKind::DynamicExpression {
            expression,
            site_id,
            ..
        } => {
            let effective_expression = view.effective_expression_for_site(*site_id)?;
            if let Some(expression) = effective_expression {
                visitor(expression)?;
            } else {
                visitor(expression.as_ref())?;
            }
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let branches = branches.clone();
            let fallback = *fallback;
            for branch in &branches {
                let expression = view
                    .effective_expression_for_site(branch.selector_site_id)?
                    .unwrap_or(branch.condition_expression());
                visitor(expression)?;
                walk_tir_view_expression_payload_node(
                    view,
                    branch.body,
                    visitor,
                    visited_templates,
                )?;
            }
            if let Some(fallback_id) = fallback {
                walk_tir_view_expression_payload_node(
                    view,
                    fallback_id,
                    visitor,
                    visited_templates,
                )?;
            }
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
            ..
        } => {
            let header = header.clone();
            let header_sites = *header_sites;
            let body = *body;
            let aggregate_wrapper = *aggregate_wrapper;
            visit_loop_header_effective_expressions(view, &header, header_sites, visitor)?;
            walk_tir_view_expression_payload_node(view, body, visitor, visited_templates)?;
            if let Some(wrapper_id) = aggregate_wrapper {
                walk_tir_view_expression_payload_node(
                    view,
                    wrapper_id,
                    visitor,
                    visited_templates,
                )?;
            }
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let reference = *reference;
            let child_view = view.structural_child(reference)?;
            if visited_templates.insert(child_view.identity()) {
                let child_root_node_id = {
                    let child_root_template = child_view.root_template()?;
                    child_root_template.root
                };
                let child_root_node_ref = child_root_node_id;

                // Child references still carry a complete effective view
                // identity. Follow that view through the store rather than
                // silently treating the reference as an opaque leaf.
                walk_tir_view_expression_payload_node(
                    &child_view,
                    child_root_node_ref,
                    visitor,
                    visited_templates,
                )?;
            }
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let template_id = *template;
            let insert_view = view.structural_helper(template_id)?;
            if visited_templates.insert(insert_view.identity()) {
                // Insert contributions inherit the parent phase and view context,
                // so they recurse through a child `TirView` instead of walking the
                // store directly. A missing insert template or view context is an
                // explicit internal error from the structural-helper transition.
                let insert_root_node_id = {
                    let insert_root_template = insert_view.root_template()?;
                    insert_root_template.root
                };
                let insert_root_node_ref = insert_root_node_id;

                walk_tir_view_expression_payload_node(
                    &insert_view,
                    insert_root_node_ref,
                    visitor,
                    visited_templates,
                )?;
            }
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {}
    }

    Ok(())
}

/// Visits the effective expressions for one loop header, resolving overrides
/// through the view when present.
///
/// WHAT: matches the header shape against its allocated expression sites and
///       calls the visitor for each site, preferring the overlay override and
///       falling back to the stored structural expression. A mismatched shape is
///       reported as an internal invariant error.
/// WHY: loop-header sites share the same `ExpressionSiteId` key space as
///      dynamic-expression and branch-selector sites; resolving them through the
///      view keeps overlay resolution in one place.
fn visit_loop_header_effective_expressions(
    view: &TirView<'_>,
    header: &TemplateLoopHeader,
    header_sites: TemplateLoopHeaderExpressionSites,
    visitor: &mut impl FnMut(&Expression) -> Result<(), CompilerError>,
) -> Result<(), CompilerError> {
    match (header, header_sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
        ) => {
            let expression = view
                .effective_expression_for_site(site_id)?
                .unwrap_or(condition.as_ref());
            visitor(expression)?;
        }

        (
            TemplateLoopHeader::Range { range, .. },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            let start_expression = view
                .effective_expression_for_site(start)?
                .unwrap_or(&range.start);
            visitor(start_expression)?;

            let end_expression = view
                .effective_expression_for_site(end)?
                .unwrap_or(&range.end);
            visitor(end_expression)?;

            if let Some(step_site_id) = step {
                let step_expression = if let Some(expression) =
                    view.effective_expression_for_site(step_site_id)?
                {
                    expression
                } else {
                    range.step.as_ref().ok_or_else(|| {
                        CompilerError::compiler_error(
                            "TIR view expression-payload walk found a range loop step site without a structural step expression.",
                        )
                    })?
                };
                visitor(step_expression)?;
            }
        }

        (
            TemplateLoopHeader::Collection { iterable, .. },
            TemplateLoopHeaderExpressionSites::Collection { iterable: site_id },
        ) => {
            let expression = view
                .effective_expression_for_site(site_id)?
                .unwrap_or(iterable.as_ref());
            visitor(expression)?;
        }

        _ => {
            return Err(CompilerError::compiler_error(
                "TIR view expression-payload walk found mismatched loop-header expression sites.",
            ));
        }
    }

    Ok(())
}
