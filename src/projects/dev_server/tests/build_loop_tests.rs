//! Tests for build-loop state transitions and queued rebuild behavior.

use super::{
    DevBuildExecutor, ProjectBuildExecutor, run_builds_until_stable, run_single_build_cycle,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BackendBuilder, BuildResult, FileKind, OutputFile, Project, ProjectBuilder,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, OutputPlan, OutputWriteSummary, SingleFileOutputPlan,
    ValidatedOutputPlan, WriteMode, WriteOptions, write_project_outputs,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigValueLocation,
    BuildInputName, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::{BoundaryFailure, CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, DiagnosticSeverity, NamingConvention, PreparedFailure,
};
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::dev_server::dev_client::tests::run_client_test;
use crate::projects::dev_server::sse::tests::{close_sse, connect_sse, read_generations};
use crate::projects::dev_server::state::{DevServerState, OutputCapturePoint};
use crate::projects::dev_server::watch;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::{Config, ProjectConfigError};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn naming_convention_warning(name: StringId) -> CompilerDiagnostic {
    CompilerDiagnostic::identifier_naming_convention(name, NamingConvention::CamelCase, None)
}
fn warning_accumulator(
    warnings: Vec<CompilerDiagnostic>,
    string_table: StringTable,
) -> BoundaryAccumulator {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(&string_table);
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(warnings),
        string_table,
        None,
        false,
        None,
    );
    accumulator.append_prepared(batch, None);
    accumulator
}

fn append_prepared_failure(
    accumulator: &mut BoundaryAccumulator,
    failure: PreparedFailure,
    source_database: Option<Arc<crate::compiler_frontend::source::SourceDatabase>>,
) {
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            accumulator.append_prepared(batch, source_database.map(|source| (source, None)));
        }
        PreparedFailure::Infrastructure(errors) => {
            accumulator.append_infrastructure(errors, source_database);
        }
        PreparedFailure::Mixed { batch, error } => {
            accumulator
                .append_prepared(batch, source_database.clone().map(|source| (source, None)));
            accumulator.append_infrastructure(error, source_database);
        }
    }
}

fn freeze_prepared_failure(failure: PreparedFailure) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    append_prepared_failure(&mut accumulator, failure, None);
    accumulator.finish(InstallMode::Frozen)
}

fn infrastructure_failure(error: CompilerError) -> BoundaryFailure {
    freeze_prepared_failure(PreparedFailure::Infrastructure(error.into()))
}

fn dev_server_failure(path: &Path, message: &str) -> PreparedFailure {
    PreparedFailure::Infrastructure(
        CompilerError::file_error(path, message)
            .with_error_type(ErrorType::DevServer)
            .into(),
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warning_accumulator: BoundaryAccumulator::with_root_string_table(&StringTable::new()),
        source_database: None,
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("project")),
        warning_accumulator: BoundaryAccumulator::with_root_string_table(&StringTable::new()),
        source_database: None,
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn html_build_result_with_warning() -> BuildResult {
    let mut string_table = StringTable::new();
    let warning = naming_convention_warning(string_table.get_or_intern("dev_warning".to_string()));

    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html><body>Hello</body></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        warning_accumulator: warning_accumulator(vec![warning], string_table),
        source_database: None,
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn html_build_result_with_invalid_page_url_style() -> BuildResult {
    let mut build_result = html_build_result();
    build_result.config.html_section.page_url_style = Some(String::from("bad_style"));
    build_result
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(project_root.to_path_buf()),
        warning_accumulator: BoundaryAccumulator::with_root_string_table(&StringTable::new()),
        source_database: None,
        output_owner: owner,
        directory_output_plan: Some(ValidatedOutputPlan {
            output_root: project_root.join(output_folder),
            project_root: project_root.to_path_buf(),
            entry_root: project_root.to_path_buf(),
            owner,
            setting_span: None,
        }),
    }
}

struct FakeExecutor {
    responses: Mutex<Vec<Result<BuildResult, BoundaryFailure>>>,
    call_count: AtomicUsize,
    on_call: Option<Box<dyn Fn(usize) + Send + Sync>>,
    on_write: Option<Box<dyn Fn() -> Result<(), PreparedFailure> + Send>>,
}

impl FakeExecutor {
    fn new(responses: Vec<Result<BuildResult, BoundaryFailure>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            call_count: AtomicUsize::new(0),
            on_call: None,
            on_write: None,
        }
    }

    fn with_on_call(
        responses: Vec<Result<BuildResult, BoundaryFailure>>,
        on_call: Box<dyn Fn(usize) + Send + Sync>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses),
            call_count: AtomicUsize::new(0),
            on_call: Some(on_call),
            on_write: None,
        }
    }
}

impl DevBuildExecutor for FakeExecutor {
    fn build(
        &mut self,
        _entry_file: &Path,
        _flags: &[crate::compiler_frontend::Flag],
    ) -> Result<BuildResult, BoundaryFailure> {
        let call_index = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(ref callback) = self.on_call {
            callback(call_index);
        }

        self.responses
            .lock()
            .expect("responses mutex should not be poisoned")
            .remove(0)
    }

    fn write_outputs(
        &mut self,
        build_result: &mut BuildResult,
        entry_file: &Path,
    ) -> Result<OutputWriteSummary, PreparedFailure> {
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
                setting_span: None,
            })
        };
        let summary = write_project_outputs(
            &mut build_result.project,
            &WriteOptions {
                output_plan,
                write_mode: WriteMode::AlwaysWrite,
            },
            build_result.warning_accumulator.string_table_mut(),
        )?;
        if let Some(callback) = &self.on_write {
            callback()?;
        }
        Ok(summary)
    }
}

struct InvalidOutputWarningBuilder;

impl BackendBuilder for InvalidOutputWarningBuilder {
    fn build_backend(
        &self,
        _project_compilation: crate::build_system::build::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[crate::compiler_frontend::Flag],
        string_table: &mut StringTable,
    ) -> Result<Project, PreparedFailure> {
        Ok(Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("../escape.js"),
                FileKind::Js(String::from("console.log('broken');")),
            )],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic([".js"]),
            warnings: vec![naming_convention_warning(
                string_table.get_or_intern("x".to_string()),
            )],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
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
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
}

#[test]
fn successful_rebuild_updates_output_and_watch_roots_from_new_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
}

#[test]
fn failed_build_marks_state_and_stores_error_page() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let failure = infrastructure_failure(CompilerError::compiler_error("boom"));
    let mut executor = FakeExecutor::new(vec![Err(failure)]);

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
}

#[test]
fn build_without_declared_entry_page_is_treated_as_failure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut warning_build = html_build_result_with_warning();
    warning_build.project.entry_page_rel = None;
    let mut executor = FakeExecutor::new(vec![Ok(warning_build)]);

    let report = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    assert!(!report.build_ok);

    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(!build_state.last_build_ok);
    let summary = &build_state.last_build_messages_summary;
    assert!(summary.contains("did not declare a dev entry page"));
    assert!(summary.contains("COMPILER BUG:"));
    assert!(!summary.contains("OPERATIONAL FAILURE:"));
    assert_eq!(
        summary
            .lines()
            .filter(|line| line.starts_with("E|MOTH-INFRA-0001|"))
            .count(),
        1,
    );
    assert_eq!(
        summary
            .lines()
            .filter(|line| line.starts_with("W|"))
            .count(),
        1,
    );
    assert_eq!(summary.lines().count(), 2);
    let html = build_state
        .last_error_html
        .as_ref()
        .expect("failed publication should retain the error page");
    assert_eq!(
        html.matches("data-diagnostic-code=\"MOTH-INFRA-0001\"")
            .count(),
        1,
    );
}

#[test]
fn invalid_site_config_preserves_interned_diagnostic_values() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = FakeExecutor::new(vec![Ok(html_build_result_with_invalid_page_url_style())]);

    let report = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    assert!(!report.build_ok);

    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(
        build_state
            .last_build_messages_summary
            .contains("bad_style")
    );
    assert!(
        build_state
            .last_build_messages_summary
            .contains("'trailing_slash', 'no_trailing_slash', or 'ignore'")
    );
}

#[test]
fn invalid_site_config_failure_preserves_warning_after_config_error() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let mut build_result = html_build_result_with_warning();
    build_result.config.html_section.page_url_style = Some(String::from("bad_style"));
    let mut executor = FakeExecutor::new(vec![Ok(build_result)]);
    let state = Arc::new(DevServerState::new(root.join("dev")));

    let entry_file = root.join("main.moth");
    let report = run_single_build_cycle(&state, &mut executor, &entry_file, &[]);
    assert!(!report.build_ok);
    assert!(report.success_messages.is_none());
    assert!(
        report.watch_scope.is_some(),
        "site-config parsing should fail after a successful compile and output write"
    );
    assert!(
        root.join("dev/index.html").exists(),
        "the build should write outputs before site-config parsing fails"
    );

    let build_state = state.build_state.lock().expect("build state");
    assert!(!build_state.last_build_ok);
    let rows: Vec<_> = build_state.last_build_messages_summary.lines().collect();
    assert_eq!(
        rows.len(),
        2,
        "one failure should contain exactly two records"
    );
    assert_eq!(
        rows.iter().filter(|row| row.starts_with("E|")).count(),
        1,
        "site-config failure should render exactly one error"
    );
    assert_eq!(
        rows.iter().filter(|row| row.starts_with("W|")).count(),
        1,
        "the successful build's warning should be retained exactly once"
    );

    let error_fields: Vec<_> = rows[0].split('|').collect();
    assert_eq!(error_fields.len(), 5);
    assert_eq!(error_fields[0], "E");
    assert_eq!(error_fields[1], "MOTH-CONFIG-0001");
    assert_eq!(
        error_fields[4],
        "Invalid value 'bad_style' for config setting 'page_url_style'. Expected 'trailing_slash', 'no_trailing_slash', or 'ignore'."
    );

    let warning_fields: Vec<_> = rows[1].split('|').collect();
    assert_eq!(warning_fields.len(), 5);
    assert_eq!(warning_fields[0], "W");
    assert_eq!(warning_fields[1], "MOTH-RULE-0021");
    assert_eq!(
        warning_fields[4],
        "Identifier 'dev_warning' should use CamelCase"
    );
    assert!(
        rows[0].starts_with("E|") && rows[1].starts_with("W|"),
        "the config error should render before the build warning"
    );

    let html = build_state
        .last_error_html
        .as_ref()
        .expect("site-config failure should publish an error page");
    assert_eq!(html.matches("<article class=\"diagnostic\"").count(), 2);
    assert_eq!(
        html.matches("data-diagnostic-code=\"MOTH-CONFIG-0001\"")
            .count(),
        1
    );
    assert_eq!(
        html.matches("data-diagnostic-code=\"MOTH-RULE-0021\"")
            .count(),
        1
    );
    assert!(
        html.find("data-diagnostic-code=\"MOTH-CONFIG-0001\"")
            .expect("error page should render config error first")
            < html
                .find("data-diagnostic-code=\"MOTH-RULE-0021\"")
                .expect("error page should render warning after config error"),
        "the config error card should precede the warning card"
    );
}

#[test]
fn build_version_increments_on_each_attempt() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = FakeExecutor::new(vec![Ok(html_build_result()), Ok(html_build_result())]);

    let first = run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());
    let second =
        run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &Vec::new());

    assert_eq!(first.version, 1);
    assert_eq!(second.version, 2);
}

#[test]
fn queued_rebuild_runs_when_files_change_during_build() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
}

#[test]
fn rebuild_loop_stops_at_max_consecutive_rebuilds() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    use super::MAX_CONSECUTIVE_REBUILDS;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
}

#[test]
fn successful_build_with_warnings_preserves_structured_success_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let mut executor = FakeExecutor::new(vec![Ok(html_build_result_with_warning())]);

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let outcome = run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &[]);

    assert!(outcome.build_ok);
    let messages = outcome
        .success_messages
        .expect("successful build with warnings should carry structured warnings");
    assert_eq!(messages.warning_count(), 1);
    assert_eq!(messages.error_count(), 0);
    assert!(
        state
            .build_state
            .lock()
            .expect("build state")
            .last_build_messages_summary
            .contains("Identifier naming convention"),
        "summary should name the warning",
    );
}

#[test]
fn successful_build_without_warnings_has_no_success_messages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let mut executor = FakeExecutor::new(vec![Ok(html_build_result())]);

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let outcome = run_single_build_cycle(&state, &mut executor, &root.join("main.moth"), &[]);

    assert!(outcome.build_ok);
    assert!(
        outcome.success_messages.is_none(),
        "clean successful builds should not allocate an empty warning container"
    );
}

#[test]
fn rebuild_loop_success_with_warnings_updates_summary() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
            .contains("Identifier naming convention"),
        "state summary should surface warning titles to SSE/state consumers, got: {}",
        build_state.last_build_messages_summary
    );
}

#[cfg(unix)]
#[test]
fn project_build_executor_preserves_non_utf8_entry_path_as_file_failure() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let entry_file = PathBuf::from(OsString::from_vec(b"entry-\xff.moth".to_vec()));
    let mut executor = ProjectBuildExecutor::new(
        ProjectBuilder::new(HtmlProjectBuilder::new()),
        BuildConfigInputSet::new(),
    );
    let failure = match executor.build(&entry_file, &[]) {
        Ok(_) => panic!("an unrepresentable entry path must not reach compilation"),
        Err(failure) => failure,
    };
    let error = &failure
        .infrastructure_errors()
        .expect("invalid host encoding must retain its filesystem failure")
        .primary;
    assert_eq!(error.error_type, ErrorType::File);
    assert_eq!(error.host_path.as_deref(), Some(entry_file.as_path()));
}

#[test]
fn project_build_executor_preserves_warnings_when_output_write_fails() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    #[cfg(feature = "timers")]
    let timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let mut executor = ProjectBuildExecutor::new(
        ProjectBuilder::new(InvalidOutputWarningBuilder),
        BuildConfigInputSet::new(),
    );
    let mut compiled = executor
        .build(&entry_file, &[])
        .expect("source should compile");
    let prepared_failure = match executor.write_outputs(&mut compiled, &entry_file) {
        Ok(_) => panic!("invalid output path should fail writing"),
        Err(failure) => failure,
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

    let failure = compiled.take_output_failure(prepared_failure);
    assert_eq!(failure.error_count(), 1);
    assert_eq!(failure.warning_count(), 1);
    let mut warnings = failure.reports.iter_severity(DiagnosticSeverity::Warning);
    let (warning_report, warning) = warnings
        .next()
        .expect("failed output write should preserve its warning");
    assert!(warnings.next().is_none());
    let error = &failure
        .infrastructure_errors()
        .expect("output write failure should be wrapped for rendering")
        .primary;
    assert_eq!(
        error.host_path.as_deref(),
        Some(Path::new("../escape.js")),
        "output path failures should retain the rejected host path"
    );
    assert!(
        error.source_span.is_none(),
        "path-only output failures must not fabricate a source excerpt"
    );
    assert!(
        DiagnosticRenderContext::for_report(warning_report)
            .site_position(warning.primary_site())
            .is_none(),
        "synthetic warnings must not fabricate a source span"
    );
}

#[test]
fn project_build_executor_writes_the_validated_directory_plan() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "version #Config of String = \"authored\"\nproject #= (\n    name = \"docs\",\n    version = version,\n    entry_root = \"src\",\n)\nhtml #= (\n    dev_output = \"preview\",\n    release_output = \"release\",\n)\n",
    )
    .expect("should write project config");
    fs::write(
        source_root.join("@page.moth"),
        "@project version\n#[:<h1>[version]</h1>]\n",
    )
    .expect("should write page source");
    let mut build_config_inputs = BuildConfigInputSet::new();
    build_config_inputs
        .insert(BuildConfigInputEntry::new(
            BuildInputName::new("version").expect("test input name should validate"),
            PrimitiveBuildValue::String("cli".to_owned()),
            BuildConfigValueLocation::Command(BuildCommandLocation::new(1)),
        ))
        .expect("test command input should be unique");
    let mut executor = ProjectBuildExecutor::new(
        ProjectBuilder::new(HtmlProjectBuilder::new()),
        build_config_inputs,
    );
    #[cfg(feature = "timers")]
    let timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let mut build_result = executor
        .build(&root, &[])
        .expect("directory dev build should compile");
    executor
        .write_outputs(&mut build_result, &root)
        .expect("directory output should write");
    let first_output = fs::read_to_string(root.join("preview/index.html"))
        .expect("first directory dev build should write its page");
    assert!(
        first_output.contains("<h1>cli</h1>"),
        "first directory dev build must render the retained CLI input"
    );

    #[cfg(feature = "timers")]
    let timing_snapshot = timing_session.finish();

    #[cfg(feature = "timers")]
    let second_timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let mut second_build_result = executor
        .build(&root, &[])
        .expect("repeated directory dev build should compile");
    executor
        .write_outputs(&mut second_build_result, &root)
        .expect("repeated output should write");
    #[cfg(feature = "timers")]
    let _second_timing_snapshot = second_timing_session.finish();
    assert_eq!(
        second_build_result
            .directory_output_plan
            .as_ref()
            .expect("repeated directory build should return its output plan")
            .output_root,
        root.join("preview")
    );
    let second_output = fs::read_to_string(root.join("preview/index.html"))
        .expect("repeated directory dev build should write its page");
    assert!(
        second_output.contains("<h1>cli</h1>"),
        "repeated directory dev build must retain and render the CLI input"
    );

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
}

#[cfg(feature = "timers")]
#[test]
fn dev_cycle_records_build_and_write_and_drains_one_collection_per_build() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(source_root.join("@page.moth"), "#[:<h1>Dev Cycle</h1>]\n")
        .expect("should write page source");

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = ProjectBuildExecutor::new(
        ProjectBuilder::new(HtmlProjectBuilder::new()),
        BuildConfigInputSet::new(),
    );

    let first = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());
    let second = run_single_build_cycle(&state, &mut executor, &root, &Vec::new());

    let first_snapshot = first
        .timing_snapshot
        .expect("every dev cycle must drain a timing snapshot");
    let second_snapshot = second
        .timing_snapshot
        .expect("every dev cycle must drain a timing snapshot");

    assert_eq!(
        first.build_duration,
        first_snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "command.dev.build_write"
            })
            .expect("first dev cycle must retain a build/write row")
            .total,
        "first report.build_duration must exactly equal the structured command.dev.build_write total"
    );
    assert_eq!(
        second.build_duration,
        second_snapshot
            .timings
            .iter()
            .find(|observation| {
                observation.metric.descriptor().stable_name == "command.dev.build_write"
            })
            .expect("second dev cycle must retain a build/write row")
            .total,
        "second report.build_duration must exactly equal the structured command.dev.build_write total"
    );

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
}

#[cfg(feature = "timers")]
#[test]
fn failed_dev_build_still_drains_timing_snapshot() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let failure = freeze_prepared_failure(dev_server_failure(
        &root.join("main.moth"),
        "synthetic failure",
    ));
    let mut executor = FakeExecutor::new(vec![Err(failure)]);

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
}

/// Each generation changes both the page and companions, including binary Wasm bytes.
fn generation_build_result(generation: u8) -> BuildResult {
    let mut result = multi_page_html_build_result();
    result.project.output_files[0] = OutputFile::new(
        PathBuf::from("index.html"),
        FileKind::Html(format!("<html><body>generation-{generation}</body></html>")),
    );
    result.project.output_files.push(OutputFile::new(
        PathBuf::from("page.js"),
        FileKind::Js(format!("generation-{generation}")),
    ));
    result.project.output_files.push(OutputFile::new(
        PathBuf::from("page.wasm"),
        FileKind::Wasm(vec![0, 97, 115, 109, generation]),
    ));
    result
}

fn request_bytes(state: Arc<DevServerState>, request: &str) -> Vec<u8> {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind dev request listener");
    let address = listener.local_addr().expect("listener address");
    let worker = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept dev request");
        crate::projects::dev_server::http::handle_connection(stream, state)
            .expect("serve dev request");
    });
    let mut client = TcpStream::connect(address).expect("connect dev client");
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("bound dev client read");
    client
        .write_all(request.as_bytes())
        .expect("send dev request");
    client.shutdown(Shutdown::Write).expect("finish request");
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .expect("read dev response");
    worker.join().expect("dev request worker should finish");
    response
}

fn get_bytes(state: &Arc<DevServerState>, path: &str) -> Vec<u8> {
    let response = request_bytes(
        Arc::clone(state),
        &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
    );
    assert!(response.starts_with(b"HTTP/1.1 200 "));
    let body = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .expect("response has header separator")
        + 4;
    response[body..].to_vec()
}

fn get_page(state: &Arc<DevServerState>) -> String {
    String::from_utf8(get_bytes(state, "/")).expect("page is UTF-8")
}

fn post_entry_error(state: &Arc<DevServerState>, version: u64) -> String {
    let body = serde_json::json!({
        "build": version,
        "entry": "index.html",
        "invocation": "startup-1",
        "category": "entry_error",
        "code": 301,
        "message": "Integer overflow",
    })
    .to_string();
    let response = request_bytes(
        Arc::clone(state),
        &format!(
            "POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len(),
        ),
    );
    String::from_utf8(response).expect("report response is UTF-8")
}

#[test]
fn recovered_build_generation_clears_lock_poison_for_http_reads_and_reports() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create poison recovery project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let mut first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    let first_report = run_single_build_cycle(&state, &mut first, &entry, &[]);
    assert!(first_report.build_ok);
    assert!(!state.build_state.is_poisoned());

    // Poison the publication lock from a thread that dies while holding it. The join observes
    // the panic, so the test channels only this deterministic state, not sleep timing.
    let poisoned = Arc::clone(&state);
    let panicking = std::thread::spawn(move || {
        let _held = poisoned
            .build_state
            .lock()
            .expect("first lock acquisition succeeds");
        panic!("poison the publication lock");
    });
    assert!(panicking.join().is_err());
    assert!(state.build_state.is_poisoned());

    let mut next = FakeExecutor::new(vec![Ok(generation_build_result(2))]);
    let recovered = run_single_build_cycle(&state, &mut next, &entry, &[]);
    assert!(recovered.build_ok);
    assert_eq!(recovered.version, 2);
    assert!(!state.build_state.is_poisoned());

    let page = get_page(&state);
    assert!(page.contains("generation-2"));
    assert!(page.contains("const build = \"2\";"));
    assert!(post_entry_error(&state, 2).starts_with("HTTP/1.1 204 "));
    assert!(post_entry_error(&state, 2).starts_with("HTTP/1.1 204 "));
}

#[test]
fn output_reader_capture_blocks_rebuild_until_generation_bytes_are_read() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let mut first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    assert!(run_single_build_cycle(&state, &mut first, &entry, &[]).build_ok);

    let (captured_sender, captured_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    *state.capture_hook.lock().expect("read hook") = Some(Box::new(move |point| {
        if matches!(point, OutputCapturePoint::BeforeLock) {
            return;
        }
        captured_sender.send(()).expect("signal captured state");
        release_receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("release reader");
    }));
    let reader_state = Arc::clone(&state);
    let reader = std::thread::spawn(move || get_page(&reader_state));
    captured_receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("reader captures generation before output read");

    let (compiled_sender, compiled_receiver) = std::sync::mpsc::channel();
    let compile_state = Arc::clone(&state);
    let mut next = FakeExecutor::with_on_call(
        vec![Ok(generation_build_result(2))],
        Box::new(move |_| {
            // Compilation must run while the reader owns publication, not behind that lock.
            assert!(matches!(
                compile_state.build_state.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            compiled_sender
                .send(())
                .expect("signal concurrent compilation");
        }),
    );
    let rebuild_state = Arc::clone(&state);
    let rebuild =
        std::thread::spawn(move || run_single_build_cycle(&rebuild_state, &mut next, &entry, &[]));
    compiled_receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("rebuild compiles between capture and read");
    release_sender.send(()).expect("release reader capture");
    let old_page = reader.join().expect("reader should finish");
    assert!(old_page.contains("generation-1"));
    assert!(old_page.contains("const build = \"1\";"));
    assert!(!old_page.contains("generation-2"));
    assert_eq!(rebuild.join().expect("rebuild finishes").version, 2);
    *state.capture_hook.lock().expect("read hook") = None;
    let current_page = get_page(&state);
    assert!(current_page.contains("generation-2"));
    assert!(current_page.contains("const build = \"2\";"));
}

#[test]
fn output_write_and_metadata_publication_exclude_concurrent_get() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let mut first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    assert!(run_single_build_cycle(&state, &mut first, &entry, &[]).build_ok);

    let (written_sender, written_receiver) = std::sync::mpsc::channel();
    let (publish_sender, publish_receiver) = std::sync::mpsc::channel();
    let mut next = FakeExecutor::new(vec![Ok(generation_build_result(2))]);
    next.on_write = Some(Box::new(move || {
        written_sender.send(()).expect("signal written outputs");
        publish_receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("release metadata publication");
        Ok(())
    }));
    let rebuild_state = Arc::clone(&state);
    let rebuild =
        std::thread::spawn(move || run_single_build_cycle(&rebuild_state, &mut next, &entry, &[]));
    written_receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("new outputs reach write barrier");
    assert!(
        fs::read_to_string(temp.path().join("dev/index.html"))
            .expect("written page")
            .contains("generation-2")
    );
    assert!(matches!(
        state.build_state.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));

    let (get_sender, get_receiver) = std::sync::mpsc::channel();
    *state.capture_hook.lock().expect("read hook") = Some(Box::new(move |point| {
        if matches!(point, OutputCapturePoint::BeforeLock) {
            get_sender.send(()).expect("signal GET capture attempt");
        }
    }));
    let reader_state = Arc::clone(&state);
    let reader = std::thread::spawn(move || get_page(&reader_state));
    get_receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("GET attempts before publication");
    publish_sender
        .send(())
        .expect("publish completed generation");
    assert_eq!(rebuild.join().expect("rebuild finishes").version, 2);
    let page = reader.join().expect("GET finishes");
    assert!(page.contains("generation-2"));
    assert!(page.contains("const build = \"2\";"));
    assert!(!page.contains("const build = \"1\";"));
    *state.capture_hook.lock().expect("read hook") = None;
}

#[test]
fn companion_requests_cross_publication_but_reports_keep_page_generation() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let mut executor = FakeExecutor::new(vec![
        Ok(generation_build_result(1)),
        Ok(generation_build_result(2)),
    ]);
    assert!(run_single_build_cycle(&state, &mut executor, &entry, &[]).build_ok);
    let old_page = get_page(&state);
    assert!(old_page.contains("generation-1"));
    assert!(old_page.contains("const build = \"1\";"));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-1");
    assert!(run_single_build_cycle(&state, &mut executor, &entry, &[]).build_ok);

    assert_eq!(get_bytes(&state, "/page.js"), b"generation-2");
    assert_eq!(get_bytes(&state, "/page.wasm"), vec![0, 97, 115, 109, 2]);
    assert!(post_entry_error(&state, 1).starts_with("HTTP/1.1 409 "));
    assert!(
        state
            .runtime_reports
            .lock()
            .expect("report ledger")
            .keys
            .is_empty()
    );
    let current = get_page(&state);
    assert!(current.contains("generation-2"));
    assert!(current.contains("const build = \"2\";"));
    let other_before_report = get_bytes(&state, "/docs/basics/");
    assert!(post_entry_error(&state, 2).starts_with("HTTP/1.1 204 "));
    assert!(post_entry_error(&state, 2).starts_with("HTTP/1.1 204 "));
    assert_eq!(
        state
            .runtime_reports
            .lock()
            .expect("report ledger")
            .keys
            .len(),
        1
    );
    assert_eq!(
        get_page(&state),
        current,
        "runtime failure must not replace compiled HTML"
    );
    let other = String::from_utf8(get_bytes(&state, "/docs/basics/")).expect("other page");
    assert!(other.contains("Docs"));
    assert_eq!(other.as_bytes(), other_before_report);
    let build = state.build_state.lock().expect("build state");
    assert!(build.last_build_ok);
    assert!(build.last_error_html.is_none());
    assert_eq!(build.last_build_version, 2);
}

#[test]
fn stale_page_that_missed_the_publication_broadcast_reloads_on_connection() {
    // WHAT: capture a generation-1 page, publish generation 2 before that page registers SSE,
    //       straddle its companions across the publication, then run the page's own client
    //       against the generation a real SSE connection announces.
    // WHY: the missed broadcast and the rejected stale report produce no further rebuild, so
    //      only the connection handshake can start the fresh invocation without a source edit.
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let mut executor = FakeExecutor::new(vec![
        Ok(generation_build_result(1)),
        Ok(generation_build_result(2)),
    ]);
    assert!(run_single_build_cycle(&state, &mut executor, &entry, &[]).build_ok);
    let stale_page = get_page(&state);
    assert!(stale_page.contains("generation-1"));

    let missed = run_single_build_cycle(&state, &mut executor, &entry, &[]);
    assert_eq!((missed.version, missed.clients_notified), (2, 0));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-2");
    assert!(post_entry_error(&state, 1).starts_with("HTTP/1.1 409 "));

    let Some(mut connection) = connect_sse(&state) else {
        return;
    };
    let announced = read_generations(&mut connection.client, 1).remove(0);
    close_sse(&state, connection);
    assert_eq!(announced, "2");

    let announced = serde_json::Value::String(announced);
    let assertions = format!(
        r#"
const tab = page(clientScript, 'accept');
tab.context.__moth_record_entry_failure(301, 'Integer overflow', null);
tab.sources[0].listeners.generation({{data: {announced}}});
assert.equal(tab.reloads(), 1);
"#
    );
    run_client_test(&stale_page, &assertions);

    // The reloaded page carries the announced generation, so its own connection stays put.
    let current_page = get_page(&state);
    let assertions = format!(
        r#"
const tab = page(clientScript, 'accept');
tab.sources[0].listeners.generation({{data: {announced}}});
assert.equal(tab.reloads(), 0);
"#
    );
    run_client_test(&current_page, &assertions);
}

#[test]
fn output_write_failure_publishes_diagnostics_not_partial_success() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let output = temp.path().join("dev");
    let state = Arc::new(DevServerState::new(output.clone()));
    let mut first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    assert!(run_single_build_cycle(&state, &mut first, &entry, &[]).build_ok);
    let mut next = FakeExecutor::new(vec![Ok(generation_build_result(2))]);
    next.on_write = Some(Box::new(move || {
        // A late emission/finalisation failure can leave bytes changed without completing a batch.
        fs::write(output.join("index.html"), "<html>partial-generation-2")
            .expect("leave partially written page");
        Err(dev_server_failure(&output, "publication write failed"))
    }));
    let failed = run_single_build_cycle(&state, &mut next, &entry, &[]);
    assert!(!failed.build_ok);
    assert_eq!(failed.version, 2);
    let page = get_page(&state);
    assert!(page.contains("publication write failed"));
    assert!(!page.contains("partial-generation-2"));
    assert!(!page.contains("generation-1"));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-2");
    assert!(post_entry_error(&state, 2).starts_with("HTTP/1.1 409 "));
    let build = state.build_state.lock().expect("build state");
    assert!(!build.last_build_ok);
    assert!(build.last_error_html.is_some());
}

/// Records the existing writer's explicit branch, not timestamps or a parallel output writer.
struct RecordingProjectExecutor {
    inner: ProjectBuildExecutor,
    last_write: Option<OutputWriteSummary>,
}

impl DevBuildExecutor for RecordingProjectExecutor {
    fn build(
        &mut self,
        entry_file: &Path,
        flags: &[crate::compiler_frontend::Flag],
    ) -> Result<BuildResult, BoundaryFailure> {
        let result = self.inner.build(entry_file, flags);
        if let Err(messages) = &result {
            let mut diagnostics = messages.reports.iter_severity(DiagnosticSeverity::Error);
            let (report, diagnostic) = diagnostics
                .next()
                .expect("one invalid export should have an error report");
            assert!(
                diagnostics.next().is_none(),
                "one invalid export should produce exactly one diagnostic"
            );
            assert_eq!(
                diagnostic.reason_key(),
                Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"),
            );
            let mut origin_labels = diagnostic
                .durable_labels()
                .filter(|label| matches!(label.message, DurableLabelMessage::BuiltinFailureOrigin));
            let origin = origin_labels
                .next()
                .expect("invalid export should retain its builtin failure origin label");
            assert!(
                origin_labels.next().is_none(),
                "invalid export should have exactly one builtin failure origin label"
            );
            assert!(
                DiagnosticRenderContext::for_report(report)
                    .site_position(origin.site)
                    .is_some(),
                "the builtin failure origin label should retain a resolvable source site"
            );
        }
        result
    }

    fn write_outputs(
        &mut self,
        build_result: &mut BuildResult,
        entry_file: &Path,
    ) -> Result<OutputWriteSummary, PreparedFailure> {
        let summary = self.inner.write_outputs(build_result, entry_file)?;
        self.last_write = Some(summary.clone());
        Ok(summary)
    }
}

fn assert_served_entry_outcome(
    state: &Arc<DevServerState>,
    version: u64,
    error_code: Option<u32>,
) -> String {
    use crate::compiler_tests::integration_test_runner::assertions::{
        RuntimeEvent, execute_html_harness_for_test,
    };

    let page = get_page(state);
    assert!(page.contains(&format!("const build = \"{version}\";")));
    assert!(page.contains("static-survives"));

    // The existing Node harness owns application execution. Remove only the dev-only client,
    // which needs a browser EventSource, then give it the bytes actually returned by HTTP.
    let mut application_html = page.clone();
    let client_start = application_html
        .find(crate::projects::dev_server::dev_client::DEV_CLIENT_MARKER)
        .expect("served page installs dev client");
    let client_end = client_start
        + application_html[client_start..]
            .find("</script>")
            .expect("dev client script closes")
        + "</script>".len();
    application_html.drain(client_start..client_end);
    let mut served_result = html_build_result();
    served_result.project.output_files[0] = OutputFile::new(
        PathBuf::from("index.html"),
        FileKind::Html(application_html),
    );
    let rendered = execute_html_harness_for_test(&mut served_result)
        .expect("served application executes in existing Node harness");
    let expected = match error_code {
        Some(code) => RuntimeEvent::EntryFailure { code },
        None => RuntimeEvent::Console {
            text: "value-50000".to_owned(),
        },
    };
    assert_eq!(rendered.events(), &[expected]);
    assert!(rendered.runtime_error_message().is_none());
    assert_eq!(
        rendered.combined_output(),
        if error_code.is_some() {
            ""
        } else {
            "value-50000"
        },
    );
    page
}

#[test]
fn m02_real_rebuild_write_serve_tracks_private_body_entry_outcomes_and_invalid_exports() {
    use crate::build_system::output::OutputWriteOutcome;

    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create real rebuild project");
    let root = temp.path();
    let source = root.join("src");
    fs::create_dir_all(source.join("support")).expect("create support module");
    fs::create_dir_all(source.join("other")).expect("create other page");
    fs::write(
        root.join("config.moth"),
        "project #= (name = \"rebuild_contract\", entry_root = \"src\")\nhtml #= ()\n",
    )
    .expect("write config");
    fs::write(
        source.join("@page.moth"),
        "@support product\n\
         asset #= @site.css\n\
         #[:<main>static-survives</main><link rel=\"stylesheet\" href=\"[asset]\">]\n\
         argument ~= 50000\n\
         result = product(argument)!\n\
         io.line([:value-[result]])\n",
    )
    .expect("write homepage");
    fs::write(source.join("site.css"), "body { color: green; }").expect("write supporting asset");
    fs::write(
        source.join("other/@page.moth"),
        "#[:<main>other-page</main>]\n",
    )
    .expect("write independent page");

    const PUBLIC_API: &str = "\nexport:\n\
        product |value I32| -> I32, Error!:\n\
            return multiply(value)\n\
        ;\n\
        success_only |value I32| -> I32:\n\
            return plain(value)\n\
        ;\n\
    ;\n";
    let helper_path = source.join("support/+package.moth");
    let successful = format!(
        "multiply |value I32| -> I32:\n    return value\n;\n\
         plain |value I32| -> I32:\n    return value\n;\n{PUBLIC_API}"
    );
    let failing_entry = format!(
        "multiply |value I32| -> I32:\n    return value * value\n;\n\
         plain |value I32| -> I32:\n    return value\n;\n{PUBLIC_API}"
    );
    let invalid_export = format!(
        "multiply |value I32| -> I32:\n    return value\n;\n\
         plain |value I32| -> I32:\n    return value * value\n;\n{PUBLIC_API}"
    );
    fs::write(&helper_path, &successful).expect("write successful private helpers");
    let state = Arc::new(DevServerState::new(root.join("dev")));
    let mut executor = RecordingProjectExecutor {
        inner: ProjectBuildExecutor::new(
            ProjectBuilder::new(HtmlProjectBuilder::new()),
            BuildConfigInputSet::new(),
        ),
        last_write: None,
    };

    let first = run_single_build_cycle(&state, &mut executor, root, &[]);
    assert!(
        first.build_ok,
        "{}",
        state
            .build_state
            .lock()
            .expect("build state")
            .last_build_messages_summary
    );
    let first_page = assert_served_entry_outcome(&state, first.version, None);
    assert_eq!(
        executor
            .last_write
            .as_ref()
            .expect("first write summary")
            .outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::Written),
    );

    // Only the private body changes. The public Error! signature and entry source stay identical.
    fs::write(&helper_path, &failing_entry).expect("widen private helper");
    let second = run_single_build_cycle(&state, &mut executor, root, &[]);
    assert!(
        second.build_ok,
        "{}",
        state
            .build_state
            .lock()
            .expect("build state")
            .last_build_messages_summary
    );
    let failed_entry_page = assert_served_entry_outcome(&state, second.version, Some(301));
    assert_ne!(first_page, failed_entry_page);
    assert!(post_entry_error(&state, second.version).starts_with("HTTP/1.1 204 "));
    assert!(post_entry_error(&state, second.version).starts_with("HTTP/1.1 204 "));
    assert_eq!(
        state
            .runtime_reports
            .lock()
            .expect("report ledger")
            .keys
            .len(),
        1
    );
    assert!(state.build_state.lock().expect("build state").last_build_ok);
    assert!(
        String::from_utf8(get_bytes(&state, "/other/"))
            .expect("other page")
            .contains("other-page")
    );

    fs::write(&helper_path, &successful).expect("restore private helper");
    let restored = run_single_build_cycle(&state, &mut executor, root, &[]);
    assert!(restored.build_ok);
    assert_served_entry_outcome(&state, restored.version, None);
    assert_eq!(
        executor
            .last_write
            .as_ref()
            .expect("restore summary")
            .outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::Written),
    );
    assert!(post_entry_error(&state, second.version).starts_with("HTTP/1.1 409 "));

    let unchanged = run_single_build_cycle(&state, &mut executor, root, &[]);
    assert!(unchanged.build_ok);
    assert_served_entry_outcome(&state, unchanged.version, None);
    let summary = executor.last_write.as_ref().expect("unchanged summary");
    assert_eq!(
        summary.outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::SkippedUnchanged),
    );
    assert!(
        summary
            .destinations()
            .iter()
            .all(|destination| { destination.outcome == OutputWriteOutcome::SkippedUnchanged })
    );
    let successful_bytes = fs::read(root.join("dev/index.html")).expect("latest successful page");
    let supporting_asset = get_bytes(&state, "/site.css");
    assert_eq!(supporting_asset, b"body { color: green; }");

    // A previously legal success-only export gains an escaping implicit failure.
    // No stale successful output may stand in for this diagnosed attempt.
    fs::write(&helper_path, &invalid_export).expect("invalidate success-only export");
    executor.last_write = None;
    let rejected = run_single_build_cycle(&state, &mut executor, root, &[]);
    assert!(!rejected.build_ok);
    assert_eq!(rejected.version, unchanged.version + 1);
    assert!(
        executor.last_write.is_none(),
        "diagnosed compilation must never write"
    );
    assert_eq!(
        fs::read(root.join("dev/index.html")).expect("old artifact remains"),
        successful_bytes,
    );
    let error_page = get_page(&state);
    assert!(!error_page.contains("static-survives"));
    assert!(error_page.contains("success_only"));
    assert!(
        state
            .build_state
            .lock()
            .expect("build state")
            .last_error_html
            .is_some()
    );
    assert_eq!(get_bytes(&state, "/site.css"), supporting_asset);
    assert!(post_entry_error(&state, rejected.version).starts_with("HTTP/1.1 409 "));
}
