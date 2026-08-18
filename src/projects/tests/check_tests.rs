//! Tests for the frontend-only `check` command flow.

#[cfg(feature = "timers")]
use super::run_check_for_tests;
use super::{execute_check, format_terse_summary_line};
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::{
    display_line_number, relative_display_path_from_root, resolve_source_file_path,
};
#[cfg(unix)]
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidConfigReason, InvalidOutputFolderReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::unused_temp_path;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
#[cfg(feature = "timers")]
use crate::timing::{TimingMetric, start_benchmark_collection};
use std::fs;
use std::path::PathBuf;
#[cfg(feature = "timers")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[test]
fn check_compiles_single_file_without_writing_artifacts() {
    let root = unused_temp_path("single_file");
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

/// Check records its bootstrap and frontend boundaries in owner order.
#[cfg(feature = "timers")]
#[test]
fn successful_check_finishes_bootstrap_before_frontend() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let root = unused_temp_path("check_timer_stage_boundaries");
    fs::create_dir_all(&root).expect("should create temporary project root");
    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let outcome = execute_check(
        entry_file
            .to_str()
            .expect("temporary path should be valid UTF-8"),
    );
    let snapshot = timing_session.finish();

    assert!(!outcome.messages.has_errors());
    assert_timing_sequence(
        &snapshot,
        &[
            TimingMetric::BuildBootstrapTotal,
            TimingMetric::BuildFrontendTotal,
        ],
    );

    fs::remove_dir_all(&root).expect("should remove temporary project root");
}

/// Config AST stages use their dedicated v1 identities and finish in owner order.
#[cfg(feature = "timers")]
#[test]
fn config_ast_timers_use_dedicated_identities() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let root = unused_temp_path("check_config_timer_stage_boundaries");
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n")
        .expect("should write config file");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let outcome = execute_check(root.to_str().expect("temporary path should be valid UTF-8"));
    let snapshot = timing_session.finish();

    assert!(!outcome.messages.has_errors());
    assert_timing_sequence(
        &snapshot,
        &[
            TimingMetric::ConfigAstTotal,
            TimingMetric::ConfigAstEnvironment,
            TimingMetric::ConfigAstEmit,
            TimingMetric::ConfigAstFinalise,
        ],
    );

    fs::remove_dir_all(&root).expect("should remove temporary project root");
}

#[cfg(feature = "timers")]
fn assert_timing_sequence(
    snapshot: &crate::timing::BenchmarkObservationSnapshot,
    expected_metrics: &[TimingMetric],
) {
    let positions = expected_metrics
        .iter()
        .map(|metric| {
            snapshot
                .timings
                .iter()
                .position(|aggregate| aggregate.metric == *metric && aggregate.samples > 0)
                .unwrap_or_else(|| panic!("missing timing metric {:?}", metric))
        })
        .collect::<Vec<_>>();
    assert!(
        positions.windows(2).all(|window| window[0] < window[1]),
        "timing metrics must follow canonical schema order: {expected_metrics:?}"
    );
}

#[test]
fn check_retains_source_package_warning() {
    let root = unused_temp_path("check_source_package_warning");
    let package = root.join("src/warnpkg");
    let src = root.join("src");
    fs::create_dir_all(&package).expect("should create package root");
    fs::create_dir_all(&src).expect("should create entry root");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write project root");
    fs::write(
        package.join("+package.moth"),
        "export:\n    run || -> Int:\n        value ~= \"hello\"\n        result ~= \"unset\"\n\n        if value is:\n            \"one\" => result = \"one\"\n            \"one\" => result = \"one\"\n            else => result = \"other\"\n        ;\n        return 1\n    ;\n;\n",
    )
    .expect("should write warning package root");

    let outcome = execute_check(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
    );
    assert!(
        !outcome.messages.has_errors(),
        "check should not treat a source-package warning as an error"
    );
    assert!(
        outcome.messages.warning_count() >= 1,
        "check should retain the source-package warning"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[cfg(unix)]
#[test]
fn check_rejects_symlinked_directory_output_roots_before_frontend_work() {
    use std::os::unix::fs::symlink;

    for (case_name, target_name, expected_reason) in [
        (
            "sibling",
            "outside",
            InvalidOutputFolderReason::ResolvesOutsideProjectRoot,
        ),
        (
            "entry",
            "src",
            InvalidOutputFolderReason::InsideOrEqualToEntryRoot,
        ),
    ] {
        let root = unused_temp_path(&format!("check_output_symlink_{case_name}"));
        let source_root = root.join("src");
        let outside = unused_temp_path(&format!("check_output_symlink_target_{case_name}"));
        fs::create_dir_all(&source_root).expect("should create source root");
        fs::create_dir_all(&outside).expect("should create outside root");
        let output_root = root.join("dev");
        if target_name == "src" {
            symlink(&source_root, &output_root).expect("should create entry-root symlink");
        } else {
            symlink(&outside, &output_root).expect("should create sibling symlink");
        }
        fs::write(
            root.join("config.moth"),
            "entry_root #= \"src\"\ndev_folder #= \"dev\"\noutput_folder #= \"release\"\n",
        )
        .expect("should write config");
        fs::write(source_root.join("@page.moth"), "#[:<h1>Check</h1>]\n")
            .expect("should write source");

        let outcome = execute_check(
            root.to_str()
                .expect("temporary project path should be valid UTF-8"),
        );
        assert!(outcome.messages.has_errors());
        assert!(outcome.messages.error_diagnostics().any(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::InvalidOutputFolder {
                        reason,
                        ..
                    },
                    ..
                } if *reason == expected_reason
            )
        }));
        assert!(!outside.join("index.html").exists());
        assert!(!source_root.join("index.html").exists());

        fs::remove_dir_all(&root).expect("should remove project root");
        fs::remove_dir_all(&outside).expect("should remove target root");
    }
}

/// Stable source-facing identity for one frontend diagnostic.
type DiagnosticIdentityRow = (&'static str, Option<&'static str>, String, i32);

/// Create a directory project whose `@page.moth` holds `source`.
///
/// WHAT: returns an unmanaged temp project root containing only the authored source file.
/// WHY: the parity test reuses one project shape for both `execute_check` and `build_project`.
fn write_page_project(prefix: &str, source: &str) -> PathBuf {
    let root = unused_temp_path(prefix);
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
    // Repeated literal patterns make the later duplicate arms unreachable,
    // producing three `MOTH-RULE-0022` warnings. Both `check` and
    // `build_project` succeed.
    let warning_source = "\
value ~= \"hello\"
result ~= \"unset\"

if value is:
    \"hello\" => result = \"one\"
    \"hello\" => result = \"two\"
    \"hello\" => result = \"three\"
    \"hello\" => result = \"four\"
    else => result = \"other\"
;

[:pattern_unreachable_after_duplicate_literal_warning result=[result]]
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

/// Baseline test: run_check records command.check.total with exactly one sample.
#[cfg(feature = "timers")]
#[test]
fn run_check_records_command_check_total() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let root = unused_temp_path("check_run_check_timer");
    fs::create_dir_all(&root).expect("should create temporary project root");
    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let status = super::run_check(
        entry_file
            .to_str()
            .expect("temporary path should be valid UTF-8"),
        super::CheckOptions::default(),
    );
    let snapshot = timing_session.finish();

    assert_eq!(
        status,
        crate::projects::command_status::CommandStatus::Success
    );
    let command_total = snapshot
        .timings
        .iter()
        .find(|observation| observation.metric.descriptor().stable_name == "command.check.total")
        .expect("command.check.total must be recorded");
    assert_eq!(command_total.samples, 1);

    fs::remove_dir_all(&root).expect("should remove temporary project root");
}

/// Boundary regression: a scripted check duration is recorded before rendering and
/// renderer work cannot change the command total.
///
/// WHAT: proves execute/classify -> capture scripted duration -> render ordering
///       by injecting a renderer callback that performs observable work after capture.
/// WHY:  the structured command total must equal the scripted duration exactly,
///       regardless of renderer work, pinning the execution-to-presentation boundary.
#[cfg(feature = "timers")]
#[test]
fn check_command_total_excludes_renderer_work() {
    let _test_guard = crate::timing::lock_instrumentation_tests();

    let outcome = super::CheckOutcome {
        messages: CompilerMessages::empty(StringTable::new()),
        status: crate::projects::command_status::CommandStatus::Success,
    };
    let scripted_duration = Duration::from_millis(37);
    let renderer_calls = AtomicUsize::new(0);

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let (status, _) = run_check_for_tests(
        outcome,
        super::CheckOptions::default(),
        scripted_duration,
        |outcome, duration| {
            // Simulate renderer work after capture. The scripted duration must
            // remain the recorded total regardless of this work.
            std::thread::sleep(Duration::from_millis(5));
            assert_eq!(duration, scripted_duration);
            assert!(
                !outcome.messages.has_errors(),
                "renderer receives an outcome whose classification is already decided"
            );
            renderer_calls.fetch_add(1, Ordering::SeqCst);
        },
    );
    let snapshot = timing_session.finish();

    assert_eq!(
        status,
        crate::projects::command_status::CommandStatus::Success
    );
    assert_eq!(
        renderer_calls.load(Ordering::SeqCst),
        1,
        "renderer must run after capture"
    );

    let command_total = snapshot
        .timings
        .iter()
        .find(|observation| observation.metric.descriptor().stable_name == "command.check.total")
        .expect("command.check.total must be recorded");

    // The structured total must equal the scripted duration exactly, proving
    // renderer work did not enter the captured boundary.
    assert_eq!(command_total.total, scripted_duration);
    assert_eq!(command_total.samples, 1, "exactly one command-total sample");
}
