//! Self-tests for integration result and artifact assertions.
//!
//! WHAT: protects diagnostic rendering, text normalization, and artifact absence contracts.
//! WHY: assertion regressions can silently weaken the suite without changing compilation.

use super::super::assertions::{
    ArtifactIndexError, HtmlShellViolation, build_artifact_index_error, compare_text_golden,
    discover_golden_expectation, html_shell_violation, normalize_text_for_comparison,
    validate_failure_result, validate_golden_outputs, validate_success_result,
};
use super::super::types::{
    DiagnosticAssertion, ExactWarningExpectation, GoldenExpectation, SecondaryLabelAssertion,
};
use super::super::{
    BackendId, DiagnosticMatchMode, ExpectedOutcome, FailureExpectation, FailureKind, GoldenMode,
    SuccessExpectation, TestCaseSpec, WarningExpectation,
};
use super::synthetic_build_results::{
    VALID_HTML, VALID_HTML_WASM, acceptance_only_expectation, build_result_with_index_html,
    build_result_with_output_files, success_test_case,
};
use crate::build_system::build::{BuildResult, FileKind};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, InvalidAssignmentTargetReason,
    InvalidOutputFolderReason,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::{Path, PathBuf};

const DIAGNOSTICS_SOURCE: &str = include_str!("../assertions/diagnostics.rs");

fn test_location(path: InternedPath) -> SourceLocation {
    test_location_at(path, 0, 0)
}

fn test_location_at(
    path: InternedPath,
    raw_line_number: i32,
    raw_char_column: i32,
) -> SourceLocation {
    SourceLocation::new(
        path,
        CharPosition {
            line_number: raw_line_number,
            char_column: raw_char_column,
        },
        CharPosition {
            line_number: raw_line_number,
            char_column: raw_char_column + 1,
        },
    )
}

fn diagnostic_messages(codes: &[&str]) -> CompilerMessages {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let diagnostics = codes
        .iter()
        .map(|code| match *code {
            "MOTH-RULE-0044" => CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::ImmutableBinding,
                None,
                None,
                None,
                None,
                None,
                test_location(source_path.clone()),
            ),
            "MOTH-SYNTAX-0003" => {
                CompilerDiagnostic::unexpected_trailing_comma(test_location(source_path.clone()))
            }
            other => panic!("test diagnostic code is not constructed: {other}"),
        })
        .collect();

    CompilerMessages::from_diagnostics(diagnostics, string_table)
}

fn diagnostic_expectation(
    expected_codes: &[&str],
    diagnostic_match: DiagnosticMatchMode,
    diagnostic_match_reason: Option<&str>,
) -> FailureExpectation {
    FailureExpectation {
        warnings: WarningExpectation::Ignore,
        message_contains: Vec::new(),
        diagnostic_codes: expected_codes
            .iter()
            .map(|code| (*code).to_owned())
            .collect(),
        diagnostic_assertions: Vec::new(),
        diagnostic_match,
        diagnostic_match_reason: diagnostic_match_reason.map(str::to_owned),
    }
}

#[test]
fn failure_message_contains_uses_structured_render_output() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let variable_name = string_table.intern("value");
    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(variable_name),
        None,
        None,
        None,
        None,
        test_location(source_path),
    );
    let messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    let expectation = FailureExpectation {
        warnings: WarningExpectation::Ignore,
        diagnostic_codes: vec!["MOTH-RULE-0044".to_string()],
        diagnostic_assertions: Vec::new(),
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
        message_contains: vec!["Cannot reassign `value`".to_string()],
    };

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn failure_message_contains_includes_rendered_label_text() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let label_text = string_table.intern("secondary context lives here");
    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        None,
        None,
        None,
        None,
        None,
        test_location(source_path.clone()),
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        test_location(source_path),
        Some(DiagnosticLabelMessage::RenderedText(label_text)),
    )]);
    let messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    let expectation = FailureExpectation {
        warnings: WarningExpectation::Ignore,
        diagnostic_codes: vec!["MOTH-RULE-0044".to_string()],
        diagnostic_assertions: Vec::new(),
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
        message_contains: vec!["secondary context lives here".to_string()],
    };

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

/// Source-text architecture ban, not behavior evidence.
///
/// WHAT: checks that the diagnostic assertion owner does not reintroduce the removed legacy
///       error conversion by name.
/// WHY: the behavior this protects — failure-message assertions reading typed render-boundary
///      output — is owned by `failure_message_contains_uses_structured_render_output` and
///      `failure_message_contains_includes_rendered_label_text` above. Text matching cannot
///      prove that behavior: an alias, a reformat or an equivalent reimplementation would all
///      pass. This ban is kept only as a cheap reintroduction tripwire and is scheduled to move
///      into the owned structured architecture audit.
#[test]
fn diagnostics_source_does_not_name_the_removed_legacy_error_conversion() {
    let removed_conversion_name = ["to", "_", "legacy", "_", "error"].concat();

    assert!(
        !DIAGNOSTICS_SOURCE.contains(&removed_conversion_name),
        "the removed legacy error conversion must not be reintroduced by name",
    );
}

#[test]
fn exact_diagnostic_matching_ignores_order() {
    let messages = diagnostic_messages(&["MOTH-SYNTAX-0003", "MOTH-RULE-0044"]);
    let expectation = diagnostic_expectation(
        &["MOTH-RULE-0044", "MOTH-SYNTAX-0003"],
        DiagnosticMatchMode::Exact,
        None,
    );

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn exact_diagnostic_matching_reports_unexpected_extra() {
    let messages = diagnostic_messages(&["MOTH-RULE-0044", "MOTH-SYNTAX-0003"]);
    let expectation = diagnostic_expectation(&["MOTH-RULE-0044"], DiagnosticMatchMode::Exact, None);

    let result = validate_failure_result(messages, &expectation, Path::new("."));
    let reason = result
        .failure_reason
        .expect("unexpected diagnostic should fail matching");

    assert!(
        reason.contains("Unexpected codes: MOTH-SYNTAX-0003"),
        "{reason}"
    );
    assert!(!reason.contains("Missing codes"), "{reason}");
}

#[test]
fn exact_diagnostic_matching_reports_duplicate_count_mismatch() {
    let messages = diagnostic_messages(&["MOTH-RULE-0044", "MOTH-RULE-0044"]);
    let expectation = diagnostic_expectation(&["MOTH-RULE-0044"], DiagnosticMatchMode::Exact, None);

    let result = validate_failure_result(messages, &expectation, Path::new("."));
    let reason = result
        .failure_reason
        .expect("duplicate diagnostic should fail matching");

    assert!(reason.contains("Count-mismatched codes"), "{reason}");
    assert!(reason.contains("expected 1, actual 2"), "{reason}");
    assert!(!reason.contains("Unexpected codes"), "{reason}");
}

#[test]
fn exact_diagnostic_matching_keeps_missing_and_unexpected_categories_distinct() {
    let messages = diagnostic_messages(&["MOTH-SYNTAX-0003"]);
    let expectation = diagnostic_expectation(&["MOTH-RULE-0044"], DiagnosticMatchMode::Exact, None);

    let result = validate_failure_result(messages, &expectation, Path::new("."));
    let reason = result
        .failure_reason
        .expect("different diagnostic should fail matching");

    assert!(reason.contains("Missing codes: MOTH-RULE-0044"), "{reason}");
    assert!(
        reason.contains("Unexpected codes: MOTH-SYNTAX-0003"),
        "{reason}"
    );
    assert!(!reason.contains("Count-mismatched codes"), "{reason}");
}

#[test]
fn justified_contains_matching_accepts_extra_diagnostics() {
    let messages = diagnostic_messages(&["MOTH-RULE-0044", "MOTH-SYNTAX-0003"]);
    let expectation = diagnostic_expectation(
        &["MOTH-RULE-0044"],
        DiagnosticMatchMode::Contains,
        Some("independent parser recovery can emit a second diagnostic"),
    );

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn justified_contains_matching_accepts_extra_expected_code_occurrences() {
    let messages = diagnostic_messages(&["MOTH-RULE-0044", "MOTH-RULE-0044"]);
    let expectation = diagnostic_expectation(
        &["MOTH-RULE-0044"],
        DiagnosticMatchMode::Contains,
        Some("independent recovery may repeat this diagnostic"),
    );

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn contains_matching_requires_every_expected_occurrence() {
    let messages = diagnostic_messages(&["MOTH-RULE-0044"]);
    let expectation = diagnostic_expectation(
        &["MOTH-RULE-0044", "MOTH-RULE-0044"],
        DiagnosticMatchMode::Contains,
        Some("two independent sites must report the same diagnostic"),
    );

    let result = validate_failure_result(messages, &expectation, Path::new("."));
    let reason = result
        .failure_reason
        .expect("a missing expected occurrence should fail matching");

    assert!(reason.contains("Count-mismatched codes"), "{reason}");
    assert!(reason.contains("expected 2, actual 1"), "{reason}");
    assert!(!reason.contains("Unexpected codes"), "{reason}");
}

/// Renders one fixture-relative path as the UTF-8 text a diagnostic location carries.
///
/// A lossy conversion here would build a location for a path the fixture does not own, so an
/// unrepresentable fixture root fails the test instead.
#[track_caller]
fn fixture_path_text(fixture_root: &Path, relative_path: &str) -> String {
    let path = fixture_root.join(relative_path);
    path.to_str()
        .unwrap_or_else(|| panic!("fixture path {path:?} should be valid UTF-8"))
        .to_owned()
}

fn structured_diagnostic_messages(fixture_root: &Path) -> CompilerMessages {
    let mut string_table = StringTable::new();
    let primary_path = InternedPath::from_single_str(
        &fixture_path_text(fixture_root, "input/main.moth"),
        &mut string_table,
    );
    let secondary_path = InternedPath::from_single_str(
        &fixture_path_text(fixture_root, "input/helper.moth"),
        &mut string_table,
    );
    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        None,
        None,
        None,
        None,
        Some(test_location_at(secondary_path, 3, 4)),
        test_location_at(primary_path, 2, 1),
    );

    CompilerMessages::from_diagnostic(diagnostic, string_table)
}

fn structured_diagnostic_expectation(assertion: DiagnosticAssertion) -> FailureExpectation {
    FailureExpectation {
        warnings: WarningExpectation::Ignore,
        message_contains: Vec::new(),
        diagnostic_codes: vec!["MOTH-RULE-0044".to_owned()],
        diagnostic_assertions: vec![assertion],
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
    }
}

#[test]
fn structured_diagnostic_assertions_consume_compiler_identity_and_locations() {
    let _tmp_fixture_root = tempfile::tempdir().expect("should create temp dir");
    let fixture_root = _tmp_fixture_root.path().to_path_buf();
    let input_root = fixture_root.join("input");
    fs::create_dir_all(&input_root).expect("should create temporary fixture input directory");
    fs::write(input_root.join("main.moth"), "main").expect("should write primary source");
    fs::write(input_root.join("helper.moth"), "helper").expect("should write secondary source");
    let fixture_root = fs::canonicalize(&fixture_root).expect("fixture root should canonicalize");

    let expectation = structured_diagnostic_expectation(DiagnosticAssertion {
        code: "MOTH-RULE-0044".to_owned(),
        occurrence: 1,
        reason: Some("invalid_assignment_target.immutable_binding".to_owned()),
        path: Some("input/main.moth".to_owned()),
        line: Some(3),
        column: Some(2),
        count: Some(1),
        secondary_labels: vec![SecondaryLabelAssertion {
            occurrence: 1,
            path: Some("input/helper.moth".to_owned()),
            line: Some(4),
            column: Some(5),
        }],
    });

    let result = validate_failure_result(
        structured_diagnostic_messages(&fixture_root),
        &expectation,
        &fixture_root,
    );

    assert!(result.passed, "{:?}", result.failure_reason);
}

fn relative_structured_diagnostic_messages(scope: &str) -> CompilerMessages {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str(scope, &mut string_table);
    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        None,
        None,
        None,
        None,
        None,
        test_location_at(source_path, 2, 1),
    );

    CompilerMessages::from_diagnostic(diagnostic, string_table)
}

#[test]
fn structured_diagnostic_assertions_resolve_relative_scopes_under_input_root() {
    let _tmp_fixture_root = tempfile::tempdir().expect("should create temp dir");
    let fixture_root = _tmp_fixture_root.path().to_path_buf();
    let input_root = fixture_root.join("input");
    fs::create_dir_all(input_root.join("nested"))
        .expect("should create temporary fixture input directory");
    fs::write(input_root.join("@page.moth"), "page").expect("should write source");
    fs::write(input_root.join("nested/helper.moth"), "helper").expect("should write nested source");
    let fixture_root = fs::canonicalize(&fixture_root).expect("fixture root should canonicalize");

    for (scope, expected_path) in [
        ("@page.moth", "input/@page.moth"),
        ("input/@page.moth", "input/@page.moth"),
        ("nested/helper.moth", "input/nested/helper.moth"),
        ("input/nested/helper.moth", "input/nested/helper.moth"),
        (
            "nested/helper.moth/declaration.header",
            "input/nested/helper.moth",
        ),
        (
            "input/nested/helper.moth/declaration.header",
            "input/nested/helper.moth",
        ),
    ] {
        let expectation = structured_diagnostic_expectation(DiagnosticAssertion {
            code: "MOTH-RULE-0044".to_owned(),
            occurrence: 1,
            reason: Some("invalid_assignment_target.immutable_binding".to_owned()),
            path: Some(expected_path.to_owned()),
            line: Some(3),
            column: Some(2),
            count: Some(1),
            secondary_labels: Vec::new(),
        });

        let result = validate_failure_result(
            relative_structured_diagnostic_messages(scope),
            &expectation,
            &fixture_root,
        );

        assert!(
            result.passed,
            "scope {scope:?}: {:?}",
            result.failure_reason
        );
    }
}

#[test]
fn structured_diagnostic_mismatches_report_code_occurrence_field_expected_and_actual() {
    let fixture_root = std::env::current_dir().expect("test should have a current directory");
    let expectation = structured_diagnostic_expectation(DiagnosticAssertion {
        code: "MOTH-RULE-0044".to_owned(),
        occurrence: 1,
        reason: Some("invalid_assignment_target.temporary_not_assignable".to_owned()),
        path: Some("wrong.moth".to_owned()),
        line: Some(8),
        column: Some(9),
        count: Some(2),
        secondary_labels: vec![SecondaryLabelAssertion {
            occurrence: 1,
            path: Some("wrong-helper.moth".to_owned()),
            line: Some(10),
            column: Some(11),
        }],
    });

    let result = validate_failure_result(
        structured_diagnostic_messages(&fixture_root),
        &expectation,
        &fixture_root,
    );
    let reason = result
        .failure_reason
        .expect("structured mismatches should fail matching");

    for field in ["count", "reason", "path", "line", "column"] {
        assert!(reason.contains(&format!("field '{field}'")), "{reason}");
    }
    assert!(
        reason.contains("secondary_labels occurrence 1 field 'path'"),
        "{reason}"
    );
    assert!(
        reason.contains("code 'MOTH-RULE-0044' occurrence 1"),
        "{reason}"
    );
    assert!(reason.contains("expected 'wrong.moth'"), "{reason}");
    assert!(reason.contains("actual 'input/main.moth'"), "{reason}");
}

#[test]
fn structured_reason_assertion_is_not_satisfied_by_a_reasonless_diagnostic() {
    let fixture_root = std::env::current_dir().expect("test should have a current directory");
    // The authored reason is the exact text the report renders for an absent reason key. A
    // placeholder comparison would let this pass, which is the whole failure this guards.
    let expectation = FailureExpectation {
        warnings: WarningExpectation::Ignore,
        message_contains: Vec::new(),
        diagnostic_codes: vec!["MOTH-SYNTAX-0003".to_owned()],
        diagnostic_assertions: vec![DiagnosticAssertion {
            code: "MOTH-SYNTAX-0003".to_owned(),
            occurrence: 1,
            reason: Some("no reason key".to_owned()),
            path: None,
            line: None,
            column: None,
            count: None,
            secondary_labels: Vec::new(),
        }],
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
    };

    let result = validate_failure_result(
        diagnostic_messages(&["MOTH-SYNTAX-0003"]),
        &expectation,
        &fixture_root,
    );
    let reason = result
        .failure_reason
        .expect("a diagnostic with no reason key cannot satisfy a reason contract");

    assert!(reason.contains("field 'reason'"), "{reason}");
    assert!(reason.contains("actual '<no reason key>'"), "{reason}");
}

#[test]
fn structured_secondary_label_matching_ignores_primary_labels_and_reports_missing_occurrences() {
    let expectation = structured_diagnostic_expectation(DiagnosticAssertion {
        code: "MOTH-RULE-0044".to_owned(),
        occurrence: 1,
        reason: None,
        path: None,
        line: None,
        column: None,
        count: None,
        secondary_labels: vec![SecondaryLabelAssertion {
            occurrence: 2,
            path: Some("helper.moth".to_owned()),
            line: Some(4),
            column: None,
        }],
    });

    let result = validate_failure_result(
        structured_diagnostic_messages(Path::new(".")),
        &expectation,
        Path::new("."),
    );
    let reason = result
        .failure_reason
        .expect("missing secondary label occurrence should fail matching");

    assert!(reason.contains("secondary_labels occurrence 2"), "{reason}");
    assert!(
        reason.contains("only 1 secondary label occurrence(s) present"),
        "{reason}"
    );
}

fn exact_warning_expectation(codes: &[&str]) -> WarningExpectation {
    WarningExpectation::Exact(ExactWarningExpectation {
        expected_codes: codes.iter().map(|code| (*code).to_owned()).collect(),
    })
}

fn warning_build_result(codes: &[&str]) -> BuildResult {
    let mut result = build_result_with_index_html(VALID_HTML);
    let mut string_table = StringTable::new();
    let alias = string_table.intern("Alias");
    let symbol = string_table.intern("symbol");
    let warnings = codes
        .iter()
        .map(|code| match *code {
            "MOTH-RULE-0022" => {
                CompilerDiagnostic::unreachable_match_arm(SourceLocation::default())
            }
            "MOTH-IMPORT-0003" => CompilerDiagnostic::dependency_alias_case_mismatch(
                alias,
                symbol,
                SourceLocation::default(),
            ),
            other => panic!("test warning code is not constructed: {other}"),
        })
        .collect();
    result.string_table = string_table;
    result.warnings = warnings;
    result
}

#[test]
fn exact_warning_codes_match_success_warnings_independent_of_order() {
    let expectation = SuccessExpectation {
        warnings: exact_warning_expectation(&["MOTH-IMPORT-0003", "MOTH-RULE-0022"]),
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let result = validate_success_result(
        &case,
        warning_build_result(&["MOTH-RULE-0022", "MOTH-IMPORT-0003"]),
        &expectation,
    );

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn exact_warning_codes_report_missing_and_unexpected_codes() {
    let expectation = SuccessExpectation {
        warnings: exact_warning_expectation(&["MOTH-RULE-0022"]),
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let result = validate_success_result(
        &case,
        warning_build_result(&["MOTH-IMPORT-0003"]),
        &expectation,
    );
    let reason = result
        .failure_reason
        .expect("different warning code should fail matching");

    assert!(reason.contains("Missing warning codes"), "{reason}");
    assert!(reason.contains("Unexpected warning codes"), "{reason}");
    assert!(
        !reason.contains("Count-mismatched warning codes"),
        "{reason}"
    );
}

#[test]
fn exact_warning_codes_report_duplicate_count_mismatch() {
    let expectation = SuccessExpectation {
        warnings: exact_warning_expectation(&["MOTH-RULE-0022", "MOTH-RULE-0022"]),
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let result = validate_success_result(
        &case,
        warning_build_result(&["MOTH-RULE-0022"]),
        &expectation,
    );
    let reason = result
        .failure_reason
        .expect("duplicate warning count should fail matching");

    assert!(
        reason.contains("Count-mismatched warning codes"),
        "{reason}"
    );
    assert!(reason.contains("expected 2, actual 1"), "{reason}");
    assert!(!reason.contains("Unexpected warning codes"), "{reason}");
}

#[test]
fn ignore_and_forbid_keep_their_structured_warning_behaviour() {
    let ignored = SuccessExpectation {
        warnings: WarningExpectation::Ignore,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    let ignored_case = success_test_case(BackendId::Html, ignored.clone());
    let ignored_result = validate_success_result(
        &ignored_case,
        warning_build_result(&["MOTH-RULE-0022"]),
        &ignored,
    );
    assert!(ignored_result.passed, "{:?}", ignored_result.failure_reason);

    let forbidden = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        ..ignored
    };
    let forbidden_case = success_test_case(BackendId::Html, forbidden.clone());
    let forbidden_result = validate_success_result(
        &forbidden_case,
        warning_build_result(&["MOTH-RULE-0022"]),
        &forbidden,
    );
    assert!(!forbidden_result.passed);
    assert!(
        forbidden_result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Expected no warnings"))
    );
}

#[test]
fn exact_warning_codes_match_warnings_retained_in_failed_compilation_messages() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let warning = CompilerDiagnostic::unreachable_match_arm(test_location(source_path.clone()));
    let error = CompilerDiagnostic::unexpected_trailing_comma(test_location(source_path));
    let messages = CompilerMessages::from_diagnostics(vec![error, warning], string_table);
    // diagnostic_codes owns the error contract only; warning_codes independently owns the
    // warning. A warning code must never appear in diagnostic_codes for a failed compilation.
    let expectation = FailureExpectation {
        warnings: exact_warning_expectation(&["MOTH-RULE-0022"]),
        message_contains: Vec::new(),
        diagnostic_codes: vec!["MOTH-SYNTAX-0003".to_owned()],
        diagnostic_assertions: Vec::new(),
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
    };

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn warnings_ignore_truly_ignores_warnings_on_a_failed_compilation() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let warning = CompilerDiagnostic::unreachable_match_arm(test_location(source_path.clone()));
    let error = CompilerDiagnostic::unexpected_trailing_comma(test_location(source_path));
    let messages = CompilerMessages::from_diagnostics(vec![error, warning], string_table);
    let expectation = FailureExpectation {
        warnings: WarningExpectation::Ignore,
        message_contains: Vec::new(),
        diagnostic_codes: vec!["MOTH-SYNTAX-0003".to_owned()],
        diagnostic_assertions: Vec::new(),
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
    };

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(result.passed, "{:?}", result.failure_reason);
}

#[test]
fn warning_code_cannot_satisfy_failure_diagnostic_codes() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let warning = CompilerDiagnostic::unreachable_match_arm(test_location(source_path.clone()));
    let error = CompilerDiagnostic::unexpected_trailing_comma(test_location(source_path));
    let messages = CompilerMessages::from_diagnostics(vec![error, warning], string_table);
    // Authoring the warning code as a diagnostic code must fail: the warning is not in the
    // error-severity stream, so the multiset reports it as missing.
    let expectation = FailureExpectation {
        warnings: exact_warning_expectation(&["MOTH-RULE-0022"]),
        message_contains: Vec::new(),
        diagnostic_codes: vec!["MOTH-SYNTAX-0003".to_owned(), "MOTH-RULE-0022".to_owned()],
        diagnostic_assertions: Vec::new(),
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
    };

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(
        !result.passed,
        "a warning code must not satisfy error diagnostic assertions"
    );
    let reason = result
        .failure_reason
        .as_deref()
        .expect("a warning-as-error mismatch should report a reason");
    assert!(
        reason.contains("Missing codes") && reason.contains("MOTH-RULE-0022"),
        "unexpected reason: {reason}"
    );
}

#[test]
fn warning_prose_cannot_satisfy_error_message_contains() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let warning = CompilerDiagnostic::unreachable_match_arm(test_location(source_path.clone()));
    let error = CompilerDiagnostic::unexpected_trailing_comma(test_location(source_path));
    let messages = CompilerMessages::from_diagnostics(vec![error, warning], string_table);
    // The unreachable-match-arm warning prose must not satisfy message_contains because the
    // fragment check only inspects error-severity diagnostics.
    let expectation = FailureExpectation {
        warnings: WarningExpectation::Ignore,
        message_contains: vec!["Unreachable match arm".to_owned()],
        diagnostic_codes: vec!["MOTH-SYNTAX-0003".to_owned()],
        diagnostic_assertions: Vec::new(),
        diagnostic_match: DiagnosticMatchMode::Exact,
        diagnostic_match_reason: None,
    };

    let result = validate_failure_result(messages, &expectation, Path::new("."));

    assert!(
        !result.passed,
        "warning prose must not satisfy error-only message_contains"
    );
    let reason = result
        .failure_reason
        .as_deref()
        .expect("a warning-prose mismatch should report a reason");
    assert!(
        reason.contains("not found in any emitted error"),
        "unexpected reason: {reason}"
    );
}

// ─── Normalization unit tests ───────────────────────────────────────────────

#[test]
fn normalization_replaces_fn_counter_suffix() {
    assert_eq!(
        normalize_text_for_comparison("moth_rhs_and_fn0"),
        "moth_rhs_and_fnN"
    );
    assert_eq!(
        normalize_text_for_comparison("moth_start_fn1"),
        "moth_start_fnN"
    );
}

#[test]
fn normalization_replaces_local_counter_suffix() {
    assert_eq!(
        normalize_text_for_comparison("moth_calls_l0"),
        "moth_calls_lN"
    );
    assert_eq!(
        normalize_text_for_comparison("moth_lhs_l1 moth_value_l3"),
        "moth_lhs_lN moth_value_lN"
    );
}

#[test]
fn normalization_replaces_hir_tmp_counters() {
    assert_eq!(
        normalize_text_for_comparison("moth___hir_tmp_0_l4"),
        "moth___hir_tmp_N_lN"
    );
    assert_eq!(
        normalize_text_for_comparison("moth___hir_tmp_3_l13"),
        "moth___hir_tmp_N_lN"
    );
}

#[test]
fn normalization_replaces_template_fn_counters() {
    assert_eq!(
        normalize_text_for_comparison("moth___template_fn_0_fn3"),
        "moth___template_fn_N_fnN"
    );
    assert_eq!(
        normalize_text_for_comparison("moth___template_fn_2_fn5"),
        "moth___template_fn_N_fnN"
    );
}

#[test]
fn normalization_replaces_frag_counters() {
    assert_eq!(
        normalize_text_for_comparison("moth___moth_frag_0_fn2"),
        "moth___moth_frag_N_fnN"
    );
}

#[test]
fn normalization_preserves_runtime_library_names() {
    let input =
        "__moth_read __moth_write __moth_binding __moth_assign_value __moth_result_fallback";
    assert_eq!(normalize_text_for_comparison(input), input);
}

#[test]
fn normalization_is_deterministic() {
    let input = "function moth_rhs_and_fn0(moth_calls_l2) { moth___hir_tmp_3_l13; }";
    let first = normalize_text_for_comparison(input);
    let second = normalize_text_for_comparison(input);
    assert_eq!(first, second);
}

#[test]
fn normalization_does_not_mask_semantic_name_change() {
    let a = normalize_text_for_comparison("moth_rhs_and_fn0");
    let b = normalize_text_for_comparison("moth_rhs_or_fn0");
    assert_ne!(
        a, b,
        "different base names must still differ after normalization"
    );
}

#[test]
fn normalization_preserves_non_moth_identifiers() {
    let input = "function foo(x) { return x + 1; }";
    assert_eq!(normalize_text_for_comparison(input), input);
}

#[test]
fn normalization_preserves_base_name_segment() {
    let result = normalize_text_for_comparison("moth_rhs_and_fn0");
    assert!(
        result.starts_with("moth_rhs_and_fn"),
        "base name must be preserved: {result}"
    );
    assert!(
        result.ends_with('N'),
        "only the counter is replaced: {result}"
    );
}

fn absence_expectation(forbidden: Vec<String>) -> SuccessExpectation {
    SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: forbidden,
    }
}

fn absence_test_case(expectation: SuccessExpectation) -> TestCaseSpec {
    TestCaseSpec {
        display_name: "absence-contract".to_string(),
        case_id: "absence-contract".to_string(),
        manifest_relative_path: "absence-contract".to_string(),
        fixture_root: PathBuf::from("."),
        tags: Vec::new(),
        contract: None,
        role: None,
        backend_id: BackendId::Html,
        entry_path: PathBuf::from("."),
        flags: Vec::new(),
        expected: ExpectedOutcome::Success(expectation),
    }
}

#[test]
fn acceptance_only_html_baseline_rejects_broken_html() {
    let expectation = acceptance_only_expectation();
    let case = success_test_case(BackendId::Html, expectation.clone());
    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("index.html"),
        FileKind::Html("<!DOCTYPE html><html><head></head><body></body>".to_owned()),
    )]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(!result.passed);
    assert!(
        result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("html baseline contract"))
    );
}

#[test]
fn absence_contract_passes_when_forbidden_path_not_built() {
    let expectation = absence_expectation(vec!["api/index.html".to_string()]);
    let case = absence_test_case(expectation.clone());
    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("index.html"),
        FileKind::Html(VALID_HTML.to_owned()),
    )]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(
        result.passed,
        "absence contract should pass when the forbidden path is not among built artifacts"
    );
}

#[test]
fn absence_contract_fails_when_forbidden_path_is_built() {
    let expectation = absence_expectation(vec!["api/index.html".to_string()]);
    let case = absence_test_case(expectation.clone());
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
        (
            PathBuf::from("api/index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
    ]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(
        !result.passed,
        "absence contract should fail when the forbidden path is built"
    );
    let reason = result
        .failure_reason
        .expect("failure should carry a reason");
    assert!(
        reason.contains("api/index.html"),
        "failure reason should name the forbidden path: {reason}"
    );
}

#[test]
fn absence_contract_ignores_not_built_files() {
    let expectation = absence_expectation(vec!["api/index.html".to_string()]);
    let case = absence_test_case(expectation.clone());
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
        (PathBuf::from("api/index.html"), FileKind::NotBuilt),
    ]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(
        result.passed,
        "NotBuilt files must not count as emitted artifacts"
    );
}

#[test]
fn strict_text_goldens_ignore_lf_vs_crlf_differences() {
    assert!(
        compare_text_golden("<p>a\r\nb</p>\r\n", "<p>a\nb</p>\n", GoldenMode::Strict).is_none()
    );
}

#[test]
fn normalized_text_goldens_ignore_lf_vs_crlf_differences() {
    assert!(
        compare_text_golden(
            "<p>moth_rhs_and_fn0\r\n</p>\r\n",
            "<p>moth_rhs_and_fn7\n</p>\n",
            GoldenMode::Normalized,
        )
        .is_none()
    );
}

#[test]
fn normalized_comparison_strips_core_css_after_crlf_normalization() {
    let normalized =
        normalize_text_for_comparison("<style>\r\nbody { color: red; }\r\n</style>\r\nok");
    assert!(normalized.contains("<style>/* CORE_CSS */</style>"));
    assert!(!normalized.contains("body { color: red; }"));
}

#[test]
fn strict_golden_validation_treats_crlf_and_lf_as_equivalent_for_text() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let golden_dir = root.join("golden");
    fs::create_dir_all(&golden_dir).expect("should create golden dir");
    fs::write(golden_dir.join("index.html"), "<p>a\r\nb</p>\r\n")
        .expect("should write CRLF golden");

    let build_result = build_result_with_index_html("<p>a\nb</p>\n");
    let golden = discover_golden_expectation(&golden_dir, None)
        .expect("golden inventory should be discovered");
    let mismatch = validate_golden_outputs(&build_result, &golden);
    assert!(
        mismatch.is_none(),
        "strict text golden checks should ignore line-ending-only differences"
    );
}

#[test]
fn normalized_golden_validation_treats_crlf_and_lf_as_equivalent_for_text() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let golden_dir = root.join("golden");
    fs::create_dir_all(&golden_dir).expect("should create golden dir");
    fs::write(golden_dir.join("index.html"), "moth_rhs_and_fn0\r\n")
        .expect("should write CRLF golden");

    let build_result = build_result_with_index_html("moth_rhs_and_fn8\n");
    let golden = discover_golden_expectation(&golden_dir, Some(GoldenMode::Normalized))
        .expect("golden inventory should be discovered");
    let mismatch = validate_golden_outputs(&build_result, &golden);
    assert!(
        mismatch.is_none(),
        "normalized golden checks should ignore counter and line-ending drift"
    );
}

#[test]
fn nested_golden_validation_compares_relative_paths() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let golden_dir = root.join("golden");
    let golden_file = golden_dir.join("nested").join("page.html");
    fs::create_dir_all(golden_file.parent().expect("nested parent should exist"))
        .expect("should create nested golden directory");
    fs::write(&golden_file, "<p>nested</p>\n").expect("should write nested golden");

    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("nested/page.html"),
        FileKind::Html("<p>nested</p>\n".to_owned()),
    )]);
    let golden = discover_golden_expectation(&golden_dir, None)
        .expect("golden inventory should be discovered");

    assert!(validate_golden_outputs(&build_result, &golden).is_none());
}

// --- Built-artifact index identity contracts ------------------------------------------------
//
// Every success assertion reads artifacts through one index. These tests own the index's
// construction contract: an ambiguous artifact set must be rejected before any assertion
// consumes it, because a first-match lookup would otherwise inspect one artifact and ignore
// the rest. Rejections are identified by their variant, not by their wording; one separate
// test owns the rendered text.

/// Rejects and returns the typed reason, so each test names the variant it is about.
#[track_caller]
fn artifact_index_rejection(build_result: &BuildResult, why: &str) -> ArtifactIndexError {
    match build_artifact_index_error(build_result) {
        Some(error) => error,
        None => panic!("{why}"),
    }
}

#[test]
fn artifact_index_rejects_duplicate_normalized_paths() {
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
        (
            PathBuf::from("index.html"),
            FileKind::Html(
                "<!DOCTYPE html><html><head></head><body>second</body></html>".to_owned(),
            ),
        ),
    ]);

    let error =
        artifact_index_rejection(&build_result, "two artifacts at one path must be rejected");
    let ArtifactIndexError::DuplicatePath { path } = error else {
        panic!("a repeated spelling is a duplicate path, not {error:?}");
    };
    assert_eq!(path, "index.html");
}

#[test]
fn artifact_index_rejects_paths_that_differ_only_by_ascii_case() {
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("assets/Page.js"),
            FileKind::Js("// first".to_owned()),
        ),
        (
            PathBuf::from("assets/page.js"),
            FileKind::Js("// second".to_owned()),
        ),
    ]);

    let error = artifact_index_rejection(&build_result, "case-aliasing artifacts must be rejected");
    let ArtifactIndexError::PortabilityAlias { first, second } = error else {
        panic!("two spellings of one output identity are an alias, not {error:?}");
    };
    assert_eq!(first, "assets/Page.js");
    assert_eq!(second, "assets/page.js");
}

#[test]
fn artifact_index_keeps_non_ascii_case_differences_distinct() {
    // The canonical output-path identity folds ASCII case only, so the harness must not apply
    // broader Unicode folding and reject a pair production treats as two destinations.
    let build_result = build_result_with_output_files(vec![
        (PathBuf::from("Å.js"), FileKind::Js("// upper".to_owned())),
        (PathBuf::from("å.js"), FileKind::Js("// lower".to_owned())),
    ]);

    assert!(
        build_artifact_index_error(&build_result).is_none(),
        "non-ASCII case differences are distinct output destinations"
    );
}

#[test]
fn artifact_index_rejects_paths_the_output_writer_would_reject() {
    // The writer refuses parent segments in a relative output path. The harness must refuse the
    // same destinations instead of indexing something that could never be emitted.
    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("assets/../page.js"),
        FileKind::Js("// escaped".to_owned()),
    )]);

    let error = artifact_index_rejection(
        &build_result,
        "an artifact path the writer rejects must not be indexed",
    );
    let ArtifactIndexError::InvalidOutputPath { path, reason } = error else {
        panic!("a parent segment is an invalid output path, not {error:?}");
    };
    assert_eq!(path, "assets/../page.js");
    assert_eq!(reason, InvalidOutputFolderReason::ParentDirectorySegment);
}

#[test]
fn artifact_index_rejects_reserved_device_artifact_names() {
    // `CON.js` cannot be written on Windows, so a build that emits it is not portable and the
    // harness must not silently accept it on Unix.
    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("CON.js"),
        FileKind::Js("// reserved".to_owned()),
    )]);

    let error = artifact_index_rejection(
        &build_result,
        "a reserved device basename must not be indexed",
    );
    let ArtifactIndexError::InvalidOutputPath { path, reason } = error else {
        panic!("a reserved device name is an invalid output path, not {error:?}");
    };
    assert_eq!(path, "CON.js");
    assert_eq!(reason, InvalidOutputFolderReason::InvalidPathComponent);
}

#[test]
fn artifact_index_accepts_distinct_paths_with_separator_differences() {
    // Windows-style separators normalize to the portable form; that is normalization, not
    // aliasing, and must not be rejected.
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("assets").join("one.js"),
            FileKind::Js("// one".to_owned()),
        ),
        (
            PathBuf::from("assets").join("two.js"),
            FileKind::Js("// two".to_owned()),
        ),
    ]);

    assert!(build_artifact_index_error(&build_result).is_none());
}

#[test]
fn artifact_index_ignores_not_built_outputs() {
    // A NotBuilt entry is a decision not to emit, not an artifact, so it can share a path
    // with nothing and must not create a duplicate.
    let build_result = build_result_with_output_files(vec![
        (PathBuf::from("index.html"), FileKind::NotBuilt),
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
    ]);

    assert!(build_artifact_index_error(&build_result).is_none());
}

#[test]
#[cfg(unix)]
fn artifact_index_rejects_non_utf8_artifact_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let invalid_name = OsString::from_vec(vec![b'b', b'a', 0xff, b'd', b'.', b'j', b's']);
    let build_result = build_result_with_output_files(vec![(
        PathBuf::from(invalid_name),
        FileKind::Js("// invalid".to_owned()),
    )]);

    let error = artifact_index_rejection(
        &build_result,
        "a non-UTF-8 artifact path must be rejected, not lossily replaced",
    );
    assert!(
        matches!(error, ArtifactIndexError::NonUtf8Path { .. }),
        "a non-UTF-8 path is an encoding rejection, not {error:?}"
    );
}

#[test]
fn artifact_index_rejections_name_the_offending_paths() {
    // The variants above carry the identity; this test owns the operator-facing wording, so a
    // reworded message cannot silently make every other rejection test unreadable.
    let duplicate = ArtifactIndexError::DuplicatePath {
        path: "index.html".to_owned(),
    }
    .to_string();
    assert!(
        duplicate.contains("more than one artifact at 'index.html'"),
        "duplicate rejection must name the duplicated path: {duplicate}"
    );

    let alias = ArtifactIndexError::PortabilityAlias {
        first: "assets/Page.js".to_owned(),
        second: "assets/page.js".to_owned(),
    }
    .to_string();
    assert!(
        alias.contains("assets/Page.js") && alias.contains("assets/page.js"),
        "alias rejection must name both spellings: {alias}"
    );

    let invalid = ArtifactIndexError::InvalidOutputPath {
        path: "assets/../page.js".to_owned(),
        reason: InvalidOutputFolderReason::ParentDirectorySegment,
    }
    .to_string();
    assert!(
        invalid.contains("assets/../page.js") && invalid.contains("ParentDirectorySegment"),
        "invalid-destination rejection must name the path and the writer's reason: {invalid}"
    );
}

#[test]
fn duplicate_artifact_paths_fail_the_case_as_a_harness_failure() {
    let expectation = acceptance_only_expectation();
    let case = success_test_case(BackendId::Html, expectation.clone());
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
    ]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(!result.passed, "an ambiguous artifact set cannot pass");
    assert_eq!(
        result.failure_kind,
        Some(FailureKind::HarnessFailed),
        "artifact ambiguity is a harness fact, not an expectation violation"
    );
    assert!(
        result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Artifact inventory is ambiguous")),
        "failure must explain the ambiguity: {:?}",
        result.failure_reason
    );
}

#[test]
fn artifact_absence_contract_normalizes_the_authored_path() {
    // The authored path is normalized before lookup, so a Windows-style authored path cannot
    // silently miss an artifact the build really produced.
    let expectation = absence_expectation(vec!["assets\\page.js".to_string()]);
    let case = absence_test_case(expectation.clone());
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
        (
            PathBuf::from("assets").join("page.js"),
            FileKind::Js("// page".to_owned()),
        ),
    ]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(!result.passed, "the forbidden artifact was produced");
    assert!(
        result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("to not exist")),
        "failure must report the absence violation: {:?}",
        result.failure_reason
    );
}

// --- Document shell contract ----------------------------------------------------------------
//
// The HTML and HTML-Wasm baselines both claim to check document structure. These tests own what
// that claim means: the shell markers appear exactly once, in order, with the doctype first and
// the closing html tag last. An unordered "contains" loop proved none of that.

#[test]
fn html_shell_contract_accepts_the_emitted_document_shell() {
    assert_eq!(html_shell_violation(VALID_HTML), None);
    assert_eq!(html_shell_violation(VALID_HTML_WASM), None);
}

#[test]
fn html_shell_contract_requires_the_doctype_to_open_the_document() {
    // A leading comment or stray text before the doctype changes how a browser parses the page.
    let with_leading_comment = format!("<!-- generated -->{VALID_HTML}");

    assert_eq!(
        html_shell_violation(&with_leading_comment),
        Some(HtmlShellViolation::MissingDoctypePrefix)
    );
}

#[test]
fn html_shell_contract_requires_the_document_to_close() {
    let truncated = VALID_HTML.replace("</html>\n", "");

    assert_eq!(
        html_shell_violation(&truncated),
        Some(HtmlShellViolation::MissingClosingHtml)
    );
}

#[test]
fn html_shell_contract_reports_a_missing_marker() {
    let without_head = VALID_HTML.replace("  <head>\n", "");

    assert_eq!(
        html_shell_violation(&without_head),
        Some(HtmlShellViolation::MissingMarker { marker: "<head>" })
    );
}

#[test]
fn html_shell_contract_rejects_a_repeated_marker() {
    // Two heads is not a document; the previous check accepted it because one `<head>` existed.
    let duplicated_head = VALID_HTML.replace("  <head>\n", "  <head>\n  <head>\n");

    assert_eq!(
        html_shell_violation(&duplicated_head),
        Some(HtmlShellViolation::RepeatedMarker {
            marker: "<head>",
            occurrences: 2,
        })
    );
}

#[test]
fn html_shell_contract_rejects_an_inverted_head_and_body() {
    let inverted = "<!DOCTYPE html>\n<html lang=\"en\">\n  <body style=\"\">\n  </body>\n  <head>\n  </head>\n</html>\n";

    assert_eq!(
        html_shell_violation(inverted),
        Some(HtmlShellViolation::OutOfOrderMarker {
            marker: "<body style=\"",
            must_follow: "</head>",
        })
    );
}

#[test]
fn html_shell_contract_ignores_marker_text_inside_script_and_style_content() {
    // The shell inserts script sources and the core stylesheet as opaque payloads. A JavaScript
    // string that happens to spell `</body>` is a string, not a second closing-body element, and
    // rejecting the document for it would fail a page whose structure is correct.
    let with_marker_like_payload = VALID_HTML.replace(
        "  </body>",
        "<style>/* <head> */</style>\n<script>const text = \"</body>\";</script>\n  </body>",
    );

    assert_eq!(html_shell_violation(&with_marker_like_payload), None);
}

#[test]
fn html_shell_contract_still_rejects_a_marker_repeated_in_markup() {
    // The opaque-content allowance must not extend to real markup: a second closing-body element
    // outside any script or style is still a structural violation.
    let duplicated_body_close = VALID_HTML.replace("  </body>", "  </body>\n  </body>");

    assert_eq!(
        html_shell_violation(&duplicated_body_close),
        Some(HtmlShellViolation::RepeatedMarker {
            marker: "</body>",
            occurrences: 2,
        })
    );
}

// --- Golden artifact kind and encoding contracts ---------------------------------------------
//
// A golden names a file. Comparing a directory or an unbuilt path as empty bytes let an empty
// golden pass against a path that holds no file, and lossy UTF-8 silently rewrote the expected
// text before comparing it.

/// Writes one golden file and returns the discovered expectation for it.
#[track_caller]
fn golden_expectation_with(
    golden_dir: &Path,
    relative_path: &str,
    contents: impl AsRef<[u8]>,
    mode: Option<GoldenMode>,
) -> GoldenExpectation {
    let golden_file = golden_dir.join(relative_path);
    fs::create_dir_all(
        golden_file
            .parent()
            .expect("golden file should have a parent"),
    )
    .expect("should create golden directory");
    fs::write(&golden_file, contents).expect("should write golden file");

    discover_golden_expectation(golden_dir, mode).expect("golden inventory should be discovered")
}

#[test]
fn golden_validation_rejects_a_directory_where_a_file_is_expected() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    // An empty golden is exactly the case the old empty-bytes conversion could satisfy.
    let golden = golden_expectation_with(&golden_dir, "index.html", "", None);

    let build_result =
        build_result_with_output_files(vec![(PathBuf::from("index.html"), FileKind::Directory)]);

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("a directory cannot satisfy a file golden");
    assert_eq!(kind, FailureKind::StrictGoldenMismatch);
    assert!(reason.contains("produced a directory"), "{reason}");
}

#[test]
fn golden_validation_rejects_a_path_the_build_did_not_emit() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "index.html", "", None);

    let build_result =
        build_result_with_output_files(vec![(PathBuf::from("index.html"), FileKind::NotBuilt)]);

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("an unbuilt path cannot satisfy a file golden");
    assert_eq!(kind, FailureKind::StrictGoldenMismatch);
    assert!(reason.contains("index.html"), "{reason}");
}

#[test]
fn golden_validation_rejects_invalid_utf8_text_goldens() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "index.html", [0x66, 0x6f, 0xff, 0x6f], None);

    let build_result = build_result_with_index_html("fo\u{fffd}o");

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("an invalid-UTF-8 text golden is a harness failure, not a silent match");
    assert_eq!(kind, FailureKind::HarnessFailed);
    assert!(reason.contains("not valid UTF-8"), "{reason}");
}

#[test]
fn golden_validation_compares_binary_goldens_by_bytes() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "page.wasm", [0x00, 0x61, 0x73, 0x6d], None);

    let matching = build_result_with_output_files(vec![(
        PathBuf::from("page.wasm"),
        FileKind::Wasm(vec![0x00, 0x61, 0x73, 0x6d]),
    )]);
    assert!(validate_golden_outputs(&matching, &golden).is_none());

    let differing = build_result_with_output_files(vec![(
        PathBuf::from("page.wasm"),
        FileKind::Wasm(vec![0x00, 0x61, 0x73, 0x00]),
    )]);
    let (_, kind) = validate_golden_outputs(&differing, &golden)
        .expect("different bytes must fail a binary golden");
    assert_eq!(kind, FailureKind::StrictGoldenMismatch);
}

#[test]
fn golden_validation_rejects_an_ambiguous_artifact_set_before_comparing() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "index.html", VALID_HTML, None);

    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML.to_owned()),
        ),
    ]);

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("duplicate artifact paths must be rejected before content comparison");
    assert_eq!(kind, FailureKind::HarnessFailed);
    assert!(
        reason.contains("Artifact inventory is ambiguous"),
        "{reason}"
    );
}

#[test]
fn golden_validation_rejects_a_js_golden_satisfied_by_generic_bytes() {
    // Identical bytes emitted as a generic byte artifact are not the JavaScript artifact the
    // golden names. Deciding the comparison from the produced kind let this pass.
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "page.js", "console.log(1);", None);

    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("page.js"),
        FileKind::Bytes(b"console.log(1);".to_vec()),
    )]);

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("a byte artifact cannot satisfy a JavaScript golden");
    assert_eq!(kind, FailureKind::StrictGoldenMismatch);
    assert!(
        reason.contains("expects a js artifact") && reason.contains("binary artifact"),
        "{reason}"
    );
}

#[test]
fn golden_validation_rejects_a_wasm_golden_satisfied_by_generic_bytes() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "page.wasm", [0x00, 0x61, 0x73, 0x6d], None);

    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("page.wasm"),
        FileKind::Bytes(vec![0x00, 0x61, 0x73, 0x6d]),
    )]);

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("a byte artifact cannot satisfy a wasm golden");
    assert_eq!(kind, FailureKind::StrictGoldenMismatch);
    assert!(
        reason.contains("expects a wasm artifact") && reason.contains("binary artifact"),
        "{reason}"
    );
}

#[test]
fn golden_validation_rejects_an_html_golden_produced_as_javascript() {
    // The golden lives outside the universal `index.html` baseline path, so nothing else would
    // have noticed that the build emitted the wrong artifact kind at this destination.
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "fragment.html", "<p>a</p>", None);

    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("fragment.html"),
        FileKind::Js("<p>a</p>".to_owned()),
    )]);

    let (reason, kind) = validate_golden_outputs(&build_result, &golden)
        .expect("a JavaScript artifact cannot satisfy an HTML golden");
    assert_eq!(kind, FailureKind::StrictGoldenMismatch);
    assert!(
        reason.contains("expects a html artifact") && reason.contains("js artifact"),
        "{reason}"
    );
}

#[test]
fn golden_validation_accepts_a_binary_golden_produced_as_bytes() {
    // The mismatch checks above must not make every byte artifact unmatchable: a golden with no
    // text or wasm extension claims exactly the writer's generic byte artifact.
    let root = tempfile::tempdir().expect("should create temp dir");
    let golden_dir = root.path().join("golden");
    let golden = golden_expectation_with(&golden_dir, "logo.png", [0x89, 0x50, 0x4e, 0x47], None);

    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("logo.png"),
        FileKind::Bytes(vec![0x89, 0x50, 0x4e, 0x47]),
    )]);

    assert!(validate_golden_outputs(&build_result, &golden).is_none());
}
