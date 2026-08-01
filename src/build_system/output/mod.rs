//! Focused output subsystem: policy, manifest ownership, and writer preflight.
//!
//! WHAT: owns the build-system output policy (profiles, builder identity, owners and
//! validated plans), the output manifest format, and final output batch emission.
//! WHY: build orchestration must stay focused on compilation; output ownership,
//! validation and writing each have a single owner that CLI and the dev server share.
//!
//! Phase 1 introduces the policy owner. Manifest and writer responsibilities move here
//! in later phases without leaving compatibility shims behind.

mod policy;

pub(crate) use policy::{BuildProfile, OutputFolderClassification, classify_output_folder};
