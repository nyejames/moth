//! Self-tests for canonical fixture discovery and backend expansion.
//!
//! WHAT: protects fixture loading, manifest-backed ordering, and backend selection.
//! WHY: the loader translates repository fixtures into executable test cases.

use super::super::fixture::{load_canonical_case_specs, load_test_suite_from_root};
use super::super::runner::select_cases;
use super::super::types::GoldenMode;
use super::super::{
    BackendId, CaseRole, EXPECT_FILE_NAME, GOLDEN_DIR_NAME, INPUT_DIR_NAME, MANIFEST_FILE_NAME,
    TestRunnerOptions,
};
use crate::compiler_tests::test_support::temp_dir;
use std::fs;
use std::path::Path;

#[test]
fn rejects_failure_fixture_without_diagnostic_codes() {
    let root = temp_dir("failure_contract_missing_codes");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "x = 1\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("fixture should be rejected");
    };
    assert!(
        error.contains("diagnostic_codes"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn accepts_failure_fixture_without_message_contains() {
    let root = temp_dir("failure_contract_codes_only");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "x = 1\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
    )
    .expect("should write expect file");

    load_canonical_case_specs(&case_root, None)
        .expect("diagnostic-code-only failure fixtures should be accepted");

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn rejects_canonical_fixture_without_expectation_before_execution() {
    let root = temp_dir("missing_expectation");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(
        input_root.join("#page.moth"),
        "not valid Moth source\n",
    )
    .expect("should write fixture source");

    let expected_path = case_root.join(EXPECT_FILE_NAME);
    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("fixture without an expectation file should be rejected");
    };
    assert!(
        error.contains("Canonical case 'case'")
            && error.contains("missing required expectation file")
            && error.contains(&case_root.display().to_string())
            && error.contains(&expected_path.display().to_string()),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn rejects_acceptance_only_fixture_with_golden_artifacts() {
    let root = temp_dir("acceptance_only_golden_artifacts");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_root).expect("should create fixture golden directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(golden_root.join("index.html"), "<h1>ok</h1>\n")
        .expect("should write fixture golden");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("acceptance-only fixture with golden artifacts should be rejected");
    };
    assert!(
        error.contains("acceptance_only") && error.contains("golden artifacts"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn accepts_acceptance_only_without_fixture_specific_source_marker() {
    let root = temp_dir("acceptance_only_without_source_marker");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "#[:not_a_contract_marker]\n")
        .expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("acceptance-only should not require a fixture-specific source marker");
    let super::super::types::ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert!(!expectation.golden.is_present());
    assert_eq!(expectation.golden.mode, None);

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn empty_backend_golden_directory_has_no_contract() {
    let root = temp_dir("empty_backend_golden_directory");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(case_root.join(GOLDEN_DIR_NAME).join("html"))
        .expect("should create empty golden directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("empty golden directory should not create a contract");
    let super::super::types::ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert!(!expectation.golden.is_present());
    assert_eq!(expectation.golden.mode, None);

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn empty_nested_golden_directory_has_no_contract() {
    let root = temp_dir("empty_nested_golden_directory");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(case_root.join(GOLDEN_DIR_NAME).join("html").join("nested"))
        .expect("should create empty nested golden directory");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("empty nested golden directory should not create a contract");
    let super::super::types::ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert!(!expectation.golden.is_present());
    assert_eq!(expectation.golden.mode, None);

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn explicit_golden_mode_without_files_is_rejected() {
    let root = temp_dir("explicit_golden_mode_without_files");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\ngolden_mode = \"strict\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("explicit golden mode without files should be rejected");
    };
    assert!(
        error.contains("golden_mode") && error.contains("no golden files"),
        "{error}"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn nested_golden_files_use_relative_inventory_paths() {
    let root = temp_dir("nested_golden_file_inventory");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    let nested_file = golden_root.join("nested").join("page.html");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(nested_file.parent().expect("nested parent should exist"))
        .expect("should create nested golden directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(&nested_file, "<h1>ok</h1>\n").expect("should write nested golden file");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("nested golden file should create a contract");
    let super::super::types::ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert_eq!(expectation.golden.mode, Some(GoldenMode::Strict));
    assert_eq!(
        expectation.golden.inventory.files[0].relative_path,
        "nested/page.html"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn accepts_backend_matrix_and_expands_case_variants() {
    let root = temp_dir("backend_matrix");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \".\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n\n[backends.html_wasm]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write matrix expect file");

    let cases = load_canonical_case_specs(&case_root, None).expect("matrix case should parse");
    let names = cases
        .iter()
        .map(|case| case.display_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["case [html]", "case [html_wasm]"]);

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn backend_filter_limits_loaded_case_variants() {
    let root = temp_dir("backend_filter");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \".\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n\n[backends.html_wasm]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write matrix expect file");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"matrix\"]\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root).expect("suite should load");
    let suite_cases = select_cases(
        suite.cases,
        &TestRunnerOptions {
            backend_filter: Some(BackendId::HtmlWasm),
            ..TestRunnerOptions::default()
        },
    );
    assert_eq!(suite_cases.len(), 1);
    assert_eq!(suite_cases[0].display_name, "case [html_wasm]");

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn manifest_metadata_survives_backend_expansion() {
    let root = temp_dir("manifest_metadata_expansion");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \".\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n\n[backends.html_wasm]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write matrix expect file");
    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"integration\", \"maps\"]\ncontract = \"language.maps.get_alias_exclusivity\"\nrole = \"primary\"\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root).expect("metadata matrix case should load");
    assert_eq!(suite.cases.len(), 2);
    assert_eq!(
        suite
            .cases
            .iter()
            .map(|case| case.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["case [html]", "case [html_wasm]"]
    );

    for case in &suite.cases {
        assert_eq!(case.case_id, "case");
        assert_eq!(case.manifest_relative_path, "case");
        assert_eq!(case.tags, vec!["integration", "maps"]);
        assert_eq!(
            case.contract.as_deref(),
            Some("language.maps.get_alias_exclusivity")
        );
        assert_eq!(case.role, Some(CaseRole::Primary));
    }

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn matrix_cases_resolve_backend_specific_golden_directories() {
    let root = temp_dir("matrix_backend_goldens");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_html_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    let golden_wasm_root = case_root.join(GOLDEN_DIR_NAME).join("html_wasm");

    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_html_root).expect("should create html golden directory");
    fs::create_dir_all(&golden_wasm_root).expect("should create wasm golden directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(golden_html_root.join("index.html"), "<h1>html</h1>\n")
        .expect("should write html golden");
    fs::write(golden_wasm_root.join("index.html"), "<h1>wasm</h1>\n")
        .expect("should write wasm golden");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \".\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n\n[backends.html_wasm]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write matrix expect file");

    let cases = load_canonical_case_specs(&case_root, None).expect("cases should parse");
    assert_eq!(cases.len(), 2);

    let html_case = cases
        .iter()
        .find(|case| case.backend_id == BackendId::Html)
        .expect("html case should exist");
    let wasm_case = cases
        .iter()
        .find(|case| case.backend_id == BackendId::HtmlWasm)
        .expect("html_wasm case should exist");

    let super::super::types::ExpectedOutcome::Success(html_expectation) = &html_case.expected
    else {
        panic!("html case should have a success expectation");
    };
    let super::super::types::ExpectedOutcome::Success(wasm_expectation) = &wasm_case.expected
    else {
        panic!("html_wasm case should have a success expectation");
    };
    assert_eq!(html_expectation.golden.mode, Some(GoldenMode::Strict));
    assert_eq!(wasm_expectation.golden.mode, Some(GoldenMode::Strict));
    assert_eq!(
        html_expectation.golden.inventory.files[0].relative_path,
        "index.html"
    );
    assert_eq!(
        wasm_expectation.golden.inventory.files[0].relative_path,
        "index.html"
    );
    assert_eq!(
        html_expectation.golden.inventory.files[0].absolute_path,
        fs::canonicalize(golden_html_root.join("index.html"))
            .expect("html golden should canonicalize")
    );
    assert_eq!(
        wasm_expectation.golden.inventory.files[0].absolute_path,
        fs::canonicalize(golden_wasm_root.join("index.html"))
            .expect("wasm golden should canonicalize")
    );

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn accepts_success_fixture_with_golden_only_assertion() {
    let root = temp_dir("success_contract_golden_assertion");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_root).expect("should create fixture golden directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(golden_root.join("index.html"), "<h1>ok</h1>\n").expect("should write golden file");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None).expect("fixture should be accepted");
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].display_name, "case [html]");

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn accepts_success_fixture_with_artifact_assertion() {
    let root = temp_dir("success_contract_artifact_assertion");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n\n[[backends.html.artifact_assertions]]\npath = \"index.html\"\nkind = \"html\"\nmust_contain = [\"ok\"]\n",
    )
    .expect("should write expect file");

    load_canonical_case_specs(&case_root, None)
        .expect("artifact assertion-only fixture should be accepted");

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn accepts_success_fixture_with_rendered_output_assertion() {
    let root = temp_dir("success_contract_rendered_output");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nrendered_output_contains = [\"ok\"]\n",
    )
    .expect("should write expect file");

    load_canonical_case_specs(&case_root, None)
        .expect("rendered-output assertion-only fixture should be accepted");

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn each_new_rendered_output_form_satisfies_success_completeness() {
    let fields = [
        ("exact", "rendered_output_exact = \"\""),
        (
            "ordered",
            "rendered_output_contains_in_order = [\"first\", \"second\"]",
        ),
        (
            "exactly_once",
            "rendered_output_contains_exactly_once = [\"once\"]",
        ),
    ];

    for (name, field) in fields {
        let root = temp_dir(&format!("rendered_output_success_completeness_{name}"));
        let case_root = root.join("case");
        let input_root = case_root.join(INPUT_DIR_NAME);
        fs::create_dir_all(&input_root).expect("should create input directory");
        fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
        fs::write(
            case_root.join(EXPECT_FILE_NAME),
            format!("[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n{field}\n"),
        )
        .expect("should write expect file");

        load_canonical_case_specs(&case_root, None)
            .unwrap_or_else(|error| panic!("{name} should satisfy completeness: {error}"));

        fs::remove_dir_all(&root).expect("should clean up");
    }
}

#[test]
fn accepts_success_fixture_with_artifact_absence_assertion() {
    let root = temp_dir("success_contract_artifact_absence");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nartifacts_must_not_exist = [\"unexpected.html\"]\n",
    )
    .expect("should write expect file");

    load_canonical_case_specs(&case_root, None)
        .expect("artifact-absence assertion-only fixture should be accepted");

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn accepts_success_fixture_with_exact_warning_contract() {
    let root = temp_dir("success_contract_exact_warning");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"exact\"\nwarning_codes = [\"MOTH-RULE-0022\"]\n",
    )
    .expect("should write expect file");

    load_canonical_case_specs(&case_root, None)
        .expect("a non-empty exact-warning contract should be accepted");

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn rejects_failure_fixture_with_authored_golden_mode() {
    let root = temp_dir("failure_golden_mode");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::create_dir_all(&golden_root).expect("should create golden directory");
    fs::write(input_root.join("#page.moth"), "x = 1\n").expect("should write source");
    fs::write(golden_root.join("index.html"), "<h1>ok</h1>\n").expect("should write golden file");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\ngolden_mode = \"strict\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("a failure backend with authored golden_mode must be rejected");
    };
    assert!(
        error.contains("mode = \"failure\"") && error.contains("must not author 'golden_mode'"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn rejects_failure_fixture_with_discovered_file_backed_golden() {
    let root = temp_dir("failure_golden_files");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::create_dir_all(&golden_root).expect("should create golden directory");
    fs::write(input_root.join("#page.moth"), "x = 1\n").expect("should write source");
    fs::write(golden_root.join("index.html"), "<h1>ok</h1>\n").expect("should write golden file");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("a failure backend with discovered golden files must be rejected");
    };
    assert!(
        error.contains("mode = \"failure\"") && error.contains("golden artifacts"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn accepts_failure_fixture_without_any_golden() {
    let root = temp_dir("failure_no_golden");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "x = 1\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
    )
    .expect("should write expect file");

    load_canonical_case_specs(&case_root, None)
        .expect("a failure backend without goldens should be accepted");

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn rejects_baseline_only_success_fixture() {
    let root = temp_dir("baseline_only_success");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("baseline-only success fixture should be rejected");
    };
    assert!(
        error.contains("baseline_only_success")
            && error.contains("html")
            && error.contains("must author at least one accepted success contract"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn default_forbidden_warnings_do_not_satisfy_success_completeness() {
    let root = temp_dir("default_forbidden_warnings_not_contract");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("default warnings = forbid should not satisfy completeness");
    };
    assert!(
        error.contains("warnings = \"exact\" with warning_codes"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn rejects_unsafe_configured_entries() {
    let unsafe_entries = [
        "",
        " /outside ",
        "/outside",
        "../outside",
        "nested/./intro.mtf",
        "C:/outside",
    ];

    for entry in unsafe_entries {
        let root = temp_dir("unsafe_configured_entry");
        let case_root = root.join("case");
        let input_root = case_root.join(INPUT_DIR_NAME);
        fs::create_dir_all(&input_root).expect("should create fixture input directory");
        fs::write(input_root.join("intro.mtf"), "#[:ok]\n")
            .expect("should write nested entry source");
        fs::write(
            case_root.join(EXPECT_FILE_NAME),
            format!(
                "entry = \"{entry}\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n"
            ),
        )
        .expect("should write expect file");

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("unsafe configured entry should be rejected: {entry}");
        };
        assert!(error.contains("invalid entry"), "unexpected: {error}");

        fs::remove_dir_all(&root).expect("should clean up");
    }
}

#[test]
fn accepts_exact_directory_entry_and_returns_canonical_input() {
    let root = temp_dir("exact_directory_entry");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("intro.mtf"), "#[:ok]\n").expect("should write entry source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \".\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("exact directory entry should remain valid");
    assert_eq!(
        cases[0].entry_path,
        fs::canonicalize(input_root).expect("input root should canonicalize")
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[test]
fn accepts_nested_contained_entry_and_returns_canonical_path() {
    let root = temp_dir("nested_contained_entry");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let entry_path = input_root.join("nested").join("intro.mtf");
    fs::create_dir_all(
        entry_path
            .parent()
            .expect("nested input parent should exist"),
    )
    .expect("should create nested input directory");
    fs::write(&entry_path, "#[:ok]\n").expect("should write nested entry source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \"nested/intro.mtf\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let cases =
        load_canonical_case_specs(&case_root, None).expect("nested contained entry should load");
    assert_eq!(
        cases[0].entry_path,
        fs::canonicalize(entry_path).expect("nested entry should canonicalize")
    );

    fs::remove_dir_all(&root).expect("should clean up");
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(unix)]
fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_input_directory_symlink_escape() {
    let root = temp_dir("input_directory_symlink_escape");
    let outside = temp_dir("input_directory_symlink_escape_target");
    let case_root = root.join("case");
    let input_link = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&case_root).expect("should create fixture root");
    fs::create_dir_all(&outside).expect("should create outside input root");
    fs::write(outside.join("#page.moth"), "#[:ok]\n").expect("should write outside source");
    if symlink_directory(&outside, &input_link).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("input directory symlink escaping the fixture should be rejected");
    };
    assert!(
        error.contains("input directory") && error.contains("outside"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_entry_symlink_escape() {
    let root = temp_dir("entry_symlink_escape");
    let outside = temp_dir("entry_symlink_escape_target");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let entry_link = input_root.join("escape.mtf");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&outside).expect("should create outside root");
    let outside_entry = outside.join("intro.mtf");
    fs::write(&outside_entry, "#[:ok]\n").expect("should write outside entry");
    if symlink_file(&outside_entry, &entry_link).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \"escape.mtf\"\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("entry symlink escaping the input directory should be rejected");
    };
    assert!(
        error.contains("entry 'escape.mtf'") && error.contains("outside"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_contained_golden_file_symlink() {
    let root = temp_dir("golden_contained_file_symlink");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_root).expect("should create golden directory");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(golden_root.join("real.html"), "<h1>ok</h1>\n")
        .expect("should write real golden file");
    if symlink_file(
        &golden_root.join("real.html"),
        &golden_root.join("link.html"),
    )
    .is_err()
    {
        fs::remove_dir_all(&root).expect("should clean up root");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("contained golden file symlink should be rejected");
    };
    assert!(error.contains("symlink"), "unexpected: {error}");

    fs::remove_dir_all(&root).expect("should clean up root");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_escaping_golden_file_symlink() {
    let root = temp_dir("golden_escaping_file_symlink");
    let outside = temp_dir("golden_escaping_file_symlink_target");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_root).expect("should create golden directory");
    fs::create_dir_all(&outside).expect("should create outside target");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    let outside_file = outside.join("stolen.html");
    fs::write(&outside_file, "<h1>stolen</h1>\n").expect("should write outside golden");
    if symlink_file(&outside_file, &golden_root.join("escape.html")).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("escaping golden file symlink should be rejected");
    };
    assert!(error.contains("symlink"), "unexpected: {error}");

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_golden_directory_symlink() {
    let root = temp_dir("golden_directory_symlink");
    let outside = temp_dir("golden_directory_symlink_target");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_root).expect("should create golden directory");
    fs::create_dir_all(&outside).expect("should create outside target");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(outside.join("nested.html"), "<h1>stolen</h1>\n")
        .expect("should write outside golden");
    if symlink_directory(&outside, &golden_root.join("linked_dir")).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("golden directory symlink should be rejected");
    };
    assert!(error.contains("symlink"), "unexpected: {error}");

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_backend_golden_root_symlink() {
    let root = temp_dir("golden_root_symlink");
    let outside = temp_dir("golden_root_symlink_target");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_parent = case_root.join(GOLDEN_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&golden_parent).expect("should create golden parent directory");
    fs::create_dir_all(&outside).expect("should create outside target");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(outside.join("index.html"), "<h1>stolen</h1>\n")
        .expect("should write outside golden");
    if symlink_directory(&outside, &golden_parent.join("html")).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("backend golden root symlink should be rejected");
    };
    assert!(
        error.contains("Golden path") && error.contains("symlink"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_golden_parent_symlink() {
    let root = temp_dir("golden_parent_symlink");
    let outside = temp_dir("golden_parent_symlink_target");
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_parent = case_root.join(GOLDEN_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::create_dir_all(&outside).expect("should create outside target");
    fs::write(input_root.join("#page.moth"), "#[:ok]\n").expect("should write fixture source");
    let outside_golden_root = outside.join("html");
    fs::create_dir_all(&outside_golden_root).expect("should create outside golden backend");
    fs::write(outside_golden_root.join("index.html"), "<h1>stolen</h1>\n")
        .expect("should write outside golden");
    if symlink_directory(&outside, &golden_parent).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("golden parent symlink should be rejected");
    };
    assert!(
        error.contains("Golden parent") && error.contains("symlink"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}
