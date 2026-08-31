//! Focused output policy, manifest and writer subsystem.
//!
//! WHAT: owns portable output-folder classification, validated plans, manifest recovery and
//! filesystem-safe batch emission.
//! WHY: output ownership, cleanup and destination safety must exist once so CLI and the dev
//! server never drift. Backend-specific route semantics stay with the builder.

pub(crate) mod manifest;
mod orchestrator;
mod output_path;
mod policy;
mod writer;
pub(crate) use writer::OutputRejectionReason;
// Write outcomes are inspected by name only from the build-output tests; production callers
// consume the summary returned by `write_project_outputs` without naming its parts.
#[cfg(test)]
pub(crate) use writer::{OutputDestinationOutcome, OutputWriteOutcome, OutputWriteSummary};

#[cfg(test)]
mod tests;

pub(crate) use output_path::{OutputPathIdentity, output_path_identity};
pub(crate) use policy::ValidatedOutputFolder;
pub(crate) use policy::canonical_output_root_for_identity;
pub(crate) use policy::classify_output_folder;
pub(crate) use policy::validate_output_folder_containment;

pub(crate) use policy::{
    BuilderKind, CleanupPolicy, OutputOwner, OutputPlan, SingleFileOutputPlan,
    ValidatedDirectoryOutputSettings, ValidatedOutputPlan,
};

pub(crate) use manifest::validate_relative_output_path;

pub(crate) use orchestrator::{WriteMode, WriteOptions, write_project_outputs};
