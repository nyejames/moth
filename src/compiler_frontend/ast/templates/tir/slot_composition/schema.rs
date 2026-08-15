//! TIR slot schema extraction and placeholder expansion.
//!
//! WHAT: discovers declared `$slot` targets from TIR nodes, collects slot
//!       placeholders in document order, and expands those placeholders with
//!       routed contributions. This module owns TIR-native schema discovery and
//!       wrapper expansion for those routed contributions.
//!
//! WHY: TIR-native slot composition discovers wrapper slots and substitutes
//!      fill content from TIR nodes. Keeping schema discovery separate from
//!      contribution routing lets each phase stay focused.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::template_slots::TemplateSlotError;
use crate::compiler_frontend::ast::templates::tir::contribution_shape::{
    ContributionShape, classify_tir_contribution_node,
};
use crate::compiler_frontend::ast::templates::tir::node::TirSlotPlaceholder;
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_root;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrBranch, TemplateIrId, TemplateIrNode, TemplateIrNodeId,
    TemplateIrNodeKind, TemplateIrStore, TemplateWrapperSetId,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidTemplateSlotReason};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashSet;
use std::collections::BTreeSet;

use super::child_wrappers::wrap_tir_node_in_wrappers_into;
use super::contributions::RoutedTirSlotContributions;
use super::helpers::{SlotResolutionComposition, internal_compiler_error, tir_tree_has_slots};

/// Typed result for placeholder collection and slot expansion.
type SlotSchemaResult<T> = Result<T, TemplateError>;

/// Narrow error boundary for the schema walk itself.
///
/// WHAT: keeps source slot diagnostics and malformed TIR authority in their
///       owning typed lanes while the single schema traversal serves both
///       composition and handoff.
/// WHY: composition preserves the same source/infrastructure distinction through
///      `TemplateError`, while HIR handoff can move the original `CompilerError`
///      without reverse-converting a `DiagnosticPayload`.
#[derive(Debug)]
pub(crate) enum SlotSchemaError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

type SlotSchemaCollectionResult<T> = Result<T, SlotSchemaError>;

impl From<CompilerDiagnostic> for SlotSchemaError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        SlotSchemaError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<Box<CompilerDiagnostic>> for SlotSchemaError {
    fn from(diagnostic: Box<CompilerDiagnostic>) -> Self {
        SlotSchemaError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for SlotSchemaError {
    fn from(error: CompilerError) -> Self {
        SlotSchemaError::Infrastructure(Box::new(error))
    }
}

impl From<SlotSchemaError> for TemplateError {
    fn from(error: SlotSchemaError) -> Self {
        match error {
            SlotSchemaError::Diagnostic(diagnostic) => TemplateError::Diagnostic(diagnostic),
            SlotSchemaError::Infrastructure(error) => TemplateError::Infrastructure(error),
        }
    }
}

impl From<SlotSchemaError> for TemplateSlotError {
    fn from(error: SlotSchemaError) -> Self {
        match error {
            SlotSchemaError::Diagnostic(diagnostic) => TemplateSlotError::Diagnostic(diagnostic),
            SlotSchemaError::Infrastructure(error) => TemplateSlotError::Infrastructure(error),
        }
    }
}

/// Keeps the internal handoff lane lossless for malformed authority and
/// preserves a source diagnostic's reason and primary location if one reaches
/// this boundary unexpectedly.
impl From<SlotSchemaError> for CompilerError {
    fn from(error: SlotSchemaError) -> Self {
        match error {
            SlotSchemaError::Infrastructure(error) => *error,
            SlotSchemaError::Diagnostic(diagnostic) => {
                let diagnostic = *diagnostic;
                CompilerError::new(
                    format!(
                        "TIR HIR handoff slot schema validation produced a source diagnostic: kind={:?}, payload={:?}",
                        diagnostic.kind, diagnostic.payload
                    ),
                    diagnostic.primary_location,
                    ErrorType::Compiler,
                )
            }
        }
    }
}

fn schema_infrastructure_error(message: impl Into<String>) -> SlotSchemaError {
    SlotSchemaError::Infrastructure(Box::new(CompilerError::compiler_error(message)))
}
// ------------------------
//  Slot schema
// ------------------------

/// Slot schema discovered from TIR nodes.
///
/// WHAT: records which slot keys (default, named, positional) a wrapper
///       template declares.
/// WHY: slot composition needs one schema type derived from authoritative TIR
///      nodes so discovery and routing share the same slot keys.
#[derive(Debug, Default, Clone)]
pub(crate) struct TirSlotSchema {
    pub(crate) has_default_slot: bool,
    pub(crate) named_slots: FxHashSet<StringId>,
    pub(crate) positional_slots: BTreeSet<usize>,
}

impl TirSlotSchema {
    /// Returns true when the wrapper declares at least one slot target.
    pub(crate) fn has_any_slots(&self) -> bool {
        self.has_default_slot || !self.named_slots.is_empty() || !self.positional_slots.is_empty()
    }

    /// Returns true when `target` matches a slot declared by this schema.
    pub(crate) fn accepts_target(&self, target: &SlotKey) -> bool {
        match target {
            SlotKey::Default => self.has_default_slot,
            SlotKey::Named(name) => self.named_slots.contains(name),
            SlotKey::Positional(index) => self.positional_slots.contains(index),
        }
    }

    /// Returns the loose-fill target selected by the wrapper slot schema.
    ///
    /// WHAT: selects the smallest positional slot when one exists, otherwise
    ///       the default slot. Named-only schemas and slot-less schemas return
    ///       no target because inherited loose content cannot address them.
    /// WHY: wrapper composition and runtime handoff must agree on one
    ///      positional-before-default policy regardless of where a slot appears
    ///      in the structural TIR tree.
    pub(crate) fn loose_fill_target_key(&self) -> Option<SlotKey> {
        self.positional_slots
            .iter()
            .next()
            .copied()
            .map(SlotKey::Positional)
            .or_else(|| self.has_default_slot.then_some(SlotKey::Default))
    }

    /// Returns positional slot indexes in ascending numeric order.
    pub(crate) fn ordered_positional_slots(&self) -> Vec<usize> {
        self.positional_slots.iter().copied().collect()
    }

    /// Returns named slot keys sorted by resolved source spelling.
    pub(crate) fn ordered_named_slots(&self, string_table: &StringTable) -> Vec<StringId> {
        let mut names = self.named_slots.iter().copied().collect::<Vec<_>>();

        names.sort_by(|left, right| {
            string_table
                .resolve(*left)
                .cmp(string_table.resolve(*right))
        });

        names
    }

    /// Returns the deterministic slot allocation order: default first, positional
    /// slots in numeric order, then named slots by resolved source spelling.
    ///
    /// WHY: both the focused slot-routing tests and the TIR-native runtime slot
    ///      plan builder need the same deterministic ordering so source-plan
    ///      allocation stays stable regardless of which composition path produced
    ///      the runtime plan.
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

    /// Records a single slot placeholder's key in this schema.
    ///
    /// WHAT: updates `has_default_slot`, `named_slots`, or `positional_slots`
    ///       according to the slot key. Returns a diagnostic if a second default
    ///       slot is declared.
    /// WHY: both TIR-native and atom-based schema discovery need the same slot
    ///      key recording behavior, so the validation rule is centralized on the
    ///      shared schema type.
    pub(crate) fn record_key(
        &mut self,
        key: &SlotKey,
        error_location: SourceLocation,
    ) -> SlotSchemaCollectionResult<()> {
        match key {
            SlotKey::Default => {
                if self.has_default_slot {
                    return Err(SlotSchemaError::Diagnostic(Box::new(
                        CompilerDiagnostic::invalid_template_slot(
                            InvalidTemplateSlotReason::MultipleDefaultSlots,
                            None,
                            error_location,
                        ),
                    )));
                }
                self.has_default_slot = true;
            }

            SlotKey::Named(name) => {
                self.named_slots.insert(*name);
            }

            SlotKey::Positional(index) => {
                self.positional_slots.insert(*index);
            }
        }

        Ok(())
    }
}

/// Discovers all declared slot targets from a TIR template's root node.
///
/// WHAT: walks the TIR tree starting at `template_id`'s root node, recording
///       every `Slot` node's key into the schema. Recurses into `ChildTemplate`,
///       `BranchChain`, and `Loop` nodes to find nested slot declarations.
/// WHY: TIR-native slot composition needs to know which slots a wrapper
///      declares before it can route contributions.
pub(crate) fn collect_tir_slot_schema(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> SlotSchemaCollectionResult<TirSlotSchema> {
    increment_ast_counter(AstCounter::TirSlotSchemaWalks);
    let Some(template) = store.get_template(template_id) else {
        return Err(schema_infrastructure_error(
            "TIR slot schema extraction: template ID was not present in the store.",
        ));
    };

    let mut schema = TirSlotSchema::default();
    collect_tir_slot_schema_from_node(
        store,
        template.root,
        &mut schema,
        template.location.to_owned(),
    )?;

    Ok(schema)
}

/// Recursively traverses TIR nodes to identify all `$slot` declarations.
///
/// WHAT: dispatches on `TemplateIrNodeKind` and records slot keys found in
///       `Slot` nodes. Nested structures that can contain further slot
///       declarations are walked recursively.
/// WHY: wrapper templates may declare slots inside branches, loops, or nested
///      child templates, so a single root walk must reach every reachable node.
fn collect_tir_slot_schema_from_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    schema: &mut TirSlotSchema,
    error_location: SourceLocation,
) -> SlotSchemaCollectionResult<()> {
    let Some(node) = store.get_node(node_id) else {
        return Err(schema_infrastructure_error(
            "TIR slot schema extraction: node ID was not present in the store.",
        ));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                collect_tir_slot_schema_from_node(
                    store,
                    *child_id,
                    schema,
                    error_location.to_owned(),
                )?;
            }
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            schema.record_key(&placeholder.key, error_location)?;
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template_id = reference.root;
            let Some(child_template) = store.get_template(template_id) else {
                return Err(schema_infrastructure_error(
                    "TIR slot schema extraction: child template ID was not present in the store.",
                ));
            };

            collect_tir_slot_schema_from_node(
                store,
                child_template.root,
                schema,
                error_location.to_owned(),
            )?;
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                collect_tir_slot_schema_from_node(
                    store,
                    branch.body,
                    schema,
                    error_location.to_owned(),
                )?;
            }

            if let Some(fallback_id) = fallback {
                collect_tir_slot_schema_from_node(
                    store,
                    *fallback_id,
                    schema,
                    error_location.to_owned(),
                )?;
            }
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_tir_slot_schema_from_node(store, *body, schema, error_location.to_owned())?;

            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                collect_tir_slot_schema_from_node(
                    store,
                    *aggregate_wrapper_id,
                    schema,
                    error_location.to_owned(),
                )?;
            }
        }

        // InsertContribution nodes carry routed content, not slot declarations.
        TemplateIrNodeKind::InsertContribution { .. } => {}

        // Text, dynamic expressions, and aggregate-output markers cannot
        // declare new slot targets.
        TemplateIrNodeKind::Text { .. } => {}
        TemplateIrNodeKind::DynamicExpression { .. } => {}
        TemplateIrNodeKind::AggregateOutput => {}

        // Loop control and runtime slot sites are not slot declarations.
        TemplateIrNodeKind::LoopControl { .. } => {}
        TemplateIrNodeKind::RuntimeSlotSite { .. } => {}
    }

    Ok(())
}

/// Collects every `TirSlotPlaceholder` from a TIR tree in
/// document/materialization order.
///
/// WHAT: walks the TIR tree rooted at `root_node_id` and appends each `Slot`
///       node's placeholder to the result vector, preserving the order in which
///       a depth-first document traversal encounters them. Recurses into
///       `ChildTemplate`, `BranchChain`, and `Loop` bodies so nested slot
///       declarations are included. Ignores `RuntimeSlotSite` leaves, which are
///       already-resolved sites rather than unresolved placeholders.
/// WHY: runtime slot-site planning needs slot placeholders in the exact order
///      the final materialization pass will encounter them, so the
///      cursor-based site assignment in `materialize_slot_placeholder` matches
///      each placeholder to the correct pre-planned site. TIR remains the sole
///      authority for slot-placeholder discovery.
pub(crate) fn collect_tir_slot_placeholders_in_order(
    store: &TemplateIrStore,
    root_node_id: TemplateIrNodeId,
) -> SlotSchemaResult<Vec<TirSlotPlaceholder>> {
    increment_ast_counter(AstCounter::TirSlotSchemaWalks);
    let mut placeholders = Vec::new();
    collect_tir_slot_placeholders_from_node(store, root_node_id, &mut placeholders)?;
    Ok(placeholders)
}

/// Recursive helper for `collect_tir_slot_placeholders_in_order`.
///
/// WHAT: dispatches on `TemplateIrNodeKind` and appends `Slot` placeholders to
///       the accumulator in traversal order. Nested structures that can contain
///       further slot placeholders are walked depth-first.
/// WHY: keeping the recursion separate from the public entry point lets the
///      caller own the result vector while the helper stays focused on
///      per-node dispatch.
fn collect_tir_slot_placeholders_from_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    placeholders: &mut Vec<TirSlotPlaceholder>,
) -> SlotSchemaResult<()> {
    let Some(node) = store.get_node(node_id) else {
        return Err(internal_compiler_error(
            "TIR slot placeholder collection: node ID was not present in the store.",
        ));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                collect_tir_slot_placeholders_from_node(store, *child_id, placeholders)?;
            }
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            placeholders.push(placeholder.clone());
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            // Nested child templates may declare their own slots. Walking their
            // root naturally collects those placeholders in document order.
            let template_id = reference.root;
            let Some(child_template) = store.get_template(template_id) else {
                return Err(internal_compiler_error(
                    "TIR slot placeholder collection: child template ID was not present in the store.",
                ));
            };

            collect_tir_slot_placeholders_from_node(store, child_template.root, placeholders)?;
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                collect_tir_slot_placeholders_from_node(store, branch.body, placeholders)?;
            }

            if let Some(fallback_id) = fallback {
                collect_tir_slot_placeholders_from_node(store, *fallback_id, placeholders)?;
            }
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_tir_slot_placeholders_from_node(store, *body, placeholders)?;

            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                collect_tir_slot_placeholders_from_node(
                    store,
                    *aggregate_wrapper_id,
                    placeholders,
                )?;
            }
        }

        // InsertContribution nodes carry routed content, not unresolved placeholders.
        TemplateIrNodeKind::InsertContribution { .. } => {}

        // Text, dynamic expressions, and aggregate-output markers are leaves
        // without slot placeholders.
        TemplateIrNodeKind::Text { .. } => {}
        TemplateIrNodeKind::DynamicExpression { .. } => {}
        TemplateIrNodeKind::AggregateOutput => {}

        // Loop control signals and already-resolved runtime slot sites do not
        // carry unresolved slot placeholders.
        TemplateIrNodeKind::LoopControl { .. } => {}
        TemplateIrNodeKind::RuntimeSlotSite { .. } => {}
    }

    Ok(())
}

/// Expands slot placeholders in a wrapper template's TIR tree with routed
/// contributions.
///
/// WHAT: walks the wrapper template's TIR nodes, replaces each `Slot` node
///       with the contributions routed to that slot key, and recurses into
///       nested child templates that have their own slot definitions.
/// WHY: this completes the TIR slot-composition pipeline: schema extraction →
///      routing → expansion.
///
/// The expansion is non-destructive: it builds new nodes in the store but does
/// not modify existing nodes. The original wrapper template's TIR tree is
/// preserved so callers can decide whether to replace the wrapper's root or
/// create a new template entry.
#[cfg(test)]
pub(crate) fn expand_tir_slot_placeholders(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    routed_contributions: &RoutedTirSlotContributions,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let mut slot_compositions = Vec::new();
    expand_tir_slot_placeholders_into(
        store,
        wrapper_template_id,
        routed_contributions,
        string_table,
        &mut slot_compositions,
    )
}

pub(crate) fn expand_tir_slot_placeholders_into(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    routed_contributions: &RoutedTirSlotContributions,
    string_table: &StringTable,
    slot_compositions: &mut Vec<SlotResolutionComposition>,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let Some(template) = store.get_template(wrapper_template_id) else {
        return Err(internal_compiler_error(
            "TIR slot expansion: wrapper template ID was not present in the store.",
        ));
    };

    // Fast path: if the wrapper tree contains no Slot nodes, the original root
    // is already the correct result. This avoids allocating a fresh sequence
    // that is structurally identical to the existing root.
    if !tir_tree_has_slots(store, template.root)? {
        return Ok(template.root);
    }

    expand_tir_slot_placeholders_from_node(
        store,
        template.root,
        routed_contributions,
        string_table,
        slot_compositions,
    )
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
    routed_contributions: &RoutedTirSlotContributions,
    string_table: &StringTable,
    slot_compositions: &mut Vec<SlotResolutionComposition>,
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
                    slot_compositions,
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
            let contribution_nodes = routed_contributions
                .contributions
                .nodes_for_slot(&placeholder.key);

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
                        slot_compositions,
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
            let Some(child_template) = store.get_template(child_template_id).cloned() else {
                return Err(internal_compiler_error(
                    "TIR slot expansion: child template ID was not present in the store.",
                ));
            };

            let child_schema = collect_tir_slot_schema(store, child_template_id)?;

            if !child_schema.has_any_slots() {
                // The child template has no slot declarations of its own, so it
                // cannot receive any of the routed contributions. Leave the
                // reference unchanged because it has no slot composition work.
                return Ok(node_id);
            }

            let expanded_child_root = expand_tir_slot_placeholders_from_node(
                store,
                child_template.root,
                routed_contributions,
                string_table,
                slot_compositions,
            )?;

            let expanded_child_template = TemplateIr::new(
                expanded_child_root,
                child_template.style.to_owned(),
                child_template.kind.to_owned(),
                child_template.summary.to_owned(),
                child_template.location.to_owned(),
            );

            let expanded_child_template_id = store.push_template(expanded_child_template);

            let occurrence_id = store.next_child_template_occurrence_id();
            let expanded_reference = TemplateTirChildReference::new(
                expanded_child_template_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
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
                    slot_compositions,
                )?;

                if expanded_body_id != branch.body {
                    any_branch_changed = true;
                    expanded_branches.push(
                        TemplateIrBranch::new(
                            branch.selector.to_owned(),
                            expanded_body_id,
                            branch.location.to_owned(),
                        )
                        .with_selector_site_id(branch.selector_site_id),
                    );
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
                        slot_compositions,
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
                slot_compositions,
            )?;

            let mut any_part_changed = expanded_body_id != *body;

            let expanded_aggregate_wrapper = match aggregate_wrapper {
                Some(aggregate_wrapper_id) => {
                    let expanded_aggregate_wrapper_id = expand_tir_slot_placeholders_from_node(
                        store,
                        *aggregate_wrapper_id,
                        routed_contributions,
                        string_table,
                        slot_compositions,
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
        TemplateIrNodeKind::RuntimeSlotSite { .. } => Ok(node_id),
    }
}

/// Applies a module-local `$children(..)` wrapper set to a single TIR node.
///
/// WHAT: resolves the wrapper set into module-local wrapper template IDs and
///       delegates to `wrap_tir_node_in_wrappers_into`, which composes each
///       slot-bearing wrapper around the supplied node and prepends each
///       slot-less wrapper before it.
/// WHY: slot expansion needs to apply the inherited and applied wrapper sets
///      stored on a `TirSlotPlaceholder` while keeping the same
///      slot-resolution-overlay bookkeeping that direct child-wrapper
///      application uses.
fn apply_tir_wrapper_set_to_node(
    store: &mut TemplateIrStore,
    node_id: TemplateIrNodeId,
    wrapper_set_id: TemplateWrapperSetId,
    string_table: &StringTable,
    slot_compositions: &mut Vec<SlotResolutionComposition>,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
        internal_compiler_error("TIR slot expansion: placeholder referenced a missing wrapper set.")
    })?;

    let wrapper_template_ids: Vec<TemplateIrId> = wrapper_set
        .wrappers
        .iter()
        .map(|template_ref| template_ref.root)
        .collect();

    wrap_tir_node_in_wrappers_into(
        store,
        node_id,
        &wrapper_template_ids,
        string_table,
        slot_compositions,
    )
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
    slot_compositions: &mut Vec<SlotResolutionComposition>,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let mut current_node_id = node_id;

    let shape = classify_tir_contribution_node(store, current_node_id)?;
    if let Some(wrapper_set_id) = placeholder.child_wrapper_set
        && shape.is_child_template_contribution
        && !shape.skips_parent_child_wrappers
    {
        current_node_id = apply_tir_wrapper_set_to_node(
            store,
            current_node_id,
            wrapper_set_id,
            string_table,
            slot_compositions,
        )?;
    }

    let post_shape = classify_tir_contribution_node(store, current_node_id)?;
    if let Some(wrapper_set_id) = placeholder.applied_child_wrapper_set
        && !placeholder.skip_parent_child_wrappers
        && post_shape.is_child_template_contribution
    {
        current_node_id = apply_tir_wrapper_set_to_node(
            store,
            current_node_id,
            wrapper_set_id,
            string_table,
            slot_compositions,
        )?;
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
                .control_flow_node_id_in_subtree(template.root)
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

        if !shape.skips_parent_child_wrappers {
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

            let mut copied = template;
            copied.conditional_child_wrapper_set = Some(merged_wrapper_set_id);
            copied.summary.wrapper_count =
                required_wrapper_set_count(store, merged_wrapper_set_id)?;
            let copied_id = store.push_template(copied);

            let new_reference =
                TemplateTirChildReference::new(copied_id, reference.phase, reference.context);
            (new_reference, node.location.to_owned())
        }

        TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. } => {
            let wrapper_count = required_wrapper_set_count(store, wrapper_set_id)?;
            let mut summary = summarize_existing_root(store, node_id);
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
