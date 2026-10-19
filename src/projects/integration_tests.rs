//! Compiler integration-test service for the command-line developer tooling.
//!
//! The CLI owns argument parsing and presentation; the compiler owns suite loading, execution and
//! test-runner failures.

pub use crate::compiler_tests::integration_test_runner::{
    BackendId, IntegrationRunSummary, TestRunnerError, TestRunnerErrorKind, TestRunnerOptions,
    run_all_test_cases,
};
