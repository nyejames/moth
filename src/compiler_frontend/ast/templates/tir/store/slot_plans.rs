//! Slot-plan reservation, commit validation and committed lookup.

use super::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::ids::TemplateSlotPlanId;
use crate::compiler_frontend::ast::templates::tir::slot_plan::TemplateSlotPlan;
use crate::compiler_frontend::compiler_errors::CompilerError;

/// Reserved entries occupy a stable ID but are invisible through
/// [`TemplateIrStore::get_slot_plan`]. A reserved ID must be committed exactly
/// once; an uncommitted reservation stays unreachable.
#[derive(Debug)]
pub(super) enum SlotPlanSlot {
    Reserved,
    Committed(TemplateSlotPlan),
}

impl TemplateIrStore {
    /// Reserves a stable slot-plan ID that is invisible until commit.
    ///
    /// Contribution markers need the owning plan ID while sources and sites are
    /// still local drafts. Ordinary `get_slot_plan` must not observe that
    /// incomplete state.
    pub(crate) fn reserve_slot_plan(&mut self) -> TemplateSlotPlanId {
        let id = TemplateSlotPlanId::new(self.slot_plans.len());
        self.slot_plans.push(SlotPlanSlot::Reserved);
        id
    }

    /// Publishes one complete slot plan into a reserved ID.
    ///
    /// Validates immediate plan shape before replacing `Reserved`. On error the
    /// reserved ID stays unpublished. Recursive marker and view validation stay
    /// in preparation.
    pub(crate) fn commit_slot_plan(
        &mut self,
        id: TemplateSlotPlanId,
        slot_plan: TemplateSlotPlan,
    ) -> Result<(), CompilerError> {
        match self.slot_plans.get(id.index()) {
            Some(SlotPlanSlot::Reserved) => {}
            Some(SlotPlanSlot::Committed(_)) => {
                return Err(CompilerError::compiler_error(format!(
                    "TIR store slot plan {id} was committed more than once."
                )));
            }
            None => {
                return Err(CompilerError::compiler_error(format!(
                    "TIR store cannot commit unknown slot plan {id}."
                )));
            }
        }

        self.validate_immediate_slot_plan_shape(&slot_plan)?;

        match self.slot_plans.get_mut(id.index()) {
            Some(slot @ SlotPlanSlot::Reserved) => {
                *slot = SlotPlanSlot::Committed(slot_plan);
                Ok(())
            }
            Some(SlotPlanSlot::Committed(_)) => Err(CompilerError::compiler_error(format!(
                "TIR store slot plan {id} was committed more than once."
            ))),
            None => Err(CompilerError::compiler_error(format!(
                "TIR store cannot commit unknown slot plan {id}."
            ))),
        }
    }

    fn validate_immediate_slot_plan_shape(
        &self,
        slot_plan: &TemplateSlotPlan,
    ) -> Result<(), CompilerError> {
        for (index, source) in slot_plan.contribution_sources.iter().enumerate() {
            if source.source.0 != index {
                return Err(CompilerError::compiler_error(format!(
                    "TIR store rejected slot-plan contribution source {} at index {index}.",
                    source.source.0
                )));
            }

            if self.get_node(source.render_root).is_none() {
                return Err(CompilerError::compiler_error(format!(
                    "TIR store rejected slot-plan contribution source {index} with missing render root {}.",
                    source.render_root
                )));
            }
        }

        for (index, site) in slot_plan.slot_sites.iter().enumerate() {
            if site.site.0 != index {
                return Err(CompilerError::compiler_error(format!(
                    "TIR store rejected slot-plan site {} at index {index}.",
                    site.site.0
                )));
            }

            if self.get_node(site.render_root).is_none() {
                return Err(CompilerError::compiler_error(format!(
                    "TIR store rejected slot-plan site {index} with missing render root {}.",
                    site.render_root
                )));
            }
        }

        Ok(())
    }

    /// Returns a committed slot plan. Reserved IDs are invisible.
    pub(crate) fn get_slot_plan(&self, id: TemplateSlotPlanId) -> Option<&TemplateSlotPlan> {
        match self.slot_plans.get(id.index()) {
            Some(SlotPlanSlot::Committed(plan)) => Some(plan),
            Some(SlotPlanSlot::Reserved) | None => None,
        }
    }
}
