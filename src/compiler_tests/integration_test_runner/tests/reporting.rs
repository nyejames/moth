//! Self-tests for deterministic integration-case listing output.
//!
//! WHAT: protects grouped listing and audit inventory reporting.
//! WHY: both reporting modes must expose retained metadata without invoking case execution.

use super::super::policy::evaluate_suite;
use super::super::reporting::{build_suite_inventory_report, format_case_listing};
use super::super::types::{
    DiagnosticAssertion, ExactWarningExpectation, GoldenExpectation, RenderedOutputExpectation,
    SuccessContract,
};
use super::super::{
    BackendId, CaseRole, DiagnosticMatchMode, ExpectedOutcome, FailureExpectation,
    SuccessExpectation, TestCaseSpec, TestSuiteSpec, WarningExpectation,
};
use std::path::PathBuf;

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
