//! Self-tests grouped by integration test runner ownership.
//!
//! Each child module keeps the helpers and assertions for one runner concern
//! close to the code it protects.

mod assertions;
mod execution;
mod expectations;
mod fixture;
mod manifest;
mod policy;
mod rendered_output;
mod reporting;
mod runner;
mod selection;
mod synthetic_build_results;
mod terse_reporting;
mod wasm_baseline;
