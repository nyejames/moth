//! TIR-native slot contribution routing.
//!
//! WHAT: partitions fill-template content into the slot buckets declared by a
//!       wrapper template's schema. Explicit `$insert(...)` helpers are routed
//!       by target key; remaining loose content is coalesced into chunks and
//!       assigned to positional slots first, then the default slot.
//!
//! WHY: this is the TIR-native routing phase. Separating routing from schema
//!      discovery and placeholder expansion keeps each file focused on one
//!      step of the composition pipeline.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateIrNodeId, TemplateIrStore,
};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use rustc_hash::FxHashMap;

use super::helpers::{
    children_of_node, extra_loose_content_without_default_slot_error, internal_compiler_error,
    location_for_template, loose_content_without_default_slot_error, root_node_id_for_template,
    unknown_slot_target_error,
};
use super::schema::{TirSlotSchema, collect_tir_slot_schema};

/// Typed result for slot-contribution routing.
type ContributionResult<T> = Result<T, TemplateError>;

/// Partitioned TIR node IDs bucketed by slot target.
///
/// WHAT: holds the routed TIR content for each slot key.
/// WHY: TIR-native slot composition needs to partition fill content by target
///      slot before expanding placeholders or building runtime plans.
#[derive(Debug, Default)]
pub(crate) struct TirSlotContributions {
    pub(crate) default_nodes: Vec<TemplateIrNodeId>,
    pub(crate) named_nodes: FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    pub(crate) positional_nodes: FxHashMap<usize, Vec<TemplateIrNodeId>>,
}

impl TirSlotContributions {
    /// Appends routed nodes to the default-slot bucket.
    pub(crate) fn extend_default_nodes(&mut self, nodes: Vec<TemplateIrNodeId>) {
        self.default_nodes.extend(nodes);
    }

    /// Appends routed nodes to one named-slot bucket.
    pub(crate) fn extend_named_nodes(&mut self, name: StringId, nodes: Vec<TemplateIrNodeId>) {
        self.named_nodes.entry(name).or_default().extend(nodes);
    }

    /// Appends routed nodes to one positional-slot bucket.
    pub(crate) fn extend_positional_nodes(&mut self, index: usize, nodes: Vec<TemplateIrNodeId>) {
        self.positional_nodes
            .entry(index)
            .or_default()
            .extend(nodes);
    }

    /// Returns the routed node IDs for a given slot key.
    ///
    /// WHAT: looks up the bucket matching `key`, returning an empty slice when
    ///       the slot received no contributions.
    /// WHY: callers (placeholder expansion, runtime plan building) need a
    ///      uniform view of every slot's content, including empty slots.
    pub(crate) fn nodes_for_slot(&self, key: &SlotKey) -> &[TemplateIrNodeId] {
        match key {
            SlotKey::Default => &self.default_nodes,
            SlotKey::Named(name) => self
                .named_nodes
                .get(name)
                .map(|nodes| nodes.as_slice())
                .unwrap_or(&[]),
            SlotKey::Positional(index) => self
                .positional_nodes
                .get(index)
                .map(|nodes| nodes.as_slice())
                .unwrap_or(&[]),
        }
    }
}

/// Result of routing fill content against a wrapper's slot schema.
///
/// WHAT: carries the bucketed contributions produced by routing fill content
///       against a wrapper's slot schema.
/// WHY: a named return keeps the routed buckets explicit at the stage boundary.
///      A test-only `schema` field is retained so focused tests can assert the
///      wrapper schema without re-deriving it.
#[derive(Debug)]
pub(crate) struct RoutedTirSlotContributions {
    #[cfg(test)]
    pub(crate) schema: TirSlotSchema,
    pub(crate) contributions: TirSlotContributions,
}

/// Routes fill template content against a wrapper's slot schema.
///
/// WHAT: discovers the wrapper's slot schema, walks the fill template's TIR
///       nodes, buckets explicit `InsertContribution` nodes by their target
///       slot key, groups remaining nodes into loose contribution chunks,
///       and routes loose chunks to positional slots first, then the default
///       slot.
/// WHY: TIR-native slot composition needs a single routing entry point that
///      partitions authored fill content before expansion or runtime planning.
pub(crate) fn route_tir_slot_contributions(
    store: &TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    fill_template_id: TemplateIrId,
    string_table: &StringTable,
) -> ContributionResult<RoutedTirSlotContributions> {
    increment_ast_counter(AstCounter::TirContributionRoutingCalls);
    let schema = collect_tir_slot_schema(store, wrapper_template_id)?;

    if !schema.has_any_slots() {
        return Err(internal_compiler_error(
            "Internal template wrapper state error: expected at least one '$slot' while composing.",
        ));
    }

    route_tir_fill_against_schema(store, &schema, fill_template_id, string_table)
}

/// Routes fill template content against a pre-collected wrapper slot schema.
///
/// WHAT: walks the fill template's TIR nodes, buckets explicit
///       `InsertContribution` nodes by their target slot key, groups remaining
///       nodes into loose contribution chunks, and routes loose chunks to
///       positional slots first, then the default slot.
/// WHY: head-chain composition reads the wrapper schema and routes fill
///      content from the one module store. Separating schema collection from
///      fill routing keeps one fill-walking owner without duplicating the
///      insert/loose-content traversal.
pub(super) fn route_tir_fill_against_schema(
    store: &TemplateIrStore,
    schema: &TirSlotSchema,
    fill_template_id: TemplateIrId,
    string_table: &StringTable,
) -> ContributionResult<RoutedTirSlotContributions> {
    let fill_root = root_node_id_for_template(store, fill_template_id)?;
    let fill_children = children_of_node(store, fill_root)?;
    let fill_location = location_for_template(store, fill_template_id)?;

    let mut contributions = TirSlotContributions::default();
    let mut loose_nodes = Vec::new();

    // Walk authored fill content exactly once. Explicit `$insert(...)` helpers
    // (either as `InsertContribution` nodes or as `ChildTemplate` references to
    // a `SlotInsert` helper) are bucketed by target key; everything else becomes
    // loose content that flows into positional or default slots.
    for child_id in fill_children {
        let Some(child_node) = store.get_node(child_id) else {
            return Err(internal_compiler_error(
                "TIR slot routing: fill template child node ID was not present in the store.",
            ));
        };

        // Resolve an explicit slot-insert helper to its target key and body
        // template. Both `InsertContribution` nodes and `ChildTemplate`
        // references to a `SlotInsert` helper represent the same authored
        // `$insert("name")` construct at different TIR construction stages.
        let insert_info = match &child_node.kind {
            TemplateIrNodeKind::InsertContribution { template } => Some(*template),
            TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                let template = store.get_template(reference.root).ok_or_else(|| {
                    internal_compiler_error(
                        "TIR slot routing: child template ID was not present in the store.",
                    )
                })?;

                if matches!(template.kind, TemplateType::SlotInsert(_)) {
                    Some(reference.root)
                } else {
                    None
                }
            }

            // Slots in fill content are slot declarations, not contributions.
            // They would only appear inside a wrapper, so they are ignored here.
            TemplateIrNodeKind::Slot { .. } => {
                continue;
            }

            // All other node kinds are loose content that must be grouped and
            // routed to positional/default slots.
            _ => {
                loose_nodes.push(child_id);
                continue;
            }
        };

        if let Some(insert_template_id) = insert_info {
            let target_template = store.get_template(insert_template_id).ok_or_else(|| {
                internal_compiler_error(
                    "TIR slot routing: slot-insert helper referenced a missing template.",
                )
            })?;

            let TemplateType::SlotInsert(target_key) = &target_template.kind else {
                return Err(internal_compiler_error(
                    "TIR slot routing: slot-insert helper is not a SlotInsert template.",
                ));
            };

            if !schema.accepts_target(target_key) {
                return Err(unknown_slot_target_error(
                    target_key,
                    target_template.location.to_owned(),
                )
                .into());
            }

            // The slot-insert helper is a routing marker: its body content fills
            // the target slot. Expand the helper's root children into the target
            // bucket so the composed tree contains the actual content, not the
            // marker node.
            let contribution_nodes = collect_insert_contribution_content(
                store,
                insert_template_id,
                schema,
                &mut contributions,
            )?;

            match target_key {
                SlotKey::Default => contributions.extend_default_nodes(contribution_nodes),
                SlotKey::Named(name) => contributions.extend_named_nodes(*name, contribution_nodes),
                SlotKey::Positional(index) => {
                    contributions.extend_positional_nodes(*index, contribution_nodes);
                }
            }
        } else {
            // ChildTemplate references that are not slot-insert helpers are
            // ordinary fill content.
            loose_nodes.push(child_id);
        }
    }

    // Route loose content to positional slots first, then to the default slot.
    // Keep the closest positional slot receiving the next authored chunk.
    let loose_chunks = collect_loose_tir_contributions(loose_nodes, store, string_table)?;
    let ordered_positional_slots = schema.ordered_positional_slots();

    for (chunk_index, chunk) in loose_chunks.into_iter().enumerate() {
        if let Some(slot_index) = ordered_positional_slots.get(chunk_index) {
            contributions.extend_positional_nodes(*slot_index, chunk.nodes);
            continue;
        }

        if schema.has_default_slot {
            contributions.extend_default_nodes(chunk.nodes);
            continue;
        }

        // Formatting whitespace around explicit insert contributions carries no
        // value when the wrapper has nowhere to render loose content. Discard it
        // while preserving diagnostics for every meaningful loose contribution.
        if tir_nodes_are_whitespace_only_text(&chunk.nodes, store, string_table)? {
            continue;
        }

        if schema.positional_slots.is_empty() {
            return Err(loose_content_without_default_slot_error(fill_location).into());
        }

        return Err(extra_loose_content_without_default_slot_error(fill_location).into());
    }

    Ok(RoutedTirSlotContributions {
        #[cfg(test)]
        schema: schema.clone(),
        contributions,
    })
}

/// Expands a SlotInsert helper referenced by an `InsertContribution` node into
/// the node IDs that should fill the helper's target slot.
///
/// WHAT: walks the helper's root children. Nested `InsertContribution` nodes are
///       recursively routed to their own target slots; other nodes become
///       contributions to the helper's target slot.
/// WHY: the `InsertContribution` node itself is just a routing marker. The
///      composed tree must contain the helper's body content, not the marker.
fn collect_insert_contribution_content(
    store: &TemplateIrStore,
    insert_template_id: TemplateIrId,
    schema: &TirSlotSchema,
    contributions: &mut TirSlotContributions,
) -> ContributionResult<Vec<TemplateIrNodeId>> {
    let insert_root = root_node_id_for_template(store, insert_template_id)?;
    let insert_children = children_of_node(store, insert_root)?;
    let mut target_nodes = Vec::new();

    for child_id in insert_children {
        let Some(child_node) = store.get_node(child_id) else {
            return Err(internal_compiler_error(
                "TIR slot routing: insert contribution child node ID was not present in the store.",
            ));
        };

        match &child_node.kind {
            TemplateIrNodeKind::InsertContribution { template } => {
                let target_template = store.get_template(*template).ok_or_else(|| {
                    internal_compiler_error(
                        "TIR slot routing: nested InsertContribution referenced a missing template.",
                    )
                })?;

                let TemplateType::SlotInsert(target_key) = &target_template.kind else {
                    return Err(internal_compiler_error(
                        "TIR slot routing: nested InsertContribution referenced a template that is not a SlotInsert helper.",
                    ));
                };

                if !schema.accepts_target(target_key) {
                    return Err(unknown_slot_target_error(
                        target_key,
                        target_template.location.to_owned(),
                    )
                    .into());
                }

                let nested_nodes =
                    collect_insert_contribution_content(store, *template, schema, contributions)?;
                match target_key {
                    SlotKey::Default => contributions.extend_default_nodes(nested_nodes),
                    SlotKey::Named(name) => contributions.extend_named_nodes(*name, nested_nodes),
                    SlotKey::Positional(index) => {
                        contributions.extend_positional_nodes(*index, nested_nodes)
                    }
                }
            }

            // Slot declarations do not appear inside a SlotInsert helper body.
            TemplateIrNodeKind::Slot { .. } => {}

            // All other nodes are the helper's body content and fill the helper's
            // own target slot.
            _ => {
                target_nodes.push(child_id);
            }
        }
    }

    Ok(target_nodes)
}

/// A group of loose TIR node IDs treated as one positional contribution chunk.
///
/// WHAT: holds one authored loose contribution as an ordered group of TIR node
///       IDs.
/// WHY: loose content must be coalesced into logical chunks before it is
///      assigned to positional slots, so body-level whitespace does not consume
///      positional slot positions.
struct LooseTirContribution {
    nodes: Vec<TemplateIrNodeId>,
}

/// Groups loose TIR node IDs into logical contribution chunks.
///
/// WHAT: walks loose nodes in authored order and splits them into chunks.
///       A `ChildTemplate` node or a `Head`-origin `Text` / `DynamicExpression`
///       node starts a new logical contribution. Whitespace-only body text
///       before a new contribution is carried with that contribution; meaningful
///       body text stays as its own chunk.
/// WHY: whitespace handling stays local to TIR while preserving the existing
///      treatment of meaningful body text. In `[row, item: [item]]`, the
///      separator after `:` belongs to the default body contribution, not the
///      preceding positional head argument.
fn collect_loose_tir_contributions(
    loose_nodes: Vec<TemplateIrNodeId>,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> ContributionResult<Vec<LooseTirContribution>> {
    let mut chunks = Vec::new();
    let mut pending_nodes = Vec::new();

    for node_id in loose_nodes {
        let node = store.get_node(node_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR slot routing: loose contribution node ID was not present in the store.",
            )
        })?;
        let starts_new_chunk = {
            matches!(&node.kind, TemplateIrNodeKind::ChildTemplate { .. })
                || matches!(
                    &node.kind,
                    TemplateIrNodeKind::Text {
                        origin: TemplateSegmentOrigin::Head,
                        ..
                    } | TemplateIrNodeKind::DynamicExpression {
                        origin: TemplateSegmentOrigin::Head,
                        ..
                    }
                )
        };

        if starts_new_chunk {
            if pending_nodes.is_empty()
                || tir_nodes_are_whitespace_only_text(&pending_nodes, store, string_table)?
            {
                pending_nodes.push(node_id);
                chunks.push(LooseTirContribution {
                    nodes: std::mem::take(&mut pending_nodes),
                });
                continue;
            }

            chunks.push(LooseTirContribution {
                nodes: std::mem::take(&mut pending_nodes),
            });
            chunks.push(LooseTirContribution {
                nodes: vec![node_id],
            });
            continue;
        }

        pending_nodes.push(node_id);
    }

    if !pending_nodes.is_empty() {
        chunks.push(LooseTirContribution {
            nodes: pending_nodes,
        });
    }

    Ok(chunks)
}

fn tir_nodes_are_whitespace_only_text(
    nodes: &[TemplateIrNodeId],
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> ContributionResult<bool> {
    if nodes.is_empty() {
        return Ok(false);
    }

    for node_id in nodes {
        let node = store.get_node(*node_id).ok_or_else(|| {
            internal_compiler_error(
                "TIR slot routing: loose contribution whitespace check found a node ID that was not present in the store.",
            )
        })?;

        let TemplateIrNodeKind::Text { text, .. } = &node.kind else {
            return Ok(false);
        };

        if !string_table.resolve(*text).trim().is_empty() {
            return Ok(false);
        }
    }

    Ok(true)
}
