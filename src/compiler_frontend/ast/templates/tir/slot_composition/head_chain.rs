//! TIR-native head-chain composition.
//!
//! Partitions root children by head/body origin, builds a chain of receiving
//! layers, and resolves each layer from a carried slot layout plus one
//! routing of its fill nodes.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::slot_layout::{
    TirSlotLayout, collect_tir_slot_layout_with_context,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;

use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

use super::helpers::{
    children_of_node, compose_wrapper_application, internal_infrastructure_failure,
    rebuild_root_sequence,
};

/// Typed result for the TIR head-chain composition family.
type HeadChainResult<T> = Result<T, TemplateError>;

/// Carries the immutable services shared by recursive chain resolution.
pub(super) struct HeadChainResolutionInputs<'a> {
    pub(super) string_table: &'a StringTable,
    pub(super) expressions: &'a AstExpressionBuilder,
    pub(super) const_values: &'a ConstValueStore,
    pub(super) external_package_registry: Option<&'a ExternalPackageRegistry>,
    pub(super) allow_runtime_plans: bool,
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

/// Composes a head-chain directly from a root node, without requiring a
/// durable `TemplateIr` entry.
///
/// WHAT: splits the root node's children at the caller-owned authored boundary,
///       builds the wrapper chain, and resolves each layer. Returns the
///       composed root node ID.
/// WHY: control-flow body roots and aggregate-wrapper candidates are node
///      roots, not published templates. Their caller constructs the root from
///      a known head prefix plus body content, so it owns the exact boundary;
///      structural body nodes must never be guessed into the head partition.
///      Composing from the root node avoids pushing a scratch `TemplateIr`
///      that would remain in the durable store without a referencing identity.
#[allow(
    clippy::too_many_arguments,
    reason = "the caller-owned head boundary stays explicit beside the shared composition services"
)]
pub(crate) fn compose_tir_head_chain_from_root(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    root_node_id: TemplateIrNodeId,
    head_node_count: usize,
    string_table: &StringTable,
    allow_runtime_plans: bool,
    external_package_registry: Option<&ExternalPackageRegistry>,
) -> HeadChainResult<TemplateIrNodeId> {
    let inputs = HeadChainResolutionInputs {
        string_table,
        expressions,
        const_values,
        external_package_registry,
        allow_runtime_plans,
    };
    compose_tir_head_chain_from_root_into(store, root_node_id, head_node_count, &inputs)
}

/// Composes a published template's own root through the head chain.
///
/// A template root knows its exact authored head/body boundary from its
/// published summary. Passing that boundary prevents a body that begins with a
/// structural child template (for example a wrapper value containing a nested
/// decorator) from being mistaken for a head receiver and consuming its
/// siblings as fill. Bare node roots receive their boundary from the caller
/// that built them.
pub(crate) fn compose_tir_head_chain_from_template(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    template_id: TemplateIrId,
    string_table: &StringTable,
    allow_runtime_plans: bool,
    external_package_registry: Option<&ExternalPackageRegistry>,
) -> HeadChainResult<TemplateIrNodeId> {
    let (root_node_id, head_node_count) = {
        let template = store.get_template(template_id).ok_or_else(|| {
            internal_infrastructure_failure(
                "TIR head-chain composition: template ID was not present in the store.",
            )
        })?;
        (template.root, template.summary.head_node_count as usize)
    };
    let inputs = HeadChainResolutionInputs {
        string_table,
        expressions,
        const_values,
        external_package_registry,
        allow_runtime_plans,
    };
    compose_tir_head_chain_from_root_into(store, root_node_id, head_node_count, &inputs)
}

fn compose_tir_head_chain_from_root_into(
    store: &mut TemplateIrStore,
    root_node_id: TemplateIrNodeId,
    head_node_count: usize,
    inputs: &HeadChainResolutionInputs,
) -> HeadChainResult<TemplateIrNodeId> {
    // Fast path: if the root is not a sequence, no receiving layer can exist
    // and the original root is unchanged.
    let Some(root_children) = root_sequence_children(store, root_node_id)? else {
        return Ok(root_node_id);
    };

    let (head_children, body_children) =
        partition_tir_children_at_head_boundary(root_children, head_node_count);

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
        return Err(internal_infrastructure_failure(
            "TIR head-chain composition: root node ID was not present in the store.",
        ));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => Ok(Some(children)),
        _ => Ok(None),
    }
}

/// Partitions root sequence children at the authored head/body boundary.
///
/// WHAT: the first `head_node_count` children are head-origin and the rest are
///       body-origin.
/// WHY: the parser records every head node before body parsing starts, and a
///      calling render unit constructs bare roots from an explicit head prefix.
///      The boundary is authoritative for both callers, so a body that starts
///      with structural nodes and no body-origin text marker still stays in the
///      body partition. Clamping is defensive against malformed stores.
fn partition_tir_children_at_head_boundary(
    children: &[TemplateIrNodeId],
    head_node_count: usize,
) -> (Vec<TemplateIrNodeId>, Vec<TemplateIrNodeId>) {
    let boundary = head_node_count.min(children.len());
    (children[..boundary].to_vec(), children[boundary..].to_vec())
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
        return Err(internal_infrastructure_failure(
            "TIR head-chain composition: child node ID was not present in the store while checking receiver.",
        ));
    };

    let TemplateIrNodeKind::ChildTemplate { reference, .. } = &node.kind else {
        return Ok(None);
    };

    let Some(template_ir) = store.get_template(reference.root) else {
        return Err(internal_infrastructure_failure(
            "TIR head-chain composition: child template ID was not present in the store.",
        ));
    };

    if matches!(
        template_ir.kind,
        TemplateType::SlotInsert(_) | TemplateType::SlotDefinition(_)
    ) {
        return Ok(None);
    }

    let layout = collect_tir_slot_layout_with_context(store, reference.root, reference.context)?;

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

    let original_span = store
        .get_node(original_node_id)
        .map(|node| node.span)
        .ok_or_else(|| {
            internal_infrastructure_failure(
                "TIR head-chain composition: original wrapper node ID was not present in the store.",
            )
        })?;

    let resolved = compose_wrapper_application(
        store,
        inputs,
        layer.wrapper_reference,
        &layer.layout,
        resolved_fill_node_ids,
        &FxHashMap::default(),
        original_span,
    )?;

    let occurrence_id = store.next_child_template_occurrence_id();
    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: resolved,
            occurrence_id,
        },
        original_span,
    )))
}
