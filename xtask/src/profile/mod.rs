//! Profiling module - Samply-backed profiling workflow for benchmark cases.
//!
//! WHAT: Provides profiling through focused submodules: option parsing,
//! Samply running, artifact layout and writers, hotspot parsing, observation
//! passes, drift reports, summaries and history storage.
//!
//! WHY: Profiling gives attribution evidence for optimization work. Each
//! profiling concern keeps one owner; this file is the structural map.
//!
//! # Module map
//! - `run.rs` owns the profiling workflow and case selection
//! - `artifacts.rs` owns run/case artifact layout and JSON writers
//! - `history.rs` owns current profile history and legacy v1-v3 adapters
//! - `drift.rs` owns drift detection and comparable previous selection
//! - `hotspots.rs`, `parse.rs`, `observations.rs` own profile data extraction
//! - `runner.rs` owns Samply invocation
//! - `summary.rs` owns agent summaries and Markdown generation
//! - `options.rs` owns profile command parsing

pub(crate) mod artifacts;
pub(crate) mod buckets;
pub(crate) mod drift;
pub(crate) mod history;
pub(crate) mod hotspots;
pub(crate) mod observations;
pub(crate) mod options;
pub(crate) mod parse;
pub(crate) mod run;
pub(crate) mod runner;
pub(crate) mod summary;

// Re-export the narrow surface needed by main.rs and mode.rs.
pub(crate) use options::{ProfileOptions, ProfileParseResult, parse_profile_args};
pub(crate) use run::run_profile_benchmarks;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
