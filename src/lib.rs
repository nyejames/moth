//! Moth compiler package root.
//!
//! Targeted `#[allow(...)]` attributes are used where needed, each with a justification
//! comment. One temporary crate-level exception is documented below and is owned by the
//! compiler source/token/diagnostic data-layout plan.

// TEMPORARY VALIDATION BRIDGE: `CompilerError` is currently a 192-byte mixed error type, so
// Rust 1.95 Clippy reports `result_large_err` at the many existing internal and infrastructure
// `Result` boundaries. The data-layout plan must replace that representation and remove this
// allowance rather than narrowing or copying it before its final Clippy gate.
#![allow(clippy::result_large_err)]

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
];

mod compiler_tests {
    pub(crate) mod integration_test_runner; // For running all integration tests and report back the results

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
    pub mod check;
    pub mod cli;
    pub(crate) mod command_status;
    pub mod dev_server;
    pub(crate) mod html_project;
    // Kept intentionally in pre-alpha as the future CLI entrypoint for interactive
    // template experimentation. This remains outside the default command surface.
    pub(crate) mod repl;
    pub(crate) mod routing;
    pub mod settings;
}
