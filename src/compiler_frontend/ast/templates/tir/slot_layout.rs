//! Slot-layout owner for one TIR wrapper tree.
//!
//! A cycle-guarded walk over the wrapper root records unique structural schema
//! targets and every placeholder occurrence. Schema discovery additionally
//! recurses into slot-attached decorator wrappers and inherited child-edge
//! overlays through their wrapper references, so collection is not a single
//! flat TIR traversal.
//!
//! A placeholder's attached `$before`/`$after` decorator wrappers contribute their named slots
//! to the same schema: the wrapper application created for that placeholder's
//! contributions is the occurrence that consumes them. A child reference's inherited
//! wrapper-context overlay contributes the same way: those decorators keep their named
//! occurrences outside the referenced template's structural root, and the enclosing
//! application's named content is resolved into that overlay.
//!
//! Inherited overlay wrappers contribute only named targets. Their default and
//! positional slots stay owned by the fixed child they decorate, so exposing those
//! would accept content the overlay can never consume.
//!
//! A child template that already owns a runtime slot plan is an opaque boundary.
//! Its placeholders are resolved sites and every child-edge overlay inside its tree
//! was filled with that plan's markers, so the enclosing application can neither
//! address those names nor fill those contexts. The walk therefore treats such an
//! edge as a leaf, matching the marker fill that deliberately stops there instead
//! of writing foreign plan markers.

use std::collections::BTreeSet;

use rustc_hash::FxHashSet;

use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::ids::{
    SlotOccurrenceId, TemplateIrId, TemplateIrNodeId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIr, TemplateIrNodeKind};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

/// Unique slot targets addressable from one wrapper tree.
///
/// Structural placeholders and slot-attached wrapper sets contribute their named
/// targets. Inherited wrapper-context overlays contribute only named targets:
/// their default and positional slots belong to the fixed child they decorate.
#[derive(Debug, Default, Clone)]
pub(crate) struct TirSlotSchema {
    pub(crate) has_default_slot: bool,
    pub(crate) named_slots: FxHashSet<StringId>,
    pub(crate) positional_slots: BTreeSet<usize>,
    /// Whether the layout saw an applicable inherited wrapper context.
    ///
    /// WHAT: set while layout collection reads a reference or child-edge overlay
    ///       and finds a non-skipped inherited wrapper set.
    /// WHY: runtime slot planning must resolve plan-qualified markers into those
    ///      overlays before the handoff store becomes read-only. Structural named
    ///      occurrences and slot-attached wrapper sets are served by ordinary site
    ///      planning, so planners use this fact to keep applications without
    ///      inherited wrapper contexts free of named-marker nodes and context fills.
    pub(crate) has_inherited_wrapper_contexts: bool,
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

    /// Retains only the routing facts a wrapper fold consumes per direct child.
    ///
    /// WHAT: records whether the wrapper declares any slots and which occurrence is
    ///       its loose-fill target.
    /// WHY: folding applies one wrapper edge around every direct child that inherits
    ///      it. Caching this compact view preserves the exact insertion decision
    ///      without retaining the schema's name and position collections.
    pub(crate) fn wrapper_slot_routing(&self) -> WrapperSlotRouting {
        WrapperSlotRouting {
            has_any_slots: self.has_any_slots(),
            loose_fill_target_key: self.loose_fill_target_key(),
        }
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

/// Slot-routing facts one fold needs to place a child at a wrapper edge.
///
/// WHAT: keeps whether the wrapper declares any slots together with its loose-fill
///       target.
/// WHY: a fold applies one inherited wrapper edge around every direct child, and
///      only these two facts steer where the child is spliced. Keeping them
///      separate from [`TirSlotSchema`] avoids retaining schema name and position
///      collections for the whole fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WrapperSlotRouting {
    pub(crate) has_any_slots: bool,
    pub(crate) loose_fill_target_key: Option<SlotKey>,
}

/// Occurrence facts needed by routing and runtime sites.
///
/// Wrapper-set IDs and the slot node's source span are enough. Callers must not
/// clone a complete `TirSlotPlaceholder` just to carry these fields.
#[derive(Debug, Clone)]
pub(crate) struct TirSlotPlaceholderRef {
    #[allow(
        dead_code,
        reason = "slot occurrence identity remains part of the layout contract for exact-view consumers"
    )]
    pub(crate) occurrence_id: SlotOccurrenceId,
    pub(crate) key: SlotKey,
    /// Occurrence-owned compile-time fallback. Runtime site planning materialises it
    /// only when no routed contribution source targets this occurrence.
    pub(crate) default: Option<ConstValueId>,
    pub(crate) child_wrapper_set: Option<TemplateWrapperSetId>,
    pub(crate) applied_child_wrapper_set: Option<TemplateWrapperSetId>,
    pub(crate) skip_parent_child_wrappers: bool,
    pub(crate) span: Option<SourceSpan>,
}

/// Complete slot layout for one wrapper tree.
#[derive(Debug, Default, Clone)]
pub(crate) struct TirSlotLayout {
    pub(crate) schema: TirSlotSchema,
    pub(crate) placeholders: Vec<TirSlotPlaceholderRef>,
}

/// Whether a template is an independently materialized runtime slot application.
///
/// WHAT: true when the published template carries a committed `runtime_slot_plan`.
/// WHY: such a template is a completed application. Its placeholders are resolved
///      sites and the child-edge overlays inside its tree were filled with its own
///      plan's markers, so an enclosing application can neither address those names
///      nor fill those contexts. Slot-layout discovery, structural expansion and
///      runtime marker fills all use this predicate to treat the edge as a leaf;
///      keeping it named keeps those three owners on one boundary definition.
pub(crate) fn template_is_independently_planned(template: &TemplateIr) -> bool {
    template.runtime_slot_plan.is_some()
}

/// Collects the slot layout of a published template.
///
/// This form carries no value context, so it sees only the template's
/// structural tree and the wrapper sets stored on its own placeholders. Use
/// [`collect_tir_slot_layout_with_context`] when a reference to the template
/// also carries inherited wrapper-context overlays.
#[cfg(test)]
pub(crate) fn collect_tir_slot_layout(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<TirSlotLayout, InfrastructureFailure> {
    collect_tir_slot_layout_with_context(store, template_id, TemplateViewContext::default())
}

/// Collects the slot layout of a published template under one exact reference context.
///
/// WHAT: additionally records the named targets of wrapper sets inherited by the
///       reference's fixed-child overlay contexts.
/// WHY: a composite `$before`/`$after` value can carry nested component decorators
///      whose named slots exist only in the value reference's wrapper-context
///      overlay, not in the referenced template's structural root. Those decorators
///      replay the receiving application's named content, so the enclosing
///      application must be able to address them.
///
/// An independently planned template yields an empty layout: its own plan owns every
/// occurrence reachable through that reference, so an enclosing application cannot
/// address any of them and must not treat the reference as a receiver.
pub(crate) fn collect_tir_slot_layout_with_context(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
    context: TemplateViewContext,
) -> Result<TirSlotLayout, InfrastructureFailure> {
    let Some(template) = store.get_template(template_id) else {
        return Err(layout_error(format!(
            "TIR slot layout: template {template_id} was not present in the store."
        )));
    };

    if template_is_independently_planned(template) {
        return Ok(TirSlotLayout::default());
    }

    increment_ast_counter(AstCounter::TirSlotSchemaWalks);
    collect_layout_from_root(store, template.root, context)
}

/// Collects the slot layout of a node tree.
///
/// A bare node tree has no reference identity, so this form carries no inherited
/// wrapper context. Nested child references still contribute their own contexts.
/// A child reference to a template that already owns a runtime slot plan is a
/// leaf: that plan resolved every occurrence under it, so none belongs to this
/// tree's layout.
pub(crate) fn collect_tir_slot_layout_from_root(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
) -> Result<TirSlotLayout, InfrastructureFailure> {
    increment_ast_counter(AstCounter::TirSlotSchemaWalks);
    collect_layout_from_root(store, root_node_id, TemplateViewContext::default())
}

/// Schema-only convenience over the shared layout collection.
#[cfg(test)]
pub(crate) fn collect_tir_slot_schema(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<TirSlotSchema, InfrastructureFailure> {
    Ok(collect_tir_slot_layout(store, template_id)?.schema)
}

/// Schema-only convenience over the context-aware layout collection.
pub(crate) fn collect_tir_slot_schema_with_context(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
    context: TemplateViewContext,
) -> Result<TirSlotSchema, InfrastructureFailure> {
    Ok(collect_tir_slot_layout_with_context(store, template_id, context)?.schema)
}

fn collect_layout_from_root(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
    reference_context: TemplateViewContext,
) -> Result<TirSlotLayout, InfrastructureFailure> {
    let mut schema = TirSlotSchema::default();
    let mut placeholders = Vec::new();
    let mut visiting_nodes = FxHashSet::default();
    let mut visiting_templates = FxHashSet::default();
    let mut visited_wrappers = FxHashSet::default();

    collect_reference_context_named_slots(
        store,
        reference_context,
        &mut schema,
        &mut visited_wrappers,
    )?;

    collect_from_node(
        store,
        root_node_id,
        &mut schema,
        &mut placeholders,
        &mut visiting_nodes,
        &mut visiting_templates,
        &mut visited_wrappers,
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
    visited_wrappers: &mut FxHashSet<TemplateWrapperReference>,
) -> Result<(), InfrastructureFailure> {
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
                    visited_wrappers,
                )?;
            }
            Ok(())
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            schema.record_key(&placeholder.key);

            // A wrapper set applied at this placeholder is part of the slot's receiving
            // interface. Its named slots are addressable by the enclosing application and
            // every wrapper occurrence replays the routed named content; its default and
            // positional slots stay owned by the wrapper's own loose fill.
            collect_wrapper_named_slots(
                store,
                placeholder.child_wrapper_set,
                schema,
                visited_wrappers,
            )?;
            collect_wrapper_named_slots(
                store,
                placeholder.applied_child_wrapper_set,
                schema,
                visited_wrappers,
            )?;

            placeholders.push(TirSlotPlaceholderRef {
                occurrence_id: placeholder.occurrence_id,
                key: placeholder.key.clone(),
                default: placeholder.default,
                child_wrapper_set: placeholder.child_wrapper_set,
                applied_child_wrapper_set: placeholder.applied_child_wrapper_set,
                skip_parent_child_wrappers: placeholder.skip_parent_child_wrappers,
                span: node.span,
            });
            Ok(())
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            // An independently planned child is a completed application: its plan
            // resolved its sites and filled every child-edge overlay under this
            // reference with its own markers. Descending would advertise names no
            // enclosing fill may deliver and collect foreign occurrences behind the
            // plan boundary, so the edge stays opaque.
            let Some(child_template) = store.get_template(reference.root) else {
                visiting_nodes.remove(&node_id);
                return Err(layout_error(format!(
                    "TIR slot layout referenced missing child template {}.",
                    reference.root
                )));
            };

            if template_is_independently_planned(child_template) {
                visiting_nodes.remove(&node_id);
                return Ok(());
            }

            // A fixed child occurrence keeps inherited decorators in its reference
            // context rather than in the child's structural root. Those decorators
            // receive the enclosing application's named content, so their named
            // occurrences are part of this tree's receiving interface.
            collect_reference_context_named_slots(
                store,
                reference.context,
                schema,
                visited_wrappers,
            )?;
            collect_from_child_template(
                store,
                reference.root,
                child_template,
                schema,
                placeholders,
                visiting_templates,
                visited_wrappers,
            )
        }

        TemplateIrNodeKind::Conditional { body, .. } => collect_from_node(
            store,
            *body,
            schema,
            placeholders,
            visiting_nodes,
            visiting_templates,
            visited_wrappers,
        ),

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
                visited_wrappers,
            )?;

            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                collect_from_node(
                    store,
                    *aggregate_wrapper_id,
                    schema,
                    placeholders,
                    visiting_nodes,
                    visiting_templates,
                    visited_wrappers,
                )?;
            }

            Ok(())
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::InsertContribution { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => Ok(()),
        // A retained position is a leaf for slot layout: it holds no placeholders yet and keeps
        // its authored document position until completion installs its value.
        TemplateIrNodeKind::RetainedNumericIntake(_) => Ok(()),
    };

    visiting_nodes.remove(&node_id);
    result
}

fn collect_from_child_template(
    store: &TemplateIrStore,
    child_template_id: TemplateIrId,
    child_template: &TemplateIr,
    schema: &mut TirSlotSchema,
    placeholders: &mut Vec<TirSlotPlaceholderRef>,
    visiting_templates: &mut FxHashSet<TemplateIrId>,
    visited_wrappers: &mut FxHashSet<TemplateWrapperReference>,
) -> Result<(), InfrastructureFailure> {
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
        visited_wrappers,
    );
    visiting_templates.remove(&child_template_id);
    result
}

fn layout_error(message: impl Into<Box<str>>) -> InfrastructureFailure {
    InfrastructureFailure::internal(CompilerStage::AstConstruction, message)
}

/// Records the named slot targets declared by a wrapper set attached to a placeholder.
///
/// WHAT: for every wrapper reference in the set, adds the named slots reachable from
///       that reference's exact tree and inherited overlay contexts, including named
///       slots of wrappers attached to its own placeholders.
/// WHY: routing accepts a name only when some receiving occurrence can consume it.
///      The wrapper application created for this placeholder's contributions is that
///      occurrence, so excluding its names would reject an insert the composition can
///      deliver. Default and positional slots are deliberately excluded: the wrapper's
///      loose fill already supplies them and the enclosing application cannot address
///      them without duplicating child content.
fn collect_wrapper_named_slots(
    store: &TemplateIrStore,
    wrapper_set_id: Option<TemplateWrapperSetId>,
    schema: &mut TirSlotSchema,
    visited_wrappers: &mut FxHashSet<TemplateWrapperReference>,
) -> Result<(), InfrastructureFailure> {
    let Some(wrapper_set_id) = wrapper_set_id else {
        return Ok(());
    };
    let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
        layout_error(format!(
            "TIR slot layout found a missing wrapper set {wrapper_set_id}."
        ))
    })?;

    for wrapper in &wrapper_set.wrappers {
        collect_wrapper_reference_named_slots(store, *wrapper, schema, visited_wrappers)?;
    }

    Ok(())
}

/// Records the named slots reachable through one exact wrapper reference.
///
/// The reference's root, phase and context form the visited identity. Two references
/// to the same template under different inherited contexts can expose different named
/// occurrences, so a template-only identity would silently drop the second context.
fn collect_wrapper_reference_named_slots(
    store: &TemplateIrStore,
    reference: TemplateWrapperReference,
    schema: &mut TirSlotSchema,
    visited_wrappers: &mut FxHashSet<TemplateWrapperReference>,
) -> Result<(), InfrastructureFailure> {
    // A wrapper may contain another application of itself. Names are already known after the
    // first exact reference, so re-entry is a repeat rather than a malformed tree.
    if !visited_wrappers.insert(reference) {
        return Ok(());
    }

    let Some(wrapper) = store.get_template(reference.root) else {
        return Err(layout_error(format!(
            "TIR slot layout found wrapper template {} missing from the store.",
            reference.root
        )));
    };

    // A wrapper reference that already owns a runtime plan is a completed
    // application, not a compile-time wrapper value. Its named occurrences and
    // child-edge overlays belong to that plan, so an enclosing fill cannot reach
    // them.
    if template_is_independently_planned(wrapper) {
        return Ok(());
    }

    collect_reference_context_named_slots(store, reference.context, schema, visited_wrappers)?;

    let mut visiting_nodes = FxHashSet::default();
    collect_named_slots_from_node(
        store,
        wrapper.root,
        schema,
        &mut visiting_nodes,
        visited_wrappers,
    )
}

/// Records named slots of wrapper sets inherited at a reference's fixed-child occurrences.
///
/// WHAT: reads the reference's wrapper-context overlay, records that an applicable
///       inherited set exists, and adds the named schema of every inherited set that
///       still applies at its occurrence.
/// WHY: nested `$before`/`$after` values keep their decorators in these overlays.
///      The enclosing named fill resolves into the same sets, so their named
///      occurrences are addressable even though no structural node in the referenced
///      template declares them. Only named targets are collected: the fixed child owns
///      the inherited wrappers' default and positional insertion points. The recorded
///      presence lets runtime slot planning skip marker construction and context
///      fills when no occurrence can consume them.
fn collect_reference_context_named_slots(
    store: &TemplateIrStore,
    context: TemplateViewContext,
    schema: &mut TirSlotSchema,
    visited_wrappers: &mut FxHashSet<TemplateWrapperReference>,
) -> Result<(), InfrastructureFailure> {
    let Some(overlay_id) = context.wrapper_context else {
        return Ok(());
    };
    let overlay = store.wrapper_context_overlay(overlay_id).ok_or_else(|| {
        layout_error(format!(
            "TIR slot layout found a missing wrapper-context overlay {overlay_id:?}."
        ))
    })?;

    for (_, wrapper_context) in &overlay.contexts {
        // `$fresh` suppresses the inherited set at this occurrence, so its names are
        // not addressable through this reference even if a stale set id is present.
        if wrapper_context.skip_parent_child_wrappers {
            continue;
        }

        // An applicable inherited set means this reference has decorator state that
        // only an overlay fill can resolve. Record the fact even when the set's
        // wrappers declare no names: a named contribution can still be replayed by a
        // nested wrapper in the same reference tree, and the planner cannot inspect
        // that tree again after handoff.
        if wrapper_context.inherited_wrapper_set.is_some() {
            schema.has_inherited_wrapper_contexts = true;
        }

        collect_wrapper_named_slots(
            store,
            wrapper_context.inherited_wrapper_set,
            schema,
            visited_wrappers,
        )?;
    }

    Ok(())
}

fn collect_named_slots_from_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    schema: &mut TirSlotSchema,
    visiting_nodes: &mut FxHashSet<TemplateIrNodeId>,
    visited_wrappers: &mut FxHashSet<TemplateWrapperReference>,
) -> Result<(), InfrastructureFailure> {
    if !visiting_nodes.insert(node_id) {
        return Err(layout_error(format!(
            "TIR slot layout encountered a node cycle at {node_id:?} while collecting wrapper named slots."
        )));
    }

    let Some(node) = store.get_node(node_id) else {
        visiting_nodes.remove(&node_id);
        return Err(layout_error(format!(
            "TIR slot layout requested missing node {node_id} while collecting wrapper named slots."
        )));
    };

    let result = match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                collect_named_slots_from_node(
                    store,
                    *child_id,
                    schema,
                    visiting_nodes,
                    visited_wrappers,
                )?;
            }
            Ok(())
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            if let SlotKey::Named(name) = &placeholder.key {
                schema.named_slots.insert(*name);
            }
            collect_wrapper_named_slots(
                store,
                placeholder.child_wrapper_set,
                schema,
                visited_wrappers,
            )?;
            collect_wrapper_named_slots(
                store,
                placeholder.applied_child_wrapper_set,
                schema,
                visited_wrappers,
            )
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            collect_wrapper_reference_named_slots(
                store,
                TemplateWrapperReference::new(reference.root, reference.phase, reference.context),
                schema,
                visited_wrappers,
            )
        }

        TemplateIrNodeKind::Conditional { body, .. } => {
            collect_named_slots_from_node(store, *body, schema, visiting_nodes, visited_wrappers)
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_named_slots_from_node(store, *body, schema, visiting_nodes, visited_wrappers)?;
            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                collect_named_slots_from_node(
                    store,
                    *aggregate_wrapper_id,
                    schema,
                    visiting_nodes,
                    visited_wrappers,
                )?;
            }
            Ok(())
        }

        _ => Ok(()),
    };

    visiting_nodes.remove(&node_id);
    result
}
