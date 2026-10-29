//! Tests for compiler-owned build command execution, output writing, and timing boundaries.

use super::{
    BuildCommandReport, BuildCommandSuccess, create_build_output_plan, run_build_command,
    run_build_command_with_output_plan, single_file_project_entry_dir,
    write_build_outputs_with_plan,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, BuilderSelection, FileKind, OutputFile, Project};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{
    BuilderIdentity, CleanupPolicy, OutputOwner, OutputPlan, SingleFileOutputPlan,
};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, InfrastructureErrors};
#[cfg(feature = "timers")]
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::{
    BoundaryAccumulator, SuccessfulWarningBoundary, SuccessfulWarningPosition,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDraft, DiagnosticSeverity, NamingConvention,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::Config;
use std::cell::OnceCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn test_project_builder() -> BuilderSelection {
    BuilderSelection::new(HtmlProjectBuilder::new())
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
    warnings: Vec<DiagnosticDraft>,
    string_table: StringTable,
) -> BuildResult {
    let mut warning_accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    if !warnings.is_empty() {
        let (strings, mut sink) = warning_accumulator.compiler_parts_mut();
        let batch = PreparedDiagnosticBatch::capture_from_root_string_table(
            DiagnosticBag::from_diagnostics(warnings),
            strings,
            None,
            || None,
        )
        .expect("test warnings should capture");
        sink.append(
            SuccessfulWarningBoundary::Project,
            SuccessfulWarningPosition::BaseModule(0),
            batch,
        );
    }

    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        source_database: None,
        warning_accumulator,
        output_owner: OutputOwner {
            builder: BuilderIdentity::html(),
            profile: BuildProfile::Dev,
        },
        directory_output_plan: None,
    }
}

fn temporary_single_file_output_plan(root: &Path, build_result: &BuildResult) -> OutputPlan {
    OutputPlan::SingleFile(SingleFileOutputPlan {
        output_root: root.to_path_buf(),
        project_root: Some(root.to_path_buf()),
        owner: build_result.output_owner.clone(),
    })
}

#[test]
fn single_file_project_entry_dir_reports_missing_parent_as_filesystem_failure() {
    let temp = tempfile::tempdir().expect("should create a temporary root");
    let missing_parent = temp.path().join("missing");
    let entry_path = missing_parent.join("main.moth");

    let failure = single_file_project_entry_dir(&entry_path)
        .expect_err("a missing containing directory must fail output planning");

    assert_eq!(
        failure.kind,
        crate::compiler_frontend::compiler_errors::InfrastructureFailureKind::Filesystem
    );
    assert_eq!(
        failure.stage,
        crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly
    );
    assert_eq!(failure.host_path.as_deref(), Some(missing_parent.as_path()));
    assert!(
        failure.message.contains("containing directory") && failure.message.contains("missing"),
        "the failure should explain which containing directory needs attention: {}",
        failure.message
    );
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
        temp.path(),
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
    let build_result = build_result_with_warnings(Vec::new(), StringTable::new());
    let outcome: Result<CompilationOutcome<BuildCommandSuccess>, InfrastructureErrors> =
        write_build_outputs_with_plan(build_result, |result| {
            Ok(temporary_single_file_output_plan(temp.path(), result))
        });

    assert!(matches!(
        outcome,
        Ok(CompilationOutcome::Success { warnings: None, .. })
    ));
}

#[test]
fn successful_build_preserves_warning_diagnostics() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let mut strings = StringTable::new();
    let name = strings.intern("unused_value");
    let warning =
        names_declarations::identifier_naming_convention(name, NamingConvention::CamelCase, None);
    let temp = tempfile::tempdir().expect("should create output directory");
    let build_result = build_result_with_warnings(vec![warning], strings);
    let outcome: Result<CompilationOutcome<BuildCommandSuccess>, InfrastructureErrors> =
        write_build_outputs_with_plan(build_result, |result| {
            Ok(temporary_single_file_output_plan(temp.path(), result))
        });

    let Ok(CompilationOutcome::Success {
        value,
        warnings: Some(messages),
    }) = outcome
    else {
        panic!("successful build should retain warning diagnostics");
    };
    assert_eq!(value.output_file_count, 1);
    assert_eq!(messages.counts().warnings, 1);
    assert_eq!(messages.counts().errors, 0);
}
#[test]
fn warning_capacity_failure_marks_build_command_completion_as_failed() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let mut strings = StringTable::new();
    let warnings = ["unused_value", "another_unused_value"]
        .into_iter()
        .map(|name| {
            let name = strings.intern(name);
            names_declarations::identifier_naming_convention(
                name,
                NamingConvention::CamelCase,
                None,
            )
        })
        .collect();
    let build_result = build_result_with_warnings(warnings, strings);
    let temp = tempfile::tempdir().expect("should create output directory");
    let outcome: Result<CompilationOutcome<BuildCommandSuccess>, InfrastructureErrors> =
        write_build_outputs_with_plan(build_result, |result| {
            result.warning_accumulator.set_record_limit_for_test(2);
            Ok(temporary_single_file_output_plan(temp.path(), result))
        });
    let completion = BuildCommandReport::completion(&outcome);

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Failure
    );
    let Ok(CompilationOutcome::Diagnosed(reports)) = outcome else {
        panic!("warning capacity exhaustion should diagnose build completion");
    };
    assert_eq!(reports.counts().errors, 1);
    assert_eq!(reports.counts().warnings, 0);
    let codes = reports
        .iter()
        .map(|(_, view)| view.descriptor().code)
        .collect::<Vec<_>>();
    assert_eq!(codes, vec!["MOTH-SYNTAX-0037"]);
}

const WARNING_PAGE: &str = "value ~= \"hello\"\nresult ~= \"unset\"\n\nif value is:\n    \"one\" => ~result = \"one\"\n    \"one\" => ~result = \"one\"\n    else => ~result = \"other\"\n;\n";

#[test]
fn clean_cli_build_releases_its_source_owner_before_presentation() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), "value = 1\n");
    let source_owner = OnceCell::new();
    let mut presented = false;

    let completion = run_build_command_with_output_plan(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        |build_result| {
            let database = build_result
                .source_database
                .as_ref()
                .expect("a config-backed build retains its project sources while output plans");
            source_owner
                .set(Arc::downgrade(database))
                .expect("output planning runs once");
            create_build_output_plan(build_result, temp.path())
        },
        |report| {
            assert!(matches!(
                report.outcome,
                Ok(CompilationOutcome::Success { warnings: None, .. })
            ));
            let source_owner = source_owner
                .get()
                .expect("output planning observed the source owner");
            assert!(
                source_owner.upgrade().is_none(),
                "a clean build must release its source owner before presentation"
            );
            presented = true;
        },
    );

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
    );
    assert!(presented);
}

#[test]
fn cli_warning_report_owns_its_identity_context_until_dropped() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("should create temporary project");
    write_directory_project(temp.path(), WARNING_PAGE);
    let mut presented = false;

    let completion = run_build_command(
        &test_project_builder(),
        temp.path().to_str().expect("project path should be UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
        temp.path(),
        |report| {
            let Ok(CompilationOutcome::Success {
                warnings: Some(warnings),
                ..
            }) = report.outcome
            else {
                panic!("a build with one frontend warning should succeed with its warning report");
            };
            let context = {
                let (warning_report, warning) = warnings
                    .iter_severity(DiagnosticSeverity::Warning)
                    .next()
                    .expect("the warning report should hold the frontend warning");
                assert_eq!(warning.descriptor().code, "MOTH-RULE-0022");
                let position = DiagnosticRenderContext::for_report(warning_report)
                    .site_position(warning.primary_site())
                    .expect("the warning should resolve through its report context");
                assert!(position.path.ends_with("@page.moth"));
                assert!(position.line.contains("\"one\" => ~result = \"one\""));
                Arc::downgrade(&warning_report.shared_context())
            };
            assert!(
                context.upgrade().is_some(),
                "the presented warning report keeps its identity context alive"
            );
            drop(warnings);
            assert!(
                context.upgrade().is_none(),
                "the warning report must be the identity context's last owner"
            );
            presented = true;
        },
    );

    assert_eq!(
        completion.status,
        crate::projects::ProjectCommandStatus::Success
    );
    assert!(presented);
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
        temp.path(),
        |report| {
            presentation_calls += 1;
            presented_duration = Some(report.duration);
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
            create_build_output_plan(build_result, temp.path())
        },
        |report| {
            if let Ok(CompilationOutcome::Success { value, .. }) = report.outcome {
                emitted_count = Some(value.output_file_count);
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
        |_| {
            Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                "test output plan failure",
            ))
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
            create_build_output_plan(build_result, temp.path())
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
