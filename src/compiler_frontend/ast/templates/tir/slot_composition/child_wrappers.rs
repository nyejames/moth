//! TIR-native owned `$children(..)` wrapper-tree construction.
//!
//! Wrapper-context overlays own direct-child inherited wrappers. This module
//! remains only for transforms that need a store-local owned wrapper tree,
//! such as explicit slot expansion.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::ast::templates::tir::slot_layout::collect_tir_slot_layout;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrId, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use super::helpers::{compose_wrapper_application, internal_compiler_error};

type ChildWrapperResult<T> = Result<T, TemplateError>;

/// Wraps a single direct child node in all inherited wrappers.
///
/// Wrapper sets are stored innermost-to-outermost, so forward iteration yields
/// `outermost(innermost(child))`. Each application receives the complete wrapper
/// reference so phase and contextual authority are preserved at the boundary.
pub(crate) fn wrap_tir_node_in_wrappers_into(
    store: &mut TemplateIrStore,
    child_node_id: TemplateIrNodeId,
    wrapper_references: &[TemplateWrapperReference],
    string_table: &StringTable,
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

    for wrapper_reference in wrapper_references {
        let layout = collect_tir_slot_layout(store, wrapper_reference.root)?;

        if layout.schema.has_any_slots() {
            let resolved = compose_wrapper_application(
                store,
                *wrapper_reference,
                &layout,
                vec![current_child_node_id],
                child_location.clone(),
                string_table,
                false,
            )?;

            let occurrence_id = store.next_child_template_occurrence_id();
            current_child_node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: resolved,
                    occurrence_id,
                },
                child_location.clone(),
            ));
        } else {
            let combined_template_id = build_tir_prepended_wrapper_template(
                store,
                *wrapper_reference,
                current_child_node_id,
                child_location.clone(),
            )?;

            let occurrence_id = store.next_child_template_occurrence_id();
            let reference = wrapper_reference
                .into_structural_child_reference()
                .with_root(combined_template_id);
            current_child_node_id = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id,
                },
                child_location.clone(),
            ));
        }
    }

    Ok(current_child_node_id)
}

/// Builds a template that prepends a slot-less wrapper before an existing child.
fn build_tir_prepended_wrapper_template(
    store: &mut TemplateIrStore,
    wrapper_reference: TemplateWrapperReference,
    child_node_id: TemplateIrNodeId,
    child_location: SourceLocation,
) -> ChildWrapperResult<TemplateIrId> {
    let wrapper_location = store
        .get_template(wrapper_reference.root)
        .map(|wrapper_template| wrapper_template.location.to_owned())
        .ok_or_else(|| {
            internal_compiler_error(
                "TIR child wrapper application: wrapper template ID was not present in the store.",
            )
        })?;

    let occurrence_id = store.next_child_template_occurrence_id();
    let wrapper_child_reference = wrapper_reference.into_structural_child_reference();
    let wrapper_node_id = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: wrapper_child_reference,
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

    let mut summary = TemplateIrSummary::default();
    summary.record_child_template();
    summary.record_child_template();

    Ok(store.push_template(TemplateIr::new(
        combined_root,
        Style::default(),
        TemplateType::String,
        summary,
        wrapper_location,
    )))
}
