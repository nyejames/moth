//! Integration test runner type definitions.
//!
//! WHAT: defines the data types used by the integration test harness.

use crate::build_system::build::BuildResult;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(crate) const SUPPORTED_BACKEND_IDS: &[&str] = &["html", "html_wasm"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TestRunnerOptions {
    pub show_warnings: bool,
    pub case_id: Option<String>,
    pub tag_filters: Vec<String>,
    pub contract: Option<String>,
    pub backend_filter: Option<BackendId>,
    pub list: bool,
    pub audit: bool,
    pub terse: bool,
}

impl TestRunnerOptions {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.audit && (self.has_selection_filters() || self.list) {
            return Err(String::from(
                "Tests command --audit cannot be combined with --case, --tag, --contract, --backend, or --list.",
            ));
        }

        if self.terse && self.list {
            return Err(String::from(
                "Tests command --terse cannot be combined with --list.",
            ));
        }

        if self.terse && self.audit {
            return Err(String::from(
                "Tests command --terse cannot be combined with --audit.",
            ));
        }

        Ok(())
    }

    pub(crate) fn has_selection_filters(&self) -> bool {
        self.case_id.is_some()
            || !self.tag_filters.is_empty()
            || self.contract.is_some()
            || self.backend_filter.is_some()
    }
}

pub(crate) struct TestSuiteSpec {
    pub cases: Vec<TestCaseSpec>,
}

#[derive(Clone)]
pub(crate) struct TestCaseSpec {
    pub display_name: String,
    pub case_id: String,
    pub manifest_relative_path: String,
    pub fixture_root: PathBuf,
    pub tags: Vec<String>,
    pub contract: Option<String>,
    pub role: Option<CaseRole>,
    pub backend_id: BackendId,
    pub entry_path: PathBuf,
    pub flags: Vec<Flag>,
    pub expected: ExpectedOutcome,
}

#[derive(Clone)]
pub(crate) enum ExpectedOutcome {
    Success(SuccessExpectation),
    Failure(FailureExpectation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ExpectationMode {
    Success,
    Failure,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RenderedOutputExpectation {
    pub exact: Option<String>,
    pub contains: Vec<String>,
    pub not_contains: Vec<String>,
    pub contains_in_order: Vec<String>,
    pub contains_exactly_once: Vec<String>,
}

impl RenderedOutputExpectation {
    pub(crate) fn is_present(&self) -> bool {
        self.exact.is_some()
            || !self.contains.is_empty()
            || !self.not_contains.is_empty()
            || !self.contains_in_order.is_empty()
            || !self.contains_exactly_once.is_empty()
    }

    pub(crate) fn assertion_count(&self) -> usize {
        usize::from(self.exact.is_some())
            + self.contains.len()
            + self.not_contains.len()
            + self.contains_in_order.len()
            + self.contains_exactly_once.len()
    }
}

#[derive(Clone)]
pub(crate) struct SuccessExpectation {
    pub warnings: WarningExpectation,
    pub success_contract: Option<SuccessContract>,
    pub artifact_assertions: Vec<ArtifactAssertion>,
    pub golden: GoldenExpectation,
    pub rendered_output: RenderedOutputExpectation,
    pub artifacts_must_not_exist: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SuccessContract {
    AcceptanceOnly,
}

#[derive(Clone)]
pub(crate) struct FailureExpectation {
    pub warnings: WarningExpectation,
    pub message_contains: Vec<String>,
    pub diagnostic_codes: Vec<String>,
    pub diagnostic_assertions: Vec<DiagnosticAssertion>,
    pub diagnostic_match: DiagnosticMatchMode,
    pub diagnostic_match_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticAssertion {
    pub code: String,
    pub occurrence: usize,
    pub reason: Option<String>,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub count: Option<usize>,
    pub secondary_labels: Vec<SecondaryLabelAssertion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SecondaryLabelAssertion {
    pub occurrence: usize,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiagnosticMatchMode {
    Exact,
    Contains,
}

impl DiagnosticMatchMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Contains => "contains",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExactWarningExpectation {
    pub expected_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WarningExpectation {
    Ignore,
    Forbid,
    Exact(ExactWarningExpectation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactKind {
    Html,
    Js,
    Wasm,
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum GoldenMode {
    #[default]
    Strict,
    Normalized,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GoldenFileInventory {
    pub files: Vec<GoldenFile>,
}

impl GoldenFileInventory {
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoldenFile {
    pub relative_path: String,
    pub absolute_path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GoldenExpectation {
    pub inventory: GoldenFileInventory,
    pub mode: Option<GoldenMode>,
}

impl GoldenExpectation {
    pub(crate) fn is_present(&self) -> bool {
        !self.inventory.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureKind {
    StrictGoldenMismatch,
    NormalizedSemanticMismatch,
    RenderedOutputMismatch,
    RenderedOutputExactMismatch,
    RenderedOutputOrderMismatch,
    RenderedOutputMultiplicityMismatch,
    HarnessFailed,
    ExpectationViolation,
}

#[derive(Clone)]
pub(crate) struct ArtifactAssertion {
    pub path: String,
    pub kind: ArtifactKind,
    pub must_contain: Vec<String>,
    pub must_not_contain: Vec<String>,
    pub must_contain_in_order: Vec<String>,
    pub must_contain_exactly_once: Vec<String>,
    pub normalized_contains: Vec<String>,
    pub normalized_not_contains: Vec<String>,
    pub validate_wasm: bool,
    pub must_export: Vec<String>,
    pub must_import: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub(crate) enum BackendId {
    Html,
    HtmlWasm,
}

impl BackendId {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "html" => Ok(Self::Html),
            "html_wasm" => Ok(Self::HtmlWasm),
            other => Err(format!(
                "Unsupported backend '{other}'. Supported backends: {}",
                SUPPORTED_BACKEND_IDS.join(", ")
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::HtmlWasm => "html_wasm",
        }
    }

    pub(crate) fn has_universal_baseline(self) -> bool {
        matches!(self, Self::Html | Self::HtmlWasm)
    }

    pub(crate) fn default_flags(self) -> Vec<Flag> {
        match self {
            Self::Html => Vec::new(),
            Self::HtmlWasm => vec![Flag::HtmlWasm],
        }
    }
}

pub(crate) struct CaseExecutionResult {
    pub passed: bool,
    pub panic_message: Option<String>,
    pub build_result: Option<BuildResult>,
    pub messages: Option<CompilerMessages>,
    pub failure_reason: Option<String>,
    pub failure_kind: Option<FailureKind>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SummaryCounts {
    pub total_tests: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
    pub expected_failures: usize,
    pub unexpected_successes: usize,
}

impl SummaryCounts {
    pub fn record(&mut self, case: &TestCaseSpec, result: &CaseExecutionResult) {
        self.total_tests += 1;

        if result.passed {
            match case.expected {
                ExpectedOutcome::Success(_) => self.passed_tests += 1,
                ExpectedOutcome::Failure(_) => self.expected_failures += 1,
            }
        } else {
            match case.expected {
                ExpectedOutcome::Success(_) => self.failed_tests += 1,
                ExpectedOutcome::Failure(_) => self.unexpected_successes += 1,
            }
        }
    }

    pub fn correct_results(&self) -> usize {
        self.passed_tests + self.expected_failures
    }

    pub fn incorrect_results(&self) -> usize {
        self.failed_tests + self.unexpected_successes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrationRunSummary {
    pub total_tests: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
    pub expected_failures: usize,
    pub unexpected_successes: usize,
}

impl IntegrationRunSummary {
    pub fn incorrect_results(&self) -> usize {
        self.failed_tests + self.unexpected_successes
    }
}

impl From<SummaryCounts> for IntegrationRunSummary {
    fn from(value: SummaryCounts) -> Self {
        Self {
            total_tests: value.total_tests,
            passed_tests: value.passed_tests,
            failed_tests: value.failed_tests,
            expected_failures: value.expected_failures,
            unexpected_successes: value.unexpected_successes,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FailureTriageEntry {
    pub case: String,
    pub backend: String,
    pub expected_outcome: &'static str,
    pub failure_reason: String,
    pub failure_kind: Option<FailureKind>,
    pub panic_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FailureTriageReport {
    pub total_tests: usize,
    pub incorrect_results: usize,
    pub failures: Vec<FailureTriageEntry>,
}

pub(crate) struct ManifestCaseSpec {
    pub id: String,
    pub path: PathBuf,
    pub tags: Vec<String>,
    pub contract: Option<String>,
    pub role: Option<CaseRole>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CaseRole {
    Primary,
    Boundary,
    Backend,
    Adversarial,
    Smoke,
}

impl CaseRole {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "primary" => Ok(Self::Primary),
            "boundary" => Ok(Self::Boundary),
            "backend" => Ok(Self::Backend),
            "adversarial" => Ok(Self::Adversarial),
            "smoke" => Ok(Self::Smoke),
            other => Err(format!(
                "unknown role '{other}'; supported roles: primary, boundary, backend, adversarial, smoke"
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Boundary => "boundary",
            Self::Backend => "backend",
            Self::Adversarial => "adversarial",
            Self::Smoke => "smoke",
        }
    }
}

pub(crate) struct ParsedExpectationFile {
    pub entry: Option<String>,
    pub backend_expectations: Vec<ParsedBackendExpectation>,
}

pub(crate) struct ParsedBackendExpectation {
    pub backend_id: BackendId,
    pub flags: Vec<Flag>,
    pub mode: ExpectationMode,
    pub warnings: WarningExpectation,
    pub success_contract: Option<SuccessContract>,
    pub message_contains: Vec<String>,
    pub diagnostic_codes: Vec<String>,
    pub diagnostic_assertions: Vec<DiagnosticAssertion>,
    pub diagnostic_match: Option<DiagnosticMatchMode>,
    pub diagnostic_match_reason: Option<String>,
    pub artifact_assertions: Vec<ArtifactAssertion>,
    pub golden_mode: Option<GoldenMode>,
    pub rendered_output: RenderedOutputExpectation,
    pub artifacts_must_not_exist: Vec<String>,
}
