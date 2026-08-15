//! TIR-native active-context subtree copying.
//!
//! WHAT: deep-copies finalized TIR subtrees into fresh trees while applying an
//! optional active slot-plan context to unresolved `Slot` placeholders.
//!
//! WHY: control-flow bodies and runtime slot wrapper roots must be copied
//! without mutating the stored originals, while still honoring the active
//! slot-plan cursor semantics required by nested runtime slot wrappers.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::ids::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::slot_plan::convert_runtime_slot_site;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::compiler_errors::CompilerError;

use crate::compiler_frontend::ast::templates::tir::copy_state::TirCopyState;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};

/// Copies a finalized TIR subtree into a fresh tree, applying an optional active
/// slot-plan context to any unresolved `Slot` placeholders.
///
/// WHAT: walks the source subtree starting at `source_node_id` and pushes a
///       freshly allocated mirror into the same store. `Slot` nodes are converted
///       to `RuntimeSlotSite` nodes when `active_slot_plan` matches the cursor,
///       exactly as atom-based materialization would do. `ChildTemplate` and
///       `InsertContribution` references are deep-copied as fresh template
///       entries; runtime-slot-handoff children keep their own plan and are not
///       reprocessed under the parent's active plan.
/// WHY: this lets `materialize_loop` reuse a module-local finalized loop body root
///      without mutating the stored root in place, while still honoring the
///      active slot-plan cursor semantics required by nested runtime slot
///      wrappers. Runtime slot handoff planning also uses it to copy a
///      module-local finalized wrapper root when that root is runtime-kind-safe.
pub(crate) fn copy_tir_subtree_with_active_slot_plan(
    source_node_id: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<TemplateIrNodeId, TemplateError> {
    increment_ast_counter(AstCounter::TirCopyPasses);
    copy_tir_node_with_active_slot_plan(source_node_id, active_slot_plan, store, copy_state, false)
}

/// Recursively copies one TIR node, translating child-template references into
/// fresh template entries and applying the active slot-plan cursor to `Slot`
/// placeholders.
fn copy_tir_node_with_active_slot_plan(
    source_node_id: TemplateIrNodeId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
    preserve_expression_site_ids: bool,
) -> Result<TemplateIrNodeId, TemplateError> {
    let source_node = store.get_node(source_node_id).cloned().ok_or_else(|| {
        TemplateError::from(CompilerError::compiler_error(
            "active-context TIR copy: source node ID was not present in the store.",
        ))
    })?;

    let location = source_node.location.clone();

    match source_node.kind {
        TemplateIrNodeKind::Text {
            text,
            byte_len,
            origin,
        } => {
            copy_state.record_text_node(byte_len as usize);

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Text {
                    text,
                    byte_len,
                    origin,
                },
                location,
            ));
            if let Some(Some(subscription)) = store
                .node_reactive_subscriptions
                .get(source_node_id.index())
                .cloned()
            {
                store.set_node_reactive_subscription(node_id, subscription);
            }
            Ok(node_id)
        }

        TemplateIrNodeKind::DynamicExpression {
            expression,
            origin,
            reactive_subscription,
            site_id,
        } => {
            copy_state.record_dynamic_expression(reactive_subscription.is_some());

            let copied_site_id = if preserve_expression_site_ids {
                site_id
            } else {
                store.next_expression_site_id()
            };
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::DynamicExpression {
                    expression,
                    origin,
                    reactive_subscription,
                    site_id: copied_site_id,
                },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::Slot { mut placeholder } => {
            if let Some(plan_id) = active_slot_plan {
                let site_id = copy_state
                    .next_runtime_slot_site_for_key(plan_id, &placeholder.key, store)
                    .ok_or_else(|| {
                        TemplateError::from(CompilerError::compiler_error(
                            "active-context TIR copy: no matching runtime slot site for a slot placeholder.",
                        ))
                    })?;

                return Ok(convert_runtime_slot_site(
                    plan_id, site_id, store, copy_state, &location,
                ));
            }

            copy_state.record_slot();
            placeholder.occurrence_id = store.next_slot_occurrence_id();
            placeholder.location = location.clone();
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Slot { placeholder },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::RuntimeSlotSite { plan, site } => {
            copy_state.record_existing_runtime_slot_site();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::RuntimeSlotSite { plan, site },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let new_child_id = copy_tir_template_with_active_slot_plan(
                reference.root,
                active_slot_plan,
                store,
                copy_state,
            )?;
            copy_state.record_child_template();

            let occurrence_id = store.next_child_template_occurrence_id();
            let reference = TemplateTirChildReference::new(
                new_child_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id,
                },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            let new_child_id = copy_tir_template_with_active_slot_plan(
                template,
                active_slot_plan,
                store,
                copy_state,
            )?;
            copy_state.record_insert_contribution();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::InsertContribution {
                    template: new_child_id,
                },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::Sequence { children } => {
            copy_state.enter_depth();
            let new_children = children
                .into_iter()
                .map(|child_id| {
                    copy_tir_node_with_active_slot_plan(
                        child_id,
                        active_slot_plan,
                        store,
                        copy_state,
                        preserve_expression_site_ids,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            copy_state.exit_depth();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: new_children,
                },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            copy_state.record_control_flow();
            copy_state.enter_depth();
            let new_branches = branches
                .into_iter()
                .map(|branch| -> Result<TemplateIrBranch, TemplateError> {
                    let new_body = copy_tir_node_with_active_slot_plan(
                        branch.body,
                        active_slot_plan,
                        store,
                        copy_state,
                        preserve_expression_site_ids,
                    )?;

                    Ok(
                        TemplateIrBranch::new(branch.selector, new_body, branch.location)
                            .with_selector_site_id(branch.selector_site_id),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let new_fallback = fallback
                .map(|fallback_id| {
                    copy_tir_node_with_active_slot_plan(
                        fallback_id,
                        active_slot_plan,
                        store,
                        copy_state,
                        preserve_expression_site_ids,
                    )
                })
                .transpose()?;
            copy_state.exit_depth();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::BranchChain {
                    branches: new_branches,
                    fallback: new_fallback,
                },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
        } => {
            copy_state.record_control_flow();
            copy_state.enter_depth();
            let new_body = copy_tir_node_with_active_slot_plan(
                body,
                active_slot_plan,
                store,
                copy_state,
                preserve_expression_site_ids,
            )?;
            let new_aggregate_wrapper = aggregate_wrapper
                .map(|wrapper_id| {
                    copy_tir_node_with_active_slot_plan(
                        wrapper_id,
                        active_slot_plan,
                        store,
                        copy_state,
                        preserve_expression_site_ids,
                    )
                })
                .transpose()?;
            copy_state.exit_depth();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Loop {
                    header,
                    header_sites,
                    body: new_body,
                    aggregate_wrapper: new_aggregate_wrapper,
                },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::LoopControl { kind } => {
            copy_state.record_control_flow();

            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::LoopControl { kind },
                location,
            ));
            Ok(node_id)
        }

        TemplateIrNodeKind::AggregateOutput => {
            let node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::AggregateOutput,
                location,
            ));
            Ok(node_id)
        }
    }
}

/// Deep-copies one template entry and its root subtree, applying the active
/// slot-plan context only when the source template is not itself a runtime slot
/// handoff.
///
/// WHAT: returns a fresh `TemplateIrId` whose root is a copy of the source
///       template's root. If the source carries its own `runtime_slot_plan`, the
///       active plan is suppressed for the copy so nested runtime slot
///       applications remain independent.
/// WHY: `ChildTemplate` and `InsertContribution` nodes inside a copied body may
///      reference helper templates that already own a runtime slot plan. Those
///      must not have the parent's active cursor applied to them.
fn copy_tir_template_with_active_slot_plan(
    source_template_id: TemplateIrId,
    active_slot_plan: Option<TemplateSlotPlanId>,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<TemplateIrId, TemplateError> {
    let source_template = store
        .get_template(source_template_id)
        .cloned()
        .ok_or_else(|| {
            TemplateError::from(CompilerError::compiler_error(
                "active-context TIR copy: source template ID was not present in the store.",
            ))
        })?;

    // Runtime slot handoff templates already resolved their own placeholders.
    // Copy them under a suppressed active plan so the outer cursor does not leak
    // into the nested application.
    let effective_active_slot_plan = if source_template.runtime_slot_plan.is_some() {
        None
    } else {
        active_slot_plan
    };

    let mut child_state = TirCopyState::new();
    child_state.runtime_slot_site_cursor = copy_state.runtime_slot_site_cursor.clone();

    let new_root = copy_tir_node_with_active_slot_plan(
        source_template.root,
        effective_active_slot_plan,
        store,
        &mut child_state,
        false,
    )?;

    // Propagate the cursor state for the active plan back to the parent copy.
    // The child template's own text/child/slot counts stay in its own summary.
    copy_state.runtime_slot_site_cursor = child_state.runtime_slot_site_cursor;

    let mut new_template = TemplateIr::new(
        new_root,
        source_template.style,
        source_template.kind,
        child_state.summary,
        source_template.location,
    );
    new_template.conditional_child_wrapper_set = source_template.conditional_child_wrapper_set;
    new_template.runtime_slot_plan = source_template.runtime_slot_plan;

    Ok(store.push_template(new_template))
}
