//! Shared helpers for TIR-native slot composition.
//!
//! Owns wrapper-application orchestration and the slot-routing diagnostics used
//! by routing and expansion.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template_slots::{
    materialize_tir_native_runtime_slot_plan, tir_contributions_need_runtime,
};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::tir::slot_layout::TirSlotLayout;
use crate::compiler_frontend::ast::templates::tir::{
    DerivedTemplateMetadata, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrStore,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidTemplateSlotReason};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

type SlotCompositionResult<T> = Result<T, TemplateError>;

/// Builds a template infrastructure failure without rendering it into the source-diagnostic lane.
pub(super) fn internal_compiler_error(message: &str) -> TemplateError {
    CompilerError::compiler_error(message).into()
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
            internal_compiler_error("TIR slot routing: template ID was not present in the store.")
        })
}

/// Returns the direct children of a node, or a single-element list for
/// non-sequence roots.
pub(super) fn children_of_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> SlotCompositionResult<Vec<TemplateIrNodeId>> {
    let Some(node) = store.get_node(node_id) else {
        return Err(internal_compiler_error(
            "TIR slot routing: node ID was not present in the store.",
        ));
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
#[cfg(test)]
pub(super) fn location_for_template(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> SlotCompositionResult<SourceLocation> {
    store
        .get_template(template_id)
        .map(|template| template.location.to_owned())
        .ok_or_else(|| {
            internal_compiler_error(
                "TIR slot routing: template ID was not present in the store while reading its location.",
            )
        })
}

/// Builds a composed wrapper template entry from an expanded root.
///
/// WHAT: creates a new `TemplateIr` entry that reuses the wrapper's style, kind
///       and location, then recomputes the summary from the expanded root.
/// WHY: the expansion is non-destructive, so the original wrapper template is
///      preserved and a new entry represents the filled result.
pub(super) fn build_composed_wrapper_template(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    expanded_root: TemplateIrNodeId,
) -> SlotCompositionResult<TemplateIrId> {
    Ok(store.push_structurally_derived_template(
        wrapper_template_id,
        expanded_root,
        DerivedTemplateMetadata::preserve_source(),
    )?)
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
        internal_compiler_error(
            "TIR head-chain composition: original root node ID was not present in the store.",
        )
    })?;

    Ok(store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: resolved_children,
        },
        original_root_node.location.to_owned(),
    )))
}

/// Routes fill nodes against a carried layout, then expands or plans the wrapper.
pub(super) fn compose_wrapper_application(
    store: &mut TemplateIrStore,
    wrapper_reference: TemplateWrapperReference,
    layout: &TirSlotLayout,
    fill_nodes: Vec<TemplateIrNodeId>,
    fill_location: SourceLocation,
    string_table: &StringTable,
    allow_runtime_plans: bool,
) -> SlotCompositionResult<TemplateTirChildReference> {
    let routed = super::contributions::route_tir_fill_nodes_against_schema(
        store,
        &layout.schema,
        &fill_nodes,
        &fill_location,
        string_table,
    )?;

    let needs_runtime = allow_runtime_plans
        && tir_contributions_need_runtime(&layout.schema, &routed, string_table, store)?;
    let composed_template_id = if needs_runtime {
        materialize_tir_native_runtime_slot_plan(
            store,
            wrapper_reference.root,
            &layout.schema,
            &routed,
            string_table,
            &fill_location,
        )?
    } else {
        let expanded_root = super::schema::expand_tir_slot_placeholders_into(
            store,
            wrapper_reference.root,
            &routed,
            string_table,
        )?;
        build_composed_wrapper_template(store, wrapper_reference.root, expanded_root)?
    };

    Ok(wrapper_reference.into_composed_child_reference(composed_template_id))
}
