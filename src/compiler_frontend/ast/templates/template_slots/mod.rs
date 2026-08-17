//! Runtime slot planning types and TIR-native materialization.
//!
//! WHAT: Provides stable IDs for runtime slot contribution sources and sites,
//! plus the TIR-native runtime slot plan materialization that produces owned
//! handoff payloads for the AST/HIR boundary.
//!
//! WHY: HIR should only consume prepared source/site plans. The runtime slot
//! planner writes side-tables into the module-scoped TIR store, then returns
//! neutral owned handoff shapes defined in `runtime_handoff.rs`.

// -------------------------
//  Submodules
// -------------------------

mod runtime_plan;

// -------------------------
//  Re-exports
// -------------------------

pub(crate) use runtime_plan::{RuntimeSlotContributionSourceId, RuntimeSlotSiteId};
pub(in crate::compiler_frontend::ast::templates) use runtime_plan::{
    materialize_tir_native_runtime_slot_plan, tir_contributions_need_runtime,
};
