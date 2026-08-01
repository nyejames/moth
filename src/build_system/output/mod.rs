//! Focused output policy subsystem.
//!
//! WHAT: owns portable output-folder classification, validated output-folder values and the
//! deterministic relative output-path identity.
//! WHY: output ownership and validation must exist once so CLI and the dev server never drift.
//! Consumers live in config validation and output planning.

mod output_path;
mod policy;

#[cfg(test)]
mod tests;

pub(crate) use output_path::output_path_identity;
pub(crate) use policy::{ValidatedOutputFolder, classify_output_folder};
