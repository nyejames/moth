//! Structural expansion of TIR slot placeholders.
//!
//! Slot schema and occurrence facts come from `tir/slot_layout.rs`. This
//! module rebuilds a wrapper tree with routed contributions spliced in place
//! of `$slot` nodes.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::contribution_shape::classify_tir_contribution_node;
use crate::compiler_frontend::ast::templates::tir::node::TirSlotPlaceholder;
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_root;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::wrapper_sets::merge_wrapper_sets;
use crate::compiler_frontend::ast::templates::tir::{
    DerivedCount, DerivedTemplateMetadata, TemplateIr, TemplateIrId, TemplateIrNode,
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplateWrapperSetId,
    conditional_wrapper_set_for_control_flow, tir_node_is_control_flow_root,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::child_wrappers::wrap_tir_node_in_wrappers_into;
use super::contributions::TirSlotContributions;
use super::helpers::internal_compiler_error;

type SlotSchemaResult<T> = Result<T, TemplateError>;

pub(crate) fn expand_tir_slot_placeholders_into(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    routed_contributions: &TirSlotContributions,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let Some(template) = store.get_template(wrapper_template_id) else {
        return Err(internal_compiler_error(
            "TIR slot expansion: wrapper template ID was not present in the store.",
        ));
    };
    let root = template.root;

    expand_tir_slot_placeholders_from_node(store, root, routed_contributions, string_table)
}

/// Recursively walks TIR nodes and produces a new TIR tree with slots expanded.
///
/// WHAT: dispatches on `TemplateIrNodeKind`, replacing `Slot` nodes with a
///       `Sequence` containing the routed contribution node IDs, and recursing
///       into structures that can contain further slot placeholders.
/// WHY: wrapper templates may declare slots inside sequences, conditionals, loops,
///      or nested child templates, so a single root walk must reach every
///      reachable slot and rebuild only the parts of the tree that changed.
fn expand_tir_slot_placeholders_from_node(
    store: &mut TemplateIrStore,
    node_id: TemplateIrNodeId,
    routed_contributions: &TirSlotContributions,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let Some(node) = store.get_node(node_id).cloned() else {
        return Err(internal_compiler_error(
            "TIR slot expansion: node ID was not present in the store.",
        ));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            let mut expanded_children = Vec::with_capacity(children.len());
            let mut any_child_changed = false;

            for child_id in children {
                let expanded_child_id = expand_tir_slot_placeholders_from_node(
                    store,
                    *child_id,
                    routed_contributions,
                    string_table,
                )?;

                if expanded_child_id != *child_id {
                    any_child_changed = true;

                    // Slot placeholders expand into a Sequence containing their
                    // contributions. Splice that Sequence into the parent so the
                    // resulting tree keeps the composed sequence flat instead
                    // of leaving nested sequences around every slot.
                    if let Some(expanded_node) = store.get_node(expanded_child_id)
                        && let TemplateIrNodeKind::Sequence {
                            children: contribution_children,
                        } = &expanded_node.kind
                    {
                        expanded_children.extend(contribution_children.iter().copied());
                        continue;
                    }
                }

                expanded_children.push(expanded_child_id);
            }

            if !any_child_changed {
                return Ok(node_id);
            }

            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: expanded_children,
                },
                node.span,
            )))
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            let contribution_nodes = routed_contributions.nodes_for_slot(&placeholder.key);

            // Only child-template contributions receive ordinary external
            // wrappers. Control-flow contributions carry the same applicable
            // wrapper set as conditional metadata so skipped work emits no
            // wrapper.
            let mut wrapped_nodes = Vec::with_capacity(contribution_nodes.len());
            for node_id in contribution_nodes {
                let current_node_id = if tir_node_is_control_flow_root(store, *node_id)? {
                    let shape = classify_tir_contribution_node(store, *node_id)?;
                    if let Some(wrapper_set_id) = conditional_wrapper_set_for_control_flow(
                        store,
                        placeholder.child_wrapper_set,
                        placeholder.applied_child_wrapper_set,
                        placeholder.skip_parent_child_wrappers,
                        shape,
                    )? {
                        attach_conditional_wrapper_set(store, *node_id, wrapper_set_id)?
                    } else {
                        *node_id
                    }
                } else {
                    apply_tir_wrapper_sets_to_contribution(
                        store,
                        *node_id,
                        placeholder,
                        string_table,
                    )?
                };

                wrapped_nodes.push(current_node_id);
            }

            // Repeated slot placeholders replay the same contribution nodes.
            // The expansion is non-consuming: it shares the routed node IDs
            // rather than moving them, so every occurrence of the same slot
            // sees identical content.
            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: wrapped_nodes,
                },
                node.span,
            )))
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_template_id = reference.root;
            let Some(child_root) = store
                .get_template(child_template_id)
                .map(|template| template.root)
            else {
                return Err(internal_compiler_error(
                    "TIR slot expansion: child template ID was not present in the store.",
                ));
            };

            let expanded_child_root = expand_tir_slot_placeholders_from_node(
                store,
                child_root,
                routed_contributions,
                string_table,
            )?;

            if expanded_child_root == child_root {
                return Ok(node_id);
            }

            let expanded_child_template_id = store.push_structurally_derived_template(
                child_template_id,
                expanded_child_root,
                DerivedTemplateMetadata::preserve_source(),
            )?;

            let occurrence_id = store.next_child_template_occurrence_id();
            let expanded_reference = reference.with_root(expanded_child_template_id);
            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: expanded_reference,
                    occurrence_id,
                },
                node.span,
            )))
        }

        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            let expanded_body_id = expand_tir_slot_placeholders_from_node(
                store,
                *body,
                routed_contributions,
                string_table,
            )?;

            if expanded_body_id == *body {
                return Ok(node_id);
            }

            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Conditional {
                    selector: selector.to_owned(),
                    selector_site_id: *selector_site_id,
                    body: expanded_body_id,
                },
                node.span,
            )))
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
        } => {
            let expanded_body_id = expand_tir_slot_placeholders_from_node(
                store,
                *body,
                routed_contributions,
                string_table,
            )?;

            let mut any_part_changed = expanded_body_id != *body;

            let expanded_aggregate_wrapper = match aggregate_wrapper {
                Some(aggregate_wrapper_id) => {
                    let expanded_aggregate_wrapper_id = expand_tir_slot_placeholders_from_node(
                        store,
                        *aggregate_wrapper_id,
                        routed_contributions,
                        string_table,
                    )?;

                    if expanded_aggregate_wrapper_id != *aggregate_wrapper_id {
                        any_part_changed = true;
                    }

                    Some(expanded_aggregate_wrapper_id)
                }

                None => None,
            };

            if !any_part_changed {
                return Ok(node_id);
            }

            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Loop {
                    header: header.to_owned(),
                    header_sites: *header_sites,
                    body: expanded_body_id,
                    aggregate_wrapper: expanded_aggregate_wrapper,
                },
                node.span,
            )))
        }

        // Text, dynamic expressions, and insert contributions cannot contain
        // slot placeholders, so they pass through unchanged.
        TemplateIrNodeKind::Text { .. } => Ok(node_id),
        TemplateIrNodeKind::DynamicExpression { .. } => Ok(node_id),
        TemplateIrNodeKind::InsertContribution { .. } => Ok(node_id),

        // Aggregate-output markers and runtime slot sites are leaves that do
        // not carry slot placeholders.
        TemplateIrNodeKind::AggregateOutput => Ok(node_id),
        TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(node_id),
    }
}

/// Applies a module-local `$children(..)` wrapper set to a single TIR node.
///
/// WHAT: resolves the wrapper set into module-local wrapper template IDs and
///       delegates to `wrap_tir_node_in_wrappers_into`, which composes each
///       slot-bearing wrapper around the supplied node and prepends each
///       slot-less wrapper before it.
fn apply_tir_wrapper_set_to_node(
    store: &mut TemplateIrStore,
    node_id: TemplateIrNodeId,
    wrapper_set_id: TemplateWrapperSetId,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
        internal_compiler_error("TIR slot expansion: placeholder referenced a missing wrapper set.")
    })?;

    let wrapper_references = wrapper_set.wrappers.to_vec();

    wrap_tir_node_in_wrappers_into(store, node_id, &wrapper_references, string_table)
}

/// Applies both inherited and applied `$children(..)` wrapper sets to a single
/// non-control-flow contribution node.
///
/// WHAT: classifies the contribution, applies `child_wrapper_set` when the
///       contribution is a child template and does not opt out via `$fresh`,
///       then applies `applied_child_wrapper_set` when the post-wrap shape is
///       still a child template and the placeholder does not skip parent
///       wrappers.
/// WHY: preserves the two-step wrapper application encoded by the slot
///      placeholder while operating on TIR node IDs.
fn apply_tir_wrapper_sets_to_contribution(
    store: &mut TemplateIrStore,
    node_id: TemplateIrNodeId,
    placeholder: &TirSlotPlaceholder,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let mut current_node_id = node_id;

    let shape = classify_tir_contribution_node(store, current_node_id)?;
    if let Some(wrapper_set_id) = placeholder.child_wrapper_set
        && shape.is_child_template_contribution()
        && !shape.skips_parent_child_wrappers()
    {
        current_node_id =
            apply_tir_wrapper_set_to_node(store, current_node_id, wrapper_set_id, string_table)?;
    }

    let post_shape = classify_tir_contribution_node(store, current_node_id)?;
    if let Some(wrapper_set_id) = placeholder.applied_child_wrapper_set
        && !placeholder.skip_parent_child_wrappers
        && post_shape.is_child_template_contribution()
    {
        current_node_id =
            apply_tir_wrapper_set_to_node(store, current_node_id, wrapper_set_id, string_table)?;
    }

    Ok(current_node_id)
}

/// Attaches a conditional `$children(..)` wrapper set to a control-flow node.
///
/// WHAT: for a `ChildTemplate` reference to a control-flow template, copies the
///       template, merges the wrapper set into its existing
///       `conditional_child_wrapper_set`, and returns a new `ChildTemplate`
///       reference to the copy. For a direct `Conditional`, `Loop` or runtime
///       slot contribution marker, creates a new `TemplateIr` rooted at that
///       node, sets the wrapper set, and returns a `ChildTemplate` reference.
/// WHY: storing wrappers on the contribution template lets folding or runtime
///      handoff apply them only when the contribution emits output.
pub(crate) fn attach_conditional_wrapper_set(
    store: &mut TemplateIrStore,
    node_id: TemplateIrNodeId,
    wrapper_set_id: TemplateWrapperSetId,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let node = store.get_node(node_id).cloned().ok_or_else(|| {
        internal_compiler_error(
            "TIR slot expansion: control-flow node ID was not present in the store.",
        )
    })?;

    let (reference, span) = match &node.kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template_id = reference.root;
            let Some(template) = store.get_template(template_id).cloned() else {
                return Err(internal_compiler_error(
                    "TIR slot expansion: control-flow child template was not present in the store.",
                ));
            };

            let merged_wrapper_set_id = merge_wrapper_sets(
                store,
                template.conditional_child_wrapper_set,
                wrapper_set_id,
            )?;

            let wrapper_count = required_wrapper_set_count(store, merged_wrapper_set_id)?;
            let copied_id = store.push_structurally_derived_template(
                template_id,
                template.root,
                DerivedTemplateMetadata {
                    head_node_count: DerivedCount::PreserveSource,
                    wrapper_count: DerivedCount::Replace(wrapper_count),
                },
            )?;
            store.set_conditional_child_wrapper_set(copied_id, merged_wrapper_set_id)?;

            let new_reference = reference.with_root(copied_id);
            (new_reference, node.span.to_owned())
        }

        TemplateIrNodeKind::Conditional { .. }
        | TemplateIrNodeKind::Loop { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {
            let wrapper_count = required_wrapper_set_count(store, wrapper_set_id)?;
            let mut summary = summarize_existing_root(store, node_id)?;
            summary.wrapper_count = wrapper_count;
            let mut template = TemplateIr::new(
                node_id,
                Style::default(),
                TemplateType::String,
                summary,
                node.span,
            );
            template.conditional_child_wrapper_set = Some(wrapper_set_id);
            let template_id = store.push_template(template);

            let new_reference = TemplateTirChildReference::new(
                template_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
            (new_reference, node.span.to_owned())
        }

        _ => return Ok(node_id),
    };

    let occurrence_id = store.next_child_template_occurrence_id();
    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        span,
    )))
}

/// Returns the wrapper count for a required wrapper-set authority.
fn required_wrapper_set_count(
    store: &TemplateIrStore,
    wrapper_set_id: TemplateWrapperSetId,
) -> SlotSchemaResult<u32> {
    let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
        internal_compiler_error(
            "TIR slot expansion: required wrapper set ID was not present in the store.",
        )
    })?;

    u32::try_from(wrapper_set.wrappers.len()).map_err(|_| {
        internal_compiler_error(
            "TIR slot expansion: wrapper-set count exceeded the supported summary range.",
        )
    })
}
