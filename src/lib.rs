//! Moth compiler package root.
//!
//! Targeted `#[allow(...)]` attributes are used where needed, each with a justification
//! comment.

pub(crate) mod timing;

/// Features the running binary was built with, in declaration order.
///
/// WHAT: the enabled Cargo features of this compiled `moth` library, as names.
/// WHY:  a machine-readable report that does not name its build configuration cannot be told
///       apart from one produced by a differently configured run of the same command. This is
///       public because `xtask` links this library and its reports describe the same build.
pub const ENABLED_FEATURES: &[&str] = &[
    #[cfg(feature = "timers")]
    "timers",
    #[cfg(feature = "detailed_timers")]
    "detailed_timers",
    #[cfg(feature = "benchmark_counters")]
    "benchmark_counters",
    #[cfg(feature = "checked_blocks")]
    "checked_blocks",
    #[cfg(feature = "async_blocks")]
    "async_blocks",
    #[cfg(feature = "show_tokens")]
    "show_tokens",
    #[cfg(feature = "show_headers")]
    "show_headers",
    #[cfg(feature = "show_ast")]
    "show_ast",
    #[cfg(feature = "show_eval")]
    "show_eval",
    #[cfg(feature = "show_hir")]
    "show_hir",
    #[cfg(feature = "show_codegen")]
    "show_codegen",
    #[cfg(feature = "show_borrow_checker")]
    "show_borrow_checker",
    #[cfg(feature = "boracle")]
    "boracle",
    #[cfg(feature = "boracle_campaign")]
    "boracle_campaign",
];

mod compiler_tests {
    pub(crate) mod integration_test_runner; // For running all integration tests and report back the results

    #[cfg(test)]
    mod config_input_origin_tests;
    #[cfg(test)]
    mod external_float_registration_tests;
    #[cfg(test)]
    mod mon_syntax_parity;
    #[cfg(test)]
    pub mod test_diagnostics;
    #[cfg(test)]
    pub mod test_fs;
    #[cfg(test)]
    pub mod test_support;
}
pub mod benchmarking;
pub mod build_system;
mod builder_surface;
mod compiler_frontend;
pub mod first_party_js;

/// Deliberate, supported convenience re-export of `moth_mon`; this exposes the same API and Rust
/// types directly, not a compatibility shim or second implementation.
pub use moth_mon as mon;

mod backends {
    pub(crate) mod backend_feature_validation;
    pub(crate) mod error_types;
    pub(crate) mod external_package_validation;
    pub(crate) mod js;
    pub(crate) mod structural_string;
    #[cfg(test)]
    mod tests;
    pub(crate) mod wasm;
}

pub mod projects {
    #[cfg(feature = "boracle")]
    pub mod boracle;
    pub mod build_command;
    pub mod check;
    pub mod dev_server;
    pub(crate) mod html_project;
    pub mod integration_tests;
    pub(crate) mod routing;
    pub mod settings;

    pub use crate::build_system::build::ProjectBuilder;
    pub use crate::compiler_frontend::Flag;
    pub use crate::compiler_frontend::build_config::{
        BuildCommandLocation, BuildConfigInputDuplicate, BuildConfigInputEntry,
        BuildConfigInputSet, BuildConfigInputValue, BuildConfigValueLocation, BuildInputName,
        BuildInputNameError, BuildInputValueError, PrimitiveBuildValue,
    };
    pub use crate::compiler_frontend::compiler_errors::BoundaryFailure;
    pub use crate::compiler_frontend::compiler_errors::CompilerError;
    pub use crate::compiler_frontend::compiler_errors::CompilerMessages;
    pub use crate::compiler_frontend::display_messages::{
        print_boundary_messages, print_compiler_messages, print_terse_boundary_messages,
        print_terse_compiler_messages,
    };
    pub use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;

    #[cfg(feature = "boracle")]
    pub use crate::compiler_frontend::analysis::borrow_checker::{
        BoracleDump, BoracleExperiment, BoracleReferenceRuleSet, BoracleRuleSelection,
    };

    /// Outcome of a completed build or check service call.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ProjectCommandStatus {
        Success,
        Failure,
    }

    /// Source diagnostic counts retained for the command's outer presentation boundary.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ProjectDiagnosticCounts {
        pub errors: usize,
        pub warnings: usize,
    }

    impl ProjectDiagnosticCounts {
        pub(crate) fn from_messages(
            messages: &CompilerMessages,
        ) -> Option<ProjectDiagnosticCounts> {
            if messages.has_infrastructure_error() {
                return None;
            }

            Some(ProjectDiagnosticCounts {
                errors: messages.error_count(),
                warnings: messages.warning_count(),
            })
        }
    }

    /// Completion facts returned after command presentation and timing have finished.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ProjectCommandCompletion {
        pub status: ProjectCommandStatus,
        pub diagnostic_counts: Option<ProjectDiagnosticCounts>,
    }
}
