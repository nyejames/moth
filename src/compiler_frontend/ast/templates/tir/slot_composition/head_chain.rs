//! TIR-native head-chain composition.
//!
//! Partitions root children by head/body origin, builds a chain of receiving
//! layers, and resolves each layer from a carried slot layout plus one
//! routing of its fill nodes.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{TemplateSegmentOrigin, TemplateType};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::slot_layout::{
    TirSlotLayout, collect_tir_slot_layout,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};

#[cfg(test)]
use crate::compiler_frontend::ast::templates::tir::TemplateIrId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::helpers::{
    children_of_node, compose_wrapper_application, internal_compiler_error, rebuild_root_sequence,
};

/// Typed result for the TIR head-chain composition family.
type HeadChainResult<T> = Result<T, TemplateError>;

/// Carries the immutable services shared by recursive chain resolution.
struct HeadChainResolutionInputs<'a> {
    string_table: &'a StringTable,
    allow_runtime_plans: bool,
}

/// A layer in the TIR head-chain: a wrapper template and the items that should
/// fill its slots.
///
/// WHAT: records one wrapper template opened by a head-origin receiver and the
///       pending items (direct nodes or nested layer references) accumulated as
///       its fill content.
/// WHY: nested head wrappers need one layer-local fill list before the chain is
///      resolved into effective TIR nodes.
struct TirChainLayer {
    wrapper_reference: TemplateWrapperReference,
    layout: TirSlotLayout,
    fill_items: Vec<TirChainItem>,
}

/// Items in the pending TIR head-chain.
///
/// WHAT: each item is either a direct TIR node that passes through unchanged,
///       or a reference to a chain layer that must be resolved into a new
///       `ChildTemplate` node.
/// WHY: this keeps pending wrapper layers separate from direct TIR nodes while
///      operating on TIR node IDs.
enum TirChainItem {
    /// A direct node ID (text, dynamic expression, non-receiver child template).
    DirectNode(TemplateIrNodeId),

    /// A reference to a chain layer that needs resolution.
    LayerRef {
        /// Index of the layer in the chain's layer vector.
        layer_index: usize,

        /// The original `ChildTemplate` node ID, used to preserve the source
        /// location when building the resolved `ChildTemplate` node.
        original_node_id: TemplateIrNodeId,
    },
}

/// Composes a template's head-chain from a template ID.
///
/// Test convenience wrapper: production callers use `compose_tir_head_chain_from_root`
/// directly to avoid pushing scratch templates for node-root composition.
#[cfg(test)]
pub(crate) fn compose_tir_head_chain(
    store: &mut TemplateIrStore,
    template_id: TemplateIrId,
    string_table: &StringTable,
    allow_runtime_plans: bool,
) -> HeadChainResult<TemplateIrNodeId> {
    let template = store.get_template(template_id).ok_or_else(|| {
        internal_compiler_error(
            "TIR head-chain composition: template ID was not present in the store.",
        )
    })?;
    let root_node_id = template.root;
    compose_tir_head_chain_from_root(store, root_node_id, string_table, allow_runtime_plans)
}

/// Composes a head-chain directly from a root node, without requiring a
/// durable `TemplateIr` entry.
///
/// WHAT: partitions the root node's children by head/body origin, builds the
///       wrapper chain, and resolves each layer. Returns the composed root
///       node ID.
/// WHY: control-flow body roots and aggregate-wrapper candidates are node
///      roots, not published templates. Composing them directly avoids pushing
///      a scratch `TemplateIr` that would remain in the durable store without
///      a referencing identity.
pub(crate) fn compose_tir_head_chain_from_root(
    store: &mut TemplateIrStore,
    root_node_id: TemplateIrNodeId,
    string_table: &StringTable,
    allow_runtime_plans: bool,
) -> HeadChainResult<TemplateIrNodeId> {
    let inputs = HeadChainResolutionInputs {
        string_table,
        allow_runtime_plans,
    };
    compose_tir_head_chain_from_root_into(store, root_node_id, &inputs)
}

fn compose_tir_head_chain_from_root_into(
    store: &mut TemplateIrStore,
    root_node_id: TemplateIrNodeId,
    inputs: &HeadChainResolutionInputs,
) -> HeadChainResult<TemplateIrNodeId> {
    // Fast path: if the root is not a sequence, no receiving layer can exist
    // and the original root is unchanged.
    let Some(root_children) = root_sequence_children(store, root_node_id)? else {
        return Ok(root_node_id);
    };

    let (head_children, body_children) = partition_tir_children_by_origin(store, root_children)?;

    let (root_items, layers) = build_tir_chain_graph(store, &head_children, &body_children)?;

    let resolved_root_children = resolve_tir_chain_items(store, &root_items, &layers, inputs)?;
    let original_root_children = children_of_node(store, root_node_id)?;

    // If no layer produced a new node (for example, every receiver had no fill
    // and stayed unresolved), return the original root to avoid an identical
    // sequence allocation.
    if resolved_root_children == original_root_children {
        return Ok(root_node_id);
    }

    rebuild_root_sequence(store, root_node_id, resolved_root_children)
}

/// Returns the children of a `Sequence` root node, or `None` if the root is not
/// a sequence.
pub(super) fn root_sequence_children(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> HeadChainResult<Option<&[TemplateIrNodeId]>> {
    let Some(node) = store.get_node(node_id) else {
        return Err(internal_compiler_error(
            "TIR head-chain composition: root node ID was not present in the store.",
        ));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => Ok(Some(children)),
        _ => Ok(None),
    }
}

/// Partitions root sequence children into head-origin and body-origin groups.
///
/// WHAT: walks children in source order. `Text` and `DynamicExpression` nodes
///       are classified by their `origin` field. `ChildTemplate` and other
///       structural nodes are head-origin until the first body-origin
///       `Text`/`DynamicExpression` is seen, after which they become body-origin.
/// WHY: the parser records head nodes before body nodes, so the first
///      body-origin `Text`/`DynamicExpression` marks the end of the head section.
fn partition_tir_children_by_origin(
    store: &TemplateIrStore,
    children: &[TemplateIrNodeId],
) -> HeadChainResult<(Vec<TemplateIrNodeId>, Vec<TemplateIrNodeId>)> {
    let mut head_children = Vec::new();
    let mut body_children = Vec::new();
    let mut saw_body_origin = false;

    for child_id in children {
        let child_node = store.get_node(*child_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR head-chain composition: child node ID was not present in the store while partitioning.",
            )
        })?;

        let is_body = match &child_node.kind {
            TemplateIrNodeKind::Text { origin, .. }
            | TemplateIrNodeKind::DynamicExpression { origin, .. } => {
                *origin == TemplateSegmentOrigin::Body
            }

            // Aggregate-output markers are compiler-internal fill content for
            // loop aggregate wrappers. They begin the body partition even
            // though they have no text/dynamic origin field.
            TemplateIrNodeKind::AggregateOutput => true,

            // Structural nodes follow the boundary set by Text/DynamicExpression
            // origin. Once a body-origin node has appeared, later structural
            // nodes are treated as body content.
            _ => saw_body_origin,
        };

        if is_body {
            saw_body_origin = true;
            body_children.push(*child_id);
        } else {
            head_children.push(*child_id);
        }
    }

    Ok((head_children, body_children))
}

/// Checks whether a head-origin child node is a wrapper template receiver.
///
/// WHAT: a `ChildTemplate` node is a receiver when its referenced template has
///       slots and its kind is not a slot helper (`SlotInsert` or
///       `SlotDefinition`).
/// WHY: only wrapper templates with unresolved slots open new receiving layers;
///      slot helpers and slot-less templates pass through unchanged.
fn is_tir_receiver(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> HeadChainResult<Option<(TemplateWrapperReference, TirSlotLayout)>> {
    let Some(node) = store.get_node(node_id) else {
        return Err(internal_compiler_error(
            "TIR head-chain composition: child node ID was not present in the store while checking receiver.",
        ));
    };

    let TemplateIrNodeKind::ChildTemplate { reference, .. } = &node.kind else {
        return Ok(None);
    };

    let Some(template_ir) = store.get_template(reference.root) else {
        return Err(internal_compiler_error(
            "TIR head-chain composition: child template ID was not present in the store.",
        ));
    };

    if matches!(
        template_ir.kind,
        TemplateType::SlotInsert(_) | TemplateType::SlotDefinition(_)
    ) {
        return Ok(None);
    }

    let layout = collect_tir_slot_layout(store, reference.root)?;

    Ok(if layout.schema.has_any_slots() {
        Some((
            TemplateWrapperReference::new(reference.root, reference.phase, reference.context),
            layout,
        ))
    } else {
        None
    })
}

/// Builds the chain graph from partitioned head and body children.
///
/// WHAT: walks head children in order. Receivers open a new layer and become a
///       `LayerRef` item routed to the active layer (or root). Non-receivers
///       become `DirectNode` items. Body children become fill for the deepest
///       active layer, or root items when no layer is active.
fn build_tir_chain_graph(
    store: &TemplateIrStore,
    head_children: &[TemplateIrNodeId],
    body_children: &[TemplateIrNodeId],
) -> HeadChainResult<(Vec<TirChainItem>, Vec<TirChainLayer>)> {
    let mut root_items = Vec::new();
    let mut layers = Vec::new();
    let mut active_layer: Option<usize> = None;

    for child_id in head_children {
        if let Some((wrapper_reference, layout)) = is_tir_receiver(store, *child_id)? {
            let layer_index = layers.len();

            push_tir_chain_item(
                &mut root_items,
                &mut layers,
                active_layer,
                TirChainItem::LayerRef {
                    layer_index,
                    original_node_id: *child_id,
                },
            );

            layers.push(TirChainLayer {
                wrapper_reference,
                layout,
                fill_items: Vec::new(),
            });
            active_layer = Some(layer_index);
            continue;
        }

        push_tir_chain_item(
            &mut root_items,
            &mut layers,
            active_layer,
            TirChainItem::DirectNode(*child_id),
        );
    }

    // Body nodes are appended after head parsing. If the head opened a receiving
    // chain, body nodes become contributions to the deepest active receiver.
    for child_id in body_children {
        push_tir_chain_item(
            &mut root_items,
            &mut layers,
            active_layer,
            TirChainItem::DirectNode(*child_id),
        );
    }

    Ok((root_items, layers))
}

/// Routes a chain item to either the root list or the active receiving layer.
fn push_tir_chain_item(
    root_items: &mut Vec<TirChainItem>,
    layers: &mut [TirChainLayer],
    active_layer: Option<usize>,
    item: TirChainItem,
) {
    match active_layer {
        Some(layer_index) => layers[layer_index].fill_items.push(item),
        None => root_items.push(item),
    }
}

/// Recursively resolves pending chain items into concrete TIR node IDs.
///
/// WHAT: direct nodes pass through; layer references trigger bottom-up
///       resolution of the wrapper's slots with the accumulated fill items.
/// WHY: bottom-up layer resolution keeps TIR-native routing and expansion
///      explicit at the point where each wrapper's fill is available.
fn resolve_tir_chain_items(
    store: &mut TemplateIrStore,
    items: &[TirChainItem],
    layers: &[TirChainLayer],
    inputs: &HeadChainResolutionInputs,
) -> HeadChainResult<Vec<TemplateIrNodeId>> {
    let mut resolved_nodes = Vec::with_capacity(items.len());

    for item in items {
        match item {
            TirChainItem::DirectNode(node_id) => {
                resolved_nodes.push(*node_id);
            }

            TirChainItem::LayerRef {
                layer_index,
                original_node_id,
            } => {
                let resolved_node = resolve_tir_chain_layer(
                    store,
                    *layer_index,
                    layers,
                    *original_node_id,
                    inputs,
                )?;
                resolved_nodes.push(resolved_node);
            }
        }
    }

    Ok(resolved_nodes)
}

/// Resolves a single chain layer by filling its wrapper's slots with the
/// accumulated fill items.
///
/// WHAT: if the layer has no fill, the wrapper stays as an unresolved
///       `ChildTemplate` reference so later use-sites can still fill its slots.
///       Otherwise, the fill items are resolved recursively, routed against the
///       wrapper's slot schema, and the wrapper's placeholders are expanded.
/// WHY: head-only wrapper references like `[format.table]` must remain usable
///      wrappers; only layers with actual fill content produce a composed
///      template entry.
fn resolve_tir_chain_layer(
    store: &mut TemplateIrStore,
    layer_index: usize,
    layers: &[TirChainLayer],
    original_node_id: TemplateIrNodeId,
    inputs: &HeadChainResolutionInputs,
) -> HeadChainResult<TemplateIrNodeId> {
    let layer = &layers[layer_index];

    if layer.fill_items.is_empty() {
        // Head-only wrapper references stay unresolved so they can be filled
        // later at a use-site, preserving the wrapper's unresolved-slot state.
        return Ok(original_node_id);
    }

    let resolved_fill_node_ids = resolve_tir_chain_items(store, &layer.fill_items, layers, inputs)?;

    let original_location = store
        .get_node(original_node_id)
        .map(|node| node.location.to_owned())
        .ok_or_else(|| {
            internal_compiler_error(
                "TIR head-chain composition: original wrapper node ID was not present in the store.",
            )
        })?;

    let resolved = compose_wrapper_application(
        store,
        layer.wrapper_reference,
        &layer.layout,
        resolved_fill_node_ids,
        original_location.clone(),
        inputs.string_table,
        inputs.allow_runtime_plans,
    )?;

    let occurrence_id = store.next_child_template_occurrence_id();
    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: resolved,
            occurrence_id,
        },
        original_location,
    )))
}
