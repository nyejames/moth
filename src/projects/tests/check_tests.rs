//! Tests for the frontend-only `check` command flow.

use super::execute_check;
#[cfg(feature = "timers")]
use super::run_check_for_tests;
#[cfg(feature = "timers")]
use super::{CheckCommandReport, CheckOutcome, run_check_command};
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::BoundaryFailure;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_tests::test_fs::assert_path_missing;
use crate::compiler_tests::test_support::run_stack_budgeted_child;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
#[cfg(feature = "timers")]
use crate::timing::{TimingMetric, start_benchmark_collection};
use std::fs;
use std::path::PathBuf;
#[cfg(feature = "timers")]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "timers")]
use std::time::Duration;

/// The builder every check test compiles under.
fn test_project_builder() -> ProjectBuilder {
    ProjectBuilder::new(HtmlProjectBuilder::new())
}

#[test]
fn check_compiles_single_file_without_writing_artifacts() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let outcome = execute_check(
        entry_file
            .to_str()
            .expect("temp file path should be valid UTF-8 for this test"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
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
}

#[cfg(unix)]
#[test]
fn check_rejects_config_filename_namesake_source_contract() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config file");
    fs::write(
        source_root.join("@page.moth"),
        "enabled #Config of Bool = true\n",
    )
    .expect("should write page source");
    fs::write(
        source_root.join("config.moth"),
        "enabled #Config of Bool = false\n",
    )
    .expect("should write namesake source");

    let outcome = execute_check(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );

    assert!(
        outcome
            .messages
            .reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, view)| view.reason_key()
                == Some("invalid_config.config_qualifier_invalid_placement"))
    );
}

#[cfg(unix)]
#[test]
fn check_reports_missing_config_input_from_unselected_source() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config file");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write page source");
    fs::write(
        source_root.join("orphan.moth"),
        "required_value #Config of Bool\n",
    )
    .expect("should write unselected config source");

    let outcome = execute_check(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );

    assert!(
        outcome
            .messages
            .reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, view)| view.reason_key() == Some("invalid_config.missing_config_input"))
    );
}

#[test]
fn check_rejects_unselected_runtime_template_as_non_root_source() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config file");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write page source");
    fs::write(source_root.join("helper.moth"), "[\"hello\"]\n")
        .expect("should write non-root source");

    let outcome = execute_check(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );

    assert!(
        outcome
            .messages
            .reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, view)| view.descriptor().code == "MOTH-RULE-0023")
    );
}

#[test]
fn check_compiles_unselected_same_module_source_closure() {
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let root = _temp.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config file");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write page source");
    fs::write(
        source_root.join("orphan.moth"),
        "@helper identity\nconsume |value I32| -> I32:\n    return identity(value)\n;\n",
    )
    .expect("should write unselected consumer source");
    fs::write(
        source_root.join("helper.moth"),
        "identity |value I32| -> I32:\n    return value\n;\n",
    )
    .expect("should write unselected provider source");

    let outcome = execute_check(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );

    assert!(
        !outcome.messages.has_errors(),
        "same-module source closure should compile in check mode"
    );
}

/// Check records its bootstrap and frontend boundaries in owner order.
#[cfg(feature = "timers")]
#[test]
fn successful_check_finishes_bootstrap_before_frontend() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let outcome = execute_check(
        entry_file
            .to_str()
            .expect("temporary path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
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
}

/// Config AST stages use their dedicated v1 identities and finish in owner order.
#[cfg(feature = "timers")]
#[test]
fn config_ast_timers_use_dedicated_identities() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config file");
    fs::write(source_root.join("@page.moth"), "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let outcome = execute_check(
        root.to_str().expect("temporary path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );
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
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let package = root.join("src/warnpkg");
    let src = root.join("src");
    fs::create_dir_all(&package).expect("should create package root");
    fs::create_dir_all(&src).expect("should create entry root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write project root");
    fs::write(
        package.join("+package.moth"),
        "export:\n    run || -> I32:\n        value ~= \"hello\"\n        result ~= \"unset\"\n\n        if value is:\n            \"one\" => result = \"one\"\n            \"one\" => result = \"one\"\n            else => result = \"other\"\n        ;\n        return 1\n    ;\n;\n",
    )
    .expect("should write warning package root");

    let outcome = execute_check(
        root.to_str()
            .expect("temporary project path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );
    assert!(
        !outcome.messages.has_errors(),
        "check should not treat a source-package warning as an error"
    );
    assert!(
        outcome.messages.warning_count() >= 1,
        "check should retain the source-package warning"
    );
}

#[cfg(unix)]
#[test]
fn check_rejects_symlinked_directory_output_roots_before_frontend_work() {
    use std::os::unix::fs::symlink;

    for (_case_name, target_name, expected_reason_key) in [
        (
            "sibling",
            "outside",
            "invalid_config.invalid_output_folder.resolves_outside_project_root",
        ),
        (
            "entry",
            "src",
            "invalid_config.invalid_output_folder.inside_or_equal_to_entry_root",
        ),
    ] {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let source_root = root.join("src");
        let _temp1 = tempfile::tempdir().expect("should create temp dir");
        let outside = _temp1.path().to_path_buf();
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
            "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= (\n    dev_output = \"dev\",\n    release_output = \"release\",\n)\n",
        )
        .expect("should write config");
        fs::write(source_root.join("@page.moth"), "#[:<h1>Check</h1>]\n")
            .expect("should write source");

        let outcome = execute_check(
            root.to_str()
                .expect("temporary project path should be valid UTF-8"),
            &BuildConfigInputSet::new(),
            &test_project_builder(),
        );
        assert!(
            outcome
                .messages
                .reports
                .iter_severity(DiagnosticSeverity::Error)
                .any(|(_, view)| view.reason_key() == Some(expected_reason_key))
        );
        assert_path_missing(&outside.join("index.html"));
        assert_path_missing(&source_root.join("index.html"));

        fs::remove_dir_all(&outside).expect("should remove target root");
    }
}

/// Stable source-facing identity for one frontend diagnostic.
type DiagnosticIdentityRow = (&'static str, Option<&'static str>, String, i32);

/// Create a directory project whose `src/@page.moth` holds `source`.
fn write_page_project(_prefix: &str, source: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), source).expect("should write @page.moth source");
    (temp, root)
}

/// Collect ordered identity rows from the frozen boundary reports.
///
/// WHAT: records each report view's stable code and reason key, plus its frozen source path and
///       one-based start line.
/// WHY: check/build parity must exercise the same immutable report data that production renders.
fn diagnostic_identity_sequence_boundary(
    failure: &BoundaryFailure,
    project_root: &std::path::Path,
) -> Vec<DiagnosticIdentityRow> {
    let canonical_project_root = project_root
        .canonicalize()
        .expect("diagnostic fixture root should canonicalize");

    failure
        .reports
        .iter()
        .map(|(report, view)| {
            let position = DiagnosticRenderContext::for_report(report)
                .site_position(view.primary_site())
                .expect("frontend diagnostics should retain a resolvable primary source span");
            let relative_path = position
                .path
                .strip_prefix(&canonical_project_root)
                .unwrap_or(position.path.as_path());
            let normalized_path = relative_path
                .to_str()
                .expect("diagnostic logical path should be valid UTF-8")
                .replace('\\', "/");
            let line = position.start.line.saturating_add(1) as i32;
            (
                view.descriptor().code,
                view.reason_key(),
                normalized_path,
                line,
            )
        })
        .collect()
}

#[test]
fn check_and_build_frontends_produce_identical_diagnostics_and_check_writes_no_artifacts() {
    let builder = ProjectBuilder::new(HtmlProjectBuilder::new());

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
    let (_warning_temp, warning_root) =
        write_page_project("check_build_parity_warning", warning_source);

    let check_warning_outcome = execute_check(
        warning_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
    );
    assert!(
        !check_warning_outcome.messages.has_errors(),
        "warning fixture should not produce frontend errors"
    );
    let mut build_warning_result = build_project(
        &builder,
        warning_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("warning fixture should build successfully");

    assert_path_missing(&warning_root.join("dev"));
    assert_path_missing(&warning_root.join("release"));
    assert_path_missing(&warning_root.join("index.html"));
    let remaining: Vec<_> = fs::read_dir(&warning_root)
        .expect("should read warning project root")
        .map(|entry| entry.expect("dirent").file_name())
        .collect();
    assert!(
        remaining.iter().any(|name| name == "config.moth")
            && remaining.iter().any(|name| name == "src"),
        "check should leave the authored config and source tree, got {remaining:?}"
    );
    let build_warning_messages = build_warning_result
        .take_warning_messages()
        .expect("build warning fixture should produce a report");
    let check_warning_identity =
        diagnostic_identity_sequence_boundary(&check_warning_outcome.messages, &warning_root);
    let build_warning_identity =
        diagnostic_identity_sequence_boundary(&build_warning_messages, &warning_root);

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
increment |value ~I32|:
    value += 1
;

count ~= 0
increment(count)
";
    let (_error_temp, error_root) = write_page_project("check_build_parity_error", error_source);

    let check_error_outcome = execute_check(
        error_root
            .to_str()
            .expect("temp project path should be valid UTF-8 for this test"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
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
        &BuildConfigInputSet::new(),
    ) else {
        panic!("error fixture should fail the build frontend");
    };

    let check_error_identity =
        diagnostic_identity_sequence_boundary(&check_error_outcome.messages, &error_root);
    let build_error_identity =
        diagnostic_identity_sequence_boundary(&build_error_messages, &error_root);

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

/// Baseline test: run_check records command.check.total with exactly one sample.
#[cfg(feature = "timers")]
#[test]
fn run_check_records_command_check_total() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let completion = run_check_command(
        entry_file
            .to_str()
            .expect("temporary path should be valid UTF-8"),
        &BuildConfigInputSet::new(),
        &test_project_builder(),
        |_| {},
    );
    let snapshot = timing_session.finish();

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
    );
    let command_total = snapshot
        .timings
        .iter()
        .find(|observation| observation.metric.descriptor().stable_name == "command.check.total")
        .expect("command.check.total must be recorded");
    assert_eq!(command_total.samples, 1);
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

    let outcome = CheckOutcome {
        messages: BoundaryAccumulator::new().finish(InstallMode::Frozen),
        status: crate::projects::ProjectCommandStatus::Success,
    };
    let scripted_duration = Duration::from_millis(37);
    let renderer_calls = AtomicUsize::new(0);

    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let completion =
        run_check_for_tests(outcome, scripted_duration, |report: &CheckCommandReport| {
            // The renderer receiving the captured duration is the ordering evidence: capture
            // already happened. Sleeping here would add wall-clock time to the test without
            // strengthening that, because the recorded total is the scripted value, not a
            // measurement of this callback.
            assert_eq!(report.duration, scripted_duration);
            assert!(
                !report.messages.has_errors(),
                "renderer receives an outcome whose classification is already decided"
            );
            renderer_calls.fetch_add(1, Ordering::SeqCst);
        });
    let snapshot = timing_session.finish();

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
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

/// Marks the child copy of this test binary that performs the stack-budgeted check.
const STACK_BUDGET_CHILD_ENV: &str = "MOTH_CHECK_STACK_BUDGET_CHILD";

/// Windows reserves 1 MiB for a process main thread by default, and the dev-built benchmark
/// harness compiles in-process on that thread.
const WINDOWS_MAIN_THREAD_STACK_BYTES: usize = 1024 * 1024;

/// Ordinary statement nesting, matching the depth that overflowed the dev-built Windows
/// benchmark gate when inline AST payloads inflated every recursive body-parser frame.
const NESTED_CONDITIONAL_DEPTH: usize = 9;

#[test]
fn nested_conditionals_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "nested_conditionals_check_within_the_default_windows_main_thread_stack",
        || nested_conditional_source(NESTED_CONDITIONAL_DEPTH),
        "nested conditionals",
    );
}

/// Constant grouped origins at the depth the flat grouping stream must check within the Windows
/// main-thread budget. The dev and release profiles both run these tests.
const NESTED_NUMERIC_GROUP_DEPTH_512: usize = 512;
const NESTED_NUMERIC_GROUP_DEPTH_2048: usize = 2048;

#[test]
fn nested_numeric_groups_512_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "nested_numeric_groups_512_check_within_the_default_windows_main_thread_stack",
        || nested_numeric_group_source(NESTED_NUMERIC_GROUP_DEPTH_512),
        "512 nested constant numeric groups",
    );
}

#[test]
fn nested_numeric_groups_2048_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "nested_numeric_groups_2048_check_within_the_default_windows_main_thread_stack",
        || nested_numeric_group_source(NESTED_NUMERIC_GROUP_DEPTH_2048),
        "2048 nested constant numeric groups",
    );
}

/// Shallow sequential declarations exercise an exported const-record chain without nested
/// parser syntax. This bounded source check also traverses the owned public projection.
const CHAINED_CONST_RECORD_DEPTH: usize = 256;

#[test]
fn chained_const_records_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "chained_const_records_check_within_the_default_windows_main_thread_stack",
        || chained_const_record_source(CHAINED_CONST_RECORD_DEPTH),
        "256 chained anonymous const records with an exported root",
    );
}

/// Runtime-leaf chains keep their operands live until the walkers visit the whole tree.
/// The depth is capped because borrow-state memory grows cubically, independently of stack use.
const RUNTIME_LEAF_NUMERIC_CHAIN_DEPTH: usize = 256;

#[test]
fn runtime_leaf_grouped_numeric_chain_256_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "runtime_leaf_grouped_numeric_chain_256_check_within_the_default_windows_main_thread_stack",
        || runtime_leaf_grouped_source(RUNTIME_LEAF_NUMERIC_CHAIN_DEPTH),
        "256 nested grouped operations over a parameter",
    );
}

#[test]
fn runtime_leaf_ungrouped_numeric_chain_256_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "runtime_leaf_ungrouped_numeric_chain_256_check_within_the_default_windows_main_thread_stack",
        || runtime_leaf_ungrouped_source(RUNTIME_LEAF_NUMERIC_CHAIN_DEPTH),
        "256 ungrouped operations over a parameter",
    );
}

/// A pending binding is typed by an independent U64 consumer before the chain reads it.
#[test]
fn pending_binding_leaf_numeric_chain_256_check_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_check_test(
        "pending_binding_leaf_numeric_chain_256_check_within_the_default_windows_main_thread_stack",
        || pending_binding_leaf_grouped_source(RUNTIME_LEAF_NUMERIC_CHAIN_DEPTH),
        "256 nested grouped operations over a pending binding",
    );
}

/// Malformed deep groups must end in one diagnostic, never a native stack overflow.
#[test]
fn unclosed_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "unclosed_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || unclosed_numeric_group_source(NESTED_NUMERIC_GROUP_DEPTH_2048),
        "MOTH-SYNTAX-0002",
        "2048 unclosed numeric groups",
    );
}

#[test]
fn stray_close_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "stray_close_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || stray_close_numeric_group_source(NESTED_NUMERIC_GROUP_DEPTH_2048),
        "MOTH-SYNTAX-0002",
        "a stray close after 2048 balanced numeric groups",
    );
}

#[test]
fn dangling_operator_in_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "dangling_operator_in_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || dangling_operator_numeric_group_source(NESTED_NUMERIC_GROUP_DEPTH_2048),
        "MOTH-SYNTAX-0024",
        "a dangling operator inside 2048 numeric groups",
    );
}

#[test]
fn consecutive_operands_in_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack()
{
    run_stack_budgeted_rejection_test(
        "consecutive_operands_in_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || consecutive_operand_numeric_group_source(NESTED_NUMERIC_GROUP_DEPTH_2048),
        "MOTH-SYNTAX-0023",
        "consecutive operands inside 2048 numeric groups",
    );
}

#[test]
fn unknown_member_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "unknown_member_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || {
            grouped_numeric_function(&format!(
                "{}.missing",
                grouped_numeric_expression("1", NESTED_NUMERIC_GROUP_DEPTH_2048)
            ))
        },
        "MOTH-RULE-0048",
        "an unknown member after 2048 numeric groups",
    );
}

#[test]
fn bang_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "bang_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || {
            grouped_numeric_function(&format!(
                "{}!",
                grouped_numeric_expression("1", NESTED_NUMERIC_GROUP_DEPTH_2048)
            ))
        },
        "MOTH-RULE-0051",
        "a fallible propagation suffix after 2048 non-fallible numeric groups",
    );
}

#[test]
fn question_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "question_after_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || {
            grouped_numeric_function(&format!(
                "{}?",
                grouped_numeric_expression("1", NESTED_NUMERIC_GROUP_DEPTH_2048)
            ))
        },
        "MOTH-RULE-0051",
        "an optional propagation suffix after 2048 non-optional numeric groups",
    );
}

#[test]
fn catch_inside_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack() {
    run_stack_budgeted_rejection_test(
        "catch_inside_numeric_groups_2048_reject_within_the_default_windows_main_thread_stack",
        || {
            grouped_numeric_function(&format!(
                "({} + 1 catch)",
                grouped_numeric_expression("1", NESTED_NUMERIC_GROUP_DEPTH_2048 - 1)
            ))
        },
        "MOTH-RULE-0051",
        "catch after the last operand inside 2048 numeric groups",
    );
}

fn run_stack_budgeted_check_test(
    test_suffix: &str,
    source: impl FnOnce() -> String,
    description: &str,
) {
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_some() {
        let (error_codes, warning_count) = check_source_on_budgeted_thread(source());
        assert!(
            error_codes.is_empty(),
            "{description} should check cleanly, found {error_codes:?}"
        );
        assert_eq!(warning_count, 0, "{description} should not warn");
        return;
    }

    run_stack_budgeted_child(
        module_path!(),
        test_suffix,
        STACK_BUDGET_CHILD_ENV,
        &format!("checking {description} must fit a {WINDOWS_MAIN_THREAD_STACK_BYTES}-byte stack"),
    );
}

/// Malformed input must end in exactly one diagnostic with `expected_code`.
fn run_stack_budgeted_rejection_test(
    test_suffix: &str,
    source: impl FnOnce() -> String,
    expected_code: &str,
    description: &str,
) {
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_some() {
        let (error_codes, warning_count) = check_source_on_budgeted_thread(source());
        assert_eq!(
            error_codes,
            [expected_code],
            "{description} should reject with exactly one {expected_code} diagnostic"
        );
        assert_eq!(warning_count, 0, "{description} should not warn");
        return;
    }

    run_stack_budgeted_child(
        module_path!(),
        test_suffix,
        STACK_BUDGET_CHILD_ENV,
        &format!("checking {description} must fit a {WINDOWS_MAIN_THREAD_STACK_BYTES}-byte stack"),
    );
}

/// Runs the check on a thread with the Windows main-thread budget and returns each error code
/// together with the warning count.
fn check_source_on_budgeted_thread(source: String) -> (Vec<String>, usize) {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let entry_file = temp.path().join("main.moth");
    fs::write(&entry_file, source).expect("should write source file");

    let worker = std::thread::Builder::new()
        .name("stack-budgeted-check".to_owned())
        .stack_size(WINDOWS_MAIN_THREAD_STACK_BYTES)
        .spawn(move || {
            let outcome = execute_check(
                entry_file
                    .to_str()
                    .expect("temp file path should be valid UTF-8 for this test"),
                &BuildConfigInputSet::new(),
                &test_project_builder(),
            );
            let error_codes: Vec<String> = outcome
                .messages
                .reports
                .iter_severity(DiagnosticSeverity::Error)
                .map(|(_, view)| view.descriptor().code.to_owned())
                .collect();
            (error_codes, outcome.messages.warning_count())
        })
        .expect("should spawn the stack-budgeted check thread");

    worker
        .join()
        .expect("stack-budgeted check thread should not panic")
}

fn nested_conditional_source(depth: usize) -> String {
    let mut source = String::from("nested |seed I32| -> I32:\n");
    for level in 1..=depth {
        source.push_str(&format!("{}if seed > 0:\n", "    ".repeat(level)));
    }
    source.push_str(&format!("{}return seed\n", "    ".repeat(depth + 1)));
    for level in (1..=depth).rev() {
        source.push_str(&format!("{};\n", "    ".repeat(level)));
    }
    source.push_str("    return seed\n;\n\nio.line([nested(1)])\n");
    source
}

/// Wraps `innermost` in `depth` numeric groups that each add one.
fn grouped_numeric_expression(innermost: &str, depth: usize) -> String {
    let mut expression = innermost.to_owned();
    for _ in 0..depth {
        expression = format!("({expression} + 1)");
    }
    expression
}

/// Chains `depth` anonymous const records through earlier-declaration references and exports the
/// last row, so the public folded-value walk covers the whole chain while parsing stays shallow.
fn chained_const_record_source(depth: usize) -> String {
    let mut source = String::from("node_0 #= (leaf = \"payload\")\n");
    let mut node = String::from("node_0");
    for level in 1..depth {
        node = format!("node_{level}");
        source.push_str(&format!("{node} #= (child = node_{})\n", level - 1));
    }
    source.push_str("\nexport:\nroot #= (child = ");
    source.push_str(&node);
    source.push_str(")\n;\n");
    source
}

fn grouped_numeric_function(expression: &str) -> String {
    format!("grouped || -> U64:\n    value = {expression}\n    return value\n;\n")
}

fn nested_numeric_group_source(depth: usize) -> String {
    grouped_numeric_function(&grouped_numeric_expression("1", depth))
}

/// A parameter leaf keeps every operation runtime, so the expression stays a binop tree until
/// the walkers visit it.
fn runtime_leaf_source(expression: &str) -> String {
    format!(
        "grouped |x U64| -> U64:\n    value = {expression}\n    return value\n;\n\nio.line([: [grouped(2)]])\n"
    )
}

fn runtime_leaf_grouped_source(depth: usize) -> String {
    runtime_leaf_source(&grouped_numeric_expression("x", depth))
}

fn runtime_leaf_ungrouped_source(depth: usize) -> String {
    runtime_leaf_source(&format!("x{}", " + 1".repeat(depth)))
}

/// The binding stays pending until the independent `take_u64` consumer types it as `U64`.
fn pending_binding_leaf_grouped_source(depth: usize) -> String {
    let expression = grouped_numeric_expression("x", depth);
    format!(
        "take_u64 |value U64|:\n    io.line([: [value]])\n;\n\n\
         grouped || -> U64:\n    x = 1\n    take_u64(x)\n    value = {expression}\n    return value\n;\n\n\
         io.line([: [grouped()]])\n"
    )
}

/// Opens `depth` groups that are never closed.
fn unclosed_numeric_group_source(depth: usize) -> String {
    grouped_numeric_function(&format!("{}1{}", "(".repeat(depth), " + 1".repeat(depth)))
}

/// Closes `depth` balanced groups and then closes once more.
fn stray_close_numeric_group_source(depth: usize) -> String {
    grouped_numeric_function(&format!("{})", grouped_numeric_expression("1", depth)))
}

/// Puts a dangling operator at the innermost group, then wraps it `depth` times.
fn dangling_operator_numeric_group_source(depth: usize) -> String {
    grouped_numeric_function(&grouped_numeric_expression("(1 + )", depth))
}

/// Puts two adjacent operands at the innermost group, then wraps it `depth` times.
fn consecutive_operand_numeric_group_source(depth: usize) -> String {
    grouped_numeric_function(&grouped_numeric_expression("(1 2)", depth))
}
