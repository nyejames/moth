//! Structural expansion of TIR slot placeholders.
//!
//! Slot schema and occurrence facts come from `tir/slot_layout.rs`. This
//! module rebuilds a wrapper tree with routed contributions spliced in place
//! of `$slot` nodes.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::contribution_shape::classify_tir_contribution_node;
use crate::compiler_frontend::ast::templates::tir::node::TirSlotPlaceholder;
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_root;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::wrapper_sets::merge_wrapper_sets;
use crate::compiler_frontend::ast::templates::tir::{
    DerivedCount, DerivedTemplateMetadata, TemplateIr, TemplateIrId, TemplateIrNode,
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplateWrapperSetId,
    conditional_wrapper_set_for_control_flow, tir_node_is_control_flow_root,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use rustc_hash::{FxHashMap, FxHashSet};

use super::child_wrappers::wrap_tir_node_in_wrappers_into;
use super::contributions::TirSlotContributions;
use super::helpers::{build_composed_wrapper_template, internal_infrastructure_failure};

type SlotSchemaResult<T> = Result<T, TemplateError>;

/// Which slot occurrences one expansion pass resolves.
///
/// WHAT: `All` replaces every routed occurrence, `NamedOnly` leaves default and
///       positional placeholders untouched.
/// WHY: a wrapper applied to a control-flow contribution must be composed before its
///       child output exists. Filling only the named slots keeps the wrapper's own loose
///       insertion point unresolved for the later conditional application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotExpansionScope {
    All,
    NamedOnly,
}

/// Resolves each named-filled wrapper set at most once per receiving application.
///
/// WHAT: memoises `source wrapper set -> named-filled wrapper set` for the lifetime of
///       one expansion of a receiver's wrapper tree.
/// WHY: sibling contributions under one occurrence, and repeated occurrences of the same
///      conditional wrapper set elsewhere in the receiver, replay the same set with the
///      same named content. A repeat fill walks the wrapper trees, publishes fresh derived
///      templates and scans the store's wrapper sets, so the results cannot deduplicate on
///      identity and every replay would retain its own equivalent copy. The cache stays
///      scoped to the application, so a set resolved against different named content is
///      never reused.
#[derive(Default)]
struct NamedFilledWrapperSetCache {
    resolved: FxHashMap<TemplateWrapperSetId, TemplateWrapperSetId>,
}

impl NamedFilledWrapperSetCache {
    fn resolve(
        &mut self,
        store: &mut TemplateIrStore,
        expressions: &AstExpressionBuilder,
        const_values: &ConstValueStore,
        wrapper_set_id: TemplateWrapperSetId,
        named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
        string_table: &StringTable,
    ) -> SlotSchemaResult<TemplateWrapperSetId> {
        // Without named content the set is already final; leaving the map empty keeps
        // the common case allocation-free.
        if named_fill.is_empty() {
            return Ok(wrapper_set_id);
        }

        if let Some(resolved_set_id) = self.resolved.get(&wrapper_set_id) {
            return Ok(*resolved_set_id);
        }

        let resolved_set_id = fill_named_slots_in_wrapper_set(
            store,
            expressions,
            const_values,
            wrapper_set_id,
            named_fill,
            string_table,
        )?;
        self.resolved.insert(wrapper_set_id, resolved_set_id);
        Ok(resolved_set_id)
    }
}

pub(crate) fn expand_tir_slot_placeholders_into(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_template_id: TemplateIrId,
    routed_contributions: &TirSlotContributions,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    expand_wrapper_template_into(
        store,
        expressions,
        const_values,
        wrapper_template_id,
        routed_contributions,
        &routed_contributions.named_nodes,
        SlotExpansionScope::All,
        string_table,
    )
}

/// Composes the routed named contributions into a wrapper's named slots.
///
/// Returns the derived wrapper template when at least one named occurrence changed, so
/// conditional wrapper application can carry the content without a second fill list.
pub(crate) fn fill_named_slot_placeholders_into(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_template_id: TemplateIrId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<Option<TemplateIrId>> {
    let Some(template) = store.get_template(wrapper_template_id) else {
        return Err(internal_infrastructure_failure(
            "TIR slot expansion: wrapper template ID was not present in the store.",
        ));
    };
    let original_root = template.root;
    let no_routed_contributions = TirSlotContributions::default();
    let mut named_filled_wrapper_sets = NamedFilledWrapperSetCache::default();

    let expanded_root = expand_tir_slot_placeholders_from_node(
        store,
        expressions,
        const_values,
        original_root,
        &no_routed_contributions,
        named_fill,
        &mut named_filled_wrapper_sets,
        SlotExpansionScope::NamedOnly,
        string_table,
    )?;

    if expanded_root == original_root {
        return Ok(None);
    }

    Ok(Some(build_composed_wrapper_template(
        store,
        wrapper_template_id,
        expanded_root,
    )?))
}

fn expand_wrapper_template_into(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_template_id: TemplateIrId,
    routed_contributions: &TirSlotContributions,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    scope: SlotExpansionScope,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let Some(template) = store.get_template(wrapper_template_id) else {
        return Err(internal_infrastructure_failure(
            "TIR slot expansion: wrapper template ID was not present in the store.",
        ));
    };
    let root = template.root;
    let mut named_filled_wrapper_sets = NamedFilledWrapperSetCache::default();

    expand_tir_slot_placeholders_from_node(
        store,
        expressions,
        const_values,
        root,
        routed_contributions,
        named_fill,
        &mut named_filled_wrapper_sets,
        scope,
        string_table,
    )
}

/// Recursively walks TIR nodes and produces a new TIR tree with slots expanded.
///
/// WHAT: dispatches on `TemplateIrNodeKind`, replacing `Slot` nodes with a
///       `Sequence` containing the routed contribution node IDs, and recursing
///       into structures that can contain further slot placeholders.
/// WHY: wrapper templates may declare slots inside sequences, conditionals, loops,
///      or nested child templates, so a single root walk must reach every
///      reachable slot and rebuild only the parts of the tree that changed.
#[allow(
    clippy::too_many_arguments,
    reason = "the walk needs the store, expression and constant owners, the routes, the named fill and its application cache, the scope and the string table"
)]
fn expand_tir_slot_placeholders_from_node(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    node_id: TemplateIrNodeId,
    routed_contributions: &TirSlotContributions,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    named_filled_wrapper_sets: &mut NamedFilledWrapperSetCache,
    scope: SlotExpansionScope,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let Some(node) = store.get_node(node_id).cloned() else {
        return Err(internal_infrastructure_failure(
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
                    expressions,
                    const_values,
                    *child_id,
                    routed_contributions,
                    named_fill,
                    named_filled_wrapper_sets,
                    scope,
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
            // Named-only expansion leaves the wrapper's own loose insertion point and
            // positional occurrences untouched for the later wrapper application.
            let contribution_nodes = match scope {
                SlotExpansionScope::All => routed_contributions.nodes_for_slot(&placeholder.key),
                SlotExpansionScope::NamedOnly => {
                    let SlotKey::Named(name) = &placeholder.key else {
                        return Ok(node_id);
                    };
                    let Some(nodes) = named_fill.get(name) else {
                        return Ok(node_id);
                    };
                    if nodes.is_empty() {
                        return Ok(node_id);
                    }
                    nodes.as_slice()
                }
            };

            // Only child-template contributions receive ordinary external
            // wrappers. Control-flow contributions carry the same applicable
            // wrapper set as conditional metadata so skipped work emits no
            // wrapper.
            //
            // Sibling control-flow contributions under this occurrence share one
            // applicable wrapper set and one named fill, and the same set can recur at
            // other occurrences of this receiving application. Resolving it anew would
            // publish an equivalent derived set on every replay.
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
                        let wrapper_set_id = named_filled_wrapper_sets.resolve(
                            store,
                            expressions,
                            const_values,
                            wrapper_set_id,
                            named_fill,
                            string_table,
                        )?;
                        attach_conditional_wrapper_set(store, *node_id, wrapper_set_id)?
                    } else {
                        *node_id
                    }
                } else {
                    apply_tir_wrapper_sets_to_contribution(
                        store,
                        expressions,
                        const_values,
                        *node_id,
                        placeholder,
                        named_fill,
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

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let child_template_id = reference.root;
            let Some(child_root) = store
                .get_template(child_template_id)
                .map(|template| template.root)
            else {
                return Err(internal_infrastructure_failure(
                    "TIR slot expansion: child template ID was not present in the store.",
                ));
            };

            let expanded_child_root = expand_tir_slot_placeholders_from_node(
                store,
                expressions,
                const_values,
                child_root,
                routed_contributions,
                named_fill,
                named_filled_wrapper_sets,
                scope,
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

            // The derived template is a structural version of this same occurrence, so it
            // keeps the occurrence identity. Retained wrapper-context overlays are keyed by
            // occurrence; a fresh key would silently drop the occurrence's inherited
            // `$children(..)` context for every consumer of the reference's view context.
            let expanded_reference = reference.with_root(expanded_child_template_id);
            Ok(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: expanded_reference,
                    occurrence_id: *occurrence_id,
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
                expressions,
                const_values,
                *body,
                routed_contributions,
                named_fill,
                named_filled_wrapper_sets,
                scope,
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
                expressions,
                const_values,
                *body,
                routed_contributions,
                named_fill,
                named_filled_wrapper_sets,
                scope,
                string_table,
            )?;

            let mut any_part_changed = expanded_body_id != *body;

            let expanded_aggregate_wrapper = match aggregate_wrapper {
                Some(aggregate_wrapper_id) => {
                    let expanded_aggregate_wrapper_id = expand_tir_slot_placeholders_from_node(
                        store,
                        expressions,
                        const_values,
                        *aggregate_wrapper_id,
                        routed_contributions,
                        named_fill,
                        named_filled_wrapper_sets,
                        scope,
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

        // A retained position is a leaf for slot expansion: its value does not exist yet, so it
        // carries no slot placeholders and stays at its authored position.
        TemplateIrNodeKind::RetainedNumericIntake(_) => Ok(node_id),
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
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    node_id: TemplateIrNodeId,
    wrapper_set_id: TemplateWrapperSetId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let wrapper_set = store.get_wrapper_set(wrapper_set_id).ok_or_else(|| {
        internal_infrastructure_failure(
            "TIR slot expansion: placeholder referenced a missing wrapper set.",
        )
    })?;

    let wrapper_references = wrapper_set.wrappers.to_vec();

    wrap_tir_node_in_wrappers_into(
        store,
        expressions,
        const_values,
        node_id,
        &wrapper_references,
        named_fill,
        string_table,
    )
}

/// Composes routed named content into every wrapper in a set that declares it.
///
/// WHAT: derives a version of each affected wrapper template whose named placeholders
///       and wrapper-local direct-child overlays are already filled, then returns the
///       replacement wrapper set.
/// WHY: wrapper sets store only wrapper references, so deferred applications (conditional
///       contribution wrappers and direct-child overlays) must carry the named content in
///       the wrapper templates themselves. Filling the named occurrences before attachment
///       keeps the deferred loose child insertion working while still replaying the
///       receiving application's named content. A wrapper may itself own fixed direct
///       children, whose nested `$children(..)` sets live in the wrapper reference's
///       overlay context rather than its structural tree; those sets are filled through
///       the same immediate-receiver overlay path so their named occurrences resolve too.
pub(crate) fn fill_named_slots_in_wrapper_set(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_set_id: TemplateWrapperSetId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateWrapperSetId> {
    let mut visiting_wrapper_sets = FxHashSet::default();
    fill_named_slots_in_wrapper_set_inner(
        store,
        expressions,
        const_values,
        wrapper_set_id,
        named_fill,
        string_table,
        &mut visiting_wrapper_sets,
    )
}

/// Resolves one wrapper set while guarding against recursive `$children(..)` graphs.
///
/// WHAT: fills each wrapper's structural named occurrences and its wrapper-local
///       direct-child overlay, sharing one in-progress set across the recursion.
/// WHY: a wrapper context may inherit the very set being resolved (directly or through
///       another wrapper), so the guard terminates that cycle instead of deriving
///       templates forever. The guard is scoped to one top-level fill; sibling
///       occurrences still resolve against the same deterministic named content.
fn fill_named_slots_in_wrapper_set_inner(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_set_id: TemplateWrapperSetId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
    visiting_wrapper_sets: &mut FxHashSet<TemplateWrapperSetId>,
) -> SlotSchemaResult<TemplateWrapperSetId> {
    if named_fill.is_empty() {
        return Ok(wrapper_set_id);
    }

    // A set already under resolution keeps its current wrappers; refilling it from its
    // own descendant context would not add names the cycle did not already expose.
    if !visiting_wrapper_sets.insert(wrapper_set_id) {
        return Ok(wrapper_set_id);
    }

    let wrapper_references = store
        .get_wrapper_set(wrapper_set_id)
        .ok_or_else(|| {
            internal_infrastructure_failure(
                "TIR slot expansion: conditional wrapper set was not present in the store.",
            )
        })?
        .wrappers
        .to_vec();

    let mut filled_references = Vec::with_capacity(wrapper_references.len());
    let mut changed = false;
    for wrapper_reference in wrapper_references {
        let filled_root = fill_wrapper_template_named_content(
            store,
            expressions,
            const_values,
            wrapper_reference.root,
            named_fill,
            string_table,
            visiting_wrapper_sets,
        )?;
        let filled_context = fill_direct_child_wrapper_names_inner(
            store,
            expressions,
            const_values,
            wrapper_reference.context,
            named_fill,
            string_table,
            visiting_wrapper_sets,
        )?;

        if filled_root.is_none() && filled_context == wrapper_reference.context {
            filled_references.push(wrapper_reference);
            continue;
        }

        changed = true;
        filled_references.push(TemplateWrapperReference::new(
            filled_root.unwrap_or(wrapper_reference.root),
            wrapper_reference.phase,
            filled_context,
        ));
    }

    visiting_wrapper_sets.remove(&wrapper_set_id);

    if !changed {
        return Ok(wrapper_set_id);
    }

    Ok(store.push_or_reuse_wrapper_set(&filled_references))
}

/// Fills routed named content into one wrapper template and its slot-owned wrapper sets.
///
/// WHAT: composes structural named occurrences, then fills the wrapper sets stored on the
///       template's slot placeholders, returning the derived template when either changed.
/// WHY: a wrapper can be applied structurally by slot expansion or by runtime handoff
///      injection, and the handoff store is read-only. Composing every named occurrence a
///      stored slot wrapper can render before the wrapper is attached to a set keeps both
///      application paths consistent and avoids an unresolved named slot at handoff.
pub(crate) fn fill_named_slots_in_wrapper_template(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    template_id: TemplateIrId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<Option<TemplateIrId>> {
    if named_fill.is_empty() {
        return Ok(None);
    }

    let mut visiting_wrapper_sets = FxHashSet::default();
    fill_wrapper_template_named_content(
        store,
        expressions,
        const_values,
        template_id,
        named_fill,
        string_table,
        &mut visiting_wrapper_sets,
    )
}

/// Shared implementation of [`fill_named_slots_in_wrapper_template`] with a caller-owned
/// recursive wrapper-set guard.
fn fill_wrapper_template_named_content(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    template_id: TemplateIrId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
    visiting_wrapper_sets: &mut FxHashSet<TemplateWrapperSetId>,
) -> SlotSchemaResult<Option<TemplateIrId>> {
    let named_template = fill_named_slot_placeholders_into(
        store,
        expressions,
        const_values,
        template_id,
        named_fill,
        string_table,
    )?;
    let candidate_template_id = named_template.unwrap_or(template_id);
    let Some(candidate_template) = store.get_template(candidate_template_id) else {
        return Err(internal_infrastructure_failure(
            "TIR slot expansion: wrapper template ID was not present while filling slot wrapper sets.",
        ));
    };
    let candidate_root = candidate_template.root;

    let Some(filled_root) = fill_slot_wrapper_sets_from_node(
        store,
        expressions,
        const_values,
        candidate_root,
        named_fill,
        string_table,
        visiting_wrapper_sets,
    )?
    else {
        return Ok(named_template);
    };

    Ok(Some(store.push_structurally_derived_template(
        candidate_template_id,
        filled_root,
        DerivedTemplateMetadata::preserve_source(),
    )?))
}

/// Recursively rewrites the slot placeholders of one template tree.
///
/// WHAT: rebuilds sequences, child-template boundaries, conditionals and loops only when a
///       descendant slot changed; other node kinds are structural owners of other state.
/// WHY: the same root walk that schema collection uses must reach every slot occurrence whose
///      wrappers can be applied by a later expansion or handoff injection. Derived child
///      templates keep their occurrence identity so retained overlays still resolve.
fn fill_slot_wrapper_sets_from_node(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    node_id: TemplateIrNodeId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
    visiting_wrapper_sets: &mut FxHashSet<TemplateWrapperSetId>,
) -> SlotSchemaResult<Option<TemplateIrNodeId>> {
    let Some(node) = store.get_node(node_id).cloned() else {
        return Err(internal_infrastructure_failure(
            "TIR slot expansion: node ID was not present while filling slot wrapper sets.",
        ));
    };
    let span = node.span;

    match node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            let mut filled_children = Vec::with_capacity(children.len());
            let mut any_child_changed = false;
            for child_id in children {
                match fill_slot_wrapper_sets_from_node(
                    store,
                    expressions,
                    const_values,
                    child_id,
                    named_fill,
                    string_table,
                    visiting_wrapper_sets,
                )? {
                    Some(filled_child_id) => {
                        any_child_changed = true;
                        filled_children.push(filled_child_id);
                    }
                    None => filled_children.push(child_id),
                }
            }

            if !any_child_changed {
                return Ok(None);
            }

            Ok(Some(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: filled_children,
                },
                span,
            ))))
        }

        TemplateIrNodeKind::Slot { placeholder } => {
            let filled_child = fill_slot_wrapper_set(
                store,
                expressions,
                const_values,
                placeholder.child_wrapper_set,
                named_fill,
                string_table,
                visiting_wrapper_sets,
            )?;
            // A placeholder that skips parent wrappers never applies its inherited set, so
            // filling that set would create content the slot contract says is unused.
            let filled_applied = if placeholder.skip_parent_child_wrappers {
                placeholder.applied_child_wrapper_set
            } else {
                fill_slot_wrapper_set(
                    store,
                    expressions,
                    const_values,
                    placeholder.applied_child_wrapper_set,
                    named_fill,
                    string_table,
                    visiting_wrapper_sets,
                )?
            };

            if filled_child == placeholder.child_wrapper_set
                && filled_applied == placeholder.applied_child_wrapper_set
            {
                return Ok(None);
            }

            let filled_placeholder = TirSlotPlaceholder::with_wrapper_sets(
                placeholder.key,
                placeholder.occurrence_id,
                filled_applied,
                filled_child,
                placeholder.skip_parent_child_wrappers,
            );
            Ok(Some(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Slot {
                    placeholder: filled_placeholder,
                },
                span,
            ))))
        }

        TemplateIrNodeKind::ChildTemplate {
            reference,
            occurrence_id,
        } => {
            let Some(child_template) = store.get_template(reference.root) else {
                return Err(internal_infrastructure_failure(
                    "TIR slot expansion: child template was not present while filling slot wrapper sets.",
                ));
            };
            let child_root = child_template.root;
            let Some(filled_child_root) = fill_slot_wrapper_sets_from_node(
                store,
                expressions,
                const_values,
                child_root,
                named_fill,
                string_table,
                visiting_wrapper_sets,
            )?
            else {
                return Ok(None);
            };

            let derived_id = store.push_structurally_derived_template(
                reference.root,
                filled_child_root,
                DerivedTemplateMetadata::preserve_source(),
            )?;
            Ok(Some(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: reference.with_root(derived_id),
                    occurrence_id,
                },
                span,
            ))))
        }

        TemplateIrNodeKind::Conditional {
            selector,
            selector_site_id,
            body,
        } => {
            let Some(filled_body) = fill_slot_wrapper_sets_from_node(
                store,
                expressions,
                const_values,
                body,
                named_fill,
                string_table,
                visiting_wrapper_sets,
            )?
            else {
                return Ok(None);
            };

            Ok(Some(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Conditional {
                    selector,
                    selector_site_id,
                    body: filled_body,
                },
                span,
            ))))
        }

        TemplateIrNodeKind::Loop {
            header,
            header_sites,
            body,
            aggregate_wrapper,
        } => {
            let filled_body = fill_slot_wrapper_sets_from_node(
                store,
                expressions,
                const_values,
                body,
                named_fill,
                string_table,
                visiting_wrapper_sets,
            )?;
            let filled_aggregate = match aggregate_wrapper {
                Some(wrapper_id) => fill_slot_wrapper_sets_from_node(
                    store,
                    expressions,
                    const_values,
                    wrapper_id,
                    named_fill,
                    string_table,
                    visiting_wrapper_sets,
                )?,
                None => None,
            };

            if filled_body.is_none() && filled_aggregate.is_none() {
                return Ok(None);
            }

            Ok(Some(store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::Loop {
                    header,
                    header_sites,
                    body: filled_body.unwrap_or(body),
                    aggregate_wrapper: filled_aggregate.or(aggregate_wrapper),
                },
                span,
            ))))
        }

        _ => Ok(None),
    }
}

/// Fills one optional slot wrapper set against the routed named content.
fn fill_slot_wrapper_set(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    wrapper_set_id: Option<TemplateWrapperSetId>,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
    visiting_wrapper_sets: &mut FxHashSet<TemplateWrapperSetId>,
) -> SlotSchemaResult<Option<TemplateWrapperSetId>> {
    let Some(wrapper_set_id) = wrapper_set_id else {
        return Ok(None);
    };

    Ok(Some(fill_named_slots_in_wrapper_set_inner(
        store,
        expressions,
        const_values,
        wrapper_set_id,
        named_fill,
        string_table,
        visiting_wrapper_sets,
    )?))
}

/// Resolves routed named content into a receiving application's direct-child wrapper overlay.
///
/// WHAT: rewrites every occurrence context that inherits a `$children(..)` wrapper set so the
///       set's wrappers carry the receiving application's named contributions at their declared
///       named occurrences, then returns the view context with the replacement overlay.
/// WHY: a receiver's fixed direct children keep their occurrence-keyed overlay context while
///      composition only expands slot placeholders. Filling the overlay's wrapper sets is the
///      only path that reaches a fixed child authored before the receiving slot, and rewriting
///      the overlay keeps the fix inside the receiver's own occurrence scope instead of
///      descending into independently owned child-template contexts.
pub(crate) fn fill_direct_child_wrapper_names_into(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    context: TemplateViewContext,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateViewContext> {
    let mut visiting_wrapper_sets = FxHashSet::default();
    fill_direct_child_wrapper_names_inner(
        store,
        expressions,
        const_values,
        context,
        named_fill,
        string_table,
        &mut visiting_wrapper_sets,
    )
}

/// Rewrites one overlay's inherited wrapper sets with the cycle-guarded fill.
fn fill_direct_child_wrapper_names_inner(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    context: TemplateViewContext,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
    visiting_wrapper_sets: &mut FxHashSet<TemplateWrapperSetId>,
) -> SlotSchemaResult<TemplateViewContext> {
    if named_fill.is_empty() {
        return Ok(context);
    }
    let Some(overlay_id) = context.wrapper_context else {
        return Ok(context);
    };
    let Some(overlay) = store.wrapper_context_overlay(overlay_id) else {
        return Err(internal_infrastructure_failure(
            "TIR slot expansion: wrapper-context overlay was not present in the store.",
        ));
    };
    // Wrapper filling mutates the store, so the occurrence contexts are detached before the
    // loop; overlays contain one entry per child occurrence and stay small.
    let source_contexts = overlay.contexts.clone();

    // Every occurrence of one receiving template shares the same wrapper set. Resolving each
    // distinct set once avoids duplicating derived wrapper templates per occurrence.
    let mut resolved_sets: FxHashMap<TemplateWrapperSetId, TemplateWrapperSetId> =
        FxHashMap::default();
    let mut filled_contexts = Vec::with_capacity(source_contexts.len());
    let mut changed = false;

    for (occurrence_id, mut wrapper_context) in source_contexts {
        if !wrapper_context.skip_parent_child_wrappers
            && let Some(wrapper_set_id) = wrapper_context.inherited_wrapper_set
        {
            let filled_set_id = match resolved_sets.get(&wrapper_set_id) {
                Some(filled_set_id) => *filled_set_id,
                None => {
                    let filled_set_id = fill_named_slots_in_wrapper_set_inner(
                        store,
                        expressions,
                        const_values,
                        wrapper_set_id,
                        named_fill,
                        string_table,
                        visiting_wrapper_sets,
                    )?;
                    resolved_sets.insert(wrapper_set_id, filled_set_id);
                    filled_set_id
                }
            };

            if filled_set_id != wrapper_set_id {
                changed = true;
                wrapper_context.inherited_wrapper_set = Some(filled_set_id);
            }
        }
        filled_contexts.push((occurrence_id, wrapper_context));
    }

    if !changed {
        return Ok(context);
    }

    let filled_overlay_id = store.allocate_wrapper_context_overlay(TirWrapperContextOverlay {
        contexts: filled_contexts,
    })?;

    Ok(context.merge(TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(filled_overlay_id),
    }))
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
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    node_id: TemplateIrNodeId,
    placeholder: &TirSlotPlaceholder,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let mut current_node_id = node_id;

    let shape = classify_tir_contribution_node(store, current_node_id)?;
    if let Some(wrapper_set_id) = placeholder.child_wrapper_set
        && shape.is_child_template_contribution()
        && !shape.skips_parent_child_wrappers()
    {
        current_node_id = apply_tir_wrapper_set_to_node(
            store,
            expressions,
            const_values,
            current_node_id,
            wrapper_set_id,
            named_fill,
            string_table,
        )?;
    }

    let post_shape = classify_tir_contribution_node(store, current_node_id)?;
    if let Some(wrapper_set_id) = placeholder.applied_child_wrapper_set
        && !placeholder.skip_parent_child_wrappers
        && post_shape.is_child_template_contribution()
    {
        current_node_id = apply_tir_wrapper_set_to_node(
            store,
            expressions,
            const_values,
            current_node_id,
            wrapper_set_id,
            named_fill,
            string_table,
        )?;
    }

    Ok(current_node_id)
}

/// Applies a slot occurrence's own `$children(..)` wrapper sets around a deferred fill.
///
/// WHAT: wraps the fill in `child_wrapper_set`, then in `applied_child_wrapper_set` unless
///       the placeholder skips parent wrappers.
/// WHY: runtime slot-site planning replaces the placeholder with a plan-qualified source
///      marker, so the node no longer carries the authored contribution's child shape.
///      The site planner only reaches this path for a direct child contribution that has
///      already passed the contribution's own `$fresh` check, which means the slot's child
///      wrappers apply exactly as structural expansion applies them. Named injections do
///      not pass through here: they keep their slot-routing identity instead of becoming
///      direct children.
pub(crate) fn apply_slot_wrapper_sets_to_deferred_fill(
    store: &mut TemplateIrStore,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    placeholder: &TirSlotPlaceholder,
    fill_root: TemplateIrNodeId,
    named_fill: &FxHashMap<StringId, Vec<TemplateIrNodeId>>,
    string_table: &StringTable,
) -> SlotSchemaResult<TemplateIrNodeId> {
    let mut current_node_id = fill_root;

    if let Some(wrapper_set_id) = placeholder.child_wrapper_set {
        current_node_id = apply_tir_wrapper_set_to_node(
            store,
            expressions,
            const_values,
            current_node_id,
            wrapper_set_id,
            named_fill,
            string_table,
        )?;
    }

    if let Some(wrapper_set_id) = placeholder.applied_child_wrapper_set
        && !placeholder.skip_parent_child_wrappers
    {
        current_node_id = apply_tir_wrapper_set_to_node(
            store,
            expressions,
            const_values,
            current_node_id,
            wrapper_set_id,
            named_fill,
            string_table,
        )?;
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
        internal_infrastructure_failure(
            "TIR slot expansion: control-flow node ID was not present in the store.",
        )
    })?;

    let (reference, span) = match &node.kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template_id = reference.root;
            let Some(template) = store.get_template(template_id).cloned() else {
                return Err(internal_infrastructure_failure(
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
        internal_infrastructure_failure(
            "TIR slot expansion: required wrapper set ID was not present in the store.",
        )
    })?;

    u32::try_from(wrapper_set.wrappers.len()).map_err(|_| {
        internal_infrastructure_failure(
            "TIR slot expansion: wrapper-set count exceeded the supported summary range.",
        )
    })
}
