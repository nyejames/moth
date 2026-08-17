//! Slot-layout owner for one TIR wrapper tree.
//!
//! One cycle-guarded walk records unique schema targets and every placeholder
//! occurrence. Schema discovery, placeholder collection and the structural
//! slot-presence probe share this walk. It is not a generic TIR visitor.

use std::collections::BTreeSet;

use rustc_hash::FxHashSet;

use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::ids::{
    SlotOccurrenceId, TemplateIrId, TemplateIrNodeId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Unique slot targets declared by a wrapper tree.
#[derive(Debug, Default, Clone)]
pub(crate) struct TirSlotSchema {
    pub(crate) has_default_slot: bool,
    pub(crate) named_slots: FxHashSet<StringId>,
    pub(crate) positional_slots: BTreeSet<usize>,
}

impl TirSlotSchema {
    pub(crate) fn has_any_slots(&self) -> bool {
        self.has_default_slot || !self.named_slots.is_empty() || !self.positional_slots.is_empty()
    }

    pub(crate) fn accepts_target(&self, target: &SlotKey) -> bool {
        match target {
            SlotKey::Default => self.has_default_slot,
            SlotKey::Named(name) => self.named_slots.contains(name),
            SlotKey::Positional(index) => self.positional_slots.contains(index),
        }
    }

    /// Smallest positional slot, otherwise the default slot.
    pub(crate) fn loose_fill_target_key(&self) -> Option<SlotKey> {
        self.positional_slots
            .iter()
            .next()
            .copied()
            .map(SlotKey::Positional)
            .or_else(|| self.has_default_slot.then_some(SlotKey::Default))
    }

    pub(crate) fn ordered_positional_slots(&self) -> Vec<usize> {
        self.positional_slots.iter().copied().collect()
    }

    pub(crate) fn ordered_named_slots(&self, string_table: &StringTable) -> Vec<StringId> {
        let mut names = self.named_slots.iter().copied().collect::<Vec<_>>();

        names.sort_by(|left, right| {
            string_table
                .resolve(*left)
                .cmp(string_table.resolve(*right))
        });

        names
    }

    /// Default, then positional slots in numeric order, then named slots by spelling.
    pub(crate) fn ordered_slot_keys(&self, string_table: &StringTable) -> Vec<SlotKey> {
        let mut keys = Vec::new();

        if self.has_default_slot {
            keys.push(SlotKey::Default);
        }

        for index in self.ordered_positional_slots() {
            keys.push(SlotKey::Positional(index));
        }

        for name in self.ordered_named_slots(string_table) {
            keys.push(SlotKey::Named(name));
        }

        keys
    }

    /// Records a unique target key. Repeated occurrences stay valid replay sites.
    pub(crate) fn record_key(&mut self, key: &SlotKey) {
        match key {
            SlotKey::Default => {
                self.has_default_slot = true;
            }

            SlotKey::Named(name) => {
                self.named_slots.insert(*name);
            }

            SlotKey::Positional(index) => {
                self.positional_slots.insert(*index);
            }
        }
    }
}

/// Occurrence facts needed by routing and runtime sites.
///
/// Wrapper-set IDs and the slot node's location are enough. Callers must not
/// clone a complete `TirSlotPlaceholder` just to carry these fields.
#[derive(Debug, Clone)]
pub(crate) struct TirSlotPlaceholderRef {
    #[allow(
        dead_code,
        reason = "slot occurrence identity remains part of the layout contract for exact-view consumers"
    )]
    pub(crate) occurrence_id: SlotOccurrenceId,
    pub(crate) key: SlotKey,
    pub(crate) child_wrapper_set: Option<TemplateWrapperSetId>,
    pub(crate) applied_child_wrapper_set: Option<TemplateWrapperSetId>,
    pub(crate) skip_parent_child_wrappers: bool,
    pub(crate) location: SourceLocation,
}

/// Complete slot layout for one wrapper tree.
#[derive(Debug, Clone)]
pub(crate) struct TirSlotLayout {
    pub(crate) schema: TirSlotSchema,
    pub(crate) placeholders: Vec<TirSlotPlaceholderRef>,
}

/// Collects the slot layout of a published template.
pub(crate) fn collect_tir_slot_layout(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<TirSlotLayout, CompilerError> {
    increment_ast_counter(AstCounter::TirSlotSchemaWalks);

    let Some(template) = store.get_template(template_id) else {
        return Err(layout_error(format!(
            "TIR slot layout: template {template_id} was not present in the store."
        )));
    };

    collect_layout_from_root(store, template.root)
}

/// Collects the slot layout of a node tree.
pub(crate) fn collect_tir_slot_layout_from_root(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
) -> Result<TirSlotLayout, CompilerError> {
    increment_ast_counter(AstCounter::TirSlotSchemaWalks);
    collect_layout_from_root(store, root_node_id)
}

/// Schema-only convenience over the one layout walk.
pub(crate) fn collect_tir_slot_schema(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<TirSlotSchema, CompilerError> {
    Ok(collect_tir_slot_layout(store, template_id)?.schema)
}

fn collect_layout_from_root(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
) -> Result<TirSlotLayout, CompilerError> {
    let mut schema = TirSlotSchema::default();
    let mut placeholders = Vec::new();
    let mut visiting_nodes = FxHashSet::default();
    let mut visiting_templates = FxHashSet::default();

    collect_from_node(
        store,
        root_node_id,
        &mut schema,
        &mut placeholders,
        &mut visiting_nodes,
        &mut visiting_templates,
    )?;

    Ok(TirSlotLayout {
        schema,
        placeholders,
    })
}

fn collect_from_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    schema: &mut TirSlotSchema,
    placeholders: &mut Vec<TirSlotPlaceholderRef>,
    visiting_nodes: &mut FxHashSet<TemplateIrNodeId>,
    visiting_templates: &mut FxHashSet<TemplateIrId>,
) -> Result<(), CompilerError> {
    if !visiting_nodes.insert(node_id) {
        return Err(layout_error(format!(
            "TIR slot layout encountered a node cycle at {node_id:?}"
        )));
    }

    let Some(node) = store.get_node(node_id) else {
        visiting_nodes.remove(&node_id);
        return Err(layout_error(format!(
            "TIR slot layout requested missing node {node_id}."
        )));
    };

    let result = match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                collect_from_node(
                    store,
                    *child_id,
                    schema,
                    placeholders,
                    visiting_nodes,
                    visiting_templates,
                )?;
            }
            Ok(())
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            schema.record_key(&placeholder.key);
            placeholders.push(TirSlotPlaceholderRef {
                occurrence_id: placeholder.occurrence_id,
                key: placeholder.key.clone(),
                child_wrapper_set: placeholder.child_wrapper_set,
                applied_child_wrapper_set: placeholder.applied_child_wrapper_set,
                skip_parent_child_wrappers: placeholder.skip_parent_child_wrappers,
                location: node.location.clone(),
            });
            Ok(())
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => collect_from_child_template(
            store,
            reference.root,
            schema,
            placeholders,
            visiting_templates,
        ),

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                collect_from_node(
                    store,
                    branch.body,
                    schema,
                    placeholders,
                    visiting_nodes,
                    visiting_templates,
                )?;
            }

            if let Some(fallback_id) = fallback {
                collect_from_node(
                    store,
                    *fallback_id,
                    schema,
                    placeholders,
                    visiting_nodes,
                    visiting_templates,
                )?;
            }

            Ok(())
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_from_node(
                store,
                *body,
                schema,
                placeholders,
                visiting_nodes,
                visiting_templates,
            )?;

            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                collect_from_node(
                    store,
                    *aggregate_wrapper_id,
                    schema,
                    placeholders,
                    visiting_nodes,
                    visiting_templates,
                )?;
            }

            Ok(())
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::InsertContribution { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(()),
    };

    visiting_nodes.remove(&node_id);
    result
}

fn collect_from_child_template(
    store: &TemplateIrStore,
    child_template_id: TemplateIrId,
    schema: &mut TirSlotSchema,
    placeholders: &mut Vec<TirSlotPlaceholderRef>,
    visiting_templates: &mut FxHashSet<TemplateIrId>,
) -> Result<(), CompilerError> {
    let Some(child_template) = store.get_template(child_template_id) else {
        return Err(layout_error(format!(
            "TIR slot layout referenced missing child template {child_template_id}."
        )));
    };

    if store.get_node(child_template.root).is_none() {
        return Err(layout_error(format!(
            "TIR slot layout found child template {child_template_id} with missing root node {}.",
            child_template.root
        )));
    }

    if !visiting_templates.insert(child_template_id) {
        return Err(layout_error(format!(
            "TIR slot layout encountered a template cycle at {child_template_id:?}"
        )));
    }

    let child_root = child_template.root;
    let mut child_nodes = FxHashSet::default();
    let result = collect_from_node(
        store,
        child_root,
        schema,
        placeholders,
        &mut child_nodes,
        visiting_templates,
    );
    visiting_templates.remove(&child_template_id);
    result
}

fn layout_error(message: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(message)
}
