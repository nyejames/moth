//! Self-tests for runner policy enforcement boundaries.
//!
//! WHAT: protects audit persistence and pre-execution hard-policy rejection.
//! WHY: policy failures must be observable without compiling or executing a case.

use super::super::errors::TestRunnerErrorKind;
use super::super::runner::run_loaded_suite;
use super::super::types::GoldenExpectation;
use super::super::{
    BackendId, CaseExecutionResult, CaseRole, ExpectedOutcome, FailureKind, SuccessExpectation,
    TestCaseSpec, TestRunnerOptions, TestSuiteSpec, WarningExpectation,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

fn suite_with_case(role: Option<CaseRole>, contract: Option<&str>) -> TestSuiteSpec {
    TestSuiteSpec {
        cases: vec![TestCaseSpec {
            display_name: "policy_case [html]".to_owned(),
            case_id: "policy_case".to_owned(),
            manifest_relative_path: "policy_case".to_owned(),
            fixture_root: PathBuf::from("."),
            tags: vec!["integration".to_owned()],
            contract: contract.map(str::to_owned),
            role,
            backend_id: BackendId::Html,
            entry_path: PathBuf::from("input/@page.moth"),
            flags: Vec::new(),
            expected: ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Forbid,
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: super::super::types::RenderedOutputExpectation {
                    contains: vec!["policy-marker".to_owned()],
                    ..Default::default()
                },
                artifacts_must_not_exist: Vec::new(),
            }),
        }],
    }
}

/// Callback that panics if called. Tests in this file use audit mode, which returns
/// before execution, so the callback should never run. Panicking here catches a
/// regression where the runner accidentally calls the callback before policy rejection.
fn panic_if_called(_case: &TestCaseSpec) -> CaseExecutionResult {
    panic!("execution callback should not be called: audit mode returns before execution")
}

#[test]
fn audit_writes_hard_findings_before_returning_failure() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let report_path = root.join("inventory.json");
    let callback_called = AtomicBool::new(false);

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Primary), None),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        |case| {
            callback_called.store(true, Ordering::SeqCst);
            panic_if_called(case)
        },
        report_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        root.join("triage.json")
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    assert!(result.is_err());
    assert!(!callback_called.load(Ordering::SeqCst));
    let error = result.expect_err("audit should fail on a hard finding");
    assert_eq!(
        error.kind,
        TestRunnerErrorKind::SuitePolicy,
        "audit should fail at suite policy: {error}"
    );
    let report = fs::read_to_string(&report_path).expect("audit should write its report");
    let report_json: serde_json::Value =
        serde_json::from_str(&report).expect("audit report should be valid JSON");
    assert_eq!(
        report_json["hard_policy_violations"][0]["code"],
        "primary_missing_contract"
    );
}

#[test]
fn normal_and_list_execution_reject_hard_findings_before_callback() {
    for list in [false, true] {
        let callback_called = AtomicBool::new(false);
        let result = run_loaded_suite(
            suite_with_case(Some(CaseRole::Primary), None),
            TestRunnerOptions {
                list,
                ..TestRunnerOptions::default()
            },
            |case| {
                callback_called.store(true, Ordering::SeqCst);
                panic_if_called(case)
            },
            "target/test-reports/unused-policy-test.json",
            "target/test-reports/unused-policy-triage-test.json",
        );

        assert!(result.is_err());
        assert!(!callback_called.load(Ordering::SeqCst));
        let error = result.expect_err("hard findings should reject the run");
        assert_eq!(
            error.kind,
            TestRunnerErrorKind::SuitePolicy,
            "normal/list runs should fail at suite policy: {error}"
        );
    }
}

#[test]
fn advisory_findings_are_serialized_without_failing_audit() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let report_path = root.join("inventory.json");

    // A backend-only primary-less contract family produces an advisory without a hard
    // finding, so audit serializes advisories and still succeeds.
    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        panic_if_called,
        report_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        root.join("triage.json")
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    assert!(result.is_ok());
    let report = fs::read_to_string(&report_path).expect("audit should write its report");
    let report_json: serde_json::Value =
        serde_json::from_str(&report).expect("audit report should be valid JSON");
    assert_eq!(
        report_json["hard_policy_violations"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        report_json["advisory_findings"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        report_json["advisory_findings"][0]["code"],
        "primary_less_contract_backend_only"
    );
}

#[test]
fn contractless_smoke_case_passes_audit_without_findings() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let report_path = root.join("inventory.json");

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Smoke), None),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        panic_if_called,
        report_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        root.join("triage.json")
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    assert!(result.is_ok());
    let report = fs::read_to_string(&report_path).expect("audit should write its report");
    let report_json: serde_json::Value =
        serde_json::from_str(&report).expect("audit report should be valid JSON");
    assert_eq!(
        report_json["hard_policy_violations"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        report_json["advisory_findings"].as_array().map(Vec::len),
        Some(0)
    );
}

#[test]
fn triage_report_write_failure_returns_error() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let inventory_path = root.join("inventory.json");
    let triage_parent = root.join("triage-parent");
    fs::write(&triage_parent, "occupied").expect("should create triage path collision");
    let triage_path = triage_parent.join("triage.json");

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions::default(),
        |_| {
            let table = StringTable::new();
            let diagnostic = CompilerDiagnostic::with_severity(
                DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
                DiagnosticSeverity::Error,
                SourceLocation::default(),
                DiagnosticPayload::None,
            );
            let messages = CompilerMessages::from_diagnostics(vec![diagnostic], table);
            CaseExecutionResult {
                passed: false,
                panic_message: None,
                build_result: None,
                messages: Some(messages),
                failure_reason: Some("forced test failure".to_owned()),
                failure_kind: Some(FailureKind::ExpectationViolation),
            }
        },
        inventory_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        triage_path
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    let error = result.expect_err("triage report write failure should be returned");
    assert_eq!(
        error.kind,
        TestRunnerErrorKind::TriageReport,
        "unexpected error kind: {:?} ({error})",
        error.kind
    );
    assert!(
        error.message.contains("Failed to create"),
        "unexpected error: {error}"
    );
}

#[test]
fn the_triage_report_says_the_run_is_incomplete_until_execution_finishes() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let inventory_path = root.join("inventory.json");
    let triage_path = root.join("triage.json");
    let observed_during_execution = triage_path.clone();

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions {
            terse: true,
            ..TestRunnerOptions::default()
        },
        move |_| {
            // Read the report from inside execution: this is exactly the window in which an
            // interrupted run would otherwise leave the previous run's result standing.
            let during = fs::read_to_string(&observed_during_execution)
                .expect("a started triage report should exist before execution");
            let during_json: serde_json::Value =
                serde_json::from_str(&during).expect("the started report should be valid JSON");
            assert_eq!(during_json["run"]["completed"], false);
            assert_eq!(during_json["total_tests"], 0);

            CaseExecutionResult {
                passed: true,
                panic_message: None,
                build_result: None,
                messages: None,
                failure_reason: None,
                failure_kind: None,
            }
        },
        inventory_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        triage_path
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    result.expect("the run should succeed");

    let after = fs::read_to_string(&triage_path).expect("the run should write its triage report");
    let after_json: serde_json::Value =
        serde_json::from_str(&after).expect("the final report should be valid JSON");
    assert_eq!(after_json["run"]["completed"], true);
    assert_eq!(after_json["schema_version"], 1);
    assert_eq!(after_json["total_tests"], 1);
    assert_eq!(after_json["run"]["command"], "tests");
}

#[test]
fn a_written_report_leaves_no_partial_file_beside_it() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let report_path = root.join("inventory.json");

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        panic_if_called,
        report_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        root.join("triage.json")
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    result.expect("the audit should succeed");

    let mut entries: Vec<String> = fs::read_dir(&root)
        .expect("the report directory should be readable")
        .map(|entry| {
            entry
                .expect("the entry should be readable")
                .file_name()
                .to_str()
                .expect("temporary names are UTF-8")
                .to_owned()
        })
        .collect();
    entries.sort();

    assert_eq!(entries, vec!["inventory.json".to_string()]);
}

#[test]
fn the_audit_report_records_the_run_and_the_repository_revision() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let report_path = root.join("inventory.json");

    run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        panic_if_called,
        report_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        root.join("triage.json")
            .to_str()
            .expect("temporary path should be UTF-8"),
    )
    .expect("the audit should succeed");

    let report = fs::read_to_string(&report_path).expect("audit should write its report");
    let json: serde_json::Value =
        serde_json::from_str(&report).expect("audit report should be valid JSON");

    assert_eq!(json["schema_version"], 8);
    assert_eq!(json["run"]["command"], "tests --audit");
    assert_eq!(json["run"]["completed"], true);
    assert_eq!(json["run"]["os"], std::env::consts::OS);
    assert_eq!(json["run"]["arch"], std::env::consts::ARCH);
    assert!(
        json["run"]["id"]
            .as_str()
            .is_some_and(|id| id.contains('-')),
        "the run identity should name a run: {}",
        json["run"]
    );
    // The suite runs inside this repository, so discovery must produce a revision rather than
    // the null a discarded Git failure used to leave behind.
    assert!(
        json["repository_revision"]["commit"].is_string(),
        "unexpected revision: {}",
        json["repository_revision"]
    );
}

#[test]
fn terse_with_list_rejected_before_callback() {
    let callback_called = AtomicBool::new(false);
    let result = run_loaded_suite(
        suite_with_case(None, None),
        TestRunnerOptions {
            terse: true,
            list: true,
            ..TestRunnerOptions::default()
        },
        |case| {
            callback_called.store(true, Ordering::SeqCst);
            panic_if_called(case)
        },
        "target/test-reports/unused-terse-list.json",
        "target/test-reports/unused-terse-list-triage.json",
    );

    assert!(result.is_err());
    assert!(!callback_called.load(Ordering::SeqCst));
    let error = result.expect_err("--terse + --list should be rejected");
    assert_eq!(
        error.kind,
        TestRunnerErrorKind::Options,
        "terse+list should fail option validation: {error}"
    );
}

#[test]
fn terse_with_audit_rejected_before_callback() {
    let callback_called = AtomicBool::new(false);
    let result = run_loaded_suite(
        suite_with_case(None, None),
        TestRunnerOptions {
            terse: true,
            audit: true,
            ..TestRunnerOptions::default()
        },
        |case| {
            callback_called.store(true, Ordering::SeqCst);
            panic_if_called(case)
        },
        "target/test-reports/unused-terse-audit.json",
        "target/test-reports/unused-terse-audit-triage.json",
    );

    assert!(result.is_err());
    assert!(!callback_called.load(Ordering::SeqCst));
    let error = result.expect_err("--terse + --audit should be rejected");
    assert_eq!(
        error.kind,
        TestRunnerErrorKind::Options,
        "terse+audit should fail option validation: {error}"
    );
}
