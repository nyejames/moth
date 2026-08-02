//! Shared helpers for TIR-native slot composition.
//!
//! WHAT: holds the small cross-cutting types, store/diagnostic helpers, and
//!       wrapper-template builders that the schema, contribution, overlay,
//!       head-chain, and child-wrapper submodules all need. Keeping them in one
//!       place means each of the responsibility-focused files can stay small and
//!       does not duplicate sequence-building or diagnostic-construction logic.
//!
//! WHY: several composition passes need to wrap a node list into a fill
//!      template, rebuild a root sequence, or report the same slot-routing
//!      diagnostics. A single shared owner keeps those operations consistent
//!      without introducing a new broad utility layer.

use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::copy_state::TirCopyState;
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::summary::summarize_existing_nodes;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrBuilder, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrStore,
    copy_tir_subtree_with_active_slot_plan,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::compiler_errors::compiler_error_to_diagnostic;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidTemplateSlotReason};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Boxed diagnostic result for the shared slot-composition helper family.
type SlotCompositionResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// Module-local wrapper/fill identity for one structural slot composition.
///
/// WHAT: records the effective wrapper view identity and the temporary fill
///       template created while resolving its slots. The wrapper keeps its
///       phase and view context while both roots remain module-local.
/// WHY: overlay allocation runs after the structural store borrow is released.
///      Carrying the wrapper/fill pair across that boundary preserves the
///      wrapper context needed by later store-local composition work.
pub(super) struct SlotResolutionComposition {
    pub(super) wrapper_reference: TemplateTirChildReference,
    pub(super) fill_reference: TemplateIrId,
}

impl SlotResolutionComposition {
    pub(super) fn new(
        wrapper_reference: TemplateTirChildReference,
        fill_reference: TemplateIrId,
    ) -> Self {
        Self {
            wrapper_reference,
            fill_reference,
        }
    }
}

/// Result of TIR composition carrying both the structurally composed root and
/// an optional slot-resolution view context.
///
/// WHAT: `root` is the structurally composed node (or the original root when no
///       composition applied). `slot_context` is `Some` when the
///       composition resolved at least one slot-bearing wrapper/fill pair, and
///       `None` when no slots were resolved.
/// WHY: production composition call sites need both the composed root for
///      structural expansion and the value context for `TemplateTirReference`
///      threading. A named struct avoids vague tuple returns and keeps the
///      overlay context explicit at the stage boundary.
#[derive(Debug)]
pub(crate) struct ComposedTirRoot {
    /// The structurally composed root node, or the original root when no
    /// composition applied.
    pub(crate) root: TemplateIrNodeId,
    /// Non-empty slot-resolution view context when the composition resolved at
    /// least one slot-bearing wrapper, or `None` when no slots were resolved.
    pub(crate) slot_context: Option<TemplateViewContext>,
}

/// Wraps an internal compiler error message as a `CompilerDiagnostic`.
///
/// WHAT: converts a `CompilerError` into the user-facing diagnostic type used
///       by this module's error boundary.
/// WHY: routing helpers return `CompilerDiagnostic`, but internal invariant
///      failures are still represented as `CompilerError` first.
pub(super) fn internal_compiler_error(message: &str) -> CompilerDiagnostic {
    compiler_error_to_diagnostic(&CompilerError::compiler_error(message))
}

/// Builds the diagnostic for an `$insert(...)` helper that targets a slot the
/// wrapper does not declare.
///
/// WHAT: builds the shared unknown-slot-target diagnostic using the
///       `InvalidTemplateSlotReason` variants.
/// WHY: TIR-native slot routing must preserve the established user-facing
///      diagnostic semantics at its own error boundary.
pub(super) fn unknown_slot_target_error(
    target: &SlotKey,
    location: SourceLocation,
) -> CompilerDiagnostic {
    match target {
        SlotKey::Default => CompilerDiagnostic::invalid_template_slot(
            InvalidTemplateSlotReason::InsertCannotTargetDefaultSlot,
            None,
            location,
        ),
        SlotKey::Named(name) => CompilerDiagnostic::invalid_template_slot(
            InvalidTemplateSlotReason::InsertTargetsUnknownNamedSlot,
            Some(*name),
            location,
        ),
        SlotKey::Positional(_) => CompilerDiagnostic::invalid_template_slot(
            InvalidTemplateSlotReason::InsertTargetsUnknownPositionalSlot,
            None,
            location,
        ),
    }
}

/// Builds the diagnostic for loose content when the wrapper has no default or
/// positional slots.
pub(super) fn loose_content_without_default_slot_error(
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_template_slot(
        InvalidTemplateSlotReason::LooseContentWithoutDefaultSlot,
        None,
        location,
    )
}

/// Builds the diagnostic for loose content that exceeds the wrapper's
/// positional slots without a default slot to absorb the remainder.
pub(super) fn extra_loose_content_without_default_slot_error(
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_template_slot(
        InvalidTemplateSlotReason::ExtraLooseContentWithoutDefaultSlot,
        None,
        location,
    )
}

/// Returns the root node ID for a template, or an internal compiler error.
pub(super) fn root_node_id_for_template(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> SlotCompositionResult<TemplateIrNodeId> {
    store
        .get_template(template_id)
        .map(|template| template.root)
        .ok_or_else(|| {
            Box::new(internal_compiler_error(
                "TIR slot routing: template ID was not present in the store.",
            ))
        })
}

/// Returns the direct children of a node, or a single-element list for
/// non-sequence roots.
pub(super) fn children_of_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> SlotCompositionResult<Vec<TemplateIrNodeId>> {
    let Some(node) = store.get_node(node_id) else {
        return Err(Box::new(internal_compiler_error(
            "TIR slot routing: node ID was not present in the store.",
        )));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => Ok(children.to_owned()),
        _ => Ok(vec![node_id]),
    }
}

/// Returns the direct `$insert(...)` helpers carried by a nested stored-insert
/// template, when its root contains only insert-contribution nodes.
///
/// WHAT: identifies the transparent carrier created by a body reference such as
///       `[stored_title]` and returns the helper IDs with their authored node
///       locations.
/// WHY: the immediate parent owns slot routing. Keeping this structural query
///      beside the TIR slot-composition helpers prevents the parser and the
///      post-parse escaped-insert validation from inventing separate shapes.
pub(crate) fn stored_insert_contribution_templates(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> Result<Option<Vec<(TemplateIrId, SourceLocation)>>, CompilerError> {
    let template = store.get_template(template_id).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR slot composition: stored insert carrier referenced a missing template.",
        )
    })?;
    let root = store.get_node(template.root).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR slot composition: stored insert carrier referenced a missing root node.",
        )
    })?;

    let TemplateIrNodeKind::Sequence { children } = &root.kind else {
        return Ok(None);
    };
    if children.is_empty() {
        return Ok(None);
    }

    let mut contributions = Vec::with_capacity(children.len());
    for child_id in children {
        let child = store.get_node(*child_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR slot composition: stored insert carrier referenced a missing child node.",
            )
        })?;
        let TemplateIrNodeKind::InsertContribution { template } = child.kind else {
            return Ok(None);
        };
        contributions.push((template, child.location.clone()));
    }

    Ok(Some(contributions))
}

/// Returns a template's source location, or an internal error when the template
/// authority is missing from the store.
pub(super) fn location_for_template(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> SlotCompositionResult<SourceLocation> {
    store
        .get_template(template_id)
        .map(|template| template.location.to_owned())
        .ok_or_else(|| {
            Box::new(internal_compiler_error(
                "TIR slot routing: template ID was not present in the store while reading its location.",
            ))
        })
}

/// Returns true if the TIR tree rooted at `node_id` contains at least one
/// `Slot` node.
///
/// WHAT: walks the tree recursively, stopping at the first slot placeholder.
/// WHY: this powers the no-slots fast path in `expand_tir_slot_placeholders`
///      without depending on `TemplateIrSummary` flags, which test helpers may
///      leave at default values.
pub(super) fn tir_tree_has_slots(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> SlotCompositionResult<bool> {
    let Some(node) = store.get_node(node_id) else {
        return Err(Box::new(internal_compiler_error(
            "TIR slot expansion: node ID was not present in the store while checking for slots.",
        )));
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                if tir_tree_has_slots(store, *child_id)? {
                    return Ok(true);
                }
            }

            Ok(false)
        }

        TemplateIrNodeKind::Slot { .. } => Ok(true),

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template_id = reference.root;
            let Some(child_template) = store.get_template(template_id) else {
                return Err(Box::new(internal_compiler_error(
                    "TIR slot expansion: child template ID was not present in the store while checking for slots.",
                )));
            };

            tir_tree_has_slots(store, child_template.root)
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            for branch in branches {
                if tir_tree_has_slots(store, branch.body)? {
                    return Ok(true);
                }
            }

            if let Some(fallback_id) = fallback
                && tir_tree_has_slots(store, *fallback_id)?
            {
                return Ok(true);
            }

            Ok(false)
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            if tir_tree_has_slots(store, *body)? {
                return Ok(true);
            }

            if let Some(aggregate_wrapper_id) = aggregate_wrapper
                && tir_tree_has_slots(store, *aggregate_wrapper_id)?
            {
                return Ok(true);
            }

            Ok(false)
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::InsertContribution { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. } => Ok(false),
    }
}

/// Builds a fill/source template from a list of node IDs.
///
/// WHAT: creates a new `String` template whose root is a sequence containing the
///       given node IDs, located at the supplied source node location.
/// WHY: both head-chain composition and slot-overlay source materialization need
///      to wrap a node-ID bucket into a stable template handle. Keeping one
///      builder avoids duplicating the sequence-plus-finish template shape.
pub(super) fn build_tir_fill_template(
    store: &mut TemplateIrStore,
    fill_node_ids: Vec<TemplateIrNodeId>,
    location_source_node_id: TemplateIrNodeId,
) -> SlotCompositionResult<TemplateIrId> {
    let location = store
        .get_node(location_source_node_id)
        .map(|node| node.location.to_owned())
        .ok_or_else(|| {
            Box::new(internal_compiler_error(
                "TIR fill/source template construction: location source node ID was not present in the store.",
            ))
        })?;

    let summary = summarize_existing_nodes(store, &fill_node_ids);
    let mut builder = TemplateIrBuilder::new(store);
    let root = builder.push_sequence_node(fill_node_ids, location.to_owned());

    Ok(builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        summary,
        location,
    ))
}

/// Builds a composed wrapper template entry from an expanded root.
///
/// WHAT: creates a new `TemplateIr` entry that reuses the wrapper's style, kind,
///       summary, and location but replaces the root with the expanded tree.
/// WHY: the expansion is non-destructive, so the original wrapper template is
///      preserved and a new entry represents the filled result.
pub(super) fn build_composed_wrapper_template(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    expanded_root: TemplateIrNodeId,
) -> SlotCompositionResult<TemplateIrId> {
    let wrapper_template = store.get_template(wrapper_template_id).ok_or_else(|| {
        Box::new(internal_compiler_error(
            "TIR head-chain composition: wrapper template ID was not present in the store.",
        ))
    })?;

    let mut summary = wrapper_template.summary.to_owned();

    // If all slot placeholders were filled, the composed tree no longer behaves
    // as a wrapper receiver. Clear the slot flags so later composition passes
    // and later fold preparation checks see the actual tree shape. Other
    // summary counts remain conservative (they may underestimate fill content),
    // which is safe for capacity planning.
    if !tir_tree_has_slots(store, expanded_root)? {
        summary.has_slots = false;
        summary.slot_count = 0;
    }

    let composed_template = TemplateIr::new(
        expanded_root,
        wrapper_template.style.to_owned(),
        wrapper_template.kind.to_owned(),
        summary,
        wrapper_template.location.to_owned(),
    );

    Ok(store.push_template(composed_template))
}

/// Deep-copies a wrapper template so each composition application owns fresh
/// `SlotOccurrenceId`s.
///
/// WHAT: copies the wrapper's root subtree and pushes a new `TemplateIr` entry
///       that reuses the original style, kind, location, conditional wrapper set,
///       and runtime slot plan. `Slot` placeholders in the copied tree receive
///       new occurrence IDs from the store, so composing the same wrapper around
///       multiple body children no longer produces colliding IDs in merged
///       slot-resolution overlays.
/// WHY: `SlotOccurrenceId`s are store-global identities. Reusing the same wrapper
///      template for many body children (for example, a `<td>` wrapper around
///      every table cell) would otherwise place identical occurrence IDs into the
///      parent's view context, making the overlay merge fail.
pub(super) fn copy_tir_wrapper_template_with_fresh_slot_occurrence_ids(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
) -> SlotCompositionResult<TemplateIrId> {
    let wrapper_template = store.get_template(wrapper_template_id).ok_or_else(|| {
        Box::new(internal_compiler_error(
            "TIR slot composition: wrapper template ID was not present in the store.",
        ))
    })?;

    let wrapper_root = wrapper_template.root;
    let wrapper_style = wrapper_template.style.to_owned();
    let wrapper_kind = wrapper_template.kind.to_owned();
    let wrapper_location = wrapper_template.location.to_owned();
    let wrapper_set = wrapper_template.conditional_child_wrapper_set;
    let runtime_slot_plan = wrapper_template.runtime_slot_plan;

    let mut copy_state = TirCopyState::new();
    let copied_root =
        copy_tir_subtree_with_active_slot_plan(wrapper_root, None, store, &mut copy_state)
            .map_err(|error| error.into_diagnostic())?;

    let mut copied_template = TemplateIr::new(
        copied_root,
        wrapper_style,
        wrapper_kind,
        copy_state.summary,
        wrapper_location,
    );
    copied_template.conditional_child_wrapper_set = wrapper_set;
    copied_template.runtime_slot_plan = runtime_slot_plan;

    Ok(store.push_template(copied_template))
}

/// Rebuilds the root sequence with resolved children.
///
/// WHAT: pushes a new `Sequence` node that mirrors the original root's location
///       but contains the resolved child node IDs.
/// WHY: composition must produce a new TIR tree without mutating the original
///      root node.
pub(super) fn rebuild_root_sequence(
    store: &mut TemplateIrStore,
    original_root_node_id: TemplateIrNodeId,
    resolved_children: Vec<TemplateIrNodeId>,
) -> SlotCompositionResult<TemplateIrNodeId> {
    let original_root_node = store.get_node(original_root_node_id).ok_or_else(|| {
        Box::new(internal_compiler_error(
            "TIR head-chain composition: original root node ID was not present in the store.",
        ))
    })?;

    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: resolved_children,
        },
        original_root_node.location.to_owned(),
    )))
}
