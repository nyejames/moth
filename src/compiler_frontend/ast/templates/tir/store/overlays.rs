//! Checked overlay allocation and shared key validation.

use super::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolutionOverlay,
    TirSlotResolutionOverlayId, TirWrapperContextOverlay, TirWrapperContextOverlayId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use rustc_hash::FxHashSet;
use std::fmt::Display;
use std::hash::Hash;

impl TemplateIrStore {
    pub(crate) fn allocate_expression_overlay(
        &mut self,
        overlay: TirExpressionOverlay,
    ) -> Result<TirExpressionOverlayId, CompilerError> {
        reject_duplicate_or_out_of_range_overlay_keys(
            overlay.overrides.iter().map(|(site_id, _)| *site_id),
            self.allocated_expression_site_count(),
            ExpressionSiteId::index,
            "expression-site",
        )?;

        let id = TirExpressionOverlayId::new(self.expression_overlays.len());
        self.expression_overlays.push(overlay);
        Ok(id)
    }

    #[allow(
        dead_code,
        reason = "slot-resolution overlay storage remains a canonical exact-view dimension without a current production allocator"
    )]
    pub(crate) fn allocate_slot_resolution_overlay(
        &mut self,
        overlay: TirSlotResolutionOverlay,
    ) -> Result<TirSlotResolutionOverlayId, CompilerError> {
        reject_duplicate_or_out_of_range_overlay_keys(
            overlay
                .resolutions
                .iter()
                .map(|(occurrence_id, _)| *occurrence_id),
            self.next_slot_occurrence as usize,
            SlotOccurrenceId::index,
            "slot-occurrence",
        )?;

        let id = TirSlotResolutionOverlayId::new(self.slot_resolution_overlays.len());
        self.slot_resolution_overlays.push(overlay);
        Ok(id)
    }

    pub(crate) fn allocate_wrapper_context_overlay(
        &mut self,
        overlay: TirWrapperContextOverlay,
    ) -> Result<TirWrapperContextOverlayId, CompilerError> {
        reject_duplicate_or_out_of_range_overlay_keys(
            overlay
                .contexts
                .iter()
                .map(|(occurrence_id, _)| *occurrence_id),
            self.allocated_child_template_occurrence_count(),
            ChildTemplateOccurrenceId::index,
            "child-template-occurrence",
        )?;

        let id = TirWrapperContextOverlayId::new(self.wrapper_context_overlays.len());
        self.wrapper_context_overlays.push(overlay);
        Ok(id)
    }

    pub(crate) fn expression_overlay(
        &self,
        id: TirExpressionOverlayId,
    ) -> Option<&TirExpressionOverlay> {
        self.expression_overlays.get(id.index())
    }

    pub(crate) fn slot_resolution_overlay(
        &self,
        id: TirSlotResolutionOverlayId,
    ) -> Option<&TirSlotResolutionOverlay> {
        self.slot_resolution_overlays.get(id.index())
    }

    pub(crate) fn wrapper_context_overlay(
        &self,
        id: TirWrapperContextOverlayId,
    ) -> Option<&TirWrapperContextOverlay> {
        self.wrapper_context_overlays.get(id.index())
    }
}

fn reject_duplicate_or_out_of_range_overlay_keys<K>(
    keys: impl Iterator<Item = K>,
    allocated_count: usize,
    index_of: impl Fn(K) -> usize,
    label: &str,
) -> Result<(), CompilerError>
where
    K: Copy + Eq + Hash + Display,
{
    let mut seen = FxHashSet::default();

    for key in keys {
        if !seen.insert(key) {
            return Err(CompilerError::compiler_error(format!(
                "TIR store rejected a duplicate {label} overlay key {key}."
            )));
        }

        if index_of(key) >= allocated_count {
            return Err(CompilerError::compiler_error(format!(
                "TIR store rejected out-of-range {label} overlay key {key}."
            )));
        }
    }

    Ok(())
}
