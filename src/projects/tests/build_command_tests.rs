//! Tests for compiler-owned build command execution, output writing, and timing boundaries.

use super::{
    BuildCommandOutcome, create_build_output_plan, run_build_command,
    run_build_command_with_output_plan, write_build_outputs_with_plan,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, FileKind, OutputFile, Project, ProjectBuilder};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, OutputPlan, SingleFileOutputPlan,
};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, NamingConvention,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::Config;
use std::fs;
use std::path::{Path, PathBuf};

fn test_project_builder() -> ProjectBuilder {
    ProjectBuilder::new(HtmlProjectBuilder::new())
}

fn write_directory_project(root: &Path, page_source: &str) {
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= (\n    dev_output = \"preview\",\n    release_output = \"release\",\n)\n",
    )
    .expect("should write project config");
    fs::write(source_root.join("@page.moth"), page_source).expect("should write page source");
}

fn build_result_with_warnings(
    warnings: Vec<CompilerDiagnostic>,
    string_table: StringTable,
) -> BuildResult {
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(warnings),
        string_table,
        None,
        false,
        None,
    );
    let mut warning_accumulator = BoundaryAccumulator::new();
    warning_accumulator.append_prepared(batch, None);
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warning_accumulator,
        source_database: None,
        output_owner: OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

fn temporary_single_file_output_plan(root: &Path, build_result: &BuildResult) -> OutputPlan {
    OutputPlan::SingleFile(SingleFileOutputPlan {
        output_root: root.to_path_buf(),
        project_root: Some(root.to_path_buf()),
        owner: build_result.output_owner,
        setting_span: None,
    })
}

#[test]
fn build_command_writes_the_validated_directory_output_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), "#[:<h1>Compiler Build Service</h1>]\n");

    let completion = run_build_command(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |_| {},
    );

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
    );
    assert!(temp.path().join("preview/index.html").is_file());
    assert!(!temp.path().join("dev/index.html").exists());
}

#[test]
fn successful_build_without_warnings_has_no_warning_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("should create output directory");
    let mut build_result = build_result_with_warnings(Vec::new(), StringTable::new());
    let outcome = write_build_outputs_with_plan(&mut build_result, |result| {
        Ok(temporary_single_file_output_plan(temp.path(), result))
    });

    assert!(matches!(
        outcome,
        BuildCommandOutcome::Success { warnings: None, .. }
    ));
}

#[test]
fn successful_build_preserves_warning_diagnostics() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let mut strings = StringTable::new();
    let name = strings.intern("unused_value");
    let warning =
        CompilerDiagnostic::identifier_naming_convention(name, NamingConvention::CamelCase, None);
    let temp = tempfile::tempdir().expect("should create output directory");
    let mut build_result = build_result_with_warnings(vec![warning], strings);
    let outcome = write_build_outputs_with_plan(&mut build_result, |result| {
        Ok(temporary_single_file_output_plan(temp.path(), result))
    });

    let BuildCommandOutcome::Success {
        warning_count,
        warnings: Some(messages),
        ..
    } = outcome
    else {
        panic!("successful build should retain warning diagnostics");
    };
    assert_eq!(warning_count, 1);
    assert_eq!(messages.warning_count(), 1);
    assert_eq!(messages.error_count(), 0);
    assert!(!messages.has_errors());
}

#[cfg(feature = "timers")]
#[test]
fn build_command_records_one_total_before_presentation() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), "value = 1\n");
    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let mut presented_duration = None;
    let mut presentation_calls = 0;

    let completion = run_build_command(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |report| {
            presented_duration = Some(report.duration);
            presentation_calls += 1;
        },
    );
    let snapshot = timing_session.finish();

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
    );
    assert_eq!(presentation_calls, 1);
    let command_total = snapshot
        .timings
        .iter()
        .find(|observation| observation.metric.descriptor().stable_name == "command.build.total")
        .expect("command.build.total must be recorded");
    assert_eq!(
        command_total.total,
        presented_duration.expect("report was presented")
    );
    assert_eq!(command_total.samples, 1);
}

#[test]
fn build_success_counts_emitted_artifacts_not_planned_ones() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), "value = 1\n");
    let mut emitted_count = None;

    let completion = run_build_command_with_output_plan(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |build_result| {
            build_result.project.output_files = vec![
                OutputFile::new(
                    PathBuf::from("index.html"),
                    FileKind::Html(String::from("<html></html>")),
                ),
                OutputFile::new(PathBuf::from("unbuilt.js"), FileKind::NotBuilt),
            ];
            create_build_output_plan(build_result)
        },
        |report| {
            if let BuildCommandOutcome::Success {
                output_file_count, ..
            } = report.outcome
            {
                emitted_count = Some(output_file_count);
            }
        },
    );

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
    );
    assert_eq!(emitted_count, Some(1));
    assert!(!temp.path().join("preview/unbuilt.js").exists());
}

#[cfg(feature = "timers")]
#[test]
fn failed_output_plan_finishes_the_command_total_without_diagnostic_counts() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), "value = 1\n");
    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let completion = run_build_command_with_output_plan(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |_| Err(CompilerError::compiler_error("test output plan failure")),
        |_| {},
    );
    let snapshot = timing_session.finish();

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Failure
    );
    assert_eq!(completion.diagnostic_counts, None);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "command.build.total"
            })
            .expect("command total must keep a dense row")
            .samples,
        1
    );
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "build.output.total"
            })
            .expect("output total must keep a dense row")
            .samples,
        1
    );
}

#[cfg(feature = "timers")]
#[test]
fn failed_output_write_finishes_the_command_total_without_diagnostic_counts() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), "value = 1\n");
    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let completion = run_build_command_with_output_plan(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |build_result| {
            build_result.project.output_files = vec![OutputFile::new(
                PathBuf::from("../escape.html"),
                FileKind::Html(String::from("<html></html>")),
            )];
            create_build_output_plan(build_result)
        },
        |_| {},
    );
    let snapshot = timing_session.finish();

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Failure
    );
    assert_eq!(completion.diagnostic_counts, None);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "command.build.total"
            })
            .expect("command total must keep a dense row")
            .samples,
        1
    );
}
