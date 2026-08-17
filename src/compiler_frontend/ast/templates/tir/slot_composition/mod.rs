//! TIR-native slot contribution routing and composition.
//!
//! Slot schema and placeholder occurrence facts come from `tir/slot_layout.rs`.
//! This directory routes fill content, expands placeholders, composes head
//! chains and owned `$children(..)` wrapper trees.
//!
//! ```text
//! slot_composition/
//! ├── mod.rs            Structural map and public re-exports
//! ├── schema.rs         Placeholder expansion
//! ├── contributions.rs  Contribution bucket routing
//! ├── head_chain.rs     Head-chain wrapper composition
//! ├── child_wrappers.rs Owned `$children(..)` wrapper trees
//! └── helpers.rs        Shared types and store/diagnostic helpers
//! ```

pub(crate) mod child_wrappers;
pub(crate) mod contributions;
mod head_chain;
mod helpers;
pub(crate) mod schema;

pub(crate) use head_chain::compose_tir_head_chain_from_root;

#[cfg(test)]
pub(crate) use head_chain::compose_tir_head_chain;

pub(crate) use contributions::TirSlotContributions;
pub(crate) use helpers::stored_insert_contribution_templates;
