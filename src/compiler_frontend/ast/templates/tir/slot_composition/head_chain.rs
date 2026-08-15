//! TIR-native head-chain composition.
//!
//! WHAT: resolves wrapper templates in a template's head section against the
//!       body content that fills their slots. It partitions root children by
//!       head/body origin, builds a chain graph of receiving layers, and
//!       resolves each layer by routing contributions and expanding slot
//!       placeholders.
//!
//! WHY: this is the TIR-native equivalent of the atom-level
//!      `compose_template_head_chain_atoms` in `template_composition.rs`. It
//!      replaces the atom-level chain graph with TIR node operations while
//!      reusing the schema, contribution, and expansion helpers.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{TemplateSegmentOrigin, TemplateType};
use crate::compiler_frontend::ast::templates::template_slots::{
    materialize_tir_native_runtime_slot_plan, tir_contributions_need_runtime,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::cell::RefCell;
use std::rc::Rc;

use super::helpers::{
    ComposedTirRoot, SlotResolutionComposition, build_composed_wrapper_template,
    build_tir_fill_template, children_of_node, internal_compiler_error, rebuild_root_sequence,
};
use super::overlays::allocate_slot_resolution_context;
use super::schema::{collect_tir_slot_schema, expand_tir_slot_placeholders_into};

/// Typed result for the TIR head-chain composition family.
type HeadChainResult<T> = Result<T, TemplateError>;

/// Bundles the shared state threaded through recursive chain resolution.
///
/// WHAT: carries the string table, accumulated slot compositions, and the
///       runtime-plan flag so recursive `resolve_tir_chain_items` /
///       `resolve_tir_chain_layer` calls stay readable without a long argument
///       list.
/// WHY: the same four values are passed unchanged through every recursion
///      level. Grouping them in one struct keeps the recursive call sites
///      short and makes it obvious that nested layers inherit the caller's
///      real composition state rather than a fresh or default context.
struct HeadChainResolutionInputs<'a> {
    string_table: &'a StringTable,
    slot_compositions: &'a mut Vec<SlotResolutionComposition>,
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
    /// Effective wrapper identity from the head-origin child occurrence.
    wrapper_reference: TemplateTirChildReference,

    /// Fill content items, in authored order. Items may be direct node IDs or
    /// references to nested chain layers that must be resolved first.
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

/// Composes a template's head-chain by resolving wrapper templates with their
/// fill content, producing a new TIR root node.
///
/// WHAT: walks the template's TIR root `Sequence` children, partitions them by
///       origin (Head vs Body), identifies head-origin wrapper templates that
///       open receiving layers, accumulates fill content for each layer, and
///       resolves each layer by routing and expanding slot contributions.
/// WHY: this is the TIR-native equivalent of the atom-level
///      `compose_template_head_chain_atoms` in `template_composition.rs`. It
///      replaces the atom-level chain graph with TIR node operations, using the
///      already-implemented `route_tir_slot_contributions` and
///      `expand_tir_slot_placeholders`.
pub(crate) fn compose_tir_head_chain(
    store: &mut TemplateIrStore,
    template_id: TemplateIrId,
    string_table: &StringTable,
    allow_runtime_plans: bool,
) -> HeadChainResult<TemplateIrNodeId> {
    let mut slot_compositions = Vec::new();
    let mut inputs = HeadChainResolutionInputs {
        string_table,
        slot_compositions: &mut slot_compositions,
        allow_runtime_plans,
    };
    compose_tir_head_chain_into(store, template_id, &mut inputs)
}

/// Internal head-chain composition that also collects slot-bearing wrapper/fill
/// pairs into `slot_compositions` for later overlay allocation.
///
/// WHY: the slot-composition entry point (`compose_tir_head_chain_with_overlays`)
///      needs the pairs to allocate slot-resolution overlays after the store
///      borrow is released. The public `compose_tir_head_chain` discards them.
fn compose_tir_head_chain_into(
    store: &mut TemplateIrStore,
    template_id: TemplateIrId,
    inputs: &mut HeadChainResolutionInputs,
) -> HeadChainResult<TemplateIrNodeId> {
    let template = store.get_template(template_id).ok_or_else(|| {
        internal_compiler_error(
            "TIR head-chain composition: template ID was not present in the store.",
        )
    })?;

    // Fast path: no child-template references means no wrapper receivers can
    // exist, and the original root is unchanged. The summary is cheap to check
    // and avoids partitioning/walking children for the common case.
    if template.summary.child_template_count == 0 {
        return Ok(template.root);
    }

    let root_node_id = template.root;

    // Fast path: if the root is not a sequence, no receiving layer can exist
    // and the original root is unchanged.
    let Some(root_children) = root_sequence_children(store, root_node_id)? else {
        return Ok(root_node_id);
    };

    // Cheap pre-scan: if no head-origin child template is a wrapper receiver,
    // the original root is unchanged and we can avoid allocating the head/body
    // partition vectors. This matters because many templates contain head
    // references (e.g. function calls) that are not slot-bearing wrappers.
    if !has_tir_head_chain_receiver(store, root_children)? {
        return Ok(root_node_id);
    }

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

/// Composes the TIR head chain on the shared module-local store and threads a
/// slot-resolution view context onto the result when the
/// composition resolved one or more slot-bearing wrappers.
///
/// WHAT: runs the existing store-local structural head-chain composition
///       (unchanged behavior), collects slot-bearing wrapper/fill pairs, then
///       releases the mutable store borrow before constructing the value
///       context after overlay payload materialization.
/// WHY: production composition call sites need both the composed root for
///      structural expansion and the value context for `TemplateTirReference`
///      threading. Keeping the orchestration in the slot-composition owner
///      avoids ad hoc overlay construction at call sites.
pub(crate) fn compose_tir_head_chain_with_overlays(
    store_handle: &Rc<RefCell<TemplateIrStore>>,
    template_id: TemplateIrId,
    string_table: &StringTable,
    allow_runtime_plans: bool,
) -> HeadChainResult<ComposedTirRoot> {
    let (composed_root, slot_compositions) = {
        let mut store = store_handle.borrow_mut();
        let mut slot_compositions = Vec::new();
        let mut inputs = HeadChainResolutionInputs {
            string_table,
            slot_compositions: &mut slot_compositions,
            allow_runtime_plans,
        };
        let composed_root = compose_tir_head_chain_into(&mut store, template_id, &mut inputs)?;
        (composed_root, slot_compositions)
    };

    let slot_context = allocate_slot_resolution_context(
        &mut store_handle.borrow_mut(),
        &slot_compositions,
        string_table,
    )?;

    Ok(ComposedTirRoot {
        root: composed_root,
        slot_context,
    })
}

/// Returns the children of a `Sequence` root node, or `None` if the root is not
/// a sequence.
///
/// WHAT: centralizes the root-shape check so callers can treat non-sequence
///       roots as uncomposable.
/// WHY: head-chain composition only applies to templates whose body is a
///      sequence of children.
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

/// Cheaply checks whether a root sequence contains a head-origin wrapper
/// receiver.
///
/// WHAT: walks children in source order, stopping at the first body-origin
///       `Text`/`DynamicExpression`. Within the head prefix, any `ChildTemplate`
///       that references a slot-bearing, non-helper template makes the template
///       a composition candidate.
/// WHY: this avoids allocating head/body vectors for the vast majority of
///      templates whose head references are not slot wrappers.
fn has_tir_head_chain_receiver(
    store: &TemplateIrStore,
    children: &[TemplateIrNodeId],
) -> HeadChainResult<bool> {
    let mut saw_body_origin = false;

    for child_id in children {
        let child_node = store.get_node(*child_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR head-chain composition: child node ID was not present in the store while scanning for receivers.",
            )
        })?;

        let is_body = match &child_node.kind {
            TemplateIrNodeKind::Text { origin, .. }
            | TemplateIrNodeKind::DynamicExpression { origin, .. } => {
                *origin == TemplateSegmentOrigin::Body
            }

            TemplateIrNodeKind::AggregateOutput => true,

            _ => saw_body_origin,
        };

        if is_body {
            saw_body_origin = true;
            continue;
        }

        if matches!(child_node.kind, TemplateIrNodeKind::ChildTemplate { .. })
            && is_tir_receiver(store, *child_id)?.is_some()
        {
            return Ok(true);
        }
    }

    Ok(false)
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
) -> HeadChainResult<Option<TemplateTirChildReference>> {
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

    let schema = collect_tir_slot_schema(store, reference.root)?;

    Ok(if schema.has_any_slots() {
        Some(*reference)
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
/// WHY: this mirrors the atom-level chain-graph construction in
///      `compose_template_head_chain_atoms` while using TIR item references.
fn build_tir_chain_graph(
    store: &TemplateIrStore,
    head_children: &[TemplateIrNodeId],
    body_children: &[TemplateIrNodeId],
) -> HeadChainResult<(Vec<TirChainItem>, Vec<TirChainLayer>)> {
    let mut root_items = Vec::new();
    let mut layers = Vec::new();
    let mut active_layer: Option<usize> = None;

    for child_id in head_children {
        if let Some(wrapper_reference) = is_tir_receiver(store, *child_id)? {
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

    // Body atoms are appended after head parsing. If the head opened a receiving
    // chain, body atoms become contributions to the deepest active receiver.
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
    inputs: &mut HeadChainResolutionInputs,
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
    inputs: &mut HeadChainResolutionInputs,
) -> HeadChainResult<TemplateIrNodeId> {
    let layer = &layers[layer_index];

    if layer.fill_items.is_empty() {
        // Head-only wrapper references stay unresolved so they can be filled
        // later at a use-site, preserving the wrapper's unresolved-slot state.
        return Ok(original_node_id);
    }

    // Module-local wrapper: structural expansion path.
    let wrapper_template_id = layer.wrapper_reference.root;

    let resolved_fill_node_ids = resolve_tir_chain_items(store, &layer.fill_items, layers, inputs)?;

    let fill_template_id =
        build_tir_fill_template(store, resolved_fill_node_ids, original_node_id)?;

    let routed = super::contributions::route_tir_slot_contributions(
        store,
        wrapper_template_id,
        fill_template_id,
        inputs.string_table,
    )?;

    // Decide between structural expansion and runtime slot planning. When the
    // fill content contains non-const-evaluable (runtime) nodes — such as
    // asset references, dynamic expressions, or loop control — the wrapper must
    // lower as a runtime slot plan so the HIR preserves slot-site boundaries
    // and loop-control semantics. Structural expansion would flatten wrapper
    // text and fill content together, which breaks `continue` inside slot
    // fills and drops runtime slot-site metadata.
    let schema = collect_tir_slot_schema(store, wrapper_template_id)?;

    let composed_template_id = if inputs.allow_runtime_plans
        && tir_contributions_need_runtime(
            &schema,
            &routed.contributions,
            inputs.string_table,
            store,
        ) {
        let original_location = store
            .get_node(original_node_id)
            .map(|node| node.location.to_owned())
            .ok_or_else(|| {
                internal_compiler_error(
                    "TIR head-chain composition: original wrapper node ID was not present in the store.",
                )
            })?;

        materialize_tir_native_runtime_slot_plan(
            store,
            wrapper_template_id,
            &routed,
            inputs.string_table,
            &original_location,
        )
        .map_err(TemplateError::from)?
    } else {
        // Const-evaluable fill: structurally expand slot placeholders into the
        // wrapper tree. This is the compile-time composition path that produces
        // a fully flattened render tree with no runtime slot-plan metadata.
        let expanded_root = expand_tir_slot_placeholders_into(
            store,
            wrapper_template_id,
            &routed,
            inputs.string_table,
            inputs.slot_compositions,
        )?;

        build_composed_wrapper_template(store, wrapper_template_id, expanded_root)?
    };

    // Record the wrapper/fill pair so the shared-store entry point can allocate
    // a slot-resolution overlay after the structural borrow is released.
    let fill_reference = fill_template_id;
    inputs
        .slot_compositions
        .push(SlotResolutionComposition::new(
            layer.wrapper_reference,
            fill_reference,
        ));

    let original_node = store.get_node(original_node_id).ok_or_else(|| {
        internal_compiler_error(
            "TIR head-chain composition: original wrapper node ID was not present in the store.",
        )
    })?;

    let original_location = original_node.location.to_owned();

    let occurrence_id = store.next_child_template_occurrence_id();
    let reference = TemplateTirChildReference::new(
        composed_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        },
        original_location,
    )))
}
