//! TIR-native `$children(..)` wrapper application.
//!
//! WHAT: wraps direct body-origin child templates in inherited wrapper
//!       templates, resolving slot-bearing wrappers by routing the child as a
//!       single loose contribution and expanding the wrapper's slot
//!       placeholders.
//!
//! WHY: this owns TIR-native child-wrapper application. Keeping it separate from
//!      head-chain composition reflects the two distinct composition sites:
//!      head-chain receivers are opened by head-origin wrappers, while child
//!      wrappers are inherited from enclosing control-flow or render-unit
//!      contexts.
//!
//! Design constraint: production direct-child wrapper application now uses
//! wrapper-context overlays. The store-local structural helpers in this module
//! remain only where a later transform needs an owned wrapper tree, such as
//! control-flow aggregate wrappers. Direct-child filtering (slot-bearing
//! receivers, control-flow children and `$fresh` suppression) is owned by
//! `apply_inherited_child_wrappers_to_body_root` in `render_unit.rs`, which
//! calls `wrap_tir_node_in_wrappers` for non-control-flow children.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::TemplateTirPhase;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use super::helpers::{
    SlotResolutionComposition, build_composed_wrapper_template, build_tir_fill_template,
    copy_tir_wrapper_template_with_fresh_slot_occurrence_ids, internal_compiler_error,
};
use super::schema::expand_tir_slot_placeholders_into;

/// Typed result for child-wrapper application.
type ChildWrapperResult<T> = Result<T, TemplateError>;

/// Wraps a single direct child `ChildTemplate` node in all inherited wrappers.
///
/// WHAT: iterates the wrapper list forward (innermost-first), composing each
///       wrapper around the current wrapped child.
/// WHY: `TemplateWrapperSet::wrappers` is stored innermost-to-outermost, so
///      forward iteration applies the innermost wrapper directly around the
///      child and each later wrapper around the previous result, yielding the
///      final `outermost(innermost(child))` nesting the store contract requires.
pub(crate) fn wrap_tir_node_in_wrappers(
    store: &mut TemplateIrStore,
    child_node_id: TemplateIrNodeId,
    wrapper_template_ids: &[TemplateIrId],
    string_table: &StringTable,
) -> ChildWrapperResult<TemplateIrNodeId> {
    let mut slot_compositions = Vec::new();
    wrap_tir_node_in_wrappers_into(
        store,
        child_node_id,
        wrapper_template_ids,
        string_table,
        &mut slot_compositions,
    )
}

/// Internal wrapper application that also collects slot-bearing wrapper/fill
/// pairs into `slot_compositions` for later overlay allocation.
///
/// WHY: the slot-composition child-wrapper entry point needs the pairs to
///      allocate slot-resolution overlays. The public
///      `wrap_tir_node_in_wrappers` discards them because body-root application
///      only needs the resulting wrapped node.
pub(super) fn wrap_tir_node_in_wrappers_into(
    store: &mut TemplateIrStore,
    child_node_id: TemplateIrNodeId,
    wrapper_template_ids: &[TemplateIrId],
    string_table: &StringTable,
    slot_compositions: &mut Vec<SlotResolutionComposition>,
) -> ChildWrapperResult<TemplateIrNodeId> {
    let child_location = store
        .get_node(child_node_id)
        .map(|node| node.location.to_owned())
        .ok_or_else(|| {
            internal_compiler_error(
                "TIR child wrapper application: child node ID was not present in the store.",
            )
        })?;

    let mut current_child_node_id = child_node_id;

    for wrapper_template_id in wrapper_template_ids.iter() {
        let wrapper_has_slots = {
            let wrapper_template = store.get_template(*wrapper_template_id).ok_or_else(|| {
                internal_compiler_error(
                    "TIR child wrapper application: wrapper template ID was not present in the store.",
                )
            })?;

            wrapper_template.summary.has_slots || wrapper_template.summary.slot_count > 0
        };

        if wrapper_has_slots {
            // The current wrapped child is the fill content for this wrapper's
            // slots. Routing treats it as a single loose contribution chunk.
            let fill_template_id =
                build_tir_fill_template(store, vec![current_child_node_id], child_node_id)?;

            // Copy the wrapper template so this child gets its own fresh
            // SlotOccurrenceIds. Without the copy, applying the same wrapper to
            // multiple body children would place identical occurrence IDs into
            // the parent's merged slot-resolution overlay, causing overlay merge
            // to fail.
            let copied_wrapper_template_id =
                copy_tir_wrapper_template_with_fresh_slot_occurrence_ids(
                    store,
                    *wrapper_template_id,
                )?;

            let routed = super::contributions::route_tir_slot_contributions(
                store,
                copied_wrapper_template_id,
                fill_template_id,
                string_table,
            )?;

            let expanded_root = expand_tir_slot_placeholders_into(
                store,
                copied_wrapper_template_id,
                &routed,
                string_table,
                slot_compositions,
            )?;

            let composed_template_id =
                build_composed_wrapper_template(store, copied_wrapper_template_id, expanded_root)?;

            // Record the wrapper/fill pair so the slot-composition entry point
            // can allocate a slot-resolution overlay after the store borrow is
            // released. The fill template persists in the store, so the overlay
            // path can re-route against it without re-discovering the wrappers.
            let wrapper_reference = TemplateTirChildReference::new(
                copied_wrapper_template_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
            let fill_reference = fill_template_id;
            slot_compositions.push(SlotResolutionComposition::new(
                wrapper_reference,
                fill_reference,
            ));

            let occurrence_id = store.next_child_template_occurrence_id();
            let reference = TemplateTirChildReference::new(
                composed_template_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
            current_child_node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id,
                },
                child_location.to_owned(),
            ));
        } else {
            // A wrapper without slots simply prepends its own content before the
            // child. Build a combined template whose root sequence is the wrapper
            // followed by the already-wrapped child.
            let combined_template_id = build_tir_prepended_wrapper_template(
                store,
                *wrapper_template_id,
                current_child_node_id,
                child_location.to_owned(),
            )?;

            let occurrence_id = store.next_child_template_occurrence_id();
            let reference = TemplateTirChildReference::new(
                combined_template_id,
                TemplateTirPhase::Parsed,
                TemplateViewContext::default(),
            );
            current_child_node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id,
                },
                child_location.to_owned(),
            ));
        }
    }

    Ok(current_child_node_id)
}

/// Builds a template that prepends a slot-less wrapper before an existing child.
///
/// WHAT: creates a new `String` template whose root sequence contains a
///       `ChildTemplate` reference to the wrapper followed by the current child.
/// WHY: slot-less wrappers put their content immediately before the wrapped
///      child in the composed TIR sequence.
fn build_tir_prepended_wrapper_template(
    store: &mut TemplateIrStore,
    wrapper_template_id: TemplateIrId,
    child_node_id: TemplateIrNodeId,
    child_location: SourceLocation,
) -> ChildWrapperResult<TemplateIrId> {
    let wrapper_location = store
        .get_template(wrapper_template_id)
        .map(|wrapper_template| wrapper_template.location.to_owned())
        .ok_or_else(|| {
            internal_compiler_error(
                "TIR child wrapper application: wrapper template ID was not present in the store.",
            )
        })?;

    let occurrence_id = store.next_child_template_occurrence_id();
    let wrapper_reference = TemplateTirChildReference::new(
        wrapper_template_id,
        TemplateTirPhase::Parsed,
        TemplateViewContext::default(),
    );
    let wrapper_node_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: wrapper_reference,
            occurrence_id,
        },
        wrapper_location.to_owned(),
    ));

    let combined_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![wrapper_node_id, child_node_id],
        },
        child_location,
    ));

    // The combined template contains two child references, so it is not a plain
    // const-evaluable string even if both constituents happen to fold.
    let mut summary = TemplateIrSummary::default();
    summary.record_child_template();
    summary.record_child_template();
    summary.is_const_evaluable_shape = false;

    Ok(store.push_template(TemplateIr::new(
        combined_root,
        Style::default(),
        TemplateType::String,
        summary,
        wrapper_location,
    )))
}
