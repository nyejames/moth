//! Build-system entry modules.
//!
//! This layer owns project discovery, configuration parsing, and backend orchestration above the
//! shared compiler frontend. The output subsystem owns artifact writing and cleanup.

// -------------------------
//  Public Modules
// -------------------------

pub(crate) mod build;
pub(crate) mod build_profile;
pub(crate) mod create_project_modules;
pub(crate) mod output;
pub(crate) mod path_validation;
pub(crate) mod project_config;
pub(crate) mod resource_unions;
pub(crate) mod utils;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use build_profile::BuildProfile;
