//! Self-tests for integration result and artifact assertions.
//!
//! WHAT: protects diagnostic rendering, text normalization, and artifact absence contracts.
//! WHY: assertion regressions can silently weaken the suite without changing compilation.

use super::super::assertions::{
    RuntimeEvent, SlotOutput, build_artifact_index_reason, compare_text_golden,
    discover_golden_expectation, execute_wasm_harness_for_test, extract_script_blocks,
    normalize_text_for_comparison, parse_harness_output, validate_failure_result,
    validate_golden_outputs, validate_rendered_output_fragments, validate_success_result,
};
use super::super::types::{
    DiagnosticAssertion, ExactWarningExpectation, GoldenExpectation, RenderedOutputExpectation,
    SecondaryLabelAssertion, SuccessContract,
};
use super::super::{
    BackendId, DiagnosticMatchMode, ExpectedOutcome, FailureExpectation, FailureKind, GoldenMode,
    SuccessExpectation, TestCaseSpec, WarningExpectation,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, OutputFile, Project};
use crate::build_system::output::{BuilderKind, CleanupPolicy, OutputOwner};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, InvalidAssignmentTargetReason,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
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

fn structured_diagnostic_messages(fixture_root: &Path) -> CompilerMessages {
    let mut string_table = StringTable::new();
    let primary_path = InternedPath::from_single_str(
        &fixture_root.join("input/main.moth").to_string_lossy(),
        &mut string_table,
    );
    let secondary_path = InternedPath::from_single_str(
        &fixture_root.join("input/helper.moth").to_string_lossy(),
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

const VALID_HTML: &str = "<!DOCTYPE html><html><head></head><body></body></html>";
const VALID_HTML_WASM: &str =
    "<!DOCTYPE html><html><head></head><body><script src=\"./page.js\"></script></body></html>";
const VALID_PAGE_JS: &str = "__moth_instantiate_wasm instance.exports.moth_start() \"./page.wasm\"";

fn build_result_with_output_files(files: Vec<(PathBuf, FileKind)>) -> BuildResult {
    let output_files = files
        .into_iter()
        .map(|(path, kind)| OutputFile::new(path, kind))
        .collect();
    BuildResult {
        project: Project {
            output_files,
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: Vec::new(),
        string_table: StringTable::new(),
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

fn build_result_with_index_html(html: &str) -> BuildResult {
    build_result_with_output_files(vec![(
        PathBuf::from("index.html"),
        FileKind::Html(html.to_owned()),
    )])
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

fn acceptance_only_expectation() -> SuccessExpectation {
    SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: Some(SuccessContract::AcceptanceOnly),
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
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

fn success_test_case(backend_id: BackendId, expectation: SuccessExpectation) -> TestCaseSpec {
    TestCaseSpec {
        display_name: "success-contract".to_string(),
        case_id: "success-contract".to_string(),
        manifest_relative_path: "success-contract".to_string(),
        fixture_root: PathBuf::from("."),
        tags: Vec::new(),
        contract: None,
        role: None,
        backend_id,
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
fn acceptance_only_html_wasm_baseline_rejects_missing_output() {
    let expectation = acceptance_only_expectation();
    let case = success_test_case(BackendId::HtmlWasm, expectation.clone());
    let build_result = build_result_with_output_files(Vec::new());

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(!result.passed);
    assert!(
        result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("html_wasm baseline contract"))
    );
}

#[test]
fn acceptance_only_html_wasm_baseline_rejects_invalid_wasm() {
    let expectation = acceptance_only_expectation();
    let case = success_test_case(BackendId::HtmlWasm, expectation.clone());
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML_WASM.to_owned()),
        ),
        (
            PathBuf::from("page.js"),
            FileKind::Js(VALID_PAGE_JS.to_owned()),
        ),
        (PathBuf::from("page.wasm"), FileKind::Wasm(vec![0, 1, 2])),
    ]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(!result.passed);
    assert!(
        result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("valid wasm bytes"))
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
fn rendered_output_fragment_validation_reports_semantic_mismatch_kind() {
    let expectation = RenderedOutputExpectation {
        contains: vec!["missing-fragment".to_string()],
        ..Default::default()
    };
    let result = validate_rendered_output_fragments("rendered text", &expectation)
        .expect("missing required fragment should fail");
    assert_eq!(result.1, FailureKind::RenderedOutputMismatch);
}

#[test]
fn rendered_output_order_allows_repeated_fragments_at_distinct_occurrences() {
    let expectation = RenderedOutputExpectation {
        contains_in_order: vec!["first".to_owned(), "second".to_owned(), "first".to_owned()],
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("first\nsecond\nfirst", &expectation).is_none());
}

#[test]
fn rendered_output_order_reports_distinct_failure_kind() {
    let expectation = RenderedOutputExpectation {
        contains_in_order: vec!["second".to_owned(), "first".to_owned()],
        ..Default::default()
    };

    let result = validate_rendered_output_fragments("first\nsecond", &expectation)
        .expect("out-of-order fragments should fail");
    assert_eq!(result.1, FailureKind::RenderedOutputOrderMismatch);
}

#[test]
fn rendered_output_exactly_once_accepts_one_occurrence_and_rejects_missing_or_duplicate() {
    let expectation = RenderedOutputExpectation {
        contains_exactly_once: vec!["once".to_owned()],
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("before\nonce\nafter", &expectation).is_none());

    let missing = validate_rendered_output_fragments("before\nafter", &expectation)
        .expect("missing exact-once fragment should fail");
    assert_eq!(missing.1, FailureKind::RenderedOutputMultiplicityMismatch);

    let duplicate = validate_rendered_output_fragments("once\nonce", &expectation)
        .expect("duplicate exact-once fragment should fail");
    assert_eq!(duplicate.1, FailureKind::RenderedOutputMultiplicityMismatch);
}

#[test]
fn rendered_output_exact_normalizes_only_line_endings() {
    let expectation = RenderedOutputExpectation {
        exact: Some("first\nsecond\nthird".to_owned()),
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("first\r\nsecond\rthird", &expectation).is_none());

    let whitespace_difference =
        validate_rendered_output_fragments("first\r\n second\rthird", &expectation)
            .expect("ordinary whitespace differences should fail exact output");
    assert_eq!(
        whitespace_difference.1,
        FailureKind::RenderedOutputExactMismatch
    );
}

#[test]
fn rendered_output_exact_accepts_empty_captured_text_only() {
    let expectation = RenderedOutputExpectation {
        exact: Some(String::new()),
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("", &expectation).is_none());
    let result = validate_rendered_output_fragments("\n", &expectation)
        .expect("a captured newline is not empty exact output");
    assert_eq!(result.1, FailureKind::RenderedOutputExactMismatch);
}

#[test]
fn rendered_output_extracts_nonempty_script_blocks_in_source_order() {
    let html = r#"
<script>first</script>
<script type="module">second</script>
<script>   </script>
"#;

    assert_eq!(
        extract_script_blocks(html),
        vec!["first".to_owned(), "second".to_owned()]
    );
}

#[test]
fn html_wasm_rendered_output_waits_for_bootstrap_completion() {
    let temp_dir = tempfile::tempdir().expect("temporary Wasm harness directory should exist");
    fs::write(
        temp_dir.path().join("page.js"),
        r#"(async () => {
    await new Promise((resolve) => setTimeout(resolve, 80));
    document.getElementById("delayed-slot").insertAdjacentHTML("beforeend", "delayed");
})();
"#,
    )
    .expect("delayed bootstrap fixture should be written");

    let output = execute_wasm_harness_for_test(temp_dir.path())
        .expect("HTML-Wasm harness should await delayed bootstrap completion");
    assert_eq!(output.combined_output(), "delayed");
}

#[test]
fn rendered_output_decodes_typed_runtime_events() {
    let output = parse_harness_output(
        r#"{"events":[{"type":"console","text":"hello"},{"type":"fragment_insert","id":"root","html":"<p>hi</p>"}]}"#,
    )
    .expect("valid runtime events should decode");

    assert_eq!(
        output.events(),
        &[
            RuntimeEvent::Console {
                text: "hello".to_owned(),
            },
            RuntimeEvent::FragmentInsert {
                id: "root".to_owned(),
                html: "<p>hi</p>".to_owned(),
            },
        ]
    );
}

#[test]
fn rendered_output_preserves_interleaved_event_chronology() {
    let output = parse_harness_output(
        r#"{"events":[{"type":"console","text":"before"},{"type":"fragment_insert","id":"root","html":"<b>one</b>"},{"type":"console","text":"after"},{"type":"fragment_insert","id":"root","html":"<b>two</b>"}]}"#,
    )
    .expect("interleaved runtime events should decode");

    assert_eq!(
        output.events(),
        &[
            RuntimeEvent::Console {
                text: "before".to_owned(),
            },
            RuntimeEvent::FragmentInsert {
                id: "root".to_owned(),
                html: "<b>one</b>".to_owned(),
            },
            RuntimeEvent::Console {
                text: "after".to_owned(),
            },
            RuntimeEvent::FragmentInsert {
                id: "root".to_owned(),
                html: "<b>two</b>".to_owned(),
            },
        ]
    );
}

#[test]
fn rendered_output_derives_channel_views_in_event_order() {
    let output = parse_harness_output(
        r#"{"events":[{"type":"console","text":"before"},{"type":"fragment_insert","id":"root","html":"<b>one</b>"},{"type":"console","text":"after"},{"type":"fragment_insert","id":"root","html":"<b>two</b>"}]}"#,
    )
    .expect("interleaved runtime events should decode");

    assert_eq!(
        output.console_lines(),
        vec!["before".to_owned(), "after".to_owned()]
    );
    assert_eq!(
        output.slot_outputs(),
        vec![
            SlotOutput {
                id: "root".to_owned(),
                html: "<b>one</b>".to_owned(),
            },
            SlotOutput {
                id: "root".to_owned(),
                html: "<b>two</b>".to_owned(),
            },
        ]
    );
    assert_eq!(
        output.combined_output(),
        "before\n<b>one</b>\nafter\n<b>two</b>"
    );
}

#[test]
fn rendered_output_rejects_unknown_or_malformed_runtime_events() {
    for (json, expected_reason) in [
        (
            r#"{"events":[{"type":"unknown","text":"value"}]}"#,
            "unknown type",
        ),
        (
            r#"{"events":[{"type":"fragment_insert","id":"root"}]}"#,
            "missing string field 'html'",
        ),
        (
            r#"{"events":[{"type":"console","text":"value","extra":true}]}"#,
            "unknown field 'extra'",
        ),
    ] {
        let reason =
            parse_harness_output(json).expect_err("malformed runtime events must fail decoding");
        assert!(reason.contains(expected_reason), "{reason}");
    }
}

#[test]
fn rendered_output_validation_reports_harness_failure_without_script_blocks() {
    let expectation = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: super::super::types::RenderedOutputExpectation {
            contains: vec!["anything".to_string()],
            ..Default::default()
        },
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let build_result = build_result_with_index_html(VALID_HTML);

    let result = validate_success_result(&case, build_result, &expectation);

    assert_eq!(result.failure_kind, Some(FailureKind::HarnessFailed));
}

#[test]
fn rendered_output_node_is_not_invoked_without_a_rendered_assertion() {
    let expectation = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: RenderedOutputExpectation::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let result = validate_success_result(
        &case,
        build_result_with_index_html(VALID_HTML),
        &expectation,
    );

    assert!(
        result.passed,
        "Node should not be needed without assertions"
    );
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

    let build_result = BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("nested/page.html"),
                FileKind::Html("<p>nested</p>\n".to_owned()),
            )],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: Vec::new(),
        string_table: StringTable::new(),
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    };
    let golden = discover_golden_expectation(&golden_dir, None)
        .expect("golden inventory should be discovered");

    assert!(validate_golden_outputs(&build_result, &golden).is_none());
}

// --- Built-artifact index identity contracts ------------------------------------------------
//
// Every success assertion reads artifacts through one index. These tests own the index's
// construction contract: an ambiguous artifact set must be rejected before any assertion
// consumes it, because a first-match lookup would otherwise inspect one artifact and ignore
// the rest.

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

    let reason = build_artifact_index_reason(&build_result)
        .expect("two artifacts at one path must be rejected");
    assert!(
        reason.contains("more than one artifact at 'index.html'"),
        "duplicate rejection must name the duplicated path: {reason}"
    );
}

#[test]
fn artifact_index_rejects_paths_that_differ_only_by_case() {
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

    let reason = build_artifact_index_reason(&build_result)
        .expect("case-aliasing artifacts must be rejected");
    assert!(
        reason.contains("differ only by case"),
        "alias rejection must name the portability contract: {reason}"
    );
    assert!(
        reason.contains("assets/Page.js") && reason.contains("assets/page.js"),
        "alias rejection must name both aliasing paths: {reason}"
    );
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

    assert_eq!(build_artifact_index_reason(&build_result), None);
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

    assert_eq!(build_artifact_index_reason(&build_result), None);
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

    let reason = build_artifact_index_reason(&build_result)
        .expect("a non-UTF-8 artifact path must be rejected, not lossily replaced");
    assert!(
        reason.contains("is not valid UTF-8"),
        "non-UTF-8 rejection must name the encoding contract: {reason}"
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
