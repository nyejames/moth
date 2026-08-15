//! TIR unit tests.
//!
//! WHAT: exercises the typed IDs, store, summary, validation and builder types
//! used by parser-emitted TIR.
//! WHY: TIR is a new internal representation; keeping its unit tests in a separate
//!      module follows the project rule that tests do not live in production files.

mod body_root_wrapper_tests;
mod builder_tests;
mod classification_tests;
mod expression_payload_walker_tests;
mod fold_cache_tests;
mod fold_final_view_tests;
mod hir_handoff_tests;
mod ids_tests;
mod overlays_tests;
mod preparation_tests;
mod render_unit_tests;
mod slot_composition_tests;
mod store_tests;
mod subtree_copy_tests;
mod summary_tests;
mod validation_support;
mod validation_tests;
mod view_tests;
mod wrapper_context_fold_tests;
