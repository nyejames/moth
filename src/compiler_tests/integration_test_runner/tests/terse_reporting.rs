//! Self-tests for the terse integration-run output formatter.
//!
//! WHAT: protects the compact one-line failure headers, panic labelling, diagnostic rendering
//!       and summary line produced by `format_terse_run_output`.
//! WHY: terse mode is the CI gate output, so its shape needs focused regression coverage.

use super::super::reporting::format_terse_run_output;
use super::super::types::{
    DiagnosticMatchMode, FailureExpectation, GoldenExpectation, RenderedOutputExpectation,
    SuccessExpectation, WarningExpectation,
};
use super::super::{
    BackendId, CaseExecutionResult, ExpectedOutcome, FailureKind, SummaryCounts, TestCaseSpec,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, OutputFile, Project};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{BuilderKind, CleanupPolicy, OutputOwner};
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_errors::RenderSourceContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityHandle, LocalSpan, SourceDatabase, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn success_case(case_id: &str, backend_id: BackendId) -> TestCaseSpec {
    TestCaseSpec {
        display_name: format!("{case_id} [{}]", backend_id.as_str()),
        case_id: case_id.to_owned(),
        manifest_relative_path: case_id.to_owned(),
        fixture_root: PathBuf::from("."),
        tags: vec!["integration".to_owned()],
        contract: None,
        role: None,
        backend_id,
        entry_path: PathBuf::from("input/@page.moth"),
        flags: Vec::new(),
        expected: ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: RenderedOutputExpectation::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    }
}

fn expected_failure_case(case_id: &str, backend_id: BackendId) -> TestCaseSpec {
    TestCaseSpec {
        display_name: format!("{case_id} [{}]", backend_id.as_str()),
        case_id: case_id.to_owned(),
        manifest_relative_path: case_id.to_owned(),
        fixture_root: PathBuf::from("."),
        tags: vec!["integration".to_owned()],
        contract: None,
        role: None,
        backend_id,
        entry_path: PathBuf::from("input/@page.moth"),
        flags: Vec::new(),
        expected: ExpectedOutcome::Failure(FailureExpectation {
            warnings: WarningExpectation::Forbid,
            message_contains: Vec::new(),
            diagnostic_codes: Vec::new(),
            diagnostic_assertions: Vec::new(),
            diagnostic_match: DiagnosticMatchMode::Contains,
            diagnostic_match_reason: None,
        }),
    }
}

/// Minimal valid `BuildResult` for formatter tests. The real pipeline always
/// produces a `BuildResult` when compilation succeeds, so a passed result
/// carries `build_result: Some(_)`. This fixture preserves that invariant
/// without coupling to semantic build pipeline internals.
fn minimal_build_result() -> BuildResult {
    let string_table = StringTable::new();
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: Vec::new(),
        string_table,
        source_database: None,
        warning_source_contexts: Vec::new(),
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

/// Valid passed result: a real successful compilation always carries a
/// `BuildResult`. This preserves the production invariant.
fn passed_result() -> CaseExecutionResult {
    CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: Some(minimal_build_result()),
        messages: None,
        failure_reason: None,
        failure_kind: None,
    }
}

/// Valid failed result with messages: when build fails, the runner carries
/// the `CompilerMessages` so diagnostics can be rendered.
fn failed_result(reason: &str, kind: FailureKind) -> CaseExecutionResult {
    let messages = error_and_warning_messages();
    CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: Some(reason.to_owned()),
        failure_kind: Some(kind),
    }
}

/// Valid unexpected success: the real runner carries `build_result: Some(_)`
/// when build succeeded but the case expected failure.
fn unexpected_success_result() -> CaseExecutionResult {
    CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: Some(minimal_build_result()),
        messages: None,
        failure_reason: Some(
            "Expected a compilation failure, but the case built successfully.".to_owned(),
        ),
        failure_kind: Some(FailureKind::ExpectationViolation),
    }
}

fn format_single_case(
    case: TestCaseSpec,
    result: CaseExecutionResult,
    show_warnings: bool,
) -> Vec<String> {
    let mut summary = SummaryCounts::default();
    summary.record(&case, &result);
    let mut result = result;
    result.finalize_build_result_report();
    format_terse_run_output(
        &[(case, result)],
        summary,
        Duration::from_secs(1),
        show_warnings,
    )
}

fn error_and_warning_messages() -> CompilerMessages {
    let string_table = StringTable::new();
    let error_diag = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Error,
        None,
        DiagnosticPayload::None,
    );
    let warning_diag = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        None,
        DiagnosticPayload::None,
    );

    CompilerMessages::from_diagnostics(vec![error_diag, warning_diag], string_table)
}

fn build_result_with_warning(
    warning: CompilerDiagnostic,
    string_table: StringTable,
) -> BuildResult {
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: vec![warning],
        string_table,
        source_database: None,
        warning_source_contexts: Vec::new(),
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

#[test]
fn terse_build_result_warning_uses_donor_context_after_terminal_finalization() {
    let project_temp = tempfile::tempdir().expect("should create project temp dir");
    let package_temp = tempfile::tempdir().expect("should create package temp dir");
    let project_root = project_temp.path().join("src");
    let package_root = package_temp.path().join("src");
    fs::create_dir_all(&project_root).expect("should create project source root");
    fs::create_dir_all(&package_root).expect("should create package source root");

    let project_path = project_root.join("@project.moth");
    let package_path = package_root.join("@package.moth");
    let project_text = "project_snapshot = 1\n";
    let package_text = "package_snapshot = 1\n";
    fs::write(&project_path, project_text).expect("should write project source");
    fs::write(&package_path, package_text).expect("should write package source");

    let mut string_table = StringTable::new();
    let mut project_database = SourceDatabase::build(
        std::iter::once(project_path.as_path()),
        &project_root,
        None,
        &mut string_table,
    )
    .expect("project source identity should build");
    let project_source_id = project_database
        .get_by_canonical_path(&project_path)
        .expect("project source should be registered")
        .id;
    project_database
        .retain_text(project_source_id, project_text.to_owned())
        .expect("project snapshot should be retained");

    let mut package_database = SourceDatabase::build(
        std::iter::once(package_path.as_path()),
        &package_root,
        None,
        &mut string_table,
    )
    .expect("package source identity should build");
    let package_source_id = package_database
        .get_by_canonical_path(&package_path)
        .expect("package source should be registered")
        .id;
    package_database
        .retain_text(package_source_id, package_text.to_owned())
        .expect("package snapshot should be retained");
    assert_eq!(
        project_source_id, package_source_id,
        "independent source databases should have colliding SourceId values"
    );

    let package_identity = StablePackageIdentity::source_package(PackageOrigin::Builder, "pkg");
    let package_database = Arc::new(package_database);
    let project_database = Arc::new(project_database);
    let mut span_builder = ExtendedSpanBuilder::new();
    let local_span =
        LocalSpan::exact(0, 1, &mut span_builder).expect("warning span should fit inline");
    let warning = CompilerDiagnostic::unreachable_match_arm(Some(SourceSpan::new(
        package_source_id,
        local_span,
    )))
    .with_primary_frozen_identity_handle(FrozenIdentityHandle::for_domain(
        package_identity.clone(),
    ));

    let mut build_result = minimal_build_result();
    build_result.warnings = vec![warning];
    build_result.string_table = string_table;
    build_result.warning_source_contexts = vec![RenderSourceContext {
        diagnostic_range: 0..0,
        source_database: Arc::clone(&package_database),
        domain: Some(package_identity),
    }];
    build_result.source_database = Some(Arc::clone(&project_database));
    drop(package_database);
    drop(project_database);

    let case = expected_failure_case("unexpected_donor_warning", BackendId::Html);
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: Some(build_result),
        messages: None,
        failure_reason: Some(
            "Expected a compilation failure, but the case built successfully.".to_owned(),
        ),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let output = format_single_case(case, result, true);
    let warning_line = output
        .iter()
        .find(|line| line.starts_with("W|"))
        .expect("terminal reporter should render the warning");
    assert!(
        warning_line.contains("@package.moth"),
        "warning should render the donor package path: {warning_line}"
    );
    assert!(
        !warning_line.contains("@project.moth"),
        "warning must not render the requester project path: {warning_line}"
    );
}

#[test]
fn terse_all_correct_produces_one_summary_line() {
    let case = success_case("arithmetic_operator_precedence", BackendId::Html);
    let output = format_single_case(case, passed_result(), false);

    assert_eq!(output, vec!["Tests: 1/1 correct in 1.00s.".to_owned()]);
}

#[test]
fn terse_expected_success_failure_shows_fail_header() {
    let case = success_case("arithmetic_operator_precedence", BackendId::Html);
    let result = failed_result(
        "Expected a successful build, but compilation failed.",
        FailureKind::ExpectationViolation,
    );
    let output = format_single_case(case, result, false);

    assert_eq!(
        output[0],
        "FAIL arithmetic_operator_precedence [html] [expectation violation]: Expected a successful build, but compilation failed."
    );
    assert_eq!(
        output.last().unwrap(),
        "Tests: 0/1 correct, 1 incorrect in 1.00s."
    );
}

#[test]
fn terse_expected_failure_success_shows_unexpected_success_header() {
    let case = expected_failure_case("invalid_assignment", BackendId::Html);
    let output = format_single_case(case, unexpected_success_result(), false);

    assert_eq!(
        output[0],
        "UNEXPECTED SUCCESS invalid_assignment [html] [expectation violation]: Expected a compilation failure, but the case built successfully."
    );
    assert!(
        !output[0].contains("E|"),
        "no invented diagnostic should appear"
    );
    assert_eq!(
        output.last().unwrap(),
        "Tests: 0/1 correct, 1 incorrect in 1.00s."
    );
}

#[test]
fn terse_compiler_messages_render_error_before_warning_when_show_warnings() {
    let case = success_case("diagnostics_case", BackendId::Html);
    let messages = error_and_warning_messages();
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: Some("Compilation failed.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let output = format_single_case(case, result, true);

    let error_line_index = output
        .iter()
        .position(|line| line.starts_with("E|"))
        .expect("an error line should appear");
    let warning_line_index = output
        .iter()
        .position(|line| line.starts_with("W|"))
        .expect("a warning line should appear");
    assert!(
        error_line_index < warning_line_index,
        "error should appear before warning: {:?}",
        output
    );
    assert!(
        output.last().unwrap().starts_with("Tests:"),
        "summary should be the final line: {:?}",
        output
    );
}

#[test]
fn terse_compiler_messages_suppress_warnings_when_show_warnings_false() {
    let case = success_case("warnings_case", BackendId::Html);
    let messages = error_and_warning_messages();
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: Some("Compilation failed.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let output = format_single_case(case, result, false);

    assert!(
        output.iter().any(|line| line.starts_with("E|")),
        "error should remain: {:?}",
        output
    );
    assert!(
        !output.iter().any(|line| line.starts_with("W|")),
        "no warning line should appear: {:?}",
        output
    );
}

#[test]
fn terse_build_result_warnings_render_when_show_warnings() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("unused_value");
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnusedVariable),
        DiagnosticSeverity::Warning,
        None,
        DiagnosticPayload::UnusedName { name },
    );
    let build_result = build_result_with_warning(warning, string_table);

    let case = expected_failure_case("unexpected_build", BackendId::Html);
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: Some(build_result),
        messages: None,
        failure_reason: Some(
            "Expected a compilation failure, but the case built successfully.".to_owned(),
        ),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let output = format_single_case(case, result, true);

    assert!(
        output[0].starts_with("UNEXPECTED SUCCESS"),
        "header should be first: {:?}",
        output[0]
    );
    assert!(
        output.iter().any(|line| line.starts_with("W|")),
        "build-result warning should appear: {:?}",
        output
    );
    assert!(
        output.last().unwrap().starts_with("Tests:"),
        "summary should be last: {:?}",
        output
    );
}

#[test]
fn terse_panic_message_is_labelled_single_line() {
    let case = success_case("panic_case", BackendId::Html);
    let result = CaseExecutionResult {
        passed: false,
        panic_message: Some("index out of bounds\n  at line 42\n  in function foo".to_owned()),
        build_result: None,
        messages: None,
        failure_reason: Some("The compiler panicked.".to_owned()),
        failure_kind: Some(FailureKind::HarnessFailed),
    };
    let output = format_single_case(case, result, false);

    assert!(
        output[0].contains("harness error"),
        "should show harness error kind: {}",
        output[0]
    );
    let panic_line = output
        .iter()
        .find(|line| line.starts_with("panic:"))
        .expect("a labelled panic line should appear");
    assert_eq!(
        panic_line,
        "panic: index out of bounds at line 42 in function foo"
    );
    assert!(
        !panic_line.contains('\n'),
        "panic text should be single line: {:?}",
        panic_line
    );
    assert!(
        !panic_line.contains('\x1b'),
        "panic line should have no ANSI codes: {:?}",
        panic_line
    );
    assert_eq!(
        output.last().unwrap(),
        "Tests: 0/1 correct, 1 incorrect in 1.00s."
    );
}

#[test]
fn terse_output_has_no_ansi_or_separators() {
    let case = success_case("ansi_test", BackendId::Html);
    let output = format_single_case(case, passed_result(), false);

    let joined = output.join(" ");
    assert!(
        !joined.contains('\x1b'),
        "no ANSI escape codes: {:?}",
        joined
    );
    assert!(!joined.contains("==="), "no separator rules: {:?}", joined);
    assert!(!joined.contains("---"), "no separator rules: {:?}", joined);
}

#[test]
fn terse_mixed_results_preserve_case_order_and_summary() {
    let case_a = success_case("case_a", BackendId::Html);
    let case_b = success_case("case_b", BackendId::Html);
    let result_a = failed_result("failed.", FailureKind::ExpectationViolation);
    let result_b = passed_result();

    let mut summary = SummaryCounts::default();
    summary.record(&case_a, &result_a);
    summary.record(&case_b, &result_b);
    let output = format_terse_run_output(
        &[(case_a, result_a), (case_b, result_b)],
        summary,
        Duration::from_secs(1),
        false,
    );

    assert_eq!(
        output.len(),
        3,
        "one failure header + one error line + one summary: {:?}",
        output
    );
    assert!(
        output[0].starts_with("FAIL case_a"),
        "first line is failure: {}",
        output[0]
    );
    assert_eq!(output[2], "Tests: 1/2 correct, 1 incorrect in 1.00s.");
}
