//! Reactive metadata reduction for neutral runtime handoffs.

use crate::compiler_frontend::ast::expressions::expression::ReactiveTemplateMetadata;
use crate::compiler_frontend::ast::templates::runtime_handoff;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

use super::{ReactiveMetadataResolver, merge_expression_metadata};

pub(super) fn merge_owned_runtime_template_handoff_metadata(
    handoff: &OwnedRuntimeTemplateHandoff,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_template_handoff(handoff, &mut |node| {
        merge_owned_runtime_template_node_metadata(node, metadata, resolver)
    })
}

pub(super) fn merge_owned_runtime_slot_application_handoff_metadata(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_slot_application_handoff(handoff, &mut |node| {
        merge_owned_runtime_template_node_metadata(node, metadata, resolver)
    })
}

/// Computes reactive template metadata for an owned runtime-template handoff.
pub(crate) fn metadata_for_owned_runtime_template_handoff(
    handoff: &OwnedRuntimeTemplateHandoff,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<ReactiveTemplateMetadata, CompilerError> {
    let mut metadata = ReactiveTemplateMetadata::template_backed();
    merge_owned_runtime_template_handoff_metadata(handoff, &mut metadata, resolver)?;
    Ok(metadata)
}

/// Computes reactive template metadata for an owned runtime slot application handoff.
pub(crate) fn metadata_for_owned_runtime_slot_application_handoff(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<ReactiveTemplateMetadata, CompilerError> {
    let mut metadata = ReactiveTemplateMetadata::template_backed();
    merge_owned_runtime_slot_application_handoff_metadata(handoff, &mut metadata, resolver)?;
    Ok(metadata)
}

fn merge_owned_runtime_template_node_metadata(
    node: &OwnedRuntimeTemplateNode,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression {
            expression,
            reactive_subscription,
            ..
        } => {
            if let Some(subscription) = reactive_subscription {
                metadata.push_subscription(subscription.clone());
            }
            merge_expression_metadata(expression, metadata, resolver)?;
        }

        OwnedRuntimeTemplateNode::BranchChain { branches, .. } => {
            for branch in branches {
                merge_branch_selector_metadata(&branch.selector, metadata, resolver)?;
            }
        }

        OwnedRuntimeTemplateNode::Loop { header, .. } => {
            merge_loop_header_metadata(header, metadata, resolver)?;
        }

        OwnedRuntimeTemplateNode::Text {
            text: _,
            reactive_subscription,
            ..
        } => {
            // Owned structural pieces are opaque to metadata; only subscriptions affect reactivity.
            if let Some(subscription) = reactive_subscription {
                metadata.push_subscription(subscription.clone());
            }
        }

        OwnedRuntimeTemplateNode::Sequence { .. }
        | OwnedRuntimeTemplateNode::ChildTemplate { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }

    Ok(())
}

fn merge_branch_selector_metadata(
    selector: &TemplateBranchSelector,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            merge_expression_metadata(condition, metadata, resolver)?;
        }

        TemplateBranchSelector::OptionPresentCapture { scrutinee, .. } => {
            merge_expression_metadata(scrutinee, metadata, resolver)?;
        }
    }

    Ok(())
}

fn merge_loop_header_metadata(
    header: &TemplateLoopHeader,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            merge_expression_metadata(condition, metadata, resolver)?;
        }

        TemplateLoopHeader::Range { range, .. } => {
            merge_expression_metadata(&range.start, metadata, resolver)?;
            merge_expression_metadata(&range.end, metadata, resolver)?;
            if let Some(step) = &range.step {
                merge_expression_metadata(step, metadata, resolver)?;
            }
        }

        TemplateLoopHeader::Collection { iterable, .. } => {
            merge_expression_metadata(iterable, metadata, resolver)?;
        }
    }

    Ok(())
}
