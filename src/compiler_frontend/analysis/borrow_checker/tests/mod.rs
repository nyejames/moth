//! Borrow-checker regression test modules.
//!
//! WHAT: groups focused tests for borrow facts, scope rules, drop sites, summaries, and pipeline
//! behavior.
//! WHY: borrow checking spans multiple internal passes, so keeping scenario-focused modules here
//! makes regressions easier to isolate.

mod borrow_checker_call_summary_tests;
mod borrow_checker_drop_site_tests;
mod borrow_checker_fact_tests;
mod borrow_checker_loop_tests;
mod borrow_checker_pipeline_tests;
mod borrow_checker_reactivity_tests;
mod borrow_checker_scope_tests;
mod state_tests;
