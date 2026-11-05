//! Runtime slot planning data shapes.
//!
//! WHAT: Defines the small planner-local shapes used while runtime slot
//! applications are materialized into the module-scoped TIR store.
//!
//! WHY: HIR consumes the neutral owned handoff produced from TIR. The runtime
//! slot planner still needs stable source/site IDs while routing, but it no
//! longer builds a separate render-plan handoff that later has to be converted
//! back into TIR.

use crate::compiler_frontend::ast::templates::tir::{
    ContributionShape, TemplateSlotContributionSourcePlan,
};

/// Stable ID for a contribution source accumulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RuntimeSlotContributionSourceId(pub(crate) usize);

/// Stable ID for a wrapper placeholder occurrence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeSlotSiteId(pub(crate) usize);

/// Draft of a single contribution source while the runtime plan is being built.
///
/// WHAT: Pairs the resolved source plan with its contribution shape and
/// control-flow class so the planner can accumulate contributions before
/// assigning stable IDs.
///
/// WHY: Source IDs must be stable across the planning pass, so drafts are
/// collected first and then converted into the final module-scoped TIR store.
/// The facts are copied onto every marker the draft plants, so recursive
/// structural named composition classifies the marker exactly as the authored
/// contribution without a parallel side map.
#[derive(Clone, Debug)]
pub(super) struct RuntimeSlotContributionSourceDraft {
    pub(super) source: TemplateSlotContributionSourcePlan,
    pub(super) shape: ContributionShape,
    /// Whether the source root emits only through template control flow.
    pub(super) control_flow: bool,
}
