//! Tests for build-loop state transitions and queued rebuild behavior.

use super::{
    DevBuildExecutor, ProjectBuildExecutor, build_once, dev_server_error_messages,
    run_builds_until_stable, run_single_build_cycle,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BackendBuilder, BuildResult, FileKind, OutputFile, Project, ProjectBuilder,
};
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, OutputPlan, SingleFileOutputPlan, ValidatedOutputPlan,
    WriteMode, WriteOptions, write_project_outputs,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerMessages, ErrorType, SourceLocation,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, RuleDiagnosticKind,
};
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_tests::test_support::temp_dir;
use crate::projects::dev_server::state::DevServerState;
use crate::projects::dev_server::watch;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::{Config, ProjectConfigError};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn unused_variable_warning(name: StringId, location: SourceLocation) -> CompilerDiagnostic {
    CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnusedVariable),
        DiagnosticSeverity::Warning,
        location,
        DiagnosticPayload::UnusedName { name },
    )
}

fn test_build_output_owner() -> OutputOwner {
    OutputOwner {
        builder: BuilderKind::Html,
        profile: BuildProfile::Dev,
    }
}

fn html_build_result() -> BuildResult {
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html><body>Hello</body></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: vec![],
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: vec![],
        string_table: StringTable::new(),
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn watch_scope(root: &Path, output_dir: &Path) -> watch::WatchScope {
    watch::WatchScope {
        output_dir: output_dir.to_path_buf(),
        targets: vec![watch::WatchTarget {
            watch_path: root.to_path_buf(),
            interest_path: None,
            recursive: true,
        }],
    }
}

fn multi_page_html_build_result() -> BuildResult {
    BuildResult {
        project: Project {
            output_files: vec![
                OutputFile::new(
                    PathBuf::from("index.html"),
                    FileKind::Html(String::from("<html><body>Home</body></html>")),
                ),
                OutputFile::new(
                    PathBuf::from("docs/basics/index.html"),
                    FileKind::Html(String::from("<html><body>Docs</body></html>")),
                ),
            ],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: vec![],
        },
        config: Config::new(PathBuf::from("project")),
        warnings: vec![],
        string_table: StringTable::new(),
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn html_build_result_without_entry_page() -> BuildResult {
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html><body>Hello</body></html>")),
            )],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::html(),
            warnings: vec![],
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: vec![],
        string_table: StringTable::new(),
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn html_build_result_with_warning() -> BuildResult {
    let mut string_table = StringTable::new();
    let warning = unused_variable_warning(
        string_table.get_or_intern("dev_warning".to_string()),
        SourceLocation::default(),
    );

    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html><body>Hello</body></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: vec![],
        },
        config: Config::new(PathBuf::from("main.moth")),
        warnings: vec![warning],
        string_table,
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn directory_build_result(project_root: &Path, output_folder: &str) -> BuildResult {
    let owner = test_build_output_owner();
    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html><body>Directory</body></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: vec![],
        },
        config: Config::new(project_root.to_path_buf()),
        warnings: vec![],
        string_table: StringTable::new(),
        output_owner: owner,
        directory_output_plan: Some(ValidatedOutputPlan {
            output_root: project_root.join(output_folder),
            project_root: project_root.to_path_buf(),
            entry_root: project_root.to_path_buf(),
            owner,
            setting_location: SourceLocation::default(),
        }),
    }
}

struct FakeExecutor {
    responses: Mutex<Vec<Result<BuildResult, CompilerMessages>>>,
    call_count: AtomicUsize,
    on_call: Option<Box<dyn Fn(usize) + Send + Sync>>,
}

impl FakeExecutor {
    fn new(responses: Vec<Result<BuildResult, CompilerMessages>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            call_count: AtomicUsize::new(0),
            on_call: None,
        }
    }

    fn with_on_call(
        responses: Vec<Result<BuildResult, CompilerMessages>>,
        on_call: Box<dyn Fn(usize) + Send + Sync>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses),
            call_count: AtomicUsize::new(0),
            on_call: Some(on_call),
        }
    }
}

impl DevBuildExecutor for FakeExecutor {
    fn build_and_write(
        &mut self,
        entry_file: &Path,
        _flags: &[crate::compiler_frontend::Flag],
    ) -> Result<BuildResult, CompilerMessages> {
        let call_index = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(ref callback) = self.on_call {
            callback(call_index);
        }

        let response = self
            .responses
            .lock()
            .expect("responses mutex should not be poisoned")
            .remove(0);

        match response {
            Ok(build_result) => {
                let project_root = entry_file
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                let output_plan = if let Some(plan) = build_result.directory_output_plan.as_ref() {
                    OutputPlan::Directory(plan.clone())
                } else {
                    OutputPlan::SingleFile(SingleFileOutputPlan {
                        output_root: project_root.join("dev"),
                        project_root: Some(project_root),
                        owner: build_result.output_owner,
                        setting_location: SourceLocation::default(),
                    })
                };
                write_project_outputs(
                    &build_result.project,
                    &WriteOptions {
                        output_plan,
                        write_mode: WriteMode::AlwaysWrite,
                    },
                    &build_result.string_table,
                )?;
                Ok(build_result)
            }
            Err(messages) => Err(messages),
        }
    }
}

struct InvalidOutputWarningBuilder;

impl BackendBuilder for InvalidOutputWarningBuilder {
    fn build_backend(
        &self,
        _project_compilation: crate::build_system::build::ProjectCompilation,
        config: &Config,
        _build_profile: BuildProfile,
        _flags: &[crate::compiler_frontend::Flag],
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        Ok(Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("../escape.js"),
                FileKind::Js(String::from("console.log('broken');")),
            )],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic([".js"]),
            warnings: vec![unused_variable_warning(
                string_table.get_or_intern("x".to_string()),
                SourceLocation::from_path(&config.entry_dir, string_table),
            )],
        })
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        Ok(())
    }

    fn frontend_surface(&self) -> BuilderSurface {
        BuilderSurface::with_mandatory_core()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        Vec::new()
    }
}

#[test]
fn successful_build_marks_state_ok_and_uses_declared_entry_page() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("success");
    fs::create_dir_all(&root).expect("should create temp root");
    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let mut executor = FakeExecutor::new(vec![Ok(multi_page_html_build_result())]);

    let report =
        run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());
    assert!(report.build_ok);
    assert_eq!(report.version, 1);

    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(build_state.last_build_ok);
    assert_eq!(
        build_state
            .entry_page_rel
            .as_ref()
            .expect("declared entry page should be set"),
        &PathBuf::from("index.html")
    );
    assert!(output_dir.join("index.html").exists());
    assert!(output_dir.join("docs/basics/index.html").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn successful_rebuild_updates_output_and_watch_roots_from_new_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("dev_output_plan_change");
    fs::create_dir_all(&root).expect("should create temp root");
    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = FakeExecutor::new(vec![
        Ok(directory_build_result(&root, "dev")),
        Ok(directory_build_result(&root, "preview")),
    ]);

    let first_report = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    assert!(first_report.build_ok);
    assert_eq!(
        state
            .build_state
            .lock()
            .expect("build state should not be poisoned")
            .output_dir,
        fs::canonicalize(&root)
            .expect("test root should canonicalize")
            .join("dev")
    );

    let second_report = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    assert!(second_report.build_ok);
    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    let canonical_root = fs::canonicalize(&root).expect("test root should canonicalize");
    assert_eq!(build_state.output_dir, canonical_root.join("preview"));
    assert_eq!(
        second_report
            .watch_scope
            .expect("successful rebuild should return a watch scope")
            .output_dir,
        canonical_root.join("preview")
    );
    assert!(root.join("preview/index.html").exists());

    drop(build_state);
    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn failed_build_marks_state_and_stores_error_page() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("failure");
    fs::create_dir_all(&root).expect("should create temp root");
    let state = Arc::new(DevServerState::new(root.join("dev")));
    let messages =
        CompilerMessages::from_error(CompilerError::compiler_error("boom"), StringTable::new());
    let mut executor = FakeExecutor::new(vec![Err(messages)]);

    let report =
        run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());
    assert!(!report.build_ok);
    assert_eq!(report.version, 1);

    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(!build_state.last_build_ok);
    assert!(build_state.last_error_html.is_some());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_without_declared_entry_page_is_treated_as_failure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("missing_entry_page");
    fs::create_dir_all(&root).expect("should create temp root");
    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = FakeExecutor::new(vec![Ok(html_build_result_without_entry_page())]);

    let report = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    assert!(!report.build_ok);

    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(!build_state.last_build_ok);
    assert!(
        build_state
            .last_build_messages_summary
            .contains("did not declare a dev entry page")
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_version_increments_on_each_attempt() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("version");
    fs::create_dir_all(&root).expect("should create temp root");
    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = FakeExecutor::new(vec![Ok(html_build_result()), Ok(html_build_result())]);

    let first = run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());
    let second =
        run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());

    assert_eq!(first.version, 1);
    assert_eq!(second.version, 2);
    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn queued_rebuild_runs_when_files_change_during_build() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("queued");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join("main.moth"), "start").expect("should write initial source file");
    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let (watch_session, watch_trigger) =
        watch::WatchSession::manual(watch_scope(&root, &output_dir));

    let watched_file = root.join("main.moth");
    let mut executor = FakeExecutor::with_on_call(
        vec![Ok(html_build_result()), Ok(html_build_result())],
        Box::new(move |call_index| {
            if call_index == 1 {
                fs::write(&watched_file, "updated")
                    .expect("should mutate watched file during first build");
                watch_trigger.notify_change();
            }
        }),
    );

    let builds = run_builds_until_stable(
        &state,
        &mut executor,
        &root.join("main.moth"),
        &Vec::new(),
        &watch_session,
    )
    .expect("build loop should complete");

    assert!(builds.watch_scope.is_some());
    assert_eq!(executor.call_count.load(Ordering::SeqCst), 2);
    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn rebuild_loop_stops_at_max_consecutive_rebuilds() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    use super::MAX_CONSECUTIVE_REBUILDS;

    let root = temp_dir("max_rebuilds");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join("main.moth"), "start").expect("should write initial source file");
    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let (watch_session, watch_trigger) =
        watch::WatchSession::manual(watch_scope(&root, &output_dir));

    // Build enough responses for every possible rebuild cycle.
    let responses: Vec<_> = (0..MAX_CONSECUTIVE_REBUILDS + 2)
        .map(|_| Ok(html_build_result()))
        .collect();

    let watched_file = root.join("main.moth");
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    // Mutate the watched file on every call so fingerprints always change.
    let mut executor = FakeExecutor::with_on_call(
        responses,
        Box::new(move |call_index| {
            counter_clone.store(call_index, Ordering::SeqCst);
            let content = format!("version_{call_index}");
            fs::write(&watched_file, content).expect("should mutate watched file during build");
            watch_trigger.notify_change();
        }),
    );

    let _builds = run_builds_until_stable(
        &state,
        &mut executor,
        &root.join("main.moth"),
        &Vec::new(),
        &watch_session,
    )
    .expect("build loop should complete despite instability");

    // The loop must stop at the safety limit rather than running forever.
    assert_eq!(
        executor.call_count.load(Ordering::SeqCst),
        MAX_CONSECUTIVE_REBUILDS
    );
    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn dev_server_error_messages_use_dev_server_error_type() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let messages = dev_server_error_messages(Path::new("x.moth"), "oops");
    assert_eq!(messages.error_count(), 1);
    let (error_type, _message, _location) = messages
        .first_infrastructure_error_for_tests()
        .expect("dev-server failure should be wrapped for rendering");
    assert_eq!(error_type, &ErrorType::DevServer);
}

#[test]
fn successful_build_with_warnings_preserves_structured_success_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("success_warnings");
    fs::create_dir_all(&root).expect("should create temp root");
    let mut executor = FakeExecutor::new(vec![Ok(html_build_result_with_warning())]);

    let outcome = build_once(&mut executor, &root.join("main.moth"), &Vec::new());

    assert!(outcome.build_succeeded);
    let messages = outcome
        .success_messages
        .expect("successful build with warnings should carry structured warnings");
    assert_eq!(messages.warning_count(), 1);
    assert_eq!(messages.error_count(), 0);
    assert!(
        outcome.diagnostics_summary.contains("Unused variable"),
        "summary should name the warning, got: {}",
        outcome.diagnostics_summary
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn successful_build_without_warnings_has_no_success_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("success_no_warnings");
    fs::create_dir_all(&root).expect("should create temp root");
    let mut executor = FakeExecutor::new(vec![Ok(html_build_result())]);

    let outcome = build_once(&mut executor, &root.join("main.moth"), &Vec::new());

    assert!(outcome.build_succeeded);
    assert!(
        outcome.success_messages.is_none(),
        "clean successful builds should not allocate an empty warning container"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn rebuild_loop_success_with_warnings_updates_summary() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("rebuild_warnings");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join("main.moth"), "start").expect("should write initial source file");
    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let (watch_session, _watch_trigger) =
        watch::WatchSession::manual(watch_scope(&root, &output_dir));

    let mut executor = FakeExecutor::new(vec![Ok(html_build_result_with_warning())]);

    let report = run_builds_until_stable(
        &state,
        &mut executor,
        &root.join("main.moth"),
        &Vec::new(),
        &watch_session,
    )
    .expect("build loop should complete");

    assert!(report.watch_scope.is_some());

    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(build_state.last_build_ok);
    assert!(
        build_state
            .last_build_messages_summary
            .contains("Unused variable"),
        "state summary should surface warning titles to SSE/state consumers, got: {}",
        build_state.last_build_messages_summary
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn project_build_executor_preserves_warnings_when_output_write_fails() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("write_failure_preserves_warnings");
    fs::create_dir_all(&root).expect("should create temp root");
    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    #[cfg(feature = "timers")]
    let timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let mut executor =
        ProjectBuildExecutor::new(ProjectBuilder::new(Box::new(InvalidOutputWarningBuilder)));
    let messages = match executor.build_and_write(&entry_file, &[]) {
        Ok(_) => panic!("invalid output path should fail writing"),
        Err(messages) => messages,
    };

    #[cfg(feature = "timers")]
    let timing_snapshot = timing_session.finish();

    #[cfg(feature = "timers")]
    assert_eq!(
        timing_snapshot
            .timings
            .iter()
            .find(|observation| observation.metric.descriptor().stable_name == "build.output.total")
            .expect("failed dev output writes retain a dense output-total row")
            .samples,
        1,
        "the failed output-plan/write span must finish before warning extension"
    );

    assert_eq!(messages.error_count(), 1);
    let warnings: Vec<_> = messages.warnings().collect();
    assert_eq!(warnings.len(), 1);
    let (_error_type, _message, location) = messages
        .first_infrastructure_error_for_tests()
        .expect("output write failure should be wrapped for rendering");
    assert_eq!(
        location.scope.to_path_buf(&messages.string_table),
        PathBuf::from("../escape.js")
    );
    assert_eq!(
        warnings[0]
            .primary_location
            .scope
            .to_path_buf(&messages.string_table),
        entry_file
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn project_build_executor_writes_the_validated_directory_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("project_executor_directory_plan");
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"preview\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write project config");
    fs::write(
        source_root.join("@page.moth"),
        "#[:<h1>Directory Executor</h1>]\n",
    )
    .expect("should write page source");

    let mut executor =
        ProjectBuildExecutor::new(ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())));
    #[cfg(feature = "timers")]
    let timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let build_result = executor
        .build_and_write(&root, &[])
        .expect("directory dev build should succeed");

    #[cfg(feature = "timers")]
    let timing_snapshot = timing_session.finish();

    #[cfg(feature = "timers")]
    assert_eq!(
        timing_snapshot
            .timings
            .iter()
            .find(|observation| observation.metric.descriptor().stable_name == "build.output.total")
            .expect("successful dev output writes retain a dense output-total row")
            .samples,
        1,
        "the output-plan/filesystem-write span must finish before the executor returns"
    );

    assert_eq!(
        build_result
            .directory_output_plan
            .as_ref()
            .expect("directory build should return its output plan")
            .output_root,
        root.join("preview")
    );
    assert!(root.join("preview/index.html").exists());
    assert!(!root.join("dev/index.html").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[cfg(feature = "timers")]
#[test]
fn dev_cycle_records_build_and_write_and_drains_one_collection_per_build() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("dev_cycle_timing");
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(root.join("config.moth"), "entry_root #= \"src\"\n").expect("should write config");
    fs::write(source_root.join("@page.moth"), "#[:<h1>Dev Cycle</h1>]\n")
        .expect("should write page source");

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor =
        ProjectBuildExecutor::new(ProjectBuilder::new(Box::new(HtmlProjectBuilder::new())));

    let first = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    let second = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());

    let first_snapshot = first
        .timing_snapshot
        .expect("every dev cycle must drain a timing snapshot");
    let second_snapshot = second
        .timing_snapshot
        .expect("every dev cycle must drain a timing snapshot");

    for snapshot in [&first_snapshot, &second_snapshot] {
        assert_eq!(
            snapshot
                .timings
                .iter()
                .find(|observation| {
                    observation.metric.descriptor().stable_name == "command.dev.build_write"
                })
                .expect("each dev cycle must retain a dense build/write row")
                .samples,
            1,
            "each dev cycle records exactly one build-and-write observation"
        );
        assert_eq!(
            snapshot
                .timings
                .iter()
                .find(|observation| {
                    observation.metric.descriptor().stable_name == "build.output.total"
                })
                .expect("each dev cycle must retain a dense output-total row")
                .samples,
            1,
            "each dev cycle records one output-plan/filesystem-write observation"
        );
    }
    #[cfg(feature = "detailed_timers")]
    {
        assert_eq!(
            first_snapshot
                .timings
                .iter()
                .find(|observation| {
                    observation.metric.descriptor().stable_name == "command.dev.cycle"
                })
                .expect("the first dev cycle must retain a dense cycle row")
                .samples,
            1,
            "each dev cycle records exactly one full-cycle observation"
        );
        assert_eq!(
            second_snapshot
                .timings
                .iter()
                .find(|observation| {
                    observation.metric.descriptor().stable_name == "command.dev.cycle"
                })
                .expect("the second dev cycle must retain a dense cycle row")
                .samples,
            1,
            "cycle observations must not leak across builds"
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[cfg(feature = "timers")]
#[test]
fn failed_dev_build_still_drains_timing_snapshot() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let root = temp_dir("dev_failed_cycle_timing");
    fs::create_dir_all(&root).expect("should create temp root");
    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = FakeExecutor::new(vec![Err(dev_server_error_messages(
        &root.join("main.moth"),
        "synthetic failure",
    ))]);

    let report =
        run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());

    assert!(!report.build_ok);
    assert!(
        report.timing_snapshot.is_some(),
        "a failed dev build must still drain its timing collection"
    );
    assert_eq!(
        report
            .timing_snapshot
            .as_ref()
            .expect("failed dev builds retain their timing snapshot")
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "command.dev.build_write"
            })
            .expect("the dev build/write total must retain a dense row")
            .samples,
        1,
        "the failed executor call must finish the dev build/write span before formatting errors"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}
