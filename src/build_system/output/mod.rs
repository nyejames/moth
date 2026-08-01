//! Focused output policy subsystem.
//!
//! WHAT: owns the pure output-folder classifier used by config diagnostics and the durable
//! validated output-folder value that output-plan construction and Phase 1D carry through
//! bootstrap.
//! WHY: output ownership and validation must exist once so CLI and the dev server never drift.

mod output_path;
mod policy;

#[cfg(test)]
mod tests;

pub(crate) use output_path::output_path_identity;
pub(crate) use policy::{ValidatedOutputFolder, classify_output_folder};
