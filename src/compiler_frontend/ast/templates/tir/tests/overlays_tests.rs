//! Focused invariants for compact overlay dimensions and value contexts.

use std::mem::size_of;

use super::super::ids::{TemplateIrId, TemplateWrapperSetId};
use super::super::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolution,
    TirSlotResolutionOverlay, TirSlotResolutionOverlayId, TirWrapperApplicationMode,
    TirWrapperContext, TirWrapperContextOverlay, TirWrapperContextOverlayId,
};
use super::super::refs::{
    TemplateTirChildReference, TemplateTirReference, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::ids::SlotOccurrenceId;

#[test]
fn optional_overlay_ids_are_compact_nonzero_indices() {
    assert_eq!(
        size_of::<Option<TirExpressionOverlayId>>(),
        size_of::<u32>()
    );
    assert_eq!(
        size_of::<Option<TirSlotResolutionOverlayId>>(),
        size_of::<u32>()
    );
    assert_eq!(
        size_of::<Option<TirWrapperContextOverlayId>>(),
        size_of::<u32>()
    );

    assert_eq!(TirExpressionOverlayId::new(0).index(), 0);
    assert_eq!(TirSlotResolutionOverlayId::new(7).index(), 7);
    assert_eq!(TirWrapperContextOverlayId::new(11).index(), 11);
}

#[test]
fn view_context_and_reference_layouts_are_pinned() {
    assert_eq!(size_of::<TemplateViewContext>(), 12);
    assert_eq!(size_of::<TemplateTirReference>(), 20);
    assert_eq!(size_of::<TemplateTirChildReference>(), 20);
    assert_eq!(size_of::<TemplateWrapperReference>(), 20);
}

#[test]
fn context_merge_preserves_last_dimension_precedence() {
    let outer = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(1)),
        slot_resolution: None,
        wrapper_context: Some(TirWrapperContextOverlayId::new(2)),
    };
    let inner = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(3)),
        slot_resolution: Some(TirSlotResolutionOverlayId::new(4)),
        wrapper_context: None,
    };

    assert_eq!(
        outer.merge(inner),
        TemplateViewContext {
            expression_overlay: Some(TirExpressionOverlayId::new(3)),
            slot_resolution: Some(TirSlotResolutionOverlayId::new(4)),
            wrapper_context: Some(TirWrapperContextOverlayId::new(2)),
        }
    );
}

#[test]
fn empty_context_is_the_default_value() {
    assert_eq!(
        TemplateViewContext::default(),
        TemplateViewContext {
            expression_overlay: None,
            slot_resolution: None,
            wrapper_context: None,
        }
    );
}

#[test]
fn overlay_payload_ids_index_typed_store_entries() {
    let mut store = super::super::store::TemplateIrStore::new();
    let expression = store
        .allocate_expression_overlay(TirExpressionOverlay::default())
        .expect("test overlay allocation");
    let slot = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let wrapper = store
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay::default())
        .expect("test overlay allocation");

    assert_eq!(store.expression_overlay(expression).map(|_| ()), Some(()));
    assert_eq!(store.slot_resolution_overlay(slot).map(|_| ()), Some(()));
    assert_eq!(store.wrapper_context_overlay(wrapper).map(|_| ()), Some(()));
}

#[test]
fn slot_resolution_payload_preserves_replay_sources() {
    let source = TemplateIrId::new(3);
    let resolution = TirSlotResolution::resolved(SlotKey::Default, vec![source]);

    assert_eq!(resolution.sources(), &[source]);
}

#[test]
fn wrapper_context_payload_preserves_application_policy() {
    let wrapper_set = TemplateWrapperSetId::new(1);
    let context = TirWrapperContext::inherited(wrapper_set);

    assert_eq!(context.inherited_wrapper_set, Some(wrapper_set));
    assert!(matches!(
        context.application_mode,
        TirWrapperApplicationMode::Always
    ));
}

#[test]
fn slot_and_wrapper_payloads_lookup_occurrences() {
    let slot_source = TemplateIrId::new(1);
    let slot_overlay = TirSlotResolutionOverlay {
        resolutions: vec![(
            SlotOccurrenceId::new(0),
            TirSlotResolution::resolved(SlotKey::Default, vec![slot_source]),
        )],
    };
    assert!(
        slot_overlay
            .resolution_for_occurrence(SlotOccurrenceId::new(0))
            .is_some()
    );

    let wrapper_overlay = TirWrapperContextOverlay {
        contexts: vec![(
            super::super::ids::ChildTemplateOccurrenceId::new(0),
            TirWrapperContext::default(),
        )],
    };
    assert!(
        wrapper_overlay
            .context_for_occurrence(super::super::ids::ChildTemplateOccurrenceId::new(0))
            .is_some()
    );
}
