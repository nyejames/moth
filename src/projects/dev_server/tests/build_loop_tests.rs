//! Tests for build-loop state transitions and queued rebuild behavior.

use super::{
    DevBuildExecutor, DevBuildRequest, DevBuildWorker, ProjectBuildExecutor,
    run_builds_until_stable, run_single_build_cycle,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildResult, BuilderSelection, FileKind, OutputFile, Project};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{
    BuilderIdentity, CleanupPolicy, OutputOwner, OutputPlan, OutputWriteSummary,
    SingleFileOutputPlan, ValidatedOutputPlan, WriteMode, WriteOptions, write_project_outputs,
};
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigValueLocation,
    BuildInputName, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::{
    BoundaryAccumulator, SuccessfulWarningBoundary, SuccessfulWarningPosition,
};
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDraft, DiagnosticSeverity, NamingConvention,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::dev_server::dev_client::tests::run_client_test;
use crate::projects::dev_server::sse::tests::{close_sse, connect_sse, read_generations};
use crate::projects::dev_server::state::{DevServerState, OutputCapturePoint};
use crate::projects::dev_server::watch;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn naming_convention_warning(name: StringId) -> DiagnosticDraft {
    names_declarations::identifier_naming_convention(name, NamingConvention::CamelCase, None)
}
fn infrastructure_failure(error: InfrastructureFailure) -> InfrastructureErrors {
    error.into()
}

fn dev_server_failure(path: &Path, message: &str) -> InfrastructureErrors {
    InfrastructureFailure::development_server(path, message).into()
}

fn test_build_output_owner() -> OutputOwner {
    OutputOwner {
        builder: BuilderIdentity::html(),
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        source_database: None,
        warning_accumulator: BoundaryAccumulator::with_root_string_table(StringTable::new()),
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("project")),
        source_database: None,
        warning_accumulator: BoundaryAccumulator::with_root_string_table(StringTable::new()),
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}

fn html_build_result_with_warning() -> BuildResult {
    let mut string_table = StringTable::new();
    let warning = naming_convention_warning(string_table.get_or_intern("dev_warning".to_string()));
    let mut warning_accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    let (strings, mut sink) = warning_accumulator.compiler_parts_mut();
    let batch = PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from_diagnostics(vec![warning]),
        strings,
        None,
        || None,
    )
    .expect("warning should capture against the build string table");
    sink.append(
        SuccessfulWarningBoundary::Project,
        SuccessfulWarningPosition::BaseModule(0),
        batch,
    );

    BuildResult {
        project: Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html><body>Hello</body></html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::html(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(PathBuf::from("main.moth")),
        source_database: None,
        warning_accumulator,
        output_owner: test_build_output_owner(),
        directory_output_plan: None,
    }
}
fn html_build_result_with_warning_capacity_failure() -> BuildResult {
    let mut build_result = html_build_result();
    {
        let (strings, mut sink) = build_result.warning_accumulator.compiler_parts_mut();
        let warnings = ["dev_warning_first", "dev_warning_second"]
            .into_iter()
            .map(|name| naming_convention_warning(strings.get_or_intern(name.to_string())))
            .collect();
        let batch = PreparedDiagnosticBatch::capture_from_root_string_table(
            DiagnosticBag::from_diagnostics(warnings),
            strings,
            None,
            || None,
        )
        .expect("warning capacity fixture should capture against the build string table");
        sink.append(
            SuccessfulWarningBoundary::Project,
            SuccessfulWarningPosition::BaseModule(0),
            batch,
        );
    }
    build_result
        .warning_accumulator
        .set_record_limit_for_test(2);
    build_result
}

#[test]
fn warning_capacity_failure_is_not_published_as_dev_success() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let root = _temp.path().to_path_buf();
    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir));
    let build_result = html_build_result_with_warning_capacity_failure();
    let executor = FakeExecutor::with_compilation_results(vec![Ok(
        successful_build_with_warnings(build_result),
    )]);

    let report = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &[]);

    assert!(!report.build_ok);
    assert!(report.success_messages.is_none());
    let build_state = state
        .build_state
        .lock()
        .expect("build state should not be poisoned");
    assert!(!build_state.last_build_ok);
    assert!(build_state.last_error_html.is_some());
    assert!(
        build_state
            .last_build_messages_summary
            .contains("MOTH-SYNTAX-0037")
    );
    assert!(
        !build_state
            .last_build_messages_summary
            .lines()
            .any(|line| line.starts_with("W|"))
    );
}

fn successful_build_with_warnings(build_result: BuildResult) -> CompilationOutcome<BuildResult> {
    CompilationOutcome::Success {
        value: build_result,
        warnings: None,
    }
}

fn html_build_result_with_invalid_page_url_style() -> BuildResult {
    let mut build_result = html_build_result();
    build_result.config.builder_settings.set_string(
        "page_url_style",
        String::from("bad_style"),
        None,
    );
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
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        config: Config::new(project_root.to_path_buf()),
        source_database: None,
        warning_accumulator: BoundaryAccumulator::with_root_string_table(StringTable::new()),
        output_owner: owner.clone(),
        directory_output_plan: Some(ValidatedOutputPlan {
            output_root: project_root.join(output_folder),
            project_root: project_root.to_path_buf(),
            entry_root: project_root.to_path_buf(),
            owner,
            setting_span: None,
        }),
    }
}

type FakeWriteCallback = Box<dyn Fn() -> Result<(), InfrastructureErrors> + Send>;

enum FakeWorkerResponse {
    Build {
        value: Box<BuildResult>,
        warnings: Option<DiagnosticReportSet>,
    },
    Diagnosed(DiagnosticReportSet),
    Infrastructure(InfrastructureErrors),
    CompilerBug,
}

#[derive(Clone)]
struct FakeExecutor {
    responses: Arc<Mutex<Vec<FakeWorkerResponse>>>,
    call_count: Arc<AtomicUsize>,
    worker_ids: Arc<Mutex<Vec<usize>>>,
    on_call: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    on_write: Arc<Mutex<Option<FakeWriteCallback>>>,
    build_config_inputs: BuildConfigInputSet,
}

impl FakeExecutor {
    fn successful_results(
        responses: Vec<Result<BuildResult, InfrastructureErrors>>,
    ) -> Vec<CompilationResult<BuildResult>> {
        responses
            .into_iter()
            .map(|result| {
                result.map(|value| CompilationOutcome::Success {
                    value,
                    warnings: None,
                })
            })
            .collect()
    }

    fn new(responses: Vec<Result<BuildResult, InfrastructureErrors>>) -> Self {
        Self::with_compilation_results(Self::successful_results(responses))
    }

    fn with_compilation_results(responses: Vec<CompilationResult<BuildResult>>) -> Self {
        Self::with_worker_responses(
            responses
                .into_iter()
                .map(|response| match response {
                    Ok(CompilationOutcome::Success { value, warnings }) => {
                        FakeWorkerResponse::Build {
                            value: Box::new(value),
                            warnings,
                        }
                    }
                    Ok(CompilationOutcome::Diagnosed(reports)) => {
                        FakeWorkerResponse::Diagnosed(reports)
                    }
                    Err(errors) => FakeWorkerResponse::Infrastructure(errors),
                })
                .collect(),
        )
    }

    fn with_worker_responses(responses: Vec<FakeWorkerResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            call_count: Arc::new(AtomicUsize::new(0)),
            worker_ids: Arc::new(Mutex::new(Vec::new())),
            on_call: None,
            on_write: Arc::new(Mutex::new(None)),
            build_config_inputs: BuildConfigInputSet::new(),
        }
    }

    fn worker_ids(&self) -> Vec<usize> {
        self.worker_ids
            .lock()
            .expect("fake worker identity lock should not be poisoned")
            .clone()
    }

    fn with_on_call(
        responses: Vec<Result<BuildResult, InfrastructureErrors>>,
        on_call: Box<dyn Fn(usize) + Send + Sync>,
    ) -> Self {
        Self {
            on_call: Some(Arc::from(on_call)),
            ..Self::new(responses)
        }
    }

    fn set_on_write(&mut self, on_write: FakeWriteCallback) {
        *self
            .on_write
            .lock()
            .expect("fake writer callback lock should not be poisoned") = Some(on_write);
    }
}

impl DevBuildExecutor for FakeExecutor {
    fn build_config_inputs(&self) -> &BuildConfigInputSet {
        &self.build_config_inputs
    }

    fn create_worker(&self, request: DevBuildRequest) -> Box<dyn DevBuildWorker> {
        let call_index = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
        self.worker_ids
            .lock()
            .expect("fake worker identity lock should not be poisoned")
            .push(call_index);
        if let Some(callback) = &self.on_call {
            callback(call_index);
        }

        let response = self
            .responses
            .lock()
            .expect("fake worker responses should not be poisoned")
            .remove(0);
        let on_write = self
            .on_write
            .lock()
            .expect("fake writer callback lock should not be poisoned")
            .take();
        Box::new(FakeWorker {
            request,
            response: Some(response),
            on_write,
        })
    }
}

struct FakeWorker {
    request: DevBuildRequest,
    response: Option<FakeWorkerResponse>,
    on_write: Option<FakeWriteCallback>,
}

impl DevBuildWorker for FakeWorker {
    fn request(&self) -> &DevBuildRequest {
        &self.request
    }

    fn build(&mut self) -> CompilationResult<BuildResult> {
        match self
            .response
            .take()
            .expect("each fake worker builds exactly once")
        {
            FakeWorkerResponse::Build { value, warnings } => Ok(CompilationOutcome::Success {
                value: *value,
                warnings,
            }),
            FakeWorkerResponse::Diagnosed(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
            FakeWorkerResponse::Infrastructure(errors) => Err(errors),
            FakeWorkerResponse::CompilerBug => crate::compiler_bug!(
                CompilerStage::DevelopmentServer,
                "synthetic compiler bug during dev worker rebuild"
            ),
        }
    }

    fn write_outputs(
        &mut self,
        build_result: &mut BuildResult,
    ) -> Result<OutputWriteSummary, PreparedFailure> {
        let project_root = self
            .request
            .entry_file
            .parent()
            .expect("fake requests use absolute entry paths")
            .to_path_buf();
        let output_plan = if let Some(plan) = build_result.directory_output_plan.as_ref() {
            OutputPlan::Directory(plan.clone())
        } else {
            OutputPlan::SingleFile(SingleFileOutputPlan {
                output_root: self.request.output_root.clone(),
                project_root: Some(project_root),
                owner: build_result.output_owner.clone(),
            })
        };
        let summary = {
            let mut diagnostic_strings =
                DiagnosticStringBuilder::new(build_result.warning_accumulator.string_table_mut());
            write_project_outputs(
                &mut build_result.project,
                &WriteOptions {
                    output_plan,
                    write_mode: self.request.write_mode,
                },
                &mut diagnostic_strings,
            )?
        };
        if let Some(callback) = self.on_write.take() {
            callback().map_err(|error| PreparedFailure::Infrastructure(Box::new(error)))?;
        }
        Ok(summary)
    }
}

#[test]
fn successful_build_marks_state_ok_and_uses_declared_entry_page() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let executor = FakeExecutor::new(vec![Ok(multi_page_html_build_result())]);

    let report = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &Vec::new());
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
    let executor = FakeExecutor::new(vec![
        Ok(directory_build_result(&root, "dev")),
        Ok(directory_build_result(&root, "preview")),
    ]);

    let first_report = run_single_build_cycle(&state, &executor, &root, &Vec::new());
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

    let second_report = run_single_build_cycle(&state, &executor, &root, &Vec::new());
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
    let failure = infrastructure_failure(InfrastructureFailure::internal(
        CompilerStage::ProjectBuildAssembly,
        "boom",
    ));
    let executor = FakeExecutor::new(vec![Err(failure)]);

    let report = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &Vec::new());
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
    let executor = FakeExecutor::with_compilation_results(vec![Ok(
        successful_build_with_warnings(warning_build),
    )]);

    let report = run_single_build_cycle(&state, &executor, &root, &Vec::new());
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
        0,
        "failed builds must not expose success warnings"
    );
    assert_eq!(summary.lines().count(), 1);
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
    let executor = FakeExecutor::new(vec![Ok(html_build_result_with_invalid_page_url_style())]);

    let report = run_single_build_cycle(&state, &executor, &root, &Vec::new());
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
fn invalid_site_config_failure_drops_success_warnings() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let mut build_result = html_build_result_with_warning();
    build_result.config.builder_settings.set_string(
        "page_url_style",
        String::from("bad_style"),
        None,
    );
    let executor = FakeExecutor::with_compilation_results(vec![Ok(
        successful_build_with_warnings(build_result),
    )]);
    let state = Arc::new(DevServerState::new(root.join("dev")));

    let entry_file = root.join("main.moth");
    let report = run_single_build_cycle(&state, &executor, &entry_file, &[]);
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
    assert_eq!(rows.len(), 1, "failed compilation carries only its error");
    assert_eq!(
        rows.iter().filter(|row| row.starts_with("E|")).count(),
        1,
        "site-config failure should render exactly one error"
    );
    assert_eq!(
        rows.iter().filter(|row| row.starts_with("W|")).count(),
        0,
        "warnings from a successful build are unavailable after a later failure"
    );

    let error_fields: Vec<_> = rows[0].split('|').collect();
    assert_eq!(error_fields.len(), 5);
    assert_eq!(error_fields[0], "E");
    assert_eq!(error_fields[1], "MOTH-CONFIG-0001");
    assert_eq!(
        error_fields[4],
        "Invalid value 'bad_style' for config setting 'page_url_style'. Expected 'trailing_slash', 'no_trailing_slash', or 'ignore'."
    );

    let html = build_state
        .last_error_html
        .as_ref()
        .expect("site-config failure should publish an error page");
    assert_eq!(html.matches("<article class=\"diagnostic\"").count(), 1);
    assert_eq!(
        html.matches("data-diagnostic-code=\"MOTH-CONFIG-0001\"")
            .count(),
        1
    );
    assert!(!html.contains("data-diagnostic-code=\"MOTH-RULE-0021\""));
}

#[test]
fn build_version_increments_on_each_attempt() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let executor = FakeExecutor::new(vec![Ok(html_build_result()), Ok(html_build_result())]);

    let first = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &Vec::new());
    let second = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &Vec::new());

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
    let executor = FakeExecutor::with_on_call(
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
        &executor,
        &root.join("main.moth"),
        &Vec::new(),
        &watch_session,
    );

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
    let executor = FakeExecutor::with_on_call(
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
        &executor,
        &root.join("main.moth"),
        &Vec::new(),
        &watch_session,
    );

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

    let build_result = html_build_result_with_warning();
    let executor = FakeExecutor::with_compilation_results(vec![Ok(
        successful_build_with_warnings(build_result),
    )]);

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let outcome = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &[]);

    assert!(outcome.build_ok);
    let messages = outcome
        .success_messages
        .expect("successful build with warnings should carry structured warnings");
    assert_eq!(messages.counts().warnings, 1);
    assert_eq!(messages.counts().errors, 0);
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

    let executor = FakeExecutor::new(vec![Ok(html_build_result())]);

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let outcome = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &[]);

    assert!(outcome.build_ok);
    assert!(
        outcome.success_messages.is_none(),
        "clean successful builds should not allocate an empty warning container"
    );
}

#[test]
fn dev_cycle_warning_context_is_released_with_its_cycle_report() {
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
    fs::write(
        source_root.join("@page.moth"),
        "value ~= \"hello\"\nresult ~= \"unset\"\n\nif value is:\n    \"one\" => result = \"one\"\n    \"one\" => result = \"one\"\n    else => result = \"other\"\n;\n",
    )
    .expect("should write page source with a frontend warning");

    let state = Arc::new(DevServerState::new(root.join("dev")));
    let executor = ProjectBuildExecutor::new(
        || BuilderSelection::new(HtmlProjectBuilder::new()),
        BuildConfigInputSet::new(),
    );
    let report = run_single_build_cycle(&state, &executor, &root, &Vec::new());
    assert!(report.build_ok);

    let context = {
        let messages = report
            .success_messages
            .as_ref()
            .expect("a successful build with a warning should carry its warning report");
        let (warning_report, warning) = messages
            .iter_severity(DiagnosticSeverity::Warning)
            .next()
            .expect("the warning report should hold the frontend warning");
        assert_eq!(warning.descriptor().code, "MOTH-RULE-0022");
        let position = DiagnosticRenderContext::for_report(warning_report)
            .site_position(warning.primary_site())
            .expect("the warning should resolve through its report context");
        assert!(position.path.ends_with("@page.moth"));
        assert!(position.line.contains("\"one\" => result = \"one\""));
        Arc::downgrade(&warning_report.shared_context())
    };
    assert!(
        context.upgrade().is_some(),
        "a held cycle report keeps its warning identity context alive"
    );

    drop(report);
    assert!(
        context.upgrade().is_none(),
        "the live dev state and executor must not retain the cycle's identity context"
    );
    assert!(
        state
            .build_state
            .lock()
            .expect("build state")
            .last_build_messages_summary
            .contains("Unreachable match arm"),
        "the dev state keeps only the rendered warning summary"
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

    let build_result = html_build_result_with_warning();
    let executor = FakeExecutor::with_compilation_results(vec![Ok(
        successful_build_with_warnings(build_result),
    )]);

    let report = run_builds_until_stable(
        &state,
        &executor,
        &root.join("main.moth"),
        &Vec::new(),
        &watch_session,
    );

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
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let current_directory = std::env::current_dir().expect("resolve absolute test path");
    let entry_file = current_directory.join(OsString::from_vec(b"entry-\xff.moth".to_vec()));
    let executor = ProjectBuildExecutor::new(
        || BuilderSelection::new(HtmlProjectBuilder::new()),
        BuildConfigInputSet::new(),
    );
    let request = DevBuildRequest {
        entry_file: entry_file.clone(),
        flags: Vec::new(),
        build_config_inputs: executor.build_config_inputs().clone(),
        output_root: current_directory.join("dev"),
        write_mode: WriteMode::SkipUnchanged,
    };
    let mut worker = executor.create_worker(request);
    let failure = match worker.build() {
        Ok(_) => panic!("an unrepresentable entry path must not reach compilation"),
        Err(failure) => failure,
    };
    let error = &failure.primary;
    assert_eq!(error.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(error.host_path.as_deref(), Some(entry_file.as_path()));
}
#[test]
fn project_build_executor_separates_warnings_from_output_write_failures() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_file = root.join("main.moth");
    fs::write(
        &entry_file,
        "value ~= \"hello\"\nresult ~= \"unset\"\n\nif value is:\n    \"one\" => result = \"one\"\n    \"one\" => result = \"one\"\n    else => result = \"other\"\n;\n",
    )
    .expect("should write source with a frontend warning");

    #[cfg(feature = "timers")]
    let timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let executor = ProjectBuildExecutor::new(
        || BuilderSelection::new(HtmlProjectBuilder::new()),
        BuildConfigInputSet::new(),
    );
    let request = DevBuildRequest {
        entry_file: entry_file.clone(),
        flags: Vec::new(),
        build_config_inputs: executor.build_config_inputs().clone(),
        output_root: root.join("dev"),
        write_mode: WriteMode::SkipUnchanged,
    };
    let mut worker = executor.create_worker(request);
    let mut compiled = match worker.build() {
        Ok(CompilationOutcome::Success {
            value,
            warnings: None,
        }) => value,
        Ok(CompilationOutcome::Success {
            warnings: Some(warnings),
            ..
        }) => {
            panic!("build_project should leave warning freezing to its output owner: {warnings:?}")
        }
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            panic!(
                "warning fixture unexpectedly diagnosed: {:?}",
                reports.counts()
            )
        }
        Err(errors) => panic!("warning fixture failed: {errors:?}"),
    };
    assert_eq!(compiled.warning_accumulator.diagnostic_count(), 1);
    compiled.project.output_files[0] = OutputFile::new(
        PathBuf::from("../escape.js"),
        FileKind::Js(String::from("console.log('broken');")),
    );
    let failure = match worker.write_outputs(&mut compiled) {
        Err(failure) => match compiled.finish_output(Err(failure)) {
            Err(errors) => errors,
            Ok(CompilationOutcome::Diagnosed(reports)) => {
                panic!(
                    "invalid output path unexpectedly produced diagnostics: {:?}",
                    reports.counts()
                )
            }
            Ok(CompilationOutcome::Success { .. }) => {
                panic!("invalid output path unexpectedly finalized successfully")
            }
        },
        Ok(_) => panic!("invalid output path should fail writing"),
    };

    #[cfg(feature = "timers")]
    {
        let timing_snapshot = timing_session.finish();
        assert_eq!(
            timing_snapshot
                .timings
                .iter()
                .find(|observation| observation.metric.descriptor().stable_name
                    == "build.output.total")
                .expect("failed dev output writes retain a dense output-total row")
                .samples,
            1,
            "the failed output-plan/write span must finish"
        );
    }

    assert_eq!(
        failure.primary.host_path.as_deref(),
        Some(Path::new("../escape.js")),
        "output path failures should retain the rejected host path"
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
    let executor = ProjectBuildExecutor::new(
        || BuilderSelection::new(HtmlProjectBuilder::new()),
        build_config_inputs,
    );
    let state = Arc::new(DevServerState::new(root.join("dev")));
    #[cfg(feature = "timers")]
    let timing_session =
        crate::timing::start_raw_benchmark_collection(true).expect("timing session should start");
    let first = run_single_build_cycle(&state, &executor, &root, &[]);
    assert!(first.build_ok);
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
    let second = run_single_build_cycle(&state, &executor, &root, &[]);
    assert!(second.build_ok);
    #[cfg(feature = "timers")]
    let _second_timing_snapshot = second_timing_session.finish();
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

    let published_output_dir = state
        .build_state
        .lock()
        .expect("build state")
        .output_dir
        .clone();
    assert_eq!(
        published_output_dir,
        root.join("preview")
            .canonicalize()
            .expect("published preview directory should canonicalize")
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
    let executor = ProjectBuildExecutor::new(
        || BuilderSelection::new(HtmlProjectBuilder::new()),
        BuildConfigInputSet::new(),
    );

    let first = run_single_build_cycle(&state, &executor, &root, &Vec::new());
    let second = run_single_build_cycle(&state, &executor, &root, &Vec::new());

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
    let failure = dev_server_failure(&root.join("main.moth"), "synthetic failure");
    let executor = FakeExecutor::new(vec![Err(failure)]);

    let report = run_single_build_cycle(&state, &executor, &root.join("main.moth"), &Vec::new());

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
fn output_reader_capture_keeps_generation_bytes_stable_across_rebuild() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    assert!(run_single_build_cycle(&state, &first, &entry, &[]).build_ok);

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

    assert!(
        matches!(
            state.build_state.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ),
        "the response must capture published metadata and bytes under the state lock"
    );

    let (compiled_sender, compiled_receiver) = std::sync::mpsc::channel();
    let compile_state = Arc::clone(&state);
    let next = FakeExecutor::with_on_call(
        vec![Ok(generation_build_result(2))],
        Box::new(move |_| {
            let build_state = compile_state
                .build_state
                .try_lock()
                .expect("worker construction must happen outside the state lock");
            drop(build_state);
            compiled_sender
                .send(())
                .expect("signal worker construction");
        }),
    );
    let rebuild_state = Arc::clone(&state);
    let rebuild =
        std::thread::spawn(move || run_single_build_cycle(&rebuild_state, &next, &entry, &[]));
    release_sender.send(()).expect("release reader capture");
    let old_page = reader.join().expect("reader should finish");
    compiled_receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("rebuild starts after captured response bytes are released");
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
fn output_writing_does_not_lock_state_and_serves_previous_snapshot_until_publication() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let output_dir = temp.path().join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    assert!(run_single_build_cycle(&state, &first, &entry, &[]).build_ok);

    let (written_sender, written_receiver) = std::sync::mpsc::channel();
    let (publish_sender, publish_receiver) = std::sync::mpsc::channel();
    let mut next = FakeExecutor::new(vec![Ok(generation_build_result(2))]);
    next.set_on_write(Box::new(move || {
        written_sender.send(()).expect("signal written outputs");
        publish_receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("release artifact publication");
        Ok(())
    }));
    let rebuild_state = Arc::clone(&state);
    let rebuild =
        std::thread::spawn(move || run_single_build_cycle(&rebuild_state, &next, &entry, &[]));
    written_receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("new outputs reach write barrier");
    assert!(
        fs::read_to_string(output_dir.join("index.html"))
            .expect("written page")
            .contains("generation-2")
    );
    assert!(
        state.build_state.try_lock().is_ok(),
        "output writing must not hold the publication lock"
    );
    let page_during_write = get_page(&state);
    assert!(page_during_write.contains("generation-1"));
    assert!(page_during_write.contains("const build = \"1\";"));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-1");

    publish_sender
        .send(())
        .expect("publish completed generation");
    assert_eq!(rebuild.join().expect("rebuild finishes").version, 2);
    let published_page = get_page(&state);
    assert!(published_page.contains("generation-2"));
    assert!(published_page.contains("const build = \"2\";"));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-2");
}

#[test]
fn companion_requests_cross_publication_but_reports_keep_page_generation() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create publication project");
    let entry = temp.path().join("main.moth");
    let state = Arc::new(DevServerState::new(temp.path().join("dev")));
    let executor = FakeExecutor::new(vec![
        Ok(generation_build_result(1)),
        Ok(generation_build_result(2)),
    ]);
    assert!(run_single_build_cycle(&state, &executor, &entry, &[]).build_ok);
    let old_page = get_page(&state);
    assert!(old_page.contains("generation-1"));
    assert!(old_page.contains("const build = \"1\";"));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-1");
    assert!(run_single_build_cycle(&state, &executor, &entry, &[]).build_ok);

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
    let executor = FakeExecutor::new(vec![
        Ok(generation_build_result(1)),
        Ok(generation_build_result(2)),
    ]);
    assert!(run_single_build_cycle(&state, &executor, &entry, &[]).build_ok);
    let stale_page = get_page(&state);
    assert!(stale_page.contains("generation-1"));

    let missed = run_single_build_cycle(&state, &executor, &entry, &[]);
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
    let first = FakeExecutor::new(vec![Ok(generation_build_result(1))]);
    assert!(run_single_build_cycle(&state, &first, &entry, &[]).build_ok);
    let mut next = FakeExecutor::new(vec![Ok(generation_build_result(2))]);
    let partial_output = output.clone();
    next.set_on_write(Box::new(move || {
        // A late emission/finalisation failure can leave bytes changed without completing a batch.
        fs::write(
            partial_output.join("index.html"),
            "<html>partial-generation-2",
        )
        .expect("leave partially written page");
        Err(dev_server_failure(
            &partial_output,
            "publication write failed",
        ))
    }));
    let failed = run_single_build_cycle(&state, &next, &entry, &[]);
    assert!(!failed.build_ok);
    assert_eq!(failed.version, 2);
    let page = get_page(&state);
    assert!(page.contains("publication write failed"));
    assert!(!page.contains("partial-generation-2"));
    assert!(!page.contains("generation-1"));
    assert_eq!(get_bytes(&state, "/page.js"), b"generation-1");
    assert_eq!(
        fs::read(output.join("page.js")).expect("writer emitted partial second generation"),
        b"generation-2"
    );
    assert!(post_entry_error(&state, 2).starts_with("HTTP/1.1 409 "));
    let build = state.build_state.lock().expect("build state");
    assert!(!build.last_build_ok);
    assert_eq!(build.served_artifacts_version, 1);
    assert!(build.last_error_html.is_some());
}

#[test]
fn exact_html_asset_uses_failed_generation_page_after_rebuild_failure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create exact HTML publication project");
    let entry = temp.path().join("main.moth");
    let output = temp.path().join("dev");
    let state = Arc::new(DevServerState::new(output));

    let mut first_build = html_build_result();
    first_build.project.output_files = vec![OutputFile::new(
        PathBuf::from("main.html"),
        FileKind::Html(String::from(
            "<html><body>generation-1 exact HTML</body></html>",
        )),
    )];
    first_build.project.entry_page_rel = Some(PathBuf::from("main.html"));
    let executor = FakeExecutor::with_worker_responses(vec![
        FakeWorkerResponse::Build {
            value: Box::new(first_build),
            warnings: None,
        },
        FakeWorkerResponse::Infrastructure(dev_server_failure(&entry, "rebuild failed")),
    ]);

    assert!(run_single_build_cycle(&state, &executor, &entry, &[]).build_ok);
    let failed = run_single_build_cycle(&state, &executor, &entry, &[]);
    assert!(!failed.build_ok);
    assert_eq!(failed.version, 2);

    let body = String::from_utf8(get_bytes(&state, "/main.html")).expect("HTML response is UTF-8");
    assert!(body.contains("Build Version: 2"), "failure page: {body}");
    assert!(body.contains("rebuild failed"), "failure page: {body}");
    assert!(
        !body.contains("generation-1 exact HTML"),
        "stale page: {body}"
    );
    assert!(
        !body.contains("Build #1 succeeded"),
        "stale dev-client version: {body}"
    );

    let build = state.build_state.lock().expect("build state");
    assert_eq!(build.last_build_version, 2);
    assert_eq!(build.served_artifacts_version, 1);
}

#[test]
fn compiler_bug_rebuild_keeps_last_good_artifacts_and_runs_the_queued_rebuild() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("create compiler-bug rebuild project");
    let root = temp.path();
    let entry = root.join("main.moth");
    fs::write(&entry, "watch this").expect("write watched entry");
    let output_dir = root.join("dev");
    let state = Arc::new(DevServerState::new(output_dir.clone()));
    let (watch_session, watch_trigger) =
        watch::WatchSession::manual(watch_scope(root, &output_dir));
    let watch_trigger = Arc::new(watch_trigger);
    let success = |generation| FakeWorkerResponse::Build {
        value: Box::new(generation_build_result(generation)),
        warnings: None,
    };
    let mut executor = FakeExecutor::with_worker_responses(vec![
        success(1),
        FakeWorkerResponse::CompilerBug,
        success(3),
    ]);
    let observed_failure = Arc::new(Mutex::new(None));
    let callback_state = Arc::clone(&state);
    let callback_observation = Arc::clone(&observed_failure);
    let callback_trigger = Arc::clone(&watch_trigger);
    executor.on_call = Some(Arc::new(move |worker_id| match worker_id {
        2 => callback_trigger.notify_change(),
        3 => {
            let (build_ok, version, served_version, summary) = {
                let build = callback_state.build_state.lock().expect("build state");
                (
                    build.last_build_ok,
                    build.last_build_version,
                    build.served_artifacts_version,
                    build.last_build_messages_summary.clone(),
                )
            };
            *callback_observation
                .lock()
                .expect("compiler-bug observation lock") = Some((
                build_ok,
                version,
                served_version,
                summary,
                get_page(&callback_state),
                get_bytes(&callback_state, "/page.js"),
            ));
        }
        _ => {}
    }));

    let initial = run_single_build_cycle(&state, &executor, &entry, &[]);
    assert!(initial.build_ok);
    assert_eq!(executor.worker_ids(), vec![1]);
    watch_trigger.notify_change();

    let rebuilt = run_builds_until_stable(&state, &executor, &entry, &[], &watch_session);
    assert!(rebuilt.watch_scope.is_some());
    assert_eq!(executor.worker_ids(), vec![1, 2, 3]);
    let (build_ok, version, served_version, summary, page, js) = observed_failure
        .lock()
        .expect("compiler-bug observation lock")
        .take()
        .expect("the queued rebuild starts a fresh worker after the compiler bug");
    assert!(!build_ok);
    assert_eq!(version, 2);
    assert_eq!(served_version, 1);
    assert!(
        summary.to_ascii_lowercase().contains("compiler bug"),
        "host state should summarize the compiler bug"
    );
    assert!(page.contains("Compiler Bug"));
    assert_eq!(js, b"generation-1");

    let build = state.build_state.lock().expect("build state");
    assert!(build.last_build_ok);
    assert_eq!(build.last_build_version, 3);
    drop(build);
    assert!(get_page(&state).contains("generation-3"));
}

/// Records the existing writer's explicit branch, not timestamps or a parallel output writer.
#[derive(Clone)]
struct RecordingProjectExecutor {
    inner: ProjectBuildExecutor,
    last_write: Arc<Mutex<Option<OutputWriteSummary>>>,
}

impl RecordingProjectExecutor {
    fn last_write(&self) -> Option<OutputWriteSummary> {
        self.last_write
            .lock()
            .expect("recorded output summary lock should not be poisoned")
            .clone()
    }

    fn clear_last_write(&self) {
        *self
            .last_write
            .lock()
            .expect("recorded output summary lock should not be poisoned") = None;
    }
}

impl DevBuildExecutor for RecordingProjectExecutor {
    fn build_config_inputs(&self) -> &BuildConfigInputSet {
        self.inner.build_config_inputs()
    }

    fn create_worker(&self, request: DevBuildRequest) -> Box<dyn DevBuildWorker> {
        Box::new(RecordingProjectWorker {
            inner: self.inner.create_worker(request),
            last_write: Arc::clone(&self.last_write),
        })
    }
}

struct RecordingProjectWorker {
    inner: Box<dyn DevBuildWorker>,
    last_write: Arc<Mutex<Option<OutputWriteSummary>>>,
}

impl DevBuildWorker for RecordingProjectWorker {
    fn request(&self) -> &DevBuildRequest {
        self.inner.request()
    }

    fn build(&mut self) -> CompilationResult<BuildResult> {
        let result = self.inner.build();
        if let Ok(CompilationOutcome::Diagnosed(messages)) = &result {
            let mut diagnostics = messages.iter_severity(DiagnosticSeverity::Error);
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
    ) -> Result<OutputWriteSummary, PreparedFailure> {
        let summary = self.inner.write_outputs(build_result)?;
        *self
            .last_write
            .lock()
            .expect("recorded output summary lock should not be poisoned") = Some(summary.clone());
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
    let executor = RecordingProjectExecutor {
        inner: ProjectBuildExecutor::new(
            || BuilderSelection::new(HtmlProjectBuilder::new()),
            BuildConfigInputSet::new(),
        ),
        last_write: Arc::new(Mutex::new(None)),
    };

    let first = run_single_build_cycle(&state, &executor, root, &[]);
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
            .last_write()
            .expect("first write summary")
            .outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::Written),
    );

    // Only the private body changes. The public Error! signature and entry source stay identical.
    fs::write(&helper_path, &failing_entry).expect("widen private helper");
    let second = run_single_build_cycle(&state, &executor, root, &[]);
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
    let restored = run_single_build_cycle(&state, &executor, root, &[]);
    assert!(restored.build_ok);
    assert_served_entry_outcome(&state, restored.version, None);
    assert_eq!(
        executor
            .last_write()
            .expect("restore summary")
            .outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::Written),
    );
    assert!(post_entry_error(&state, second.version).starts_with("HTTP/1.1 409 "));

    let unchanged = run_single_build_cycle(&state, &executor, root, &[]);
    assert!(unchanged.build_ok);
    assert_served_entry_outcome(&state, unchanged.version, None);
    let summary = executor.last_write().expect("unchanged summary");
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

    // Turning the success-only export into an escaping implicit failure rejects the build.
    // No stale successful output may stand in for this diagnosed attempt.
    fs::write(&helper_path, &invalid_export).expect("invalidate success-only export");
    executor.clear_last_write();
    let rejected = run_single_build_cycle(&state, &executor, root, &[]);
    assert!(!rejected.build_ok);
    assert_eq!(rejected.version, unchanged.version + 1);
    assert!(
        executor.last_write().is_none(),
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
