//! TIR-native slot contribution routing and composition.
//!
//! Slot schema and placeholder occurrence facts come from `tir/slot_layout.rs`.
//! This directory routes fill content, expands placeholders, composes head
//! chains and owned child-wrapper trees.
//!
//! ```text
//! slot_composition/
//! ├── mod.rs            Structural map and public re-exports
//! ├── schema.rs         Placeholder expansion
//! ├── contributions.rs  Contribution bucket routing
//! ├── head_chain.rs     Head-chain wrapper composition
//! ├── child_wrappers.rs Owned child-wrapper trees
//! └── helpers.rs        Shared types and store/diagnostic helpers
//! ```

pub(crate) mod child_wrappers;
pub(crate) mod contributions;
mod head_chain;
pub(crate) mod helpers;
pub(crate) mod schema;

pub(crate) use head_chain::{
    compose_tir_head_chain_from_root, compose_tir_head_chain_from_template,
};

pub(crate) use contributions::TirSlotContributions;
pub(crate) use helpers::stored_insert_contribution_templates;
#[cfg(test)]
pub(crate) use schema::fill_named_slot_placeholders_into;
pub(crate) use schema::{
    apply_slot_wrapper_sets_to_deferred_fill, attach_conditional_wrapper_set,
    fill_direct_child_wrapper_names_into, fill_named_slots_in_wrapper_root,
    fill_named_slots_in_wrapper_template, fill_wrapper_tree_named_contexts,
};
