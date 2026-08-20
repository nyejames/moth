//! Integration test runner for end-to-end Moth compiler coverage.
//!
//! Supports:
//! - canonical self-contained case folders under `tests/cases/<case>/`
//! - required manifest-driven case ordering and case metadata
//! - backend-specific expectation matrices from a shared input fixture

// `assertions` is crate-visible because it owns the emitted-document shell contract that the
// HTML builder's own tests consume, so the builder tests and the canonical suite cannot drift.
pub(crate) mod assertions;
mod errors;
mod execution;
mod expectations;
mod fixture;
mod manifest;
mod path_validation;
mod policy;
mod reporting;
mod runner;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use runner::run_all_test_cases;
pub use types::IntegrationRunSummary;

pub(crate) use types::{
    ArtifactAssertion, ArtifactKind, BackendId, CaseExecutionResult, CaseRole, DiagnosticAssertion,
    DiagnosticMatchMode, ExpectationMode, ExpectedOutcome, FailureExpectation, FailureKind,
    FailureTriageEntry, FailureTriageReport, GoldenMode, ManifestCaseSpec,
    ParsedBackendExpectation, ParsedExpectationFile, SuccessContract, SuccessExpectation,
    SummaryCounts, TestCaseSpec, TestRunnerOptions, TestSuiteSpec, WarningExpectation,
};

pub(crate) use policy::{PolicyEvaluation, PolicyFinding};

pub(crate) const CANONICAL_TESTS_PATH: &str = "tests/cases";
pub(crate) const MANIFEST_FILE_NAME: &str = "manifest.toml";
pub(crate) const EXPECT_FILE_NAME: &str = "expect.toml";
pub(crate) const INPUT_DIR_NAME: &str = "input";
pub(crate) const GOLDEN_DIR_NAME: &str = "golden";
pub(crate) const FAILURE_TRIAGE_REPORT_PATH: &str =
    "target/test-reports/integration_failure_triage.json";
pub(crate) const SUITE_INVENTORY_REPORT_PATH: &str =
    "target/test-reports/integration_suite_inventory.json";
pub(crate) const SEPARATOR_LINE_LENGTH: usize = 37;
