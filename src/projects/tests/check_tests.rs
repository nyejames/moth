//! Tests for the frontend-only `check` command flow.

use super::{execute_check, format_terse_summary_line};
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::{
    display_line_number, relative_display_path_from_root, resolve_source_file_path,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::temp_dir;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn check_compiles_single_file_without_writing_artifacts() {
    let root = temp_dir("single_file");
    fs::create_dir_all(&root).expect("should create temp root");
    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let outcome = execute_check(
        entry_file
            .to_str()
            .expect("temp file path should be valid UTF-8 for this test"),
    );
    assert!(
        !outcome.messages.has_errors(),
        "single-file check should compile without errors"
    );
    assert_eq!(
        outcome.messages.warning_count(),
        0,
        "warning-free single-file input should produce no check warnings"
    );
    assert_eq!(
        fs::read_dir(&root).expect("should read temp root").count(),
        1,
        "check should not write output artifacts to the source folder"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

/// Stable source-facing identity for one frontend diagnostic.
type DiagnosticIdentityRow = (&'static str, Option<&'static str>, String, i32);

/// Create a directory project whose `@page.moth` holds `source`.
///
/// WHAT: returns an unmanaged temp project root containing only the authored source file.
/// WHY: the parity test reuses one project shape for both `execute_check` and `build_project`.
fn write_page_project(prefix: &str, source: &str) -> PathBuf {
    let root = temp_dir(prefix);
    fs::create_dir_all(&root).expect("should create temp project root");
    fs::write(root.join("@page.moth"), source).expect("should write @page.moth source");
    root
}

/// Collect ordered frontend diagnostic identity rows for parity comparison.
///
/// WHAT: maps each diagnostic to its stable code, optional reason key, normalized source file
/// name, and one-based start line.
/// WHY: comparing typed identity instead of rendered prose keeps the assertion stable across
/// wording changes and proves the shared frontend contract is preserved.
fn diagnostic_identity_sequence<'a>(
    diagnostics: impl IntoIterator<Item = &'a CompilerDiagnostic>,
    string_table: &StringTable,
    project_root: &std::path::Path,
) -> Vec<DiagnosticIdentityRow> {
    let canonical_project_root = project_root
        .canonicalize()
        .expect("diagnostic fixture root should canonicalize");

    diagnostics
        .into_iter()
        .map(|diagnostic| {
            let identity = diagnostic.identity();
            let source_file =
                resolve_source_file_path(&diagnostic.primary_location.scope, string_table);
            let normalized_path =
                relative_display_path_from_root(&source_file, &canonical_project_root);
            let line = display_line_number(diagnostic.primary_location.start_pos.line_number);
            (identity.code, identity.reason_key, normalized_path, line)
        })
        .collect()
}

#[test]
fn check_and_build_frontends_produce_identical_diagnostics_and_check_writes_no_artifacts() {
    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));

    // --------------------------------------------
    //  Warning parity: success with frontend warnings
    // --------------------------------------------
    // The capture pattern makes the later arms unreachable, producing three
    // `MOTH-RULE-0022` warnings. Both `check` and `build_project` succeed.
    let warning_source = "\
value ~= \"hello\"
result ~= \"unset\"

if value is:
    captured => result = captured
    \"one\" => result = \"one\"
    \"two\" => result = \"two\"
    else => result = \"other\"
;

[:pattern_unreachable_after_capture_warning result=[result]]
";
    let warning_root = write_page_project("check_build_parity_warning", warning_source);

    let check_warning_outcome = execute_check(
        warning_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
    );
    assert!(
        !check_warning_outcome.messages.has_errors(),
        "warning fixture should not produce frontend errors"
    );

    // `check` is a no-artifact overlay: it must not create dev/release/index.html and must leave
    // the project root holding only the authored source file.
    assert!(
        !warning_root.join("dev").exists(),
        "check should not create dev output artifacts"
    );
    assert!(
        !warning_root.join("release").exists(),
        "check should not create release output artifacts"
    );
    assert!(
        !warning_root.join("index.html").exists(),
        "check should not emit backend output artifacts"
    );
    assert_eq!(
        fs::read_dir(&warning_root)
            .expect("should read warning project root")
            .count(),
        1,
        "check should leave only the authored source file in the project root"
    );

    let build_warning_result = build_project(
        &builder,
        warning_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
        &[],
    )
    .expect("warning fixture should build successfully");

    let check_warning_identity = diagnostic_identity_sequence(
        check_warning_outcome.messages.diagnostic_slice().iter(),
        &check_warning_outcome.messages.string_table,
        &warning_root,
    );
    let build_warning_identity = diagnostic_identity_sequence(
        &build_warning_result.warnings,
        &build_warning_result.string_table,
        &warning_root,
    );

    let expected_warning_identity = vec![
        ("MOTH-RULE-0022", None, "@page.moth".to_owned(), 6),
        ("MOTH-RULE-0022", None, "@page.moth".to_owned(), 7),
        ("MOTH-RULE-0022", None, "@page.moth".to_owned(), 8),
    ];
    assert_eq!(
        check_warning_identity, expected_warning_identity,
        "check should report the exact ordered frontend warning contract"
    );
    assert_eq!(
        build_warning_identity, expected_warning_identity,
        "check and build should report identical ordered frontend warning identity"
    );

    fs::remove_dir_all(&warning_root).expect("should remove warning project dir");

    // --------------------------------------------
    //  Error parity: shared frontend rejection
    // --------------------------------------------
    // Missing mutable call access has a compiler-owned reason key and is rejected by the shared
    // frontend before backend lowering.
    let error_source = "\
increment |value ~Int|:
    value += 1
;

count ~= 0
increment(count)
";
    let error_root = write_page_project("check_build_parity_error", error_source);

    let check_error_outcome = execute_check(
        error_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
    );
    assert!(
        check_error_outcome.messages.has_errors(),
        "error fixture should produce frontend errors"
    );

    let Err(build_error_messages) = build_project(
        &builder,
        error_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
        &[],
    ) else {
        panic!("error fixture should fail the build frontend");
    };

    let check_error_identity = diagnostic_identity_sequence(
        check_error_outcome.messages.diagnostic_slice().iter(),
        &check_error_outcome.messages.string_table,
        &error_root,
    );
    let build_error_identity = diagnostic_identity_sequence(
        build_error_messages.diagnostic_slice().iter(),
        &build_error_messages.string_table,
        &error_root,
    );

    let expected_error_identity = vec![(
        "MOTH-RULE-0054",
        Some("invalid_call_shape.mutable_access_required"),
        "@page.moth".to_owned(),
        6,
    )];
    assert_eq!(
        check_error_identity, expected_error_identity,
        "check should report the exact frontend error contract"
    );
    assert_eq!(
        build_error_identity, expected_error_identity,
        "check and build should report identical ordered frontend error diagnostics"
    );

    fs::remove_dir_all(&error_root).expect("should remove error project dir");
}

#[test]
fn terse_summary_line_matches_clean_success_contract() {
    let summary = format_terse_summary_line(Duration::from_millis(5), 0, 0);
    assert_eq!(summary, "Done in 5ms. No errors or warnings.");
}
