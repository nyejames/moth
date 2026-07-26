//! Self-tests for runner policy enforcement boundaries.
//!
//! WHAT: protects audit persistence and pre-execution hard-policy rejection.
//! WHY: policy failures must be observable without compiling or executing a case.

use super::super::runner::run_loaded_suite;
use super::super::types::GoldenExpectation;
use super::super::{
    BackendId, CaseExecutionResult, CaseRole, ExpectedOutcome, SuccessExpectation, TestCaseSpec,
    TestRunnerOptions, TestSuiteSpec, WarningExpectation,
};
use crate::compiler_tests::test_support::temp_dir;
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
            entry_path: PathBuf::from("input/#page.moth"),
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

fn successful_execution_result() -> CaseExecutionResult {
    CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: None,
        failure_kind: None,
    }
}

#[test]
fn audit_writes_hard_findings_before_returning_failure() {
    let root = temp_dir("runner_audit_hard_policy");
    fs::create_dir_all(&root).expect("should create temporary report directory");
    let report_path = root.join("inventory.json");
    let callback_called = AtomicBool::new(false);

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Primary), None),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        |_| {
            callback_called.store(true, Ordering::SeqCst);
            successful_execution_result()
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
    let report = fs::read_to_string(&report_path).expect("audit should write its report");
    let report_json: serde_json::Value =
        serde_json::from_str(&report).expect("audit report should be valid JSON");
    assert_eq!(
        report_json["hard_policy_violations"][0]["code"],
        "primary_missing_contract"
    );

    fs::remove_dir_all(&root).expect("should clean up temporary report directory");
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
            |_| {
                callback_called.store(true, Ordering::SeqCst);
                successful_execution_result()
            },
            "target/test-reports/unused-policy-test.json",
            "target/test-reports/unused-policy-triage-test.json",
        );

        assert!(result.is_err());
        assert!(!callback_called.load(Ordering::SeqCst));
    }
}

#[test]
fn advisory_findings_are_serialized_without_failing_audit() {
    let root = temp_dir("runner_audit_policy_advisory");
    fs::create_dir_all(&root).expect("should create temporary report directory");
    let report_path = root.join("inventory.json");

    // A backend-only primary-less contract family produces an advisory without a hard
    // finding, so audit serializes advisories and still succeeds.
    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        |_| successful_execution_result(),
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

    fs::remove_dir_all(&root).expect("should clean up temporary report directory");
}

#[test]
fn contractless_smoke_case_passes_audit_without_findings() {
    let root = temp_dir("runner_audit_contractless_smoke");
    fs::create_dir_all(&root).expect("should create temporary report directory");
    let report_path = root.join("inventory.json");

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Smoke), None),
        TestRunnerOptions {
            audit: true,
            ..TestRunnerOptions::default()
        },
        |_| successful_execution_result(),
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

    fs::remove_dir_all(&root).expect("should clean up temporary report directory");
}

#[test]
fn triage_report_write_failure_returns_error() {
    let root = temp_dir("runner_triage_write_failure");
    fs::create_dir_all(&root).expect("should create temporary report directory");
    let inventory_path = root.join("inventory.json");
    let triage_parent = root.join("triage-parent");
    fs::write(&triage_parent, "occupied").expect("should create triage path collision");
    let triage_path = triage_parent.join("triage.json");

    let result = run_loaded_suite(
        suite_with_case(Some(CaseRole::Backend), Some("backend.lowering.shared")),
        TestRunnerOptions::default(),
        |_| CaseExecutionResult {
            passed: false,
            panic_message: None,
            build_result: None,
            messages: None,
            failure_reason: Some("forced test failure".to_owned()),
            failure_kind: None,
        },
        inventory_path
            .to_str()
            .expect("temporary path should be UTF-8"),
        triage_path
            .to_str()
            .expect("temporary path should be UTF-8"),
    );

    let error = result.expect_err("triage report write failure should be returned");
    assert!(
        error.contains("Failed to create triage report directory"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up temporary report directory");
}
