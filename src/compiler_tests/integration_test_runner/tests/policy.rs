//! Self-tests for cross-case integration-suite policy.
//!
//! WHAT: protects ownership, assertion-strength and classification evaluation.
//! WHY: these rules must be emitted once by the policy owner before reporting or execution.

use super::super::policy::evaluate_suite;
use super::super::types::GoldenExpectation;
use super::super::{
    BackendId, CaseRole, DiagnosticMatchMode, ExpectedOutcome, FailureExpectation, SuccessContract,
    SuccessExpectation, TestCaseSpec, TestSuiteSpec, WarningExpectation,
};
use std::path::PathBuf;

fn suite(cases: Vec<TestCaseSpec>) -> TestSuiteSpec {
    TestSuiteSpec { cases }
}

fn success_case(
    case_id: &str,
    backend_id: BackendId,
    contract: Option<&str>,
    role: Option<CaseRole>,
    success_contract: Option<SuccessContract>,
    rendered_output: Option<&str>,
) -> TestCaseSpec {
    TestCaseSpec {
        display_name: format!("{case_id} [{}]", backend_id.as_str()),
        case_id: case_id.to_owned(),
        manifest_relative_path: case_id.to_owned(),
        fixture_root: PathBuf::from("."),
        tags: vec!["integration".to_owned()],
        contract: contract.map(str::to_owned),
        role,
        backend_id,
        entry_path: PathBuf::from("input/@page.moth"),
        flags: Vec::new(),
        expected: ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Forbid,
            success_contract,
            artifact_assertions: Vec::new(),
            golden: GoldenExpectation::default(),
            rendered_output: super::super::types::RenderedOutputExpectation {
                contains: rendered_output.map(str::to_owned).into_iter().collect(),
                ..Default::default()
            },
            artifacts_must_not_exist: Vec::new(),
        }),
    }
}

fn failure_case(
    case_id: &str,
    backend_id: BackendId,
    contract: Option<&str>,
    role: Option<CaseRole>,
    diagnostic_match: DiagnosticMatchMode,
    diagnostic_match_reason: Option<&str>,
) -> TestCaseSpec {
    TestCaseSpec {
        display_name: format!("{case_id} [{}]", backend_id.as_str()),
        case_id: case_id.to_owned(),
        manifest_relative_path: case_id.to_owned(),
        fixture_root: PathBuf::from("."),
        tags: vec!["integration".to_owned()],
        contract: contract.map(str::to_owned),
        role,
        backend_id,
        entry_path: PathBuf::from("input/@page.moth"),
        flags: Vec::new(),
        expected: ExpectedOutcome::Failure(FailureExpectation {
            warnings: WarningExpectation::Forbid,
            message_contains: Vec::new(),
            diagnostic_codes: vec!["MOTH-RULE-0001".to_owned()],
            diagnostic_assertions: Vec::new(),
            diagnostic_match,
            diagnostic_match_reason: diagnostic_match_reason.map(str::to_owned),
        }),
    }
}

#[test]
fn duplicate_primary_contract_is_emitted_once_by_policy() {
    let suite = suite(vec![
        success_case(
            "case_a",
            BackendId::Html,
            Some("language.shared"),
            Some(CaseRole::Primary),
            None,
            Some("case-a"),
        ),
        success_case(
            "case_b",
            BackendId::Html,
            Some("language.shared"),
            Some(CaseRole::Primary),
            None,
            Some("case-b"),
        ),
    ]);

    let evaluation = evaluate_suite(&suite);

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(
        evaluation.hard_findings[0].code,
        "duplicate_primary_contract"
    );
}

#[test]
fn primary_without_contract_is_emitted_once_by_policy() {
    let suite = suite(vec![success_case(
        "primary_without_contract",
        BackendId::Html,
        None,
        Some(CaseRole::Primary),
        None,
        Some("primary"),
    )]);

    let evaluation = evaluate_suite(&suite);

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(evaluation.hard_findings[0].code, "primary_missing_contract");
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn whole_case_acceptance_only_requires_smoke_role() {
    let suite = suite(vec![success_case(
        "acceptance_only",
        BackendId::Html,
        Some("acceptance.shared"),
        Some(CaseRole::Backend),
        Some(SuccessContract::AcceptanceOnly),
        None,
    )]);

    let evaluation = evaluate_suite(&suite);

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(
        evaluation.hard_findings[0].code,
        "acceptance_only_requires_smoke_role"
    );
}

#[test]
fn missing_role_is_a_hard_policy_finding() {
    // A primary owner for the same contract keeps the contract family primary-owned so the
    // missing-role finding is the only policy result for the unclassified member.
    let evaluation = evaluate_suite(&suite(vec![
        success_case(
            "primary_owner",
            BackendId::Html,
            Some("language.missing_role"),
            Some(CaseRole::Primary),
            None,
            Some("primary"),
        ),
        success_case(
            "missing_role_case",
            BackendId::HtmlWasm,
            Some("language.missing_role"),
            None,
            None,
            Some("marker"),
        ),
    ]));

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(
        evaluation.hard_findings[0].code,
        "missing_role_classification"
    );
    assert_eq!(
        evaluation.hard_findings[0].case_id.as_deref(),
        Some("missing_role_case")
    );
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn missing_contract_is_a_hard_policy_finding_for_non_smoke_cases() {
    let evaluation = evaluate_suite(&suite(vec![success_case(
        "missing_contract_case",
        BackendId::Html,
        None,
        Some(CaseRole::Boundary),
        None,
        Some("marker"),
    )]));

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(
        evaluation.hard_findings[0].code,
        "missing_contract_classification"
    );
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn missing_role_and_missing_contract_are_both_hard_for_unclassified_cases() {
    let evaluation = evaluate_suite(&suite(vec![success_case(
        "unclassified_case",
        BackendId::Html,
        None,
        None,
        None,
        Some("marker"),
    )]));

    let codes = evaluation
        .hard_findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        codes,
        vec![
            "missing_role_classification",
            "missing_contract_classification"
        ]
    );
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn contractless_smoke_case_has_no_missing_contract_finding() {
    let evaluation = evaluate_suite(&suite(vec![success_case(
        "contractless_smoke",
        BackendId::Html,
        None,
        Some(CaseRole::Smoke),
        Some(SuccessContract::AcceptanceOnly),
        None,
    )]));

    assert!(evaluation.hard_findings.is_empty());
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn missing_contains_reason_is_a_hard_policy_finding() {
    let evaluation = evaluate_suite(&suite(vec![failure_case(
        "contains_missing_reason",
        BackendId::Html,
        Some("diagnostics.contains_reason"),
        Some(CaseRole::Boundary),
        DiagnosticMatchMode::Contains,
        None,
    )]));

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(
        evaluation.hard_findings[0].code,
        "diagnostic_contains_requires_reason"
    );
    assert!(
        evaluation.hard_findings[0]
            .message
            .contains("contains_missing_reason")
    );
    assert!(
        evaluation.hard_findings[0]
            .message
            .contains("backend 'html'")
    );
}

#[test]
fn blank_contains_reason_is_a_hard_policy_finding() {
    let evaluation = evaluate_suite(&suite(vec![failure_case(
        "contains_blank_reason",
        BackendId::Html,
        Some("diagnostics.contains_reason"),
        Some(CaseRole::Boundary),
        DiagnosticMatchMode::Contains,
        Some(" \t"),
    )]));

    assert_eq!(evaluation.hard_findings.len(), 1);
    assert_eq!(
        evaluation.hard_findings[0].code,
        "diagnostic_contains_requires_reason"
    );
}

#[test]
fn justified_contains_reason_has_no_hard_policy_finding() {
    // A primary owner for the same contract keeps the family primary-owned so the only
    // relevant policy fact is the authored contains reason.
    let evaluation = evaluate_suite(&suite(vec![
        failure_case(
            "contains_justified_reason",
            BackendId::Html,
            Some("diagnostics.contains_reason"),
            Some(CaseRole::Boundary),
            DiagnosticMatchMode::Contains,
            Some("independent recovery"),
        ),
        success_case(
            "primary_owner",
            BackendId::Html,
            Some("diagnostics.contains_reason"),
            Some(CaseRole::Primary),
            None,
            Some("primary"),
        ),
    ]));

    assert!(evaluation.hard_findings.is_empty());
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn contains_findings_are_backend_local_and_deterministic() {
    let evaluation = evaluate_suite(&suite(vec![
        failure_case(
            "contains_backend_matrix",
            BackendId::Html,
            Some("diagnostics.contains_reason"),
            Some(CaseRole::Boundary),
            DiagnosticMatchMode::Contains,
            None,
        ),
        failure_case(
            "contains_backend_matrix",
            BackendId::HtmlWasm,
            Some("diagnostics.contains_reason"),
            Some(CaseRole::Boundary),
            DiagnosticMatchMode::Contains,
            Some("  "),
        ),
    ]));

    assert_eq!(evaluation.hard_findings.len(), 2);
    assert_eq!(
        evaluation
            .hard_findings
            .iter()
            .map(|finding| finding.message.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Case 'contains_backend_matrix' backend 'html' uses diagnostic_match = \"contains\" without a non-blank authored diagnostic_match_reason.",
            "Case 'contains_backend_matrix' backend 'html_wasm' uses diagnostic_match = \"contains\" without a non-blank authored diagnostic_match_reason.",
        ]
    );
}

#[test]
fn hard_findings_preserve_manifest_order_across_rules() {
    let evaluation = evaluate_suite(&suite(vec![
        success_case(
            "primary_without_contract",
            BackendId::Html,
            None,
            Some(CaseRole::Primary),
            None,
            Some("primary"),
        ),
        failure_case(
            "unclassified_contains",
            BackendId::Html,
            None,
            None,
            DiagnosticMatchMode::Contains,
            None,
        ),
        success_case(
            "unclassified_case",
            BackendId::Html,
            None,
            None,
            None,
            Some("marker"),
        ),
    ]));

    assert_eq!(
        evaluation
            .hard_findings
            .iter()
            .map(|finding| (finding.case_id.as_deref(), finding.code.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (Some("primary_without_contract"), "primary_missing_contract"),
            (Some("unclassified_contains"), "missing_role_classification"),
            (
                Some("unclassified_contains"),
                "missing_contract_classification"
            ),
            (
                Some("unclassified_contains"),
                "diagnostic_contains_requires_reason"
            ),
            (Some("unclassified_case"), "missing_role_classification"),
            (Some("unclassified_case"), "missing_contract_classification"),
        ]
    );
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn stronger_mixed_backend_contract_does_not_force_smoke_role() {
    let suite = suite(vec![
        success_case(
            "mixed",
            BackendId::Html,
            Some("mixed.shared"),
            Some(CaseRole::Backend),
            Some(SuccessContract::AcceptanceOnly),
            None,
        ),
        success_case(
            "mixed",
            BackendId::HtmlWasm,
            Some("mixed.shared"),
            Some(CaseRole::Backend),
            None,
            Some("marker"),
        ),
    ]);

    let evaluation = evaluate_suite(&suite);

    assert!(evaluation.hard_findings.is_empty());
}

#[test]
fn primary_owned_contract_family_has_no_primary_less_advisory() {
    let evaluation = evaluate_suite(&suite(vec![
        success_case(
            "primary_owner",
            BackendId::Html,
            Some("language.shared"),
            Some(CaseRole::Primary),
            None,
            Some("primary"),
        ),
        success_case(
            "backend_secondary",
            BackendId::HtmlWasm,
            Some("language.shared"),
            Some(CaseRole::Backend),
            None,
            Some("secondary"),
        ),
    ]));

    assert!(evaluation.hard_findings.is_empty());
    assert!(evaluation.advisories.is_empty());
}

#[test]
fn primary_less_backend_only_contract_family_is_advisory() {
    let evaluation = evaluate_suite(&suite(vec![
        success_case(
            "backend_owner_a",
            BackendId::Html,
            Some("backend.lowering.shared"),
            Some(CaseRole::Backend),
            None,
            Some("a"),
        ),
        success_case(
            "backend_owner_b",
            BackendId::HtmlWasm,
            Some("backend.lowering.shared"),
            Some(CaseRole::Backend),
            None,
            Some("b"),
        ),
    ]));

    assert!(evaluation.hard_findings.is_empty());
    assert_eq!(evaluation.advisories.len(), 1);
    assert_eq!(
        evaluation.advisories[0].code,
        "primary_less_contract_backend_only"
    );
    assert_eq!(
        evaluation.advisories[0].case_id.as_deref(),
        Some("backend_owner_a")
    );
    assert!(evaluation.advisories[0].message.contains("backend-only"));
}

#[test]
fn primary_less_adversarial_only_contract_family_is_advisory() {
    let evaluation = evaluate_suite(&suite(vec![
        failure_case(
            "adversarial_owner_a",
            BackendId::Html,
            Some("language.results.chain"),
            Some(CaseRole::Adversarial),
            DiagnosticMatchMode::Exact,
            None,
        ),
        failure_case(
            "adversarial_owner_b",
            BackendId::HtmlWasm,
            Some("language.results.chain"),
            Some(CaseRole::Adversarial),
            DiagnosticMatchMode::Exact,
            None,
        ),
    ]));

    assert!(evaluation.hard_findings.is_empty());
    assert_eq!(evaluation.advisories.len(), 1);
    assert_eq!(
        evaluation.advisories[0].code,
        "primary_less_contract_adversarial_only"
    );
    assert_eq!(
        evaluation.advisories[0].case_id.as_deref(),
        Some("adversarial_owner_a")
    );
    assert!(
        evaluation.advisories[0]
            .message
            .contains("adversarial-only")
    );
}

#[test]
fn primary_less_mixed_contract_family_is_visibly_distinct_advisory() {
    let evaluation = evaluate_suite(&suite(vec![
        success_case(
            "backend_member",
            BackendId::Html,
            Some("boundary.mixed"),
            Some(CaseRole::Backend),
            None,
            Some("backend"),
        ),
        success_case(
            "boundary_member",
            BackendId::HtmlWasm,
            Some("boundary.mixed"),
            Some(CaseRole::Boundary),
            None,
            Some("boundary"),
        ),
    ]));

    assert!(evaluation.hard_findings.is_empty());
    assert_eq!(evaluation.advisories.len(), 1);
    assert_eq!(evaluation.advisories[0].code, "primary_less_contract_mixed");
    assert_eq!(
        evaluation.advisories[0].case_id.as_deref(),
        Some("backend_member")
    );
}

#[test]
fn primary_less_advisories_are_one_per_family_in_manifest_order() {
    let evaluation = evaluate_suite(&suite(vec![
        success_case(
            "second_family_first",
            BackendId::Html,
            Some("backend.zeta"),
            Some(CaseRole::Backend),
            None,
            Some("zeta"),
        ),
        success_case(
            "first_family_first",
            BackendId::Html,
            Some("backend.alpha"),
            Some(CaseRole::Backend),
            None,
            Some("alpha"),
        ),
        success_case(
            "first_family_second",
            BackendId::HtmlWasm,
            Some("backend.alpha"),
            Some(CaseRole::Backend),
            None,
            Some("alpha-wasm"),
        ),
    ]));

    assert_eq!(evaluation.advisories.len(), 2);
    assert_eq!(
        evaluation
            .advisories
            .iter()
            .map(|finding| (finding.case_id.as_deref(), finding.code.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (
                Some("second_family_first"),
                "primary_less_contract_backend_only"
            ),
            (
                Some("first_family_first"),
                "primary_less_contract_backend_only"
            ),
        ]
    );
}

#[test]
fn missing_classification_findings_are_deterministic_in_manifest_order() {
    let suite = suite(vec![
        success_case("case_b", BackendId::Html, None, None, None, Some("case-b")),
        success_case("case_a", BackendId::Html, None, None, None, Some("case-a")),
    ]);

    let evaluation = evaluate_suite(&suite);
    let findings = evaluation
        .hard_findings
        .iter()
        .map(|finding| {
            (
                finding.case_id.as_deref().unwrap_or("<none>"),
                finding.code.as_str(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        findings,
        vec![
            ("case_b", "missing_role_classification"),
            ("case_b", "missing_contract_classification"),
            ("case_a", "missing_role_classification"),
            ("case_a", "missing_contract_classification"),
        ]
    );
}
