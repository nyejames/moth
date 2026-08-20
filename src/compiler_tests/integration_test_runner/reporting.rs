//! Terminal output and triage report writing for the integration test suite.
//!
//! WHAT: renders case results, writes machine-readable triage/inventory reports, and owns their
//!       stable output shapes.
//! WHY: keeping reporting here means the runner only coordinates loading, selection and execution.

use super::types::{DiagnosticMatchMode, SuccessContract};
use super::{
    BackendId, CaseExecutionResult, CaseRole, ExpectedOutcome, FailureExpectation, FailureKind,
    FailureTriageEntry, FailureTriageReport, SEPARATOR_LINE_LENGTH, SuccessExpectation,
    SummaryCounts, TestCaseSpec, WarningExpectation,
};
use super::{PolicyEvaluation, PolicyFinding};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terminal, terse,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticSeverity,
};
use saying::say;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs::{self, File};
use std::io::Write as IoWrite;
use std::path::Path;
use std::process;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SUITE_INVENTORY_SCHEMA_VERSION: u32 = 8;
const FAILURE_TRIAGE_SCHEMA_VERSION: u32 = 1;

/// What a report knows about the repository revision it describes.
///
/// A failed discovery and a repository that genuinely has nothing to report are different facts.
/// Collapsing both into `null` makes a report from outside a checkout look like a clean one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepositoryRevision {
    /// The revision Git reported.
    Commit(String),
    /// The command did not run inside a Git repository, so there is no revision.
    NotARepository,
    /// Discovery failed. The reason is kept so this is not read as a clean absence.
    Unknown { reason: String },
}

/// Identity of the run that produced a report.
///
/// `id` exists to tell two runs apart, not to order them: it pairs the process id with the
/// wall-clock nanoseconds at capture, which is enough for that and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RunIdentity {
    pub id: String,
    pub command: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub features: Vec<&'static str>,
    /// Runner thread count when the run chose one, or `None` for default parallelism.
    pub thread_count: Option<usize>,
    /// Whether the run that owns this report reached the end of the work the report describes.
    pub completed: bool,
}

impl RunIdentity {
    /// Capture the identity of a run that has started but not finished.
    pub(crate) fn started(command: &str, thread_count: Option<usize>) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());

        Self {
            id: format!("{:x}-{nanos:x}", process::id()),
            command: command.to_owned(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            features: crate::ENABLED_FEATURES.to_vec(),
            thread_count,
            completed: false,
        }
    }

    /// The same identity, marked as describing finished work.
    pub(crate) fn completed(&self) -> Self {
        Self {
            completed: true,
            ..self.clone()
        }
    }
}

pub(crate) fn format_case_listing(cases: &[TestCaseSpec]) -> String {
    if cases.is_empty() {
        return String::from("No test cases matched the selection filters.\n");
    }

    let mut listing = String::new();
    let mut index = 0;
    while index < cases.len() {
        let case = &cases[index];
        let case_id = &case.case_id;
        let _ = writeln!(listing, "case_id: {case_id}");
        let _ = writeln!(listing, "  backends:");

        while index < cases.len() && cases[index].case_id == *case_id {
            let backend_case = &cases[index];
            let _ = writeln!(
                listing,
                "    - {} ({})",
                backend_case.backend_id.as_str(),
                expected_outcome_label(&backend_case.expected)
            );
            index += 1;
        }

        let _ = writeln!(
            listing,
            "  tags: {}",
            if case.tags.is_empty() {
                "<none>".to_string()
            } else {
                case.tags.join(", ")
            }
        );
        let _ = writeln!(
            listing,
            "  contract: {}",
            case.contract.as_deref().unwrap_or("<none>")
        );
        let _ = writeln!(
            listing,
            "  role: {}\n",
            case.role.map_or("<none>", |role| role.as_str())
        );
    }

    listing
}

/// Stable machine-readable inventory for the canonical integration suite.
///
/// WHAT: records manifest metadata and the current typed expectation facts without executing a
///       case.
/// WHY: audit output is a review input for later policy phases, not a second test runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SuiteInventoryReport {
    pub schema_version: u32,
    pub run: RunIdentity,
    pub repository_revision: RepositoryRevision,
    pub manifest_case_count: usize,
    pub expanded_backend_execution_count: usize,
    pub summary: InventorySummary,
    pub cases: Vec<InventoryCase>,
    pub hard_policy_violations: Vec<PolicyFinding>,
    pub advisory_findings: Vec<PolicyFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InventorySummary {
    pub acceptance_only_backend_blocks: usize,
    // Fixture loading rejects baseline-only success backends before policy and reporting. Keep
    // this canonical audit invariant visible without reimplementing completeness classification.
    pub baseline_only_backend_blocks: usize,
    /// Cases whose every backend is acceptance-only. Legal, and counted so a reviewer sees how
    /// much of the suite claims only that compilation succeeded.
    pub smoke_role_cases: usize,
    /// Backend blocks that made warnings non-contractual. Legal, and counted because an
    /// unnecessary `ignore` silently accepts every future warning on that case.
    pub warning_ignore_backend_blocks: usize,
    /// Failure blocks that accept diagnostics beyond the authored multiset. Legal with an
    /// authored reason, and counted because a stale reason keeps the weaker contract alive.
    pub diagnostic_contains_backend_blocks: usize,
    /// Backend blocks carrying at least one weak-contract review reason.
    pub weak_contract_review_backend_blocks: usize,
    pub rendered_output_backend_blocks: usize,
    pub rendered_output_exact_backend_blocks: usize,
    pub rendered_output_order_backend_blocks: usize,
    pub rendered_output_exactly_once_backend_blocks: usize,
    pub artifact_backend_blocks: usize,
    pub golden_backend_blocks: usize,
    pub absence_backend_blocks: usize,
    pub expected_warning_backend_blocks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InventoryCase {
    pub canonical_id: String,
    pub manifest_relative_path: String,
    pub tags: Vec<String>,
    pub contract: Option<String>,
    pub role: Option<CaseRole>,
    pub backends: Vec<InventoryBackend>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InventoryBackend {
    pub backend: String,
    pub mode: &'static str,
    pub baseline_applied: bool,
    pub acceptance_only: bool,
    pub warning_mode: &'static str,
    pub warning_codes: Option<Vec<String>>,
    pub diagnostic_match: Option<DiagnosticMatchMode>,
    pub diagnostic_match_reason: Option<String>,
    pub structured_diagnostic_assertions: bool,
    pub assertion_kinds: Vec<&'static str>,
    /// Why this block is worth a weak-contract review, empty when nothing applies.
    ///
    /// These are review prompts, not policy violations: acceptance-only smoke, ignored warnings
    /// and justified contains-matching are all legal. Hard policy stays in the suite policy
    /// evaluator; this field only makes the weak contracts findable in one pass.
    pub weak_contract_reviews: Vec<&'static str>,
    pub golden_mode: Option<&'static str>,
    pub golden_present: bool,
    pub artifact_assertion_count: usize,
    pub rendered_output_assertion_count: usize,
    pub rendered_output_exact: bool,
    pub rendered_output_contains_count: usize,
    pub rendered_output_not_contains_count: usize,
    pub rendered_output_contains_in_order_count: usize,
    pub rendered_output_contains_exactly_once_count: usize,
    pub artifact_absence_assertion_count: usize,
}

pub(crate) fn build_suite_inventory_report(
    cases: &[TestCaseSpec],
    policy_evaluation: &PolicyEvaluation,
    run: &RunIdentity,
    repository_revision: RepositoryRevision,
) -> SuiteInventoryReport {
    let mut inventory_cases = Vec::<InventoryCase>::new();

    for case in cases {
        if let Some(inventory_case) = inventory_cases.last_mut()
            && inventory_case.canonical_id == case.case_id
        {
            inventory_case.backends.push(build_backend_inventory(case));
            continue;
        }

        inventory_cases.push(InventoryCase {
            canonical_id: case.case_id.clone(),
            manifest_relative_path: case.manifest_relative_path.clone(),
            tags: case.tags.clone(),
            contract: case.contract.clone(),
            role: case.role,
            backends: vec![build_backend_inventory(case)],
        });
    }

    SuiteInventoryReport {
        schema_version: SUITE_INVENTORY_SCHEMA_VERSION,
        run: run.completed(),
        repository_revision,
        manifest_case_count: inventory_cases.len(),
        expanded_backend_execution_count: cases.len(),
        summary: build_inventory_summary(&inventory_cases),
        cases: inventory_cases,
        hard_policy_violations: policy_evaluation.hard_findings.clone(),
        advisory_findings: policy_evaluation.advisories.clone(),
    }
}

fn build_inventory_summary(cases: &[InventoryCase]) -> InventorySummary {
    let mut summary = InventorySummary {
        acceptance_only_backend_blocks: 0,
        baseline_only_backend_blocks: 0,
        smoke_role_cases: cases
            .iter()
            .filter(|case| case.role == Some(CaseRole::Smoke))
            .count(),
        warning_ignore_backend_blocks: 0,
        diagnostic_contains_backend_blocks: 0,
        weak_contract_review_backend_blocks: 0,
        rendered_output_backend_blocks: 0,
        rendered_output_exact_backend_blocks: 0,
        rendered_output_order_backend_blocks: 0,
        rendered_output_exactly_once_backend_blocks: 0,
        artifact_backend_blocks: 0,
        golden_backend_blocks: 0,
        absence_backend_blocks: 0,
        expected_warning_backend_blocks: 0,
    };

    for backend in cases.iter().flat_map(|case| &case.backends) {
        let has_rendered_output = backend.rendered_output_assertion_count > 0;
        let has_artifacts = backend.artifact_assertion_count > 0;
        let has_golden = backend.golden_present;
        let has_absence = backend.artifact_absence_assertion_count > 0;
        let has_expected_warning = backend.assertion_kinds.contains(&"expected_warning");
        let ignores_warnings = backend.weak_contract_reviews.contains(&"warnings_ignored");

        if backend.acceptance_only {
            summary.acceptance_only_backend_blocks += 1;
        }
        if ignores_warnings {
            summary.warning_ignore_backend_blocks += 1;
        }
        if backend.diagnostic_match == Some(DiagnosticMatchMode::Contains) {
            summary.diagnostic_contains_backend_blocks += 1;
        }
        if !backend.weak_contract_reviews.is_empty() {
            summary.weak_contract_review_backend_blocks += 1;
        }
        if has_rendered_output {
            summary.rendered_output_backend_blocks += 1;
        }
        if backend.rendered_output_exact {
            summary.rendered_output_exact_backend_blocks += 1;
        }
        if backend.rendered_output_contains_in_order_count > 0 {
            summary.rendered_output_order_backend_blocks += 1;
        }
        if backend.rendered_output_contains_exactly_once_count > 0 {
            summary.rendered_output_exactly_once_backend_blocks += 1;
        }
        if has_artifacts {
            summary.artifact_backend_blocks += 1;
        }
        if has_golden {
            summary.golden_backend_blocks += 1;
        }
        if has_absence {
            summary.absence_backend_blocks += 1;
        }
        if has_expected_warning {
            summary.expected_warning_backend_blocks += 1;
        }
    }

    summary
}

fn build_backend_inventory(case: &TestCaseSpec) -> InventoryBackend {
    match &case.expected {
        ExpectedOutcome::Success(expectation) => InventoryBackend {
            backend: case.backend_id.as_str().to_owned(),
            mode: "success",
            baseline_applied: case.backend_id.has_universal_baseline(),
            acceptance_only: expectation.success_contract == Some(SuccessContract::AcceptanceOnly),
            warning_mode: warning_mode_label(&expectation.warnings),
            warning_codes: warning_codes(&expectation.warnings),
            diagnostic_match: None,
            diagnostic_match_reason: None,
            structured_diagnostic_assertions: false,
            assertion_kinds: success_assertion_kinds(case, expectation),
            weak_contract_reviews: success_weak_contract_reviews(expectation),
            golden_mode: expectation.golden.mode.map(golden_mode_label),
            golden_present: expectation.golden.is_present(),
            artifact_assertion_count: expectation.artifact_assertions.len(),
            rendered_output_assertion_count: expectation.rendered_output.assertion_count(),
            rendered_output_exact: expectation.rendered_output.exact.is_some(),
            rendered_output_contains_count: expectation.rendered_output.contains.len(),
            rendered_output_not_contains_count: expectation.rendered_output.not_contains.len(),
            rendered_output_contains_in_order_count: expectation
                .rendered_output
                .contains_in_order
                .len(),
            rendered_output_contains_exactly_once_count: expectation
                .rendered_output
                .contains_exactly_once
                .len(),
            artifact_absence_assertion_count: expectation.artifacts_must_not_exist.len(),
        },
        ExpectedOutcome::Failure(expectation) => InventoryBackend {
            backend: case.backend_id.as_str().to_owned(),
            mode: "failure",
            baseline_applied: false,
            acceptance_only: false,
            warning_mode: warning_mode_label(&expectation.warnings),
            warning_codes: warning_codes(&expectation.warnings),
            diagnostic_match: Some(expectation.diagnostic_match),
            diagnostic_match_reason: expectation.diagnostic_match_reason.clone(),
            structured_diagnostic_assertions: !expectation.diagnostic_assertions.is_empty(),
            assertion_kinds: failure_assertion_kinds(expectation),
            weak_contract_reviews: failure_weak_contract_reviews(expectation),
            golden_mode: None,
            golden_present: false,
            artifact_assertion_count: 0,
            rendered_output_assertion_count: 0,
            rendered_output_exact: false,
            rendered_output_contains_count: 0,
            rendered_output_not_contains_count: 0,
            rendered_output_contains_in_order_count: 0,
            rendered_output_contains_exactly_once_count: 0,
            artifact_absence_assertion_count: 0,
        },
    }
}

fn success_assertion_kinds(
    case: &TestCaseSpec,
    expectation: &SuccessExpectation,
) -> Vec<&'static str> {
    let mut kinds = Vec::new();

    if case.backend_id.has_universal_baseline() {
        kinds.push("backend_baseline");
    }
    if expectation.success_contract == Some(SuccessContract::AcceptanceOnly) {
        kinds.push("acceptance_only");
    }

    if !expectation.artifact_assertions.is_empty() {
        kinds.push("artifact_assertions");
    }
    if expectation.golden.is_present() {
        kinds.push("golden");
    }
    if expectation.rendered_output.is_present() {
        kinds.push("rendered_output");
    }
    if expectation.rendered_output.exact.is_some() {
        kinds.push("rendered_output_exact");
    }
    if !expectation.rendered_output.contains.is_empty() {
        kinds.push("rendered_output_contains");
    }
    if !expectation.rendered_output.not_contains.is_empty() {
        kinds.push("rendered_output_not_contains");
    }
    if !expectation.rendered_output.contains_in_order.is_empty() {
        kinds.push("rendered_output_contains_in_order");
    }
    if !expectation.rendered_output.contains_exactly_once.is_empty() {
        kinds.push("rendered_output_contains_exactly_once");
    }
    if !expectation.artifacts_must_not_exist.is_empty() {
        kinds.push("artifact_absence");
    }
    if matches!(&expectation.warnings, WarningExpectation::Exact(_)) {
        kinds.push("expected_warning");
    }
    kinds
}

/// Weak-contract review reasons for one successful backend block.
///
/// Each reason names a contract that is legal but proves less than a case-specific assertion
/// would. Reporting them keeps the review list in one place instead of re-deriving it from the
/// field combinations on every audit.
fn success_weak_contract_reviews(expectation: &SuccessExpectation) -> Vec<&'static str> {
    let mut reviews = Vec::new();

    if expectation.success_contract == Some(SuccessContract::AcceptanceOnly) {
        reviews.push("acceptance_only_success");
    }
    if matches!(expectation.warnings, WarningExpectation::Ignore) {
        reviews.push("warnings_ignored");
    }

    reviews
}

/// Weak-contract review reasons for one failing backend block.
fn failure_weak_contract_reviews(expectation: &FailureExpectation) -> Vec<&'static str> {
    let mut reviews = Vec::new();

    if expectation.diagnostic_match == DiagnosticMatchMode::Contains {
        reviews.push("diagnostic_match_contains");
    }
    if matches!(expectation.warnings, WarningExpectation::Ignore) {
        reviews.push("warnings_ignored");
    }

    reviews
}

fn failure_assertion_kinds(expectation: &FailureExpectation) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if !expectation.diagnostic_codes.is_empty() {
        kinds.push("diagnostic_codes");
    }
    if !expectation.diagnostic_assertions.is_empty() {
        kinds.push("diagnostic_assertions");
    }
    if !expectation.message_contains.is_empty() {
        kinds.push("message_contains");
    }
    if matches!(&expectation.warnings, WarningExpectation::Exact(_)) {
        kinds.push("expected_warning");
    }
    kinds
}

fn warning_mode_label(expectation: &WarningExpectation) -> &'static str {
    match expectation {
        WarningExpectation::Ignore => "ignore",
        WarningExpectation::Forbid => "forbid",
        WarningExpectation::Exact(_) => "exact",
    }
}

fn warning_codes(expectation: &WarningExpectation) -> Option<Vec<String>> {
    match expectation {
        WarningExpectation::Exact(exact) => Some(exact.expected_codes.clone()),
        WarningExpectation::Ignore | WarningExpectation::Forbid => None,
    }
}

fn golden_mode_label(mode: super::GoldenMode) -> &'static str {
    match mode {
        super::GoldenMode::Strict => "strict",
        super::GoldenMode::Normalized => "normalized",
    }
}

/// Discovers the current repository revision without making audit depend on Git.
///
/// Every outcome is reported as itself. A discarded Git failure would put `null` in the report,
/// which reads as "there is no revision" rather than "the revision was never learned".
pub(crate) fn discover_repository_revision() -> RepositoryRevision {
    let output = match Command::new("git").args(["rev-parse", "HEAD"]).output() {
        Ok(output) => output,
        Err(error) => {
            return RepositoryRevision::Unknown {
                reason: format!("git could not be started: {error}"),
            };
        }
    };

    if !output.status.success() {
        return classify_revision_failure(&output.stderr);
    }

    match String::from_utf8(output.stdout) {
        Ok(text) => {
            let commit = text.trim().to_owned();
            if commit.is_empty() {
                RepositoryRevision::Unknown {
                    reason: "git rev-parse HEAD printed no revision".to_owned(),
                }
            } else {
                RepositoryRevision::Commit(commit)
            }
        }
        Err(error) => RepositoryRevision::Unknown {
            reason: format!("git rev-parse HEAD printed output that is not UTF-8: {error}"),
        },
    }
}

/// Classify a failed `git rev-parse HEAD` from what Git said about it.
pub(super) fn classify_revision_failure(stderr: &[u8]) -> RepositoryRevision {
    // This text is a message for a reader, never an assertion input. A replacement character
    // cannot spell the phrase below, so a lossy decode cannot change the classification.
    let described = String::from_utf8_lossy(stderr);
    let described = described.trim();

    if described.contains("not a git repository") {
        return RepositoryRevision::NotARepository;
    }

    RepositoryRevision::Unknown {
        reason: if described.is_empty() {
            "git rev-parse HEAD failed without a message".to_owned()
        } else {
            format!("git rev-parse HEAD failed: {described}")
        },
    }
}

pub(crate) fn write_suite_inventory_report(
    report_path_str: &str,
    report: &SuiteInventoryReport,
) -> Result<(), String> {
    let report_json =
        serde_json::to_string_pretty(report).map_err(|error| format!("JSON error: {error}"))?;

    write_report_atomically(Path::new(report_path_str), report_json.as_bytes())
        .map_err(|error| format!("Failed to write the suite inventory report: {error}"))
}

/// Mark an inventory report as belonging to a run that has started and not finished.
///
/// Written before the work begins, so an interrupted run leaves a report that says so instead of
/// the previous run's output, which a reader would take for this run's result.
pub(crate) fn write_started_suite_inventory_report(
    report_path_str: &str,
    run: &RunIdentity,
) -> Result<(), String> {
    write_suite_inventory_report(
        report_path_str,
        &SuiteInventoryReport {
            schema_version: SUITE_INVENTORY_SCHEMA_VERSION,
            run: run.clone(),
            repository_revision: RepositoryRevision::Unknown {
                reason: "the run had not reached revision discovery".to_owned(),
            },
            manifest_case_count: 0,
            expanded_backend_execution_count: 0,
            summary: build_inventory_summary(&[]),
            cases: Vec::new(),
            hard_policy_violations: Vec::new(),
            advisory_findings: Vec::new(),
        },
    )
}

/// Write `bytes` to `path` so a reader never observes a partial report.
///
/// The temporary file is a sibling of the final path so the rename stays on one filesystem; a
/// cross-filesystem rename is a copy, which is the non-atomic write this exists to avoid. A
/// failure after the temporary file exists removes it, leaving the previous report in place
/// rather than a partial new one beside it.
fn write_report_atomically(report_path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = report_path.parent().ok_or_else(|| {
        format!(
            "report path '{}' has no parent directory",
            report_path.display()
        )
    })?;

    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create '{}': {error}", parent.display()))?;

    let file_name = report_path
        .file_name()
        .ok_or_else(|| format!("report path '{}' has no file name", report_path.display()))?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(format!(".{}.partial", process::id()));
    let temporary_path = parent.join(temporary_name);

    if let Err(error) = write_and_flush(&temporary_path, bytes) {
        remove_partial_report(&temporary_path);
        return Err(error);
    }

    fs::rename(&temporary_path, report_path).map_err(|error| {
        remove_partial_report(&temporary_path);
        format!(
            "Failed to move '{}' onto '{}': {error}",
            temporary_path.display(),
            report_path.display()
        )
    })
}

fn write_and_flush(temporary_path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(temporary_path)
        .map_err(|error| format!("Failed to create '{}': {error}", temporary_path.display()))?;

    file.write_all(bytes)
        .map_err(|error| format!("Failed to write '{}': {error}", temporary_path.display()))?;

    file.sync_all()
        .map_err(|error| format!("Failed to flush '{}': {error}", temporary_path.display()))
}

/// Remove a temporary file after a failed write.
///
/// A removal failure is reported but never replaces the write failure that caused it: the caller
/// is already returning the reason the report was not written.
fn remove_partial_report(temporary_path: &Path) {
    if let Err(error) = fs::remove_file(temporary_path) {
        eprintln!(
            "warning: failed to remove the partial report '{}': {error}",
            temporary_path.display()
        );
    }
}

pub(crate) fn render_case_result(
    case: &TestCaseSpec,
    result: &CaseExecutionResult,
    show_warnings: bool,
) {
    match (&case.expected, result.passed) {
        (ExpectedOutcome::Success(_), true) => say!(Green "✓ PASS"),
        (ExpectedOutcome::Failure(_), true) => say!(Green "✓ EXPECTED FAILURE"),
        (ExpectedOutcome::Success(_), false) => say!(Red "✗ FAIL"),
        (ExpectedOutcome::Failure(_), false) => say!(Yellow "✗ UNEXPECTED SUCCESS"),
    }

    if let Some(kind) = result.failure_kind {
        say!(Dark White format!("[{}]", failure_kind_label(kind)));
    }

    if let Some(reason) = &result.failure_reason {
        say!(Red reason);
    }

    if let Some(panic_message) = &result.panic_message {
        say!(Red format!("panic: {panic_message}"));
    }

    if let Some(messages) = &result.messages {
        for (diagnostic_index, diagnostic) in messages
            .diagnostics()
            .enumerate()
            .filter(|(_, diagnostic)| diagnostic.severity == DiagnosticSeverity::Error)
        {
            if result.passed && matches!(case.expected, ExpectedOutcome::Failure(_)) {
                say!(Yellow diagnostic_summary_label(diagnostic));
                continue;
            }

            terminal::print_diagnostic_with_context(
                diagnostic,
                messages.diagnostic_render_context(diagnostic_index),
            );
        }

        if show_warnings {
            for (diagnostic_index, warning) in messages
                .diagnostics()
                .enumerate()
                .filter(|(_, diagnostic)| diagnostic.severity == DiagnosticSeverity::Warning)
            {
                terminal::print_diagnostic_with_context(
                    warning,
                    messages.diagnostic_render_context(diagnostic_index),
                );
            }
        }
    } else if let Some(build_result) = &result.build_result
        && show_warnings
    {
        for warning in &build_result.warnings {
            crate::compiler_frontend::compiler_messages::render::terminal::print_diagnostic(
                warning,
                &build_result.string_table,
            );
        }
    }
}

pub(crate) fn render_backend_summary(backend_summaries: &BTreeMap<BackendId, SummaryCounts>) {
    if backend_summaries.is_empty() {
        return;
    }

    say!("\n  Backend breakdown:");
    let rule = format!("  {}", "─".repeat(SEPARATOR_LINE_LENGTH - 2));
    say!(Dark White rule);
    for (backend_id, summary) in backend_summaries {
        let incorrect = summary.incorrect_results();
        if incorrect > 0 {
            say!(
                "    ", Cyan format!("{:<9}", backend_id.as_str()),
                Reset "  total: ", Yellow summary.total_tests,
                Reset "  passed: ", Blue summary.correct_results(),
                Reset "  failed: ", Red Bold incorrect
            );
        } else {
            say!(
                "    ", Cyan format!("{:<9}", backend_id.as_str()),
                Reset "  total: ", Yellow summary.total_tests,
                Reset "  passed: ", Green Bold summary.correct_results()
            );
        }
    }
}

pub(crate) fn format_pass_percentage(correct_results: usize, total_tests: usize) -> String {
    let correct_results =
        u128::try_from(correct_results).expect("usize values always fit into u128");
    let total_tests = u128::try_from(total_tests).expect("usize values always fit into u128");
    let scaled_tenths = (correct_results * 1_000) / total_tests;

    format!("{}.{}", scaled_tenths / 10, scaled_tenths % 10)
}

pub(crate) fn expected_outcome_label(expected: &ExpectedOutcome) -> &'static str {
    match expected {
        ExpectedOutcome::Success(_) => "success",
        ExpectedOutcome::Failure(_) => "failure",
    }
}

pub(crate) fn observed_failure_reason(result: &CaseExecutionResult) -> String {
    if let Some(messages) = &result.messages
        && let Some(first_diagnostic) = messages
            .diagnostics()
            .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        let base = result
            .failure_reason
            .as_deref()
            .unwrap_or("Compilation failed.");
        let diagnostic_index = messages
            .diagnostic_slice()
            .iter()
            .position(|diagnostic| std::ptr::eq(diagnostic, first_diagnostic))
            .unwrap_or(0);
        let terse_line = terse::format_terse_diagnostic_with_context(
            first_diagnostic,
            messages.diagnostic_render_context(diagnostic_index),
        );

        return format!("{base} First diagnostic: {terse_line}");
    }

    if let Some(reason) = &result.failure_reason {
        return reason.to_owned();
    }

    if let Some(panic_message) = &result.panic_message {
        return format!("Compiler panic: {panic_message}");
    }

    "No failure reason was recorded.".to_string()
}

fn diagnostic_summary_label(diagnostic: &CompilerDiagnostic) -> String {
    let descriptor = diagnostic.kind.descriptor();
    let category = match diagnostic.kind.category() {
        DiagnosticCategory::Syntax => "Syntax Error",
        DiagnosticCategory::Type => "Type Error",
        DiagnosticCategory::Rule
        | DiagnosticCategory::Import
        | DiagnosticCategory::DeferredFeature => "Language Rule Error",
        DiagnosticCategory::Borrow => "Borrow Checker Violation",
        DiagnosticCategory::Config => "Malformed Config",
        DiagnosticCategory::Infrastructure => "Infrastructure Failure",
    };

    format!("{category} [{}]", descriptor.code)
}

pub(crate) fn write_failure_triage_report(
    report_path_str: &str,
    run: &RunIdentity,
    summary: SummaryCounts,
    failures: &[FailureTriageEntry],
) -> Result<(), String> {
    write_triage_report(
        report_path_str,
        &FailureTriageReport {
            schema_version: FAILURE_TRIAGE_SCHEMA_VERSION,
            run: run.completed(),
            total_tests: summary.total_tests,
            incorrect_results: summary.incorrect_results(),
            failures: failures.to_vec(),
        },
    )
}

/// Mark a triage report as belonging to a run that has started and not finished.
///
/// Execution is the long part of a run, so this is where a killed process would otherwise leave
/// the previous run's passing triage report standing as if it described this one.
pub(crate) fn write_started_failure_triage_report(
    report_path_str: &str,
    run: &RunIdentity,
) -> Result<(), String> {
    write_triage_report(
        report_path_str,
        &FailureTriageReport {
            schema_version: FAILURE_TRIAGE_SCHEMA_VERSION,
            run: run.clone(),
            total_tests: 0,
            incorrect_results: 0,
            failures: Vec::new(),
        },
    )
}

fn write_triage_report(report_path_str: &str, report: &FailureTriageReport) -> Result<(), String> {
    let report_json =
        serde_json::to_string_pretty(report).map_err(|error| format!("JSON error: {error}"))?;

    write_report_atomically(Path::new(report_path_str), report_json.as_bytes())
        .map_err(|error| format!("Failed to write the failure triage report: {error}"))
}

fn failure_kind_label(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::StrictGoldenMismatch => "strict golden mismatch",
        FailureKind::NormalizedSemanticMismatch => "normalized mismatch",
        FailureKind::RenderedOutputMismatch => "rendered output mismatch",
        FailureKind::RenderedOutputExactMismatch => "rendered output exact mismatch",
        FailureKind::RenderedOutputOrderMismatch => "rendered output order mismatch",
        FailureKind::RenderedOutputMultiplicityMismatch => "rendered output multiplicity mismatch",
        FailureKind::HarnessFailed => "harness error",
        FailureKind::ExpectationViolation => "expectation violation",
    }
}

fn append_failure_kind(header: &mut String, failure_kind: Option<FailureKind>) {
    if let Some(failure_kind) = failure_kind {
        header.push_str(" [");
        header.push_str(failure_kind_label(failure_kind));
        header.push(']');
    }
}

fn compact_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn format_terse_run_output(
    case_results: &[(TestCaseSpec, CaseExecutionResult)],
    summary: SummaryCounts,
    duration: std::time::Duration,
    show_warnings: bool,
) -> Vec<String> {
    let mut lines = Vec::new();

    if summary.incorrect_results() > 0 {
        for (case, result) in case_results {
            if !result.passed {
                lines.extend(format_terse_failure_lines(case, result, show_warnings));
            }
        }
    }

    lines.push(format_terse_summary_line(summary, duration));
    lines
}

fn format_terse_failure_lines(
    case: &TestCaseSpec,
    result: &CaseExecutionResult,
    show_warnings: bool,
) -> Vec<String> {
    let mut lines = Vec::new();

    let header = match &case.expected {
        ExpectedOutcome::Success(_) => {
            let mut header = format!("FAIL {} [{}]", case.case_id, case.backend_id.as_str());
            append_failure_kind(&mut header, result.failure_kind);
            if let Some(reason) = &result.failure_reason {
                header.push_str(&format!(": {}", compact_text(reason)));
            }
            header
        }
        ExpectedOutcome::Failure(_) => {
            let mut header = format!(
                "UNEXPECTED SUCCESS {} [{}]",
                case.case_id,
                case.backend_id.as_str(),
            );
            append_failure_kind(&mut header, result.failure_kind);
            if let Some(reason) = &result.failure_reason {
                header.push_str(&format!(": {}", compact_text(reason)));
            }
            header
        }
    };
    lines.push(header);

    if let Some(panic_message) = &result.panic_message {
        lines.push(format!("panic: {}", compact_text(panic_message)));
    }

    if let Some(messages) = &result.messages {
        for diagnostic_index in messages.diagnostic_display_order() {
            let diagnostic = &messages.diagnostic_slice()[diagnostic_index];

            let should_render = match diagnostic.severity {
                DiagnosticSeverity::Error => true,
                DiagnosticSeverity::Warning => show_warnings,
                DiagnosticSeverity::Note => false,
            };

            if !should_render {
                continue;
            }

            lines.push(terse::format_terse_diagnostic_with_context(
                diagnostic,
                messages.diagnostic_render_context(diagnostic_index),
            ));
        }
    }

    if show_warnings && let Some(build_result) = &result.build_result {
        let render_context = DiagnosticRenderContext::new(&build_result.string_table);
        for warning in &build_result.warnings {
            lines.push(terse::format_terse_diagnostic_with_context(
                warning,
                render_context,
            ));
        }
    }

    lines
}

fn format_terse_summary_line(summary: SummaryCounts, duration: std::time::Duration) -> String {
    let incorrect = summary.incorrect_results();
    if incorrect == 0 {
        format!(
            "Tests: {}/{} correct in {:.2}s.",
            summary.correct_results(),
            summary.total_tests,
            duration.as_secs_f64(),
        )
    } else {
        format!(
            "Tests: {}/{} correct, {} incorrect in {:.2}s.",
            summary.correct_results(),
            summary.total_tests,
            incorrect,
            duration.as_secs_f64(),
        )
    }
}
