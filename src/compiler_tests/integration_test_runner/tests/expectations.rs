//! Self-tests for integration expectation schema parsing.
//!
//! WHAT: protects backend matrix contracts and expectation-only validation.
//! WHY: malformed expectations must fail before fixture execution begins.

use super::super::expectations::parse_expectation_file;
use super::super::fixture::load_canonical_case_specs;
use super::super::types::{
    DiagnosticMatchMode, ExactWarningExpectation, SuccessContract, WarningExpectation,
};
use super::super::{EXPECT_FILE_NAME, ExpectedOutcome, GOLDEN_DIR_NAME, INPUT_DIR_NAME};
use std::fs;
use std::path::PathBuf;

fn write_fixture(_name: &str, expectation_source: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(case_root.join(EXPECT_FILE_NAME), expectation_source)
        .expect("should write expect file");
    (temp, case_root)
}

#[test]
fn accepts_explicit_acceptance_only_and_retains_typed_intent() {
    let (_root, case_root) = write_fixture(
        "explicit_acceptance_only",
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("explicit acceptance-only fixture should be accepted");
    let ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert_eq!(
        expectation.success_contract,
        Some(SuccessContract::AcceptanceOnly)
    );
}

#[test]
fn failure_diagnostic_match_defaults_to_exact_and_is_retained() {
    let (_root, case_root) = write_fixture(
        "default_diagnostic_match",
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("failure fixture should use exact diagnostic matching by default");
    let super::super::types::ExpectedOutcome::Failure(expectation) = &cases[0].expected else {
        panic!("case should have a failure expectation");
    };
    assert_eq!(expectation.diagnostic_match, DiagnosticMatchMode::Exact);
    assert_eq!(expectation.diagnostic_match_reason, None);
}

#[test]
fn diagnostic_codes_reject_a_blank_identity_entry() {
    let (_root, case_root) = write_fixture(
        "diagnostic_codes_blank",
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"   \"]\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("a blank diagnostic-code identity must be rejected");
    };
    assert!(
        error.contains("empty 'diagnostic_codes' entry"),
        "unexpected error: {error}"
    );
}

#[test]
fn diagnostic_codes_keep_exact_multisets_and_duplicates() {
    let (_root, case_root) = write_fixture(
        "diagnostic_codes_duplicates",
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0044\", \"MOTH-RULE-0044\"]\n",
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("duplicate diagnostic codes remain a valid exact multiset");
    let super::super::types::ExpectedOutcome::Failure(expectation) = &cases[0].expected else {
        panic!("case should have a failure expectation");
    };
    assert_eq!(
        expectation.diagnostic_codes,
        vec!["MOTH-RULE-0044".to_owned(), "MOTH-RULE-0044".to_owned()]
    );
}

#[test]
fn structured_diagnostic_assertions_parse_and_normalize_locations() {
    let (_root, case_root) = write_fixture(
        "structured_diagnostic_assertions",
        concat!(
            "[backends.html]\n",
            "mode = \"failure\"\n",
            "warnings = \"forbid\"\n",
            "diagnostic_codes = [\"MOTH-RULE-0044\"]\n",
            "\n",
            "[[backends.html.diagnostic_assertions]]\n",
            "code = \"MOTH-RULE-0044\"\n",
            "reason = \"invalid_assignment_target.immutable_binding\"\n",
            "path = \"input\\\\main.moth\"\n",
            "line = 3\n",
            "column = 2\n",
            "count = 1\n",
            "\n",
            "[[backends.html.diagnostic_assertions.secondary_labels]]\n",
            "occurrence = 1\n",
            "path = \"input\\\\helper.moth\"\n",
            "line = 4\n",
            "column = 5\n",
        ),
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("structured diagnostic assertions should be accepted");
    let ExpectedOutcome::Failure(expectation) = &cases[0].expected else {
        panic!("case should have a failure expectation");
    };
    let assertion = &expectation.diagnostic_assertions[0];
    assert_eq!(assertion.occurrence, 1);
    assert_eq!(
        assertion.reason.as_deref(),
        Some("invalid_assignment_target.immutable_binding")
    );
    assert_eq!(assertion.path.as_deref(), Some("input/main.moth"));
    assert_eq!(assertion.line, Some(3));
    assert_eq!(assertion.column, Some(2));
    assert_eq!(assertion.count, Some(1));
    assert_eq!(assertion.secondary_labels[0].occurrence, 1);
    assert_eq!(
        assertion.secondary_labels[0].path.as_deref(),
        Some("input/helper.moth")
    );
}

#[test]
fn unique_structured_diagnostic_code_defaults_occurrence_to_one() {
    let (_root, case_root) = write_fixture(
        "structured_unique_occurrence",
        concat!(
            "[backends.html]\n",
            "mode = \"failure\"\n",
            "warnings = \"forbid\"\n",
            "diagnostic_codes = [\"MOTH-RULE-0044\"]\n",
            "\n",
            "[[backends.html.diagnostic_assertions]]\n",
            "code = \"MOTH-RULE-0044\"\n",
            "line = 1\n",
        ),
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("unique structured diagnostic code should default occurrence");
    let ExpectedOutcome::Failure(expectation) = &cases[0].expected else {
        panic!("case should have a failure expectation");
    };
    assert_eq!(expectation.diagnostic_assertions[0].occurrence, 1);
}

#[test]
fn repeated_structured_diagnostic_code_requires_explicit_valid_unique_occurrences() {
    let cases = [
        (
            "ambiguous",
            "[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\nline = 1\n",
            "must author 'occurrence'",
        ),
        (
            "zero",
            "[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\noccurrence = 0\nline = 1\n",
            "one-based",
        ),
        (
            "beyond_multiplicity",
            "[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\noccurrence = 3\nline = 1\n",
            "contains it 2 time(s)",
        ),
    ];

    for (name, assertion, expected_error) in cases {
        let (_root, case_root) = write_fixture(
            &format!("structured_occurrence_{name}"),
            &format!(
                "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0044\", \"MOTH-RULE-0044\"]\n\n{assertion}"
            ),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("invalid occurrence selection should be rejected: {name}");
        };
        assert!(error.contains(expected_error), "unexpected error: {error}");
    }
}

#[test]
fn structured_diagnostic_assertions_reject_absent_duplicate_and_empty_selectors() {
    let cases = [
        (
            "absent",
            "diagnostic_codes = [\"MOTH-RULE-0044\"]\n\n[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-SYNTAX-0003\"\nline = 1\n",
            "absent from 'diagnostic_codes'",
        ),
        (
            "duplicate",
            "diagnostic_codes = [\"MOTH-RULE-0044\"]\n\n[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\nline = 1\n\n[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\ncolumn = 2\n",
            "duplicates diagnostic code 'MOTH-RULE-0044' occurrence 1",
        ),
        (
            "empty",
            "diagnostic_codes = [\"MOTH-RULE-0044\"]\n\n[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\n",
            "at least one structured diagnostic fact",
        ),
    ];

    for (name, fields, expected_error) in cases {
        let (_root, case_root) = write_fixture(
            &format!("structured_selector_{name}"),
            &format!("[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\n{fields}"),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("invalid structured selector should be rejected: {name}");
        };
        assert!(error.contains(expected_error), "unexpected error: {error}");
    }
}

#[test]
fn structured_diagnostic_assertions_validate_reason_location_and_count_shape() {
    let cases = [
        (
            "reason",
            "reason = \"Invalid.Reason\"\nline = 1\n",
            "lowercase snake-case",
        ),
        ("path", "path = \"   \"\n", "non-empty 'path'"),
        (
            "absolute_path",
            "path = \"/tmp/main.moth\"\n",
            "must be a relative path",
        ),
        (
            "parent_path",
            "path = \"../main.moth\"\n",
            "authored parent component",
        ),
        ("line", "line = 0\n", "positive 'line'"),
        ("column", "column = 0\n", "positive 'column'"),
        ("count", "count = 2\nline = 1\n", "'count = 2'"),
    ];

    for (name, fields, expected_error) in cases {
        let (_root, case_root) = write_fixture(
            &format!("structured_shape_{name}"),
            &format!(
                "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0044\"]\n\n[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\n{fields}"
            ),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("invalid structured assertion shape should be rejected: {name}");
        };
        assert!(error.contains(expected_error), "unexpected error: {error}");
    }
}

#[test]
fn structured_secondary_labels_require_occurrence_and_location_fact() {
    let cases = [
        (
            "missing_occurrence",
            "path = \"helper.moth\"\nline = 2\n",
            "requires a one-based 'occurrence'",
        ),
        (
            "missing_location",
            "occurrence = 1\n",
            "at least one secondary-label location fact",
        ),
    ];

    for (name, secondary_fields, expected_error) in cases {
        let (_root, case_root) = write_fixture(
            &format!("structured_secondary_{name}"),
            &format!(
                "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0044\"]\n\n[[backends.html.diagnostic_assertions]]\ncode = \"MOTH-RULE-0044\"\nline = 1\n\n[[backends.html.diagnostic_assertions.secondary_labels]]\n{secondary_fields}"
            ),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("invalid secondary-label assertion should be rejected: {name}");
        };
        assert!(error.contains(expected_error), "unexpected error: {error}");
    }
}

#[test]
fn structured_diagnostic_assertions_are_failure_only() {
    let (_root, case_root) = write_fixture(
        "structured_success_rejection",
        concat!(
            "[backends.html]\n",
            "mode = \"success\"\n",
            "warnings = \"forbid\"\n",
            "diagnostic_codes = [\"MOTH-RULE-0044\"]\n",
            "rendered_output_contains = [\"ok\"]\n",
            "\n",
            "[[backends.html.diagnostic_assertions]]\n",
            "code = \"MOTH-RULE-0044\"\n",
            "line = 1\n",
        ),
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("structured assertions on a success backend should be rejected");
    };
    assert!(
        error.contains("failure-only") && error.contains("diagnostic_assertions"),
        "unexpected error: {error}"
    );
}

#[test]
fn exact_warning_codes_are_retained_as_a_typed_multiset_contract() {
    let (_root, case_root) = write_fixture(
        "exact_warning_codes",
        "[backends.html]\nmode = \"success\"\nwarnings = \"exact\"\nwarning_codes = [\"MOTH-RULE-0022\", \"MOTH-RULE-0010\"]\n",
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("exact warning-code fixture should be accepted");
    let super::super::types::ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert_eq!(
        expectation.warnings,
        WarningExpectation::Exact(ExactWarningExpectation {
            expected_codes: vec!["MOTH-RULE-0022".to_owned(), "MOTH-RULE-0010".to_owned(),],
        })
    );
}

#[test]
fn exact_warning_codes_reject_an_authored_empty_multiset() {
    let (_root, case_root) = write_fixture(
        "exact_warning_codes_empty",
        "[backends.html]\nmode = \"success\"\nwarnings = \"exact\"\nwarning_codes = []\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("an authored empty warning-code list must not satisfy success completeness");
    };
    assert!(
        error.contains("warnings = \"exact\"") && error.contains("empty"),
        "unexpected error: {error}"
    );
}

#[test]
fn exact_warning_codes_reject_a_blank_identity_entry() {
    let (_root, case_root) = write_fixture(
        "exact_warning_codes_blank",
        "[backends.html]\nmode = \"success\"\nwarnings = \"exact\"\nwarning_codes = [\"   \"]\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("a blank warning-code identity must be rejected");
    };
    assert!(
        error.contains("empty 'warning_codes' entry"),
        "unexpected error: {error}"
    );
}

#[test]
fn removed_warning_count_spelling_is_rejected() {
    let cases = [
        (
            "backend",
            "[backends.html]\nmode = \"failure\"\nwarnings = \"exact\"\nwarning_count = 1\nwarning_codes = [\"MOTH-RULE-0022\", \"MOTH-RULE-0010\"]\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
        ),
        (
            "top_level",
            "warning_count = 1\n[backends.html]\nmode = \"failure\"\nwarnings = \"exact\"\nwarning_codes = [\"MOTH-RULE-0022\", \"MOTH-RULE-0010\"]\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
        ),
    ];

    for (shape, expectation_source) in cases {
        let (_root, case_root) = write_fixture(
            &format!("removed_warning_count_spelling_{shape}"),
            expectation_source,
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("the removed warning_count spelling should be rejected");
        };
        assert!(
            error.contains("warning_count") && error.contains("unknown field"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn exact_warning_expectations_require_warning_codes() {
    let (_root, case_root) = write_fixture(
        "exact_warning_missing_identity",
        "[backends.html]\nmode = \"success\"\nwarnings = \"exact\"\nrendered_output_contains = [\"ok\"]\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("exact warnings without a code list should be rejected");
    };
    assert!(
        error.contains("warning_codes") && error.contains("warnings = \"exact\""),
        "unexpected error: {error}"
    );
}

#[test]
fn ignore_and_forbid_reject_warning_identity_fields() {
    for mode in ["ignore", "forbid"] {
        let (_root, case_root) = write_fixture(
            &format!("{mode}_warning_identity_fields"),
            &format!(
                "[backends.html]\nmode = \"success\"\nwarnings = \"{mode}\"\nwarning_codes = []\nsuccess_contract = \"acceptance_only\"\n"
            ),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("{mode} warnings with identity fields should be rejected");
        };
        assert!(
            error.contains("warning_codes") && error.contains("warnings != \"exact\""),
            "unexpected error for {mode}: {error}"
        );
    }
}

#[test]
fn justified_contains_diagnostic_match_is_retained() {
    let (_root, case_root) = write_fixture(
        "contains_diagnostic_match",
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\ndiagnostic_match = \"contains\"\ndiagnostic_match_reason = \"independent recovery\"\n",
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("justified contains matching should be accepted");
    let super::super::types::ExpectedOutcome::Failure(expectation) = &cases[0].expected else {
        panic!("case should have a failure expectation");
    };
    assert_eq!(expectation.diagnostic_match, DiagnosticMatchMode::Contains);
    assert_eq!(
        expectation.diagnostic_match_reason.as_deref(),
        Some("independent recovery")
    );
}

#[test]
fn contains_diagnostic_match_retains_missing_and_blank_reasons_for_policy() {
    for (name, reason) in [("missing", None), ("blank", Some("   "))] {
        let reason_line = reason.map_or_else(String::new, |value| {
            format!("diagnostic_match_reason = \"{value}\"\n")
        });
        let (_root, case_root) = write_fixture(
            &format!("contains_without_reason_{name}"),
            &format!(
                "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\ndiagnostic_match = \"contains\"\n{reason_line}"
            ),
        );

        let cases = load_canonical_case_specs(&case_root, None)
            .expect("contains matching without a non-blank reason should reach policy evaluation");
        let super::super::types::ExpectedOutcome::Failure(expectation) = &cases[0].expected else {
            panic!("case should have a failure expectation");
        };
        assert!(
            expectation.diagnostic_match == DiagnosticMatchMode::Contains,
            "contains mode should be retained"
        );
        assert_eq!(expectation.diagnostic_match_reason.as_deref(), reason);
    }
}

#[test]
fn exact_diagnostic_match_rejects_authored_reason() {
    let (_root, case_root) = write_fixture(
        "exact_diagnostic_match_reason",
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\ndiagnostic_match = \"exact\"\ndiagnostic_match_reason = \"not allowed\"\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("exact matching with a reason should be rejected");
    };
    assert!(
        error.contains("diagnostic_match_reason") && error.contains("exact"),
        "unexpected error: {error}"
    );
}

#[test]
fn diagnostic_match_fields_are_failure_only() {
    let (_root, case_root) = write_fixture(
        "success_diagnostic_match_field",
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\ndiagnostic_match = \"exact\"\nrendered_output_contains = [\"ok\"]\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("diagnostic_match should be rejected on success expectations");
    };
    assert!(
        error.contains("failure-only") && error.contains("diagnostic_match"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_unknown_success_contract_value_with_backend_context() {
    let (_root, case_root) = write_fixture(
        "unknown_success_contract",
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"typecheck_only\"\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("unknown success_contract should be rejected");
    };
    assert!(
        error.contains("success_contract")
            && error.contains("typecheck_only")
            && error.contains("[backends.html]"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_acceptance_only_on_failure_backend() {
    let (_root, case_root) = write_fixture(
        "acceptance_only_failure_mode",
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("acceptance_only on a failure backend should be rejected");
    };
    assert!(
        error.contains("mode = \"failure\"") && error.contains("success_contract"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_acceptance_only_mixed_with_success_assertions() {
    let mixed_contracts = [
        (
            "artifact",
            "\n[[backends.html.artifact_assertions]]\npath = \"index.html\"\nkind = \"html\"\nmust_contain = [\"ok\"]\n",
        ),
        ("golden_mode", "\ngolden_mode = \"normalized\"\n"),
        ("rendered_output", "\nrendered_output_contains = [\"ok\"]\n"),
        (
            "artifact_absence",
            "\nartifacts_must_not_exist = [\"unexpected.html\"]\n",
        ),
    ];

    for (name, extra_contract) in mixed_contracts {
        let (_root, case_root) = write_fixture(
            &format!("acceptance_only_mixed_{name}"),
            &format!(
                "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n{extra_contract}"
            ),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("acceptance-only mixed with {name} should be rejected");
        };
        assert!(
            error.contains("acceptance_only") && error.contains("must not combine"),
            "unexpected error for {name}: {error}"
        );
    }
}

#[test]
fn rejects_removed_success_contract_spelling() {
    let removed_contract = ["compile", "_only"].concat();
    let (_root, case_root) = write_fixture(
        "removed_success_contract_spelling",
        &format!(
            "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"{removed_contract}\"\n"
        ),
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("the removed success contract spelling should be rejected");
    };
    assert!(
        error.contains(&removed_contract) && error.contains("acceptance_only"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_acceptance_only_with_authored_expected_warning() {
    let (_root, case_root) = write_fixture(
        "acceptance_only_expected_warning",
        "[backends.html]\nmode = \"success\"\nwarnings = \"exact\"\nwarning_codes = [\"MOTH-RULE-0022\"]\nsuccess_contract = \"acceptance_only\"\n",
    );

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("acceptance-only with an authored expected warning should be rejected");
    };
    assert!(
        error.contains("acceptance_only") && error.contains("expected-warning"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_error_type_expectation_key() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "x = 1\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"forbid\"\nerror_type = \"rule\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = parse_expectation_file(&case_root.join(EXPECT_FILE_NAME)) else {
        panic!("error_type key should be rejected");
    };
    assert!(error.contains("unknown field"), "unexpected error: {error}");
}

#[test]
fn rejects_legacy_top_level_expectation_contract() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "mode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("legacy fixture should be rejected");
    };
    assert!(
        error.contains("[backends.<id>]"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_backend_panic_expectation_key() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\npanic = true\nwarnings = \"forbid\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\nmessage_contains = [\"x\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = parse_expectation_file(&case_root.join(EXPECT_FILE_NAME)) else {
        panic!("panic key should be rejected");
    };
    assert!(error.contains("unknown field"), "unexpected error: {error}");
}

#[test]
fn rejects_top_level_panic_expectation_key() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "panic = true\n\n[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = parse_expectation_file(&case_root.join(EXPECT_FILE_NAME)) else {
        panic!("panic key should be rejected");
    };
    assert!(error.contains("unknown field"), "unexpected error: {error}");
}

#[test]
fn rejects_unknown_backend_matrix_key() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "entry = \".\"\n\n[backends.wasm]\nmode = \"success\"\nwarnings = \"forbid\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("fixture should be rejected");
    };
    assert!(
        error.contains("Unsupported backend 'wasm'"),
        "unexpected error: {error}"
    );
}

#[test]
fn accepts_normalized_golden_mode() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    let golden_root = case_root.join(GOLDEN_DIR_NAME).join("html");
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::create_dir_all(&golden_root).expect("should create golden directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(golden_root.join("index.html"), "<h1>ok</h1>\n").expect("should write golden");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\ngolden_mode = \"normalized\"\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("normalized golden_mode fixture should be accepted");
    assert_eq!(cases.len(), 1);
}

#[test]
fn rejects_unknown_golden_mode() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\ngolden_mode = \"fuzzy\"\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("unknown golden_mode should be rejected");
    };
    assert!(
        error.contains("golden_mode") && error.contains("fuzzy"),
        "unexpected error: {error}"
    );
}

#[test]
fn accepts_success_fixture_with_rendered_output_only() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nrendered_output_contains = [\"ok\"]\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("rendered_output_contains-only fixture should be accepted");
    assert_eq!(cases.len(), 1);
}

#[test]
fn parses_and_retains_all_rendered_output_assertion_forms() {
    let (_root, case_root) = write_fixture(
        "rendered_output_forms",
        concat!(
            "[backends.html]\n",
            "mode = \"success\"\n",
            "warnings = \"forbid\"\n",
            "rendered_output_contains = [\"contains\"]\n",
            "rendered_output_not_contains = [\"forbidden\"]\n",
            "rendered_output_contains_in_order = [\"first\", \"second\", \"first\"]\n",
            "rendered_output_contains_exactly_once = [\"once\"]\n",
        ),
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("all rendered-output forms should satisfy success completeness");
    let ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert_eq!(expectation.rendered_output.exact, None);
    assert_eq!(
        expectation.rendered_output.contains,
        vec!["contains".to_owned()]
    );
    assert_eq!(
        expectation.rendered_output.not_contains,
        vec!["forbidden".to_owned()]
    );
    assert_eq!(
        expectation.rendered_output.contains_in_order,
        vec!["first", "second", "first"]
    );
    assert_eq!(
        expectation.rendered_output.contains_exactly_once,
        vec!["once".to_owned()]
    );
}

#[test]
fn exact_output_accepts_authored_empty_string() {
    let (_root, case_root) = write_fixture(
        "rendered_output_exact_empty",
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nrendered_output_exact = \"\"\n",
    );

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("empty exact output should be a valid success contract");
    let ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert_eq!(expectation.rendered_output.exact.as_deref(), Some(""));
    assert!(expectation.rendered_output.is_present());
}

#[test]
fn rejects_exact_output_combined_with_each_other_rendered_form() {
    let fields = [
        "rendered_output_contains = [\"contains\"]",
        "rendered_output_not_contains = [\"forbidden\"]",
        "rendered_output_contains_in_order = [\"first\", \"second\"]",
        "rendered_output_contains_exactly_once = [\"once\"]",
    ];

    for (index, field) in fields.iter().enumerate() {
        let (_root, case_root) = write_fixture(
            &format!("rendered_output_exact_exclusive_{index}"),
            &format!(
                "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nrendered_output_exact = \"exact\"\n{field}\n"
            ),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("exact output combined with {field} should be rejected");
        };
        assert!(error.contains("rendered_output_exact"), "{error}");
        assert!(error.contains("must not combine"), "{error}");
    }
}

#[test]
fn rejects_each_new_rendered_form_in_acceptance_only_and_failure_modes() {
    let fields = [
        "rendered_output_exact = \"exact\"",
        "rendered_output_contains_in_order = [\"first\", \"second\"]",
        "rendered_output_contains_exactly_once = [\"once\"]",
    ];

    for (index, field) in fields.iter().enumerate() {
        for (mode, mode_label, suffix) in [
            (
                "success",
                "acceptance_only",
                "success_contract = \"acceptance_only\"",
            ),
            (
                "failure",
                "failure",
                "diagnostic_codes = [\"MOTH-RULE-0001\"]",
            ),
        ] {
            let (_root, case_root) = write_fixture(
                &format!("rendered_output_rejected_{index}_{mode_label}"),
                &format!(
                    "[backends.html]\nmode = \"{mode}\"\nwarnings = \"forbid\"\n{suffix}\n{field}\n"
                ),
            );

            let Err(error) = load_canonical_case_specs(&case_root, None) else {
                panic!("{field} should be rejected in {mode_label} mode");
            };
            // Both lanes reject the rendered form: acceptance_only cannot combine
            // with rendered-output assertions, and failure mode forbids them.
            assert!(
                error.contains("rendered_output") || error.contains("rendered-output"),
                "unexpected error: {error}"
            );
        }
    }
}

#[test]
fn rejects_invalid_ordered_and_exactly_once_authored_lists() {
    let cases = [
        (
            "ordered_minimum",
            "rendered_output_contains_in_order = [\"only\"]",
            "at least two",
        ),
        (
            "ordered_empty_value",
            "rendered_output_contains_in_order = [\"first\", \"\"]",
            "empty",
        ),
        (
            "ordered_empty_list",
            "rendered_output_contains_in_order = []",
            "at least two",
        ),
        (
            "exactly_once_empty_value",
            "rendered_output_contains_exactly_once = [\"\"]",
            "empty",
        ),
        (
            "exactly_once_empty_list",
            "rendered_output_contains_exactly_once = []",
            "at least one",
        ),
        (
            "exactly_once_duplicate",
            "rendered_output_contains_exactly_once = [\"same\", \"same\"]",
            "duplicate",
        ),
    ];

    for (name, field, expected_error) in cases {
        let (_root, case_root) = write_fixture(
            &format!("rendered_output_invalid_{name}"),
            &format!("[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n{field}\n"),
        );

        let Err(error) = load_canonical_case_specs(&case_root, None) else {
            panic!("{field} should be rejected");
        };
        assert!(error.contains(expected_error), "unexpected error: {error}");
    }
}

#[test]
fn rejects_rendered_output_in_failure_mode() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"ignore\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\nmessage_contains = [\"x\"]\nrendered_output_contains = [\"y\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("rendered_output_contains in failure mode should be rejected");
    };
    assert!(
        error.contains("rendered_output_contains"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_normalized_contains_on_wasm_artifact() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\n\n[[backends.html.artifact_assertions]]\npath = \"page.wasm\"\nkind = \"wasm\"\nnormalized_contains = [\"something\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("normalized_contains on wasm should be rejected");
    };
    assert!(error.contains("text-only"), "unexpected error: {error}");
}

#[test]
fn accepts_artifacts_must_not_exist_in_success_mode() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nartifacts_must_not_exist = [\"api\\\\index.html\"]\n",
    )
    .expect("should write expect file");

    let cases = load_canonical_case_specs(&case_root, None)
        .expect("artifacts_must_not_exist in success mode should be accepted");
    assert_eq!(cases.len(), 1);

    let ExpectedOutcome::Success(expectation) = &cases[0].expected else {
        panic!("case should have a success expectation");
    };
    assert_eq!(
        expectation.artifacts_must_not_exist,
        vec!["api/index.html".to_string()],
        "non-canonical backslash separator should be normalised to forward slashes"
    );
}

#[test]
fn rejects_artifacts_must_not_exist_in_failure_mode() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"failure\"\nwarnings = \"ignore\"\ndiagnostic_codes = [\"MOTH-RULE-0001\"]\nartifacts_must_not_exist = [\"api/index.html\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("artifacts_must_not_exist in failure mode should be rejected");
    };
    assert!(
        error.contains("artifacts_must_not_exist"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_empty_artifacts_must_not_exist_entry() {
    let _root_temp = tempfile::tempdir().expect("should create fixture temp dir");
    let root = _root_temp.path().to_path_buf();
    let case_root = root.join("case");
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nartifacts_must_not_exist = [\"\"]\n",
    )
    .expect("should write expect file");

    let Err(error) = load_canonical_case_specs(&case_root, None) else {
        panic!("empty artifacts_must_not_exist entry should be rejected");
    };
    assert!(
        error.contains("empty") && error.contains("artifacts_must_not_exist"),
        "unexpected error: {error}"
    );
}
