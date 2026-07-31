//! Self-tests for composable integration-case selection.
//!
//! WHAT: protects exact metadata matching, tag conjunctions, and stable ordering.
//! WHY: selection must operate on retained case metadata rather than display text.

use super::super::runner::select_cases;
use super::super::{
    BackendId, ExpectedOutcome, SuccessExpectation, TestCaseSpec, TestRunnerOptions,
    WarningExpectation,
};
use std::path::PathBuf;

fn case(
    case_id: &str,
    display_name: &str,
    tags: &[&str],
    contract: Option<&str>,
    backend_id: BackendId,
) -> TestCaseSpec {
    TestCaseSpec {
        display_name: display_name.to_owned(),
        case_id: case_id.to_owned(),
        manifest_relative_path: case_id.to_owned(),
        fixture_root: PathBuf::from("."),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        contract: contract.map(str::to_owned),
        role: None,
        backend_id,
        entry_path: PathBuf::from("input/@page.moth"),
        flags: Vec::new(),
        expected: ExpectedOutcome::Success(SuccessExpectation {
            warnings: WarningExpectation::Ignore,
            success_contract: None,
            artifact_assertions: Vec::new(),
            golden: Default::default(),
            rendered_output: Default::default(),
            artifacts_must_not_exist: Vec::new(),
        }),
    }
}

fn cases() -> Vec<TestCaseSpec> {
    vec![
        case(
            "case_a",
            "unrelated display name [html]",
            &["integration", "borrows"],
            Some("language.case_a"),
            BackendId::Html,
        ),
        case(
            "case_a",
            "case_a [html_wasm]",
            &["integration", "borrows"],
            Some("language.case_a"),
            BackendId::HtmlWasm,
        ),
        case(
            "case_b",
            "case_b [html]",
            &["integration", "language"],
            Some("language.case_b"),
            BackendId::Html,
        ),
        case(
            "case_c",
            "case_c [html]",
            &["integration", "borrows", "diagnostics"],
            Some("language.case_c"),
            BackendId::Html,
        ),
    ]
}

#[test]
fn selection_uses_exact_case_id_and_preserves_input_order() {
    let selected = select_cases(
        cases(),
        &TestRunnerOptions {
            case_id: Some("case_a".to_owned()),
            ..TestRunnerOptions::default()
        },
    );

    assert_eq!(
        selected
            .iter()
            .map(|case| case.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["unrelated display name [html]", "case_a [html_wasm]"]
    );
}

#[test]
fn repeated_tags_compose_as_logical_and_with_other_filters() {
    let selected = select_cases(
        cases(),
        &TestRunnerOptions {
            tag_filters: vec!["borrows".to_owned(), "diagnostics".to_owned()],
            contract: Some("language.case_c".to_owned()),
            backend_filter: Some(BackendId::Html),
            ..TestRunnerOptions::default()
        },
    );

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].case_id, "case_c");
}

#[test]
fn selection_preserves_manifest_then_backend_order() {
    let selected = select_cases(
        cases(),
        &TestRunnerOptions {
            tag_filters: vec!["integration".to_owned()],
            ..TestRunnerOptions::default()
        },
    );

    assert_eq!(
        selected
            .iter()
            .map(|case| case.display_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "unrelated display name [html]",
            "case_a [html_wasm]",
            "case_b [html]",
            "case_c [html]",
        ]
    );
}
