//! TIR-native slot schema extraction, contribution routing, and composition.
//!
//! WHAT: discovers declared `$slot` targets from TIR nodes, routes fill-template
//!       contributions into the right buckets, expands slot placeholders with
//!       those contributions, and composes wrapper templates through both the
//!       head-chain and `$children(..)` wrapper paths.
//!
//! WHY: this directory replaces the monolithic `slot_composition.rs` with
//!      focused submodules: schema discovery, contribution routing, overlay
//!      allocation, head-chain composition, child-wrapper application, and the
//!      small shared helpers they all depend on. Each submodule owns one step of
//!      the composition pipeline, while this file exposes the current module
//!      surface used by the surrounding TIR stage.
//!
//! ## Module layout
//!
//! ```text
//! slot_composition/
//! ├── mod.rs            Structural map and public re-exports
//! ├── schema.rs         Slot schema discovery and placeholder expansion
//! ├── contributions.rs  Contribution bucket routing
//! ├── overlays.rs       Slot-resolution overlay materialization and merging
//! ├── head_chain.rs     Head-chain wrapper composition
//! ├── child_wrappers.rs `$children(..)` wrapper application
//! └── helpers.rs        Shared types and store/diagnostic/builder helpers
//! ```

mod child_wrappers;
mod contributions;
mod head_chain;
mod helpers;
mod overlays;
mod schema;

// Re-exports expose the current slot-composition surface. Test-only inspection
// seams remain gated so production builds carry only active compiler APIs.

pub(crate) use schema::{
    TirSlotSchema, collect_tir_slot_placeholders_in_order, collect_tir_slot_schema,
};

pub(crate) use head_chain::{compose_tir_head_chain, compose_tir_head_chain_with_overlays};

pub(crate) use child_wrappers::wrap_tir_node_in_wrappers;

pub(crate) use overlays::merge_tir_slot_resolution_contexts;

#[cfg(test)]
pub(crate) use contributions::route_tir_slot_contributions;
pub(crate) use contributions::{RoutedTirSlotContributions, TirSlotContributions};
pub(crate) use helpers::stored_insert_contribution_templates;
#[cfg(test)]
pub(crate) use overlays::{
    materialize_tir_slot_resolution_overlay, view_context_from_slot_resolution_overlay,
};
#[cfg(test)]
pub(crate) use schema::{SlotSchemaError, expand_tir_slot_placeholders};
