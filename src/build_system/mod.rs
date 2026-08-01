//! Build-system entry modules.
//!
//! This layer owns project discovery, configuration parsing, backend orchestration, and output
//! writing above the shared compiler frontend.

// -------------------------
//  Public Modules
// -------------------------

pub(crate) mod build;
pub(crate) mod build_profile;
pub(crate) mod create_project_modules;
pub(crate) mod output;
pub(crate) mod output_cleanup;
pub(crate) mod path_validation;
pub(crate) mod project_config;
pub(crate) mod utils;

pub(crate) use build_profile::BuildProfile;
