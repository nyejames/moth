//! TIR unit tests.
//!
//! Fixture constructors and inspection helpers live in `support` so production
//! TIR files do not host test-only methods or semantic states.

pub(crate) mod builder;
pub(crate) mod support;

mod builder_tests;
mod construction_tests;
mod expression_traversal_tests;
mod fold_cache_tests;
mod fold_final_view_tests;
mod hir_handoff_tests;
mod ids_tests;
mod overlays_tests;
mod preparation_tests;
mod render_unit_tests;
mod slot_composition_tests;
mod slot_layout_tests;
mod store_tests;
mod subtree_copy_tests;
mod summary_tests;
mod view_tests;
mod wrapper_context_construction_tests;
mod wrapper_context_fold_tests;
