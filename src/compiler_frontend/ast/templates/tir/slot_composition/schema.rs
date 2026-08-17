//! Structural expansion of TIR slot placeholders.
//!
//! Slot schema and occurrence facts come from `tir/slot_layout.rs`. This
//! module rebuilds a wrapper tree with routed contributions spliced in place
//! of `$slot` nodes.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::contribution_shape::{
    ContributionShape, classify_tir_contribution_node,
};
use crate::compiler_frontend::ast::templates::tir::node::TirSlotPlaceholder;
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_root;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::{
    DerivedCount, DerivedTemplateMetadata, TemplateIr, TemplateIrBranch, TemplateIrId,
    TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplateWrapperSetId,
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
/// WHY: wrapper templates may declare slots inside sequences, branches, loops,
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
                node.location.to_owned(),
            )))
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            let contribution_nodes = routed_contributions.nodes_for_slot(&placeholder.key);

            // Apply the `$children(..)` wrapper sets carried on the placeholder,
            // Only child-template contributions receive external wrappers; text and
            // dynamic expressions pass through unchanged. Control-flow
            // contributions (branches and loops) must not be externally wrapped
            // because a skipped branch or empty loop would still render the
            // wrapper. Instead, the wrapper set is attached as a conditional
            // child-wrapper set so folding can skip it when the control flow
            // emits no output.
            let mut wrapped_nodes = Vec::with_capacity(contribution_nodes.len());
            for node_id in contribution_nodes {
                let current_node_id = if tir_node_is_control_flow_root(store, *node_id)? {
                    let shape = classify_tir_contribution_node(store, *node_id)?;
                    if let Some(wrapper_set_id) =
                        conditional_wrapper_set_for_control_flow(store, placeholder, &shape)?
                    {
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
                node.location.to_owned(),
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
                node.location.to_owned(),
            )))
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let mut expanded_branches = Vec::with_capacity(branches.len());
            let mut any_branch_changed = false;

            for branch in branches {
                let expanded_body_id = expand_tir_slot_placeholders_from_node(
                    store,
                    branch.body,
                    routed_contributions,
                    string_table,
                )?;

                if expanded_body_id != branch.body {
                    any_branch_changed = true;
                    expanded_branches.push(TemplateIrBranch::new(
                        branch.selector.to_owned(),
                        expanded_body_id,
                        branch.location.to_owned(),
                        branch.selector_site_id,
                    ));
                } else {
                    expanded_branches.push(branch.to_owned());
                }
            }

            let expanded_fallback = match fallback {
                Some(fallback_id) => {
                    let expanded_fallback_id = expand_tir_slot_placeholders_from_node(
                        store,
                        *fallback_id,
                        routed_contributions,
                        string_table,
                    )?;

                    if expanded_fallback_id != *fallback_id {
                        any_branch_changed = true;
                    }

                    Some(expanded_fallback_id)
                }

                None => None,
            };

            if !any_branch_changed {
                return Ok(node_id);
            }

            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::BranchChain {
                    branches: expanded_branches,
                    fallback: expanded_fallback,
                },
                node.location.to_owned(),
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
                node.location.to_owned(),
            )))
        }

        // Text, dynamic expressions, and insert contributions cannot contain
        // slot placeholders, so they pass through unchanged.
        TemplateIrNodeKind::Text { .. } => Ok(node_id),
        TemplateIrNodeKind::DynamicExpression { .. } => Ok(node_id),
        TemplateIrNodeKind::InsertContribution { .. } => Ok(node_id),

        // Aggregate-output markers, loop-control signals, and runtime slot
        // sites are leaves that do not carry slot placeholders.
        TemplateIrNodeKind::AggregateOutput => Ok(node_id),
        TemplateIrNodeKind::LoopControl { .. } => Ok(node_id),
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

/// Returns true when a TIR node is a control-flow root (a branch chain or loop,
/// or a child-template reference to a template whose root is control flow).
///
/// WHAT: answers whether this contribution's output depends on a branch or
///       loop being selected/active.
/// WHY: control-flow contributions must receive parent `$children(..)` wrappers
///      conditionally so skipped branches and zero-iteration loops do not
///      render empty wrappers.
fn tir_node_is_control_flow_root(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> SlotSchemaResult<bool> {
    let node = store.get_node(node_id).ok_or_else(|| {
        internal_compiler_error(
            "TIR slot expansion: contribution node ID was not present in the store while checking control flow.",
        )
    })?;

    let is_control_flow_root = match &node.kind {
        TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. } => true,
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template_id = reference.root;
            let template = store.get_template(template_id).ok_or_else(|| {
                internal_compiler_error(
                    "TIR slot expansion: module-local child template ID was not present in the TIR store while checking control flow.",
                )
            })?;

            store
                .control_flow_node_id_in_subtree(template.root)?
                .is_some()
        }
        _ => false,
    };

    Ok(is_control_flow_root)
}

/// Builds a single wrapper set containing the wrappers that should be applied
/// conditionally around a control-flow contribution.
///
/// WHAT: combines the placeholder's inherited child wrappers and applied
///       `$children(..)` wrappers, dropping each set when the corresponding
///       skip flag is set.
/// WHY: control-flow contributions receive all applicable wrappers as a
///      conditional set, so they are applied only when the control flow emits
///      output.
fn conditional_wrapper_set_for_control_flow(
    store: &mut TemplateIrStore,
    placeholder: &TirSlotPlaceholder,
    shape: &ContributionShape,
) -> SlotSchemaResult<Option<TemplateWrapperSetId>> {
    let mut combined = Vec::new();

    if let Some(wrapper_set_id) = placeholder.child_wrapper_set {
        let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR slot expansion: conditional child wrapper set ID was not present in the store.",
            )
        })?;

        if !shape.skips_parent_child_wrappers() {
            combined.extend(wrapper_set.wrappers.iter().copied());
        }
    }

    if let Some(wrapper_set_id) = placeholder.applied_child_wrapper_set {
        let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR slot expansion: conditional applied wrapper set ID was not present in the store.",
            )
        })?;

        if !placeholder.skip_parent_child_wrappers {
            combined.extend(wrapper_set.wrappers.iter().copied());
        }
    }

    if combined.is_empty() {
        Ok(None)
    } else {
        Ok(Some(store.push_or_reuse_wrapper_set(combined)))
    }
}

/// Attaches a conditional `$children(..)` wrapper set to a control-flow node.
///
/// WHAT: for a `ChildTemplate` reference to a control-flow template, copies the
///       template, merges the wrapper set into its existing
///       `conditional_child_wrapper_set`, and returns a new `ChildTemplate`
///       reference to the copy. For a direct `BranchChain` or `Loop` node,
///       creates a new `TemplateIr` whose root is that node, sets the wrapper
///       set, and returns a `ChildTemplate` reference to the new template.
/// WHY: conditional wrappers must be stored on the control-flow template so
///      folding can skip them when the branch/loop emits no output.
fn attach_conditional_wrapper_set(
    store: &mut TemplateIrStore,
    node_id: TemplateIrNodeId,
    wrapper_set_id: TemplateWrapperSetId,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let node = store.get_node(node_id).cloned().ok_or_else(|| {
        internal_compiler_error(
            "TIR slot expansion: control-flow node ID was not present in the store.",
        )
    })?;

    let (reference, location) = match &node.kind {
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
            (new_reference, node.location.to_owned())
        }

        TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. } => {
            let wrapper_count = required_wrapper_set_count(store, wrapper_set_id)?;
            let mut summary = summarize_existing_root(store, node_id)?;
            summary.wrapper_count = wrapper_count;
            let mut template = TemplateIr::new(
                node_id,
                Style::default(),
                TemplateType::String,
                summary,
                node.location.to_owned(),
            );
            template.conditional_child_wrapper_set = Some(wrapper_set_id);
            let template_id = store.push_template(template);

            let new_reference = TemplateTirChildReference::new(
                template_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
            (new_reference, node.location.to_owned())
        }

        _ => return Ok(node_id),
    };

    let occurrence_id = store.next_child_template_occurrence_id();
    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        location,
    )))
}

/// Merges an existing wrapper set with a new wrapper set.
///
/// WHAT: appends the new wrappers after the existing wrappers, preserving the
///       innermost-to-outermost storage order both sets already use.
/// WHY: a control-flow template may already carry conditional wrappers from an
///      enclosing context; this merges them without changing the established
///      nesting order.
fn merge_wrapper_sets(
    store: &mut TemplateIrStore,
    existing: Option<TemplateWrapperSetId>,
    additional: TemplateWrapperSetId,
) -> SlotSchemaResult<TemplateWrapperSetId> {
    let mut combined = Vec::new();

    if let Some(existing_id) = existing {
        let existing_set = store.get_wrapper_set(existing_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR slot expansion: existing conditional wrapper set ID was not present in the store.",
            )
        })?;
        combined.extend(existing_set.wrappers.iter().copied());
    }

    let additional_set = store.get_wrapper_set(additional).ok_or_else(|| {
        internal_compiler_error(
            "TIR slot expansion: additional conditional wrapper set ID was not present in the store.",
        )
    })?;
    combined.extend(additional_set.wrappers.iter().copied());

    Ok(store.push_or_reuse_wrapper_set(combined))
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
