//! Reactive metadata reduction for exact TIR views.

use std::collections::HashSet;

use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, TemplateIrNodeId, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
    TemplateTirPhase, TemplateWrapperSetId, TirView, TirViewIdentity, runtime_slot_plan_roots,
    runtime_slot_plan_site_render_root,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

use super::{ReactiveMetadataResolver, merge_expression_metadata};

#[derive(Default)]
struct TirViewMetadataTraversal {
    active_views: HashSet<TirViewIdentity>,
    completed_views: HashSet<TirViewIdentity>,
}

#[derive(Clone, Copy)]
enum RuntimeSlotSiteMetadataMode {
    WalkRenderPieces,
    WrapperNodeOnly,
}

/// Merges template-backed metadata through one exact TIR view.
pub(crate) fn merge_reactive_template_metadata(
    view: &TirView<'_>,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    if !view.phase().is_at_least(TemplateTirPhase::Composed) {
        return Err(CompilerError::compiler_error(format!(
            "reactive TIR metadata: view rooted at {} is below the required Composed phase",
            view.root_ref()
        )));
    }

    let mut traversal = TirViewMetadataTraversal::default();
    merge_reactive_template_metadata_from_tir_view(view, metadata, resolver, &mut traversal)
}

fn merge_reactive_template_metadata_from_tir_view(
    view: &TirView<'_>,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    let identity = view.identity();
    if traversal.completed_views.contains(&identity) {
        return Ok(());
    }
    if !traversal.active_views.insert(identity) {
        return Err(CompilerError::compiler_error(format!(
            "reactive TIR metadata: exact view {identity:?} re-entered while still active."
        )));
    }

    let result = merge_tir_view_root_contents(view, metadata, resolver, traversal);

    traversal.active_views.remove(&identity);
    if result.is_ok() {
        traversal.completed_views.insert(identity);
    }

    result
}

fn merge_tir_view_root_contents(
    view: &TirView<'_>,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    let (root_node_id, slot_plan_id, conditional_child_wrapper_set) = {
        view.expression_overlay()?;
        view.slot_resolution_overlay()?;
        view.wrapper_context_overlay()?;

        let root_template = view.root_template()?;
        (
            root_template.root,
            root_template.runtime_slot_plan,
            root_template.conditional_child_wrapper_set,
        )
    };

    if let Some(slot_plan_id) = slot_plan_id {
        merge_tir_view_node_metadata(
            view,
            root_node_id,
            RuntimeSlotSiteMetadataMode::WrapperNodeOnly,
            metadata,
            resolver,
            traversal,
        )?;

        let (contribution_roots, site_render_roots) =
            runtime_slot_plan_roots(view.store(), slot_plan_id)?;

        for source_root in contribution_roots {
            merge_tir_view_node_metadata(
                view,
                source_root,
                RuntimeSlotSiteMetadataMode::WalkRenderPieces,
                metadata,
                resolver,
                traversal,
            )?;
        }

        for site_render_root in site_render_roots {
            merge_tir_view_node_metadata(
                view,
                site_render_root,
                RuntimeSlotSiteMetadataMode::WalkRenderPieces,
                metadata,
                resolver,
                traversal,
            )?;
        }
    } else {
        merge_tir_view_node_metadata(
            view,
            root_node_id,
            RuntimeSlotSiteMetadataMode::WalkRenderPieces,
            metadata,
            resolver,
            traversal,
        )?;
    }

    if let Some(wrapper_set_id) = conditional_child_wrapper_set {
        merge_tir_view_wrapper_set_metadata(view, wrapper_set_id, metadata, resolver, traversal)?;
    }

    Ok(())
}

fn merge_tir_view_node_metadata(
    view: &TirView<'_>,
    node_ref: TemplateIrNodeId,
    runtime_slot_site_mode: RuntimeSlotSiteMetadataMode,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    let node = view.effective_node(node_ref)?;

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for &child in children {
                merge_tir_view_node_metadata(
                    view,
                    child,
                    runtime_slot_site_mode,
                    metadata,
                    resolver,
                    traversal,
                )?;
            }
        }

        TemplateIrNodeKind::DynamicExpression {
            expression,
            reactive_subscription,
            site_id,
            ..
        } => {
            if let Some(subscription) = reactive_subscription {
                metadata.push_subscription(subscription.clone());
            }
            merge_effective_expression_metadata(
                view, *site_id, expression, metadata, resolver, traversal,
            )?;
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let wrapper_set_id =
                view.effective_wrapper_context(*occurrence_id)?
                    .and_then(|context| {
                        (!context.skip_parent_child_wrappers)
                            .then_some(context.inherited_wrapper_set)
                            .flatten()
                    });
            merge_optional_wrapper_set_metadata(
                view,
                wrapper_set_id,
                metadata,
                resolver,
                traversal,
            )?;

            let child_view = view.structural_child(*reference)?;
            merge_reactive_template_metadata_from_tir_view(
                &child_view,
                metadata,
                resolver,
                traversal,
            )?;
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let insert_view = view.structural_helper(*template)?;
            merge_reactive_template_metadata_from_tir_view(
                &insert_view,
                metadata,
                resolver,
                traversal,
            )?;
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                merge_effective_expression_metadata(
                    view,
                    branch.selector_site_id,
                    branch.condition_expression(),
                    metadata,
                    resolver,
                    traversal,
                )?;
                merge_tir_view_node_metadata(
                    view,
                    branch.body,
                    runtime_slot_site_mode,
                    metadata,
                    resolver,
                    traversal,
                )?;
            }

            if let Some(fallback) = fallback {
                merge_tir_view_node_metadata(
                    view,
                    *fallback,
                    runtime_slot_site_mode,
                    metadata,
                    resolver,
                    traversal,
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
            merge_tir_view_loop_header_metadata(
                view,
                header,
                header_sites,
                metadata,
                resolver,
                traversal,
            )?;
            merge_tir_view_node_metadata(
                view,
                *body,
                runtime_slot_site_mode,
                metadata,
                resolver,
                traversal,
            )?;

            if let Some(aggregate_wrapper) = aggregate_wrapper {
                merge_tir_view_node_metadata(
                    view,
                    *aggregate_wrapper,
                    runtime_slot_site_mode,
                    metadata,
                    resolver,
                    traversal,
                )?;
            }
        }

        TemplateIrNodeKind::RuntimeSlotSite { plan, site } => {
            if matches!(
                runtime_slot_site_mode,
                RuntimeSlotSiteMetadataMode::WrapperNodeOnly
            ) {
                runtime_slot_plan_site_render_root(view.store(), *plan, *site)?;
                return Ok(());
            }

            let render_root = runtime_slot_plan_site_render_root(view.store(), *plan, *site)?;
            merge_tir_view_node_metadata(
                view,
                render_root,
                RuntimeSlotSiteMetadataMode::WalkRenderPieces,
                metadata,
                resolver,
                traversal,
            )?;
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            merge_optional_wrapper_set_metadata(
                view,
                placeholder.applied_child_wrapper_set,
                metadata,
                resolver,
                traversal,
            )?;
            merge_optional_wrapper_set_metadata(
                view,
                placeholder.child_wrapper_set,
                metadata,
                resolver,
                traversal,
            )?;

            if let Some(resolution) = view.effective_slot_resolution(placeholder.occurrence_id)? {
                for source in resolution.sources() {
                    let source_view = view.resolved_slot_source(*source)?;
                    merge_reactive_template_metadata_from_tir_view(
                        &source_view,
                        metadata,
                        resolver,
                        traversal,
                    )?;
                }
            }
        }

        TemplateIrNodeKind::Text { .. } => {
            if let Some(subscription) = view.store().node_reactive_subscription(node_ref)? {
                metadata.push_subscription(subscription.clone());
            }
        }

        TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {}
    }

    Ok(())
}

fn merge_optional_wrapper_set_metadata(
    view: &TirView<'_>,
    wrapper_set_id: Option<TemplateWrapperSetId>,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    if let Some(wrapper_set_id) = wrapper_set_id {
        merge_tir_view_wrapper_set_metadata(view, wrapper_set_id, metadata, resolver, traversal)?;
    }

    Ok(())
}

fn merge_tir_view_wrapper_set_metadata(
    view: &TirView<'_>,
    wrapper_set_id: TemplateWrapperSetId,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    let wrapper_references = view
        .store()
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "reactive TIR metadata: wrapper set {} does not exist in owning store {}",
                wrapper_set_id,
                view.root_ref()
            ))
        })?
        .wrappers
        .clone();

    for wrapper_reference in wrapper_references {
        let wrapper_view = view.wrapper(wrapper_reference)?;
        merge_reactive_template_metadata_from_tir_view(
            &wrapper_view,
            metadata,
            resolver,
            traversal,
        )?;
    }

    Ok(())
}

fn merge_tir_view_loop_header_metadata(
    view: &TirView<'_>,
    header: &TemplateLoopHeader,
    header_sites: &TemplateLoopHeaderExpressionSites,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    match (header, header_sites) {
        (
            TemplateLoopHeader::Conditional { condition },
            TemplateLoopHeaderExpressionSites::Conditional { condition: site_id },
        ) => {
            merge_effective_expression_metadata(
                view,
                *site_id,
                condition.as_ref(),
                metadata,
                resolver,
                traversal,
            )?;
        }

        (
            TemplateLoopHeader::Range { range, .. },
            TemplateLoopHeaderExpressionSites::Range { start, end, step },
        ) => {
            merge_effective_expression_metadata(
                view,
                *start,
                &range.start,
                metadata,
                resolver,
                traversal,
            )?;
            merge_effective_expression_metadata(
                view, *end, &range.end, metadata, resolver, traversal,
            )?;

            match (&range.step, *step) {
                (None, None) => {}
                (Some(step_expression), Some(step_site_id)) => {
                    merge_effective_expression_metadata(
                        view,
                        step_site_id,
                        step_expression,
                        metadata,
                        resolver,
                        traversal,
                    )?;
                }
                _ => {
                    return Err(CompilerError::compiler_error(
                        "reactive TIR metadata: loop range header/site step shape mismatch",
                    ));
                }
            }
        }

        (
            TemplateLoopHeader::Collection { iterable, .. },
            TemplateLoopHeaderExpressionSites::Collection { iterable: site_id },
        ) => {
            merge_effective_expression_metadata(
                view,
                *site_id,
                iterable.as_ref(),
                metadata,
                resolver,
                traversal,
            )?;
        }

        _ => {
            return Err(CompilerError::compiler_error(
                "reactive TIR metadata: loop header shape does not match its expression sites",
            ));
        }
    }

    Ok(())
}

fn merge_effective_expression_metadata(
    view: &TirView<'_>,
    site_id: ExpressionSiteId,
    stored: &Expression,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    if let Some(expression) = view.effective_expression_for_site(site_id)? {
        merge_view_expression_metadata(view, expression, metadata, resolver, traversal)?;
    } else {
        merge_view_expression_metadata(view, stored, metadata, resolver, traversal)?;
    }

    Ok(())
}

fn merge_view_expression_metadata(
    view: &TirView<'_>,
    expression: &Expression,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
    traversal: &mut TirViewMetadataTraversal,
) -> Result<(), CompilerError> {
    let mut candidate = expression;
    let template = loop {
        match &candidate.kind {
            ExpressionKind::Template(template) => break Some(template),
            ExpressionKind::Coerced { value, .. } => candidate = value,
            _ => break None,
        }
    };

    if let Some(template) = template {
        let nested_view = view.nested_template_value(template.tir_reference)?;
        merge_reactive_template_metadata_from_tir_view(
            &nested_view,
            metadata,
            resolver,
            traversal,
        )?;
    } else {
        merge_expression_metadata(expression, metadata, resolver)?;
    }

    Ok(())
}
