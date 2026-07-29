//! Self-tests for deterministic integration-case listing output.
//!
//! WHAT: protects grouped listing and audit inventory reporting.
//! WHY: both reporting modes must expose retained metadata without invoking case execution.

use super::super::policy::evaluate_suite;
use super::super::reporting::{
    build_suite_inventory_report, format_case_listing, format_terse_run_output,
};
use super::super::types::{
    DiagnosticAssertion, ExactWarningExpectation, GoldenExpectation, RenderedOutputExpectation,
    SuccessContract,
};
use super::super::{
    BackendId, CaseExecutionResult, CaseRole, DiagnosticMatchMode, ExpectedOutcome,
    FailureExpectation, FailureKind, SuccessExpectation, SummaryCounts, TestCaseSpec,
    TestSuiteSpec, WarningExpectation,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::path::PathBuf;
use std::time::Duration;

fn case(
    case_id: &str,
    backend_id: BackendId,
    tags: &[&str],
    contract: Option<&str>,
    role: Option<CaseRole>,
    expected: ExpectedOutcome,
) -> TestCaseSpec {
    TestCaseSpec {
        display_name: format!("{case_id} [{}]", backend_id.as_str()),
        case_id: case_id.to_owned(),
        manifest_relative_path: case_id.to_owned(),
        fixture_root: PathBuf::from("."),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        contract: contract.map(str::to_owned),
        role,
        backend_id,
        entry_path: PathBuf::from("input/#page.moth"),
        flags: Vec::new(),
        expected,
    }
}

fn report_for_cases(
    cases: &[TestCaseSpec],
    repository_commit: Option<String>,
) -> super::super::reporting::SuiteInventoryReport {
    let suite = TestSuiteSpec {
        cases: cases.to_vec(),
    };
    let policy_evaluation = evaluate_suite(&suite);
    build_suite_inventory_report(&suite.cases, &policy_evaluation, repository_commit)
}

#[test]
fn listing_groups_selected_backends_and_retains_case_metadata() {
    let listing = format_case_listing(&[
        case(
            "case_a",
            BackendId::Html,
            &["integration", "language"],
            Some("language.case_a"),
            Some(CaseRole::Primary),
            ExpectedOutcome::Failure(FailureExpectation {
                warnings: WarningExpectation::Forbid,
                message_contains: Vec::new(),
                diagnostic_codes: vec!["MOTH-RULE-0001".to_owned()],
                diagnostic_assertions: Vec::new(),
                diagnostic_match: DiagnosticMatchMode::Contains,
                diagnostic_match_reason: Some("independent recovery".to_owned()),
            }),
        ),
        case(
            "case_a",
            BackendId::HtmlWasm,
            &["integration", "language"],
            Some("language.case_a"),
            Some(CaseRole::Primary),
            ExpectedOutcome::Failure(FailureExpectation {
                warnings: WarningExpectation::Forbid,
                message_contains: Vec::new(),
                diagnostic_codes: vec!["MOTH-RULE-0001".to_owned()],
                diagnostic_assertions: Vec::new(),
                diagnostic_match: DiagnosticMatchMode::Contains,
                diagnostic_match_reason: Some("independent recovery".to_owned()),
            }),
        ),
    ]);

    assert_eq!(
        listing,
        concat!(
            "case_id: case_a\n",
            "  backends:\n",
            "    - html (failure)\n",
            "    - html_wasm (failure)\n",
            "  tags: integration, language\n",
            "  contract: language.case_a\n",
            "  role: primary\n\n",
        )
    );
}

#[test]
fn empty_listing_is_explicit() {
    assert_eq!(
        format_case_listing(&[]),
        "No test cases matched the selection filters.\n"
    );
}

#[test]
fn inventory_json_groups_backend_metadata_under_one_canonical_case() {
    let html_case = case(
        "case_a",
        BackendId::Html,
        &["integration", "language"],
        Some("language.case_a"),
        Some(CaseRole::Primary),
        ExpectedOutcome::Failure(FailureExpectation {
            warnings: WarningExpectation::Forbid,
            message_contains: Vec::new(),
            diagnostic_codes: vec!["MOTH-RULE-0001".to_owned()],
            diagnostic_assertions: vec![DiagnosticAssertion {
                code: "MOTH-RULE-0001".to_owned(),
                occurrence: 1,
                reason: Some("invalid_expression.expected_operator".to_owned()),
                path: Some("input/main.moth".to_owned()),
                line: Some(1),
                column: None,
                count: Some(1),
                secondary_labels: Vec::new(),
            }],
            diagnostic_match: DiagnosticMatchMode::Contains,
            diagnostic_match_reason: Some("independent recovery".to_owned()),
        }),
    );
    let wasm_case = case(
        "case_a",
        BackendId::HtmlWasm,
        &["integration", "language"],
        Some("language.case_a"),
        Some(CaseRole::Primary),
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: super::super::types::RenderedOutputExpectation {
                contains: vec!["ok".to_owned()],
                ..Default::default()
            },
            artifacts_must_not_exist: Vec::new(),
        }),
    );

    let report = report_for_cases(&[html_case, wasm_case], Some("0123456789abcdef".to_owned()));
    let json = serde_json::to_value(&report).expect("inventory should serialize");

    assert_eq!(json["schema_version"], 6);
    assert_eq!(json["repository_commit"], "0123456789abcdef");
    assert_eq!(json["manifest_case_count"], 1);
    assert_eq!(json["expanded_backend_execution_count"], 2);
    assert_eq!(json["cases"][0]["canonical_id"], "case_a");
    assert_eq!(json["cases"][0]["manifest_relative_path"], "case_a");
    assert_eq!(json["cases"][0]["role"], "primary");
    assert_eq!(
        json["cases"][0]["backends"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(json["cases"][0]["backends"][0]["backend"], "html");
    assert_eq!(json["cases"][0]["backends"][0]["baseline_applied"], false);
    assert_eq!(
        json["cases"][0]["backends"][0]["diagnostic_match"],
        "contains"
    );
    assert_eq!(
        json["cases"][0]["backends"][0]["diagnostic_match_reason"],
        "independent recovery"
    );
    assert_eq!(
        json["cases"][0]["backends"][0]["structured_diagnostic_assertions"],
        true
    );
    assert_eq!(
        json["cases"][0]["backends"][0]["assertion_kinds"],
        serde_json::json!(["diagnostic_codes", "diagnostic_assertions"])
    );
    assert_eq!(json["cases"][0]["backends"][1]["backend"], "html_wasm");
    assert_eq!(json["cases"][0]["backends"][1]["baseline_applied"], true);
    assert_eq!(json["cases"][0]["backends"][1]["golden_present"], false);
    assert_eq!(
        json["cases"][0]["backends"][1]["golden_mode"],
        serde_json::Value::Null
    );
    assert_eq!(json["summary"]["rendered_output_backend_blocks"], 1);
    assert_eq!(
        json["hard_policy_violations"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(json["advisory_findings"].as_array().map(Vec::len), Some(0));
}

#[test]
fn inventory_reports_acceptance_only_without_baseline_only_state() {
    let explicit_case = case(
        "explicit_acceptance_only",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: Some(SuccessContract::AcceptanceOnly),
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let report = report_for_cases(&[explicit_case], None);
    let json = serde_json::to_value(&report).expect("inventory should serialize");

    assert_eq!(json["cases"][0]["backends"][0]["baseline_applied"], true);
    assert_eq!(json["cases"][0]["backends"][0]["acceptance_only"], true);
    assert_eq!(
        json["cases"][0]["backends"][0]["assertion_kinds"],
        serde_json::json!(["backend_baseline", "acceptance_only"])
    );
    assert_eq!(json["summary"]["acceptance_only_backend_blocks"], 1);
    assert_eq!(json["summary"]["baseline_only_backend_blocks"], 0);
}

#[test]
fn inventory_reports_each_rendered_output_form_and_schema_six_summary_counts() {
    let cases = vec![
        case(
            "exact_output",
            BackendId::Html,
            &["integration"],
            None,
            None,
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Forbid,
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: RenderedOutputExpectation {
                    exact: Some(String::new()),
                    ..Default::default()
                },
                artifacts_must_not_exist: Vec::new(),
            }),
        ),
        case(
            "ordered_output",
            BackendId::Html,
            &["integration"],
            None,
            None,
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Forbid,
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: RenderedOutputExpectation {
                    contains: vec!["prefix".to_owned()],
                    not_contains: vec!["forbidden".to_owned()],
                    contains_in_order: vec!["first".to_owned(), "second".to_owned()],
                    ..Default::default()
                },
                artifacts_must_not_exist: Vec::new(),
            }),
        ),
        case(
            "exactly_once_output",
            BackendId::Html,
            &["integration"],
            None,
            None,
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Forbid,
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: RenderedOutputExpectation {
                    contains_exactly_once: vec!["once".to_owned()],
                    ..Default::default()
                },
                artifacts_must_not_exist: Vec::new(),
            }),
        ),
    ];

    let json =
        serde_json::to_value(report_for_cases(&cases, None)).expect("report should serialize");

    assert_eq!(json["schema_version"], 6);
    assert_eq!(json["summary"]["rendered_output_backend_blocks"], 3);
    assert_eq!(json["summary"]["rendered_output_exact_backend_blocks"], 1);
    assert_eq!(json["summary"]["rendered_output_order_backend_blocks"], 1);
    assert_eq!(
        json["summary"]["rendered_output_exactly_once_backend_blocks"],
        1
    );

    let exact_backend = &json["cases"][0]["backends"][0];
    assert_eq!(exact_backend["rendered_output_assertion_count"], 1);
    assert_eq!(exact_backend["rendered_output_exact"], true);
    assert_eq!(
        exact_backend["assertion_kinds"],
        serde_json::json!([
            "backend_baseline",
            "rendered_output",
            "rendered_output_exact"
        ])
    );

    let ordered_backend = &json["cases"][1]["backends"][0];
    assert_eq!(ordered_backend["rendered_output_assertion_count"], 4);
    assert_eq!(ordered_backend["rendered_output_contains_count"], 1);
    assert_eq!(ordered_backend["rendered_output_not_contains_count"], 1);
    assert_eq!(
        ordered_backend["rendered_output_contains_in_order_count"],
        2
    );
    assert_eq!(
        ordered_backend["assertion_kinds"],
        serde_json::json!([
            "backend_baseline",
            "rendered_output",
            "rendered_output_contains",
            "rendered_output_not_contains",
            "rendered_output_contains_in_order"
        ])
    );

    let exactly_once_backend = &json["cases"][2]["backends"][0];
    assert_eq!(
        exactly_once_backend["rendered_output_contains_exactly_once_count"],
        1
    );
    assert_eq!(
        exactly_once_backend["assertion_kinds"],
        serde_json::json!([
            "backend_baseline",
            "rendered_output",
            "rendered_output_contains_exactly_once"
        ])
    );
}

#[test]
fn inventory_counts_authored_expected_warning_as_a_contract() {
    let report = report_for_cases(
        &[case(
            "expected_warning",
            BackendId::Html,
            &["integration"],
            None,
            Some(CaseRole::Smoke),
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Exact(ExactWarningExpectation {
                    expected_codes: vec!["MOTH-RULE-0022".to_owned()],
                }),
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: Default::default(),
                artifacts_must_not_exist: Vec::new(),
            }),
        )],
        None,
    );
    let json = serde_json::to_value(&report).expect("inventory should serialize");

    assert_eq!(json["summary"]["expected_warning_backend_blocks"], 1);
    assert_eq!(
        json["cases"][0]["backends"][0]["warning_codes"],
        serde_json::json!(["MOTH-RULE-0022"])
    );
    assert!(
        !json["cases"][0]["backends"][0]
            .as_object()
            .expect("inventory backend should serialize as an object")
            .contains_key("warning_count")
    );
    assert_eq!(json["summary"]["baseline_only_backend_blocks"], 0);
    assert_eq!(
        json["cases"][0]["backends"][0]["assertion_kinds"],
        serde_json::json!(["backend_baseline", "expected_warning"])
    );
}

#[test]
fn inventory_serializes_exact_warning_codes_without_a_transitional_count() {
    let report = report_for_cases(
        &[case(
            "exact_warning_codes",
            BackendId::Html,
            &["integration"],
            None,
            Some(CaseRole::Smoke),
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Exact(ExactWarningExpectation {
                    expected_codes: vec![
                        "MOTH-RULE-0022".to_owned(),
                        "MOTH-RULE-0022".to_owned(),
                        "MOTH-RULE-0022".to_owned(),
                    ],
                }),
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: Default::default(),
                artifacts_must_not_exist: Vec::new(),
            }),
        )],
        None,
    );
    let json = serde_json::to_value(&report).expect("inventory should serialize");

    assert_eq!(
        json["cases"][0]["backends"][0]["warning_codes"],
        serde_json::json!(["MOTH-RULE-0022", "MOTH-RULE-0022", "MOTH-RULE-0022"])
    );
    assert!(
        !json["cases"][0]["backends"][0]
            .as_object()
            .expect("inventory backend should serialize as an object")
            .contains_key("warning_count")
    );
}

#[test]
fn report_serializes_supplied_policy_evaluation() {
    let cases = [
        case(
            "case_a",
            BackendId::Html,
            &["integration"],
            Some("language.shared"),
            Some(CaseRole::Primary),
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Forbid,
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: super::super::types::RenderedOutputExpectation {
                    contains: vec!["case-a".to_owned()],
                    ..Default::default()
                },
                artifacts_must_not_exist: Vec::new(),
            }),
        ),
        case(
            "case_b",
            BackendId::Html,
            &["integration"],
            Some("language.shared"),
            Some(CaseRole::Primary),
            ExpectedOutcome::Success(SuccessExpectation {
                warnings: WarningExpectation::Forbid,
                success_contract: None,
                artifact_assertions: Vec::new(),
                golden: GoldenExpectation::default(),
                rendered_output: super::super::types::RenderedOutputExpectation {
                    contains: vec!["case-b".to_owned()],
                    ..Default::default()
                },
                artifacts_must_not_exist: Vec::new(),
            }),
        ),
    ];

    let suite = TestSuiteSpec {
        cases: cases.to_vec(),
    };
    let policy_evaluation = evaluate_suite(&suite);
    let report = build_suite_inventory_report(&suite.cases, &policy_evaluation, None);
    assert_eq!(report.hard_policy_violations.len(), 1);
    assert_eq!(
        report.hard_policy_violations[0].code,
        "duplicate_primary_contract"
    );
}

#[test]
fn report_serializes_contains_policy_finding_once_with_typed_reason_fact() {
    let case = case(
        "contains_policy_case",
        BackendId::Html,
        &["integration"],
        Some("diagnostics.contains_reason"),
        Some(CaseRole::Boundary),
        ExpectedOutcome::Failure(FailureExpectation {
            warnings: WarningExpectation::Forbid,
            message_contains: Vec::new(),
            diagnostic_codes: vec!["MOTH-RULE-0001".to_owned()],
            diagnostic_assertions: Vec::new(),
            diagnostic_match: DiagnosticMatchMode::Contains,
            diagnostic_match_reason: Some("  ".to_owned()),
        }),
    );

    let report = report_for_cases(&[case], None);
    let json = serde_json::to_value(&report).expect("inventory should serialize");

    assert_eq!(
        json["hard_policy_violations"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        json["hard_policy_violations"][0]["code"],
        "diagnostic_contains_requires_reason"
    );
    assert!(
        json["hard_policy_violations"][0]["message"]
            .as_str()
            .is_some_and(|message| {
                message.contains("contains_policy_case") && message.contains("backend 'html'")
            })
    );
    assert_eq!(
        json["cases"][0]["backends"][0]["diagnostic_match"],
        "contains"
    );
    assert_eq!(
        json["cases"][0]["backends"][0]["diagnostic_match_reason"],
        "  "
    );
}

fn terse_result(case: TestCaseSpec, result: CaseExecutionResult) -> Vec<String> {
    let mut summary = SummaryCounts::default();
    summary.record(&case, &result);
    format_terse_run_output(&[(case, result)], summary, Duration::from_secs(1), false)
}

#[test]
fn terse_all_correct_produces_one_summary_line() {
    let case = case(
        "arithmetic_operator_precedence",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result = CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: None,
        failure_kind: None,
    };
    let output = terse_result(case, result);

    assert_eq!(output.len(), 1);
    assert!(output[0].contains("1/1 correct"), "{}", output[0]);
    assert!(!output[0].contains("incorrect"), "{}", output[0]);
}

#[test]
fn terse_expected_success_failure_shows_fail_header() {
    let case = case(
        "arithmetic_operator_precedence",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: Some("Compilation unexpectedly failed.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let output = terse_result(case, result);

    assert!(output[0].starts_with("FAIL"), "{}", output[0]);
    assert!(
        output[0].contains("arithmetic_operator_precedence"),
        "{}",
        output[0]
    );
    assert!(output[0].contains("[html]"), "{}", output[0]);
    assert!(output[0].contains("expectation violation"), "{}", output[0]);
    assert!(
        output[0].contains("Compilation unexpectedly failed"),
        "{}",
        output[0]
    );
    assert!(
        output.last().unwrap().contains("0/1 correct"),
        "{}",
        output.last().unwrap()
    );
}

#[test]
fn terse_expected_failure_success_shows_unexpected_success() {
    let case = case(
        "invalid_assignment",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Failure(FailureExpectation {
            warnings: WarningExpectation::Forbid,
            message_contains: Vec::new(),
            diagnostic_codes: Vec::new(),
            diagnostic_assertions: Vec::new(),
            diagnostic_match: DiagnosticMatchMode::Contains,
            diagnostic_match_reason: None,
        }),
    );
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: Some("Compilation succeeded but a failure was expected.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let output = terse_result(case, result);

    assert!(output[0].starts_with("UNEXPECTED SUCCESS"), "{}", output[0]);
    assert!(
        !output[0].contains("E|"),
        "no invented diagnostic should appear"
    );
}

#[test]
fn terse_diagnostics_use_e_and_w_format() {
    let string_table = StringTable::new();
    let error_diag = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Error,
        SourceLocation::default(),
        DiagnosticPayload::None,
    );
    let warning_diag = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        SourceLocation::default(),
        DiagnosticPayload::None,
    );

    let messages = CompilerMessages::from_diagnostics(vec![error_diag, warning_diag], string_table);

    let case = case(
        "diagnostics_case",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: Some("Compilation failed.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };

    let output = terse_result(case, result);
    let diag_lines: Vec<_> = output
        .iter()
        .filter(|l| l.starts_with('E') || l.starts_with('W'))
        .collect();
    assert!(
        !diag_lines.is_empty(),
        "should have at least error lines: {:?}",
        output
    );
    assert!(
        diag_lines.iter().any(|l| l.starts_with('E')),
        "should have E line: {:?}",
        output
    );
}

#[test]
fn terse_warnings_removed_when_show_warnings_false() {
    let string_table = StringTable::new();
    let error_diag = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Error,
        SourceLocation::default(),
        DiagnosticPayload::None,
    );
    let warning_diag = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        SourceLocation::default(),
        DiagnosticPayload::None,
    );

    let messages = CompilerMessages::from_diagnostics(vec![error_diag, warning_diag], string_table);

    let case = case(
        "warnings_case",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: Some("Compilation failed.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };

    let mut summary = SummaryCounts::default();
    summary.record(&case, &result);
    let output = format_terse_run_output(&[(case, result)], summary, Duration::from_secs(1), false);
    let warning_lines: Vec<_> = output.iter().filter(|l| l.starts_with('W')).collect();
    assert!(
        warning_lines.is_empty(),
        "warnings should be hidden: {:?}",
        warning_lines
    );
}

#[test]
fn terse_panic_message_appears_and_collapses_multiline() {
    let case = case(
        "panic_case",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result = CaseExecutionResult {
        passed: false,
        panic_message: Some("index out of bounds\n  at line 42\n  in function foo".to_owned()),
        build_result: None,
        messages: None,
        failure_reason: Some("The compiler panicked.".to_owned()),
        failure_kind: Some(FailureKind::HarnessFailed),
    };
    let output = terse_result(case, result);

    assert!(
        output[0].contains("harness error"),
        "should show harness error kind: {}",
        output[0]
    );
    assert!(
        output[0].contains("The compiler panicked"),
        "should show reason: {}",
        output[0]
    );
    let panic_line = output
        .iter()
        .find(|l| l.contains("index out of bounds"))
        .expect("panic text should appear");
    assert!(
        !panic_line.contains('\n'),
        "panic text should be single line: {:?}",
        panic_line
    );
}

#[test]
fn terse_output_has_no_ansi_or_separators() {
    let case = case(
        "ansi_test",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result = CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: None,
        failure_kind: None,
    };
    let output = terse_result(case, result);

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
    let case_a = case(
        "case_a",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let case_b = case(
        "case_b",
        BackendId::Html,
        &["integration"],
        None,
        None,
        ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    );
    let result_a = CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: Some("failed.".to_owned()),
        failure_kind: Some(FailureKind::ExpectationViolation),
    };
    let result_b = CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: None,
        messages: None,
        failure_reason: None,
        failure_kind: None,
    };

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
        2,
        "one failure line + one summary: {:?}",
        output
    );
    assert!(
        output[0].starts_with("FAIL case_a"),
        "first line is failure: {}",
        output[0]
    );
    assert!(output[1].contains("1/2 correct"), "summary: {}", output[1]);
    assert!(output[1].contains("1 incorrect"), "summary: {}", output[1]);
}
