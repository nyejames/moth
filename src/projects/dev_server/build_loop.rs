//! Build execution and watch-triggered rebuild coordination for the dev server.
//!
//! This module delegates compilation and artifact writing to the core build APIs, then translates
//! build outcomes into dev-server state updates and SSE reload broadcasts.

use crate::build_system::build::{self, BuildResult, BuilderSelection};
use crate::build_system::output::{
    OutputPlan, OutputWriteSummary, SingleFileOutputPlan, WriteMode, WriteOptions,
    write_project_outputs,
};
use crate::command_timing_scope;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::display_messages::print_diagnostic_reports;
use crate::projects::dev_server::error_page::{
    format_compiler_messages, render_compiler_bug_page, render_compiler_error_page,
};
use crate::projects::dev_server::sse;
use crate::projects::dev_server::state::{DevServerState, ServedArtifactSnapshot};
use crate::projects::dev_server::watch;
use crate::projects::routing::{HtmlSiteConfig, parse_html_site_config};
#[cfg(feature = "detailed_timers")]
use crate::timing_scope;
use saying::say;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct BuildCycleReport {
    pub version: u64,
    pub build_ok: bool,
    pub build_duration: Duration,
    pub clients_notified: usize,
    pub watch_scope: Option<watch::WatchScope>,
    pub success_messages: Option<DiagnosticReportSet>,
    pub output_warnings: Vec<String>,
    /// Drained timing observations for this cycle, rendered by the caller
    /// after its own status line.
    #[cfg(feature = "timers")]
    pub timing_snapshot: Option<crate::timing::BenchmarkObservationSnapshot>,
}

#[derive(Debug, Clone)]
pub struct RebuildRunReport {
    pub watch_scope: Option<watch::WatchScope>,
}

/// The host hands each cycle a fully owned request; the factory uses it to create one fresh worker.
pub(crate) struct DevBuildRequest {
    entry_file: PathBuf,
    flags: Vec<Flag>,
    build_config_inputs: BuildConfigInputSet,
    output_root: PathBuf,
    write_mode: WriteMode,
}

pub(crate) trait DevBuildExecutor: Clone + Send + Sync + 'static {
    fn build_config_inputs(&self) -> &BuildConfigInputSet;
    fn create_worker(&self, request: DevBuildRequest) -> Box<dyn DevBuildWorker>;
}

pub(crate) trait DevBuildWorker {
    fn request(&self) -> &DevBuildRequest;
    fn build(&mut self) -> CompilationResult<BuildResult>;
    fn write_outputs(
        &mut self,
        build_result: &mut BuildResult,
    ) -> Result<OutputWriteSummary, PreparedFailure>;
}

/// Immutable builder factory and common build inputs shared across dev cycles.
pub(crate) struct ProjectBuildExecutor {
    builder_factory: Arc<dyn Fn() -> BuilderSelection + Send + Sync>,
    build_config_inputs: Arc<BuildConfigInputSet>,
}

impl Clone for ProjectBuildExecutor {
    fn clone(&self) -> Self {
        Self {
            builder_factory: Arc::clone(&self.builder_factory),
            build_config_inputs: Arc::clone(&self.build_config_inputs),
        }
    }
}

impl ProjectBuildExecutor {
    pub(crate) fn new(
        builder_factory: impl Fn() -> BuilderSelection + Send + Sync + 'static,
        build_config_inputs: BuildConfigInputSet,
    ) -> Self {
        Self {
            builder_factory: Arc::new(builder_factory),
            build_config_inputs: Arc::new(build_config_inputs),
        }
    }
}

impl DevBuildExecutor for ProjectBuildExecutor {
    fn build_config_inputs(&self) -> &BuildConfigInputSet {
        &self.build_config_inputs
    }

    fn create_worker(&self, request: DevBuildRequest) -> Box<dyn DevBuildWorker> {
        Box::new(ProjectBuildWorker {
            builder: (self.builder_factory)(),
            request,
        })
    }
}

struct ProjectBuildWorker {
    builder: BuilderSelection,
    request: DevBuildRequest,
}

impl DevBuildWorker for ProjectBuildWorker {
    fn request(&self) -> &DevBuildRequest {
        &self.request
    }

    fn build(&mut self) -> CompilationResult<BuildResult> {
        let entry_path = self.request.entry_file.to_str().ok_or_else(|| {
            InfrastructureErrors::from(InfrastructureFailure::filesystem(
                CompilerStage::DevelopmentServer,
                &self.request.entry_file,
                "Dev server entry path contains invalid UTF-8 and cannot be compiled.",
            ))
        })?;

        build::build_project(
            self.builder.backend.as_ref(),
            entry_path,
            &self.request.flags,
            &self.request.build_config_inputs,
        )
        .unwrap_or_else(|never| match never {})
    }

    fn write_outputs(
        &mut self,
        build_result: &mut BuildResult,
    ) -> Result<OutputWriteSummary, PreparedFailure> {
        crate::timed_stage!(crate::timing::TimingMetric::BuildOutputTotal, {
            let output_plan = if let Some(plan) = build_result.directory_output_plan.as_ref() {
                OutputPlan::Directory(plan.clone())
            } else {
                let project_root = self
                    .request
                    .entry_file
                    .parent()
                    .expect("absolute dev entry path must have a parent")
                    .to_path_buf();
                OutputPlan::SingleFile(SingleFileOutputPlan {
                    output_root: self.request.output_root.clone(),
                    project_root: Some(project_root),
                    owner: build_result.output_owner.clone(),
                })
            };
            let mut diagnostic_strings =
                DiagnosticStringBuilder::new(build_result.warning_accumulator.string_table_mut());
            write_project_outputs(
                &mut build_result.project,
                &WriteOptions {
                    output_plan,
                    write_mode: self.request.write_mode,
                },
                &mut diagnostic_strings,
            )
        })
    }
}

enum BuildOutcomeKind {
    Success {
        entry_page_rel: PathBuf,
        html_site_config: HtmlSiteConfig,
        success_messages: Option<DiagnosticReportSet>,
        output_warnings: Vec<String>,
    },
    Failure(CompilationResult<()>),
    CompilerBug(Option<String>),
}

struct BuildOutcome {
    kind: BuildOutcomeKind,
    watch_scope: Option<watch::WatchScope>,
    output_dir: Option<PathBuf>,
}

fn finish_build_failure(
    build_result: BuildResult,
    failure: PreparedFailure,
) -> CompilationResult<()> {
    match build_result.finish_output(Err(failure))? {
        CompilationOutcome::Diagnosed(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
        CompilationOutcome::Success { value, .. } => {
            drop(value);
            crate::compiler_bug!(
                CompilerStage::DevelopmentServer,
                "a failed dev-server output step unexpectedly finalized a successful build"
            )
        }
    }
}

fn run_dev_build_worker(mut worker: Box<dyn DevBuildWorker>) -> BuildOutcome {
    let compiled = worker.build();
    let outcome = build_once(worker.as_mut(), compiled);
    drop(worker);
    outcome
}

fn build_once(
    worker: &mut dyn DevBuildWorker,
    compiled: CompilationResult<BuildResult>,
) -> BuildOutcome {
    let fallback_output_root = worker.request().output_root.clone();
    let build_result = match compiled {
        Err(errors) => {
            return failed_build_outcome(Err(errors), None, None);
        }
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            return failed_build_outcome(Ok(CompilationOutcome::Diagnosed(reports)), None, None);
        }
        Ok(CompilationOutcome::Success { value, .. }) => value,
    };

    let mut build_result = build_result;
    let output_summary = match worker.write_outputs(&mut build_result) {
        Ok(summary) => summary,
        Err(failure) => {
            return failed_build_outcome(finish_build_failure(build_result, failure), None, None);
        }
    };
    let output_warnings = output_summary.into_warning_messages();
    let output_dir = build_result
        .directory_output_plan
        .as_ref()
        .map(|plan| canonical_output_dir(&plan.output_root))
        .unwrap_or_else(|| canonical_output_dir(&fallback_output_root));
    let watch_scope = watch::WatchScope::derive(
        &build_result.config.entry_dir,
        Some(&build_result.config),
        &output_dir,
    );

    let html_site_config_result = {
        let mut diagnostic_strings =
            DiagnosticStringBuilder::new(build_result.warning_accumulator.string_table_mut());
        parse_html_site_config(&build_result.config, &mut diagnostic_strings)
            .map_err(|error| error.into_prepared_failure(diagnostic_strings.strings()))
    };
    let html_site_config = match html_site_config_result {
        Ok(config) => config,
        Err(failure) => {
            return failed_build_outcome(
                finish_build_failure(build_result, failure),
                Some(watch_scope),
                Some(output_dir),
            );
        }
    };

    let Some(entry_page_rel) = build_result.project.entry_page_rel.clone() else {
        let error = InfrastructureFailure::internal(
            CompilerStage::DevelopmentServer,
            "Build completed, but the project builder did not declare a dev entry page.",
        );
        return failed_build_outcome(
            finish_build_failure(build_result, PreparedFailure::Infrastructure(error.into())),
            Some(watch_scope),
            Some(output_dir),
        );
    };

    let warnings = match build_result.finish_output(Ok(())) {
        Ok(CompilationOutcome::Success { value, warnings }) => {
            drop(value);
            warnings
        }
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            return failed_build_outcome(
                Ok(CompilationOutcome::Diagnosed(reports)),
                Some(watch_scope),
                Some(output_dir),
            );
        }
        Err(errors) => {
            return failed_build_outcome(Err(errors), Some(watch_scope), Some(output_dir));
        }
    };

    BuildOutcome {
        kind: BuildOutcomeKind::Success {
            entry_page_rel,
            html_site_config,
            success_messages: warnings,
            output_warnings,
        },
        watch_scope: Some(watch_scope),
        output_dir: Some(output_dir),
    }
}

fn format_warning_titles(reports: &DiagnosticReportSet) -> String {
    reports
        .iter_severity(DiagnosticSeverity::Warning)
        .map(|(_, view)| view.descriptor().title.to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn run_single_build_cycle<E: DevBuildExecutor>(
    state: &Arc<DevServerState>,
    executor: &E,
    entry_file: &Path,
    flags: &[Flag],
) -> BuildCycleReport {
    command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Dev);
    #[cfg(feature = "detailed_timers")]
    timing_scope!(
        timing_guard_command_dev_cycle,
        crate::timing::TimingMetric::CommandDevCycle
    );
    let start = Instant::now();

    let (outcome, report_entry_file) = match resolve_absolute_path(entry_file) {
        Ok(absolute_entry_file) => {
            let report_entry_file = absolute_entry_file.clone();
            let output_root = {
                let build_state = state
                    .build_state
                    .lock()
                    .expect("dev-server host bug: build-state lock poisoned before build request");
                build_state.output_dir.clone()
            };
            let outcome = match resolve_absolute_path(&output_root) {
                Ok(output_root) => {
                    let factory = executor.clone();
                    let request = DevBuildRequest {
                        entry_file: absolute_entry_file,
                        flags: flags.to_vec(),
                        build_config_inputs: executor.build_config_inputs().clone(),
                        output_root,
                        write_mode: WriteMode::SkipUnchanged,
                    };
                    // The wrapper is safe because it captures only owned request/factory values;
                    // the fresh worker and all its mutable build state are dropped on unwind
                    // before host state is published.
                    match catch_unwind(AssertUnwindSafe(move || {
                        let worker = factory.create_worker(request);
                        run_dev_build_worker(worker)
                    })) {
                        Ok(outcome) => outcome,
                        Err(payload) => {
                            let report = crate::render_compiler_bug_payload(payload.as_ref());
                            if let Some(report) = &report {
                                say!(Bold Yellow report);
                            }
                            BuildOutcome {
                                kind: BuildOutcomeKind::CompilerBug(report),
                                watch_scope: None,
                                output_dir: None,
                            }
                        }
                    }
                }
                Err(errors) => failed_build_outcome(Err(errors), None, None),
            };
            (outcome, Some(report_entry_file))
        }
        Err(errors) => (failed_build_outcome(Err(errors), None, None), None),
    };
    let report_entry_file = report_entry_file.as_deref().unwrap_or(entry_file);
    let report = publish_build_outcome(state, outcome, start, report_entry_file);
    #[cfg(feature = "detailed_timers")]
    drop(timing_guard_command_dev_cycle);
    #[cfg(feature = "timers")]
    let report = {
        let mut report = report;
        report.timing_snapshot = Some(timing_session.finish());
        report
    };
    report
}

fn publish_build_outcome(
    state: &Arc<DevServerState>,
    mut outcome: BuildOutcome,
    start: Instant,
    entry_file: &Path,
) -> BuildCycleReport {
    let served_artifacts = if matches!(&outcome.kind, BuildOutcomeKind::Success { .. }) {
        match outcome.output_dir.as_deref() {
            Some(output_dir) => match capture_served_artifacts(output_dir) {
                Ok(artifacts) => Some(artifacts),
                Err(failure) => {
                    outcome.kind = BuildOutcomeKind::Failure(Err(failure.into()));
                    outcome.output_dir = None;
                    None
                }
            },
            None => {
                outcome.kind = BuildOutcomeKind::Failure(Err(InfrastructureFailure::internal(
                    CompilerStage::DevelopmentServer,
                    "Successful dev build did not provide an output root to capture.",
                )
                .into()));
                None
            }
        }
    } else {
        None
    };
    let served_html_entries = served_artifacts.as_ref().map(|artifacts| {
        let output_root = outcome
            .output_dir
            .as_deref()
            .expect("successful artifact snapshot has an output root");
        crate::projects::dev_server::static_files::validated_html_entries(output_root, artifacts)
    });

    let build_duration =
        crate::capture_command_duration!(crate::timing::TimingMetric::CommandDevBuildWrite, start,);
    let project_root = dev_server_project_root(entry_file);
    let (version, origin) = {
        let build_state = state
            .build_state
            .lock()
            .expect("dev-server host bug: build-state lock poisoned");
        (
            build_state.last_build_version.saturating_add(1),
            build_state.html_site_config.origin.clone(),
        )
    };

    let (
        build_ok,
        diagnostics_summary,
        success_messages,
        output_warnings,
        success_metadata,
        error_html,
    ) = match outcome.kind {
        BuildOutcomeKind::Success {
            entry_page_rel,
            html_site_config,
            success_messages,
            output_warnings,
        } => {
            let warnings_summary = success_messages
                .as_ref()
                .map(format_warning_titles)
                .unwrap_or_default();
            let output_warnings_summary = output_warnings.join("\n");
            let diagnostics_summary = match (
                warnings_summary.is_empty(),
                output_warnings_summary.is_empty(),
            ) {
                (true, true) => String::from("Build succeeded."),
                (false, true) => format!("Build succeeded with warnings:\n{warnings_summary}"),
                (true, false) => {
                    format!("Build succeeded with warnings:\n{output_warnings_summary}")
                }
                (false, false) => format!(
                    "Build succeeded with warnings:\n{warnings_summary}\n{output_warnings_summary}"
                ),
            };
            (
                true,
                diagnostics_summary,
                success_messages,
                output_warnings,
                Some((entry_page_rel, html_site_config)),
                None,
            )
        }
        BuildOutcomeKind::Failure(failure) => {
            let diagnostics_summary = format_compiler_messages(&failure);
            let error_html = render_compiler_error_page(&failure, &project_root, &origin, version);
            (
                false,
                diagnostics_summary,
                None,
                Vec::new(),
                None,
                Some(error_html),
            )
        }
        BuildOutcomeKind::CompilerBug(report) => {
            let details = report.map_or_else(
                || {
                    String::from(
                        "Compiler bug in the dev build worker. The worker was discarded; watching remains active.",
                    )
                },
                |report| format!("Compiler bug:\n{report}"),
            );
            let error_html = render_compiler_bug_page(&details, &origin, version);
            (false, details, None, Vec::new(), None, Some(error_html))
        }
    };

    let watch_scope = outcome.watch_scope;
    let output_dir = outcome.output_dir;
    let mut build_state = state
        .build_state
        .lock()
        .expect("dev-server host bug: build-state lock poisoned");
    build_state.last_build_version = version;
    build_state.last_build_ok = build_ok;
    build_state.last_build_messages_summary = diagnostics_summary;
    if build_ok {
        build_state.last_error_html = None;
        if let Some((entry_page_rel, html_site_config)) = success_metadata {
            build_state.entry_page_rel = Some(entry_page_rel);
            build_state.html_site_config = html_site_config;
        }
        if let Some(output_dir) = output_dir {
            build_state.output_dir = output_dir;
        }
        build_state.served_artifacts =
            served_artifacts.expect("successful dev build must have a captured artifact snapshot");
        build_state.served_html_entries = served_html_entries
            .expect("successful dev build must have validated HTML entries for its snapshot");
        build_state.served_artifacts_version = version;
    } else {
        // Failure publication keeps the last successful output root, metadata, and artifact snapshot.
        build_state.last_error_html = error_html;
    }
    drop(build_state);

    let clients_notified = sse::broadcast_generation(state);
    BuildCycleReport {
        version,
        build_ok,
        build_duration,
        clients_notified,
        watch_scope,
        success_messages,
        output_warnings,
        #[cfg(feature = "timers")]
        timing_snapshot: None,
    }
}

/// Maximum consecutive rebuilds before the loop stops to prevent infinite rebuild cycles.
/// If the build itself modifies watched files (e.g. through file-system side effects),
/// the fingerprint check would trigger indefinitely without this limit.
const MAX_CONSECUTIVE_REBUILDS: usize = 5;

pub fn run_builds_until_stable<E: DevBuildExecutor>(
    state: &Arc<DevServerState>,
    executor: &E,
    entry_file: &Path,
    flags: &[Flag],
    watch_session: &watch::WatchSession,
) -> RebuildRunReport {
    let mut build_count = 0usize;
    let latest_watch_scope = loop {
        let build_start_revision = watch_session.current_revision();

        let report = run_single_build_cycle(state, executor, entry_file, flags);

        build_count += 1;
        let report_watch_scope = report.watch_scope.clone();

        if report.build_ok {
            say!(
                "Dev build ",
                Blue "#", report.version,
                Reset " done in ",
                Green report.build_duration.as_millis(), " ms ",
                Reset "- Broadcast to ",
                Blue report.clients_notified,
                Reset " clients."
            );
            if let Some(messages) = report.success_messages {
                print_diagnostic_reports(&messages);
            }
            for warning in report.output_warnings {
                say!(Yellow warning);
            }
        } else {
            say!(
                "Dev build #",
                Yellow report.version,
                Yellow " failed. Reload broadcast to ",
                Yellow report.clients_notified,
                Yellow " clients."
            );
        }
        #[cfg(feature = "timers")]
        if let Some(snapshot) = &report.timing_snapshot {
            crate::timing::render_command_timing_summary(
                snapshot,
                crate::timing::TimingCommandKind::Dev,
                report.build_ok,
            );
        }

        // Queue one immediate follow-up build when the watch revision advances during a build.
        if watch_session.current_revision() == build_start_revision {
            break report_watch_scope;
        }

        if build_count >= MAX_CONSECUTIVE_REBUILDS {
            say!(
                Yellow "Dev server reached ",
                Yellow MAX_CONSECUTIVE_REBUILDS,
                Yellow " consecutive rebuilds without stabilising — pausing rebuild loop. ",
                Yellow "This usually means the build is modifying watched source files."
            );
            break report_watch_scope;
        }
    };

    RebuildRunReport {
        watch_scope: latest_watch_scope,
    }
}

pub fn run_watch_build_loop<E: DevBuildExecutor>(
    state: Arc<DevServerState>,
    executor: E,
    entry_file: PathBuf,
    flags: Vec<Flag>,
    initial_watch_scope: watch::WatchScope,
    poll_interval: Duration,
) {
    let mut watch_session = watch::WatchSession::start(initial_watch_scope, poll_interval);
    let mut last_seen_revision = watch_session.current_revision();

    loop {
        let seen_revision = match watch_session.wait_for_stable_change(last_seen_revision) {
            Ok(seen_revision) => seen_revision,
            Err(error) => {
                say!(
                    Yellow "Dev server watch warning: failed while waiting for file changes: ",
                    Yellow error.to_string()
                );
                return;
            }
        };

        let rebuild_report =
            run_builds_until_stable(&state, &executor, &entry_file, &flags, &watch_session);

        last_seen_revision = watch_session.current_revision().max(seen_revision);

        if let Some(next_watch_scope) = rebuild_report.watch_scope
            && next_watch_scope != *watch_session.scope()
        {
            watch_session = watch::WatchSession::start(next_watch_scope, poll_interval);
            last_seen_revision = watch_session.current_revision();
        }
    }
}

fn failed_build_outcome(
    failure: CompilationResult<()>,
    watch_scope: Option<watch::WatchScope>,
    output_dir: Option<PathBuf>,
) -> BuildOutcome {
    BuildOutcome {
        kind: BuildOutcomeKind::Failure(failure),
        watch_scope,
        output_dir,
    }
}

fn resolve_absolute_path(path: &Path) -> Result<PathBuf, InfrastructureErrors> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current_directory| current_directory.join(path))
        .map_err(|error| {
            InfrastructureFailure::filesystem(
                CompilerStage::DevelopmentServer,
                path,
                format!("Failed to resolve an absolute dev-server path: {error}"),
            )
            .into()
        })
}

fn canonical_output_dir(output_dir: &Path) -> PathBuf {
    output_dir
        .canonicalize()
        .unwrap_or_else(|_| output_dir.to_path_buf())
}

fn dev_server_project_root(entry_file: &Path) -> PathBuf {
    if entry_file.is_dir() {
        return entry_file.to_path_buf();
    }

    match entry_file.parent() {
        Some(parent) => parent.to_path_buf(),
        None => PathBuf::from("."),
    }
}

pub(crate) fn capture_served_artifacts(
    output_root: &Path,
) -> Result<ServedArtifactSnapshot, InfrastructureFailure> {
    let root = output_root
        .canonicalize()
        .map_err(|error| artifact_snapshot_failure(output_root, error.to_string()))?;
    let mut directories = vec![(root.clone(), PathBuf::new(), HashSet::new())];
    let mut artifacts = HashMap::new();

    while let Some((directory, relative_directory, mut ancestors)) = directories.pop() {
        let canonical_directory = directory
            .canonicalize()
            .map_err(|error| artifact_snapshot_failure(&directory, error.to_string()))?;
        if !ancestors.insert(canonical_directory) {
            continue;
        }

        let entries = fs::read_dir(&directory)
            .map_err(|error| artifact_snapshot_failure(&directory, error.to_string()))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| artifact_snapshot_failure(&directory, error.to_string()))?;
            let path = entry.path();
            let relative_path = relative_directory.join(entry.file_name());
            let file_type = entry
                .file_type()
                .map_err(|error| artifact_snapshot_failure(&path, error.to_string()))?;
            let target_metadata = if file_type.is_symlink() {
                match fs::metadata(&path) {
                    Ok(metadata) => Some(metadata),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        return Err(artifact_snapshot_failure(&path, error.to_string()));
                    }
                }
            } else {
                None
            };
            if file_type.is_dir()
                || target_metadata
                    .as_ref()
                    .is_some_and(std::fs::Metadata::is_dir)
            {
                directories.push((path, relative_path, ancestors.clone()));
                continue;
            }
            if !file_type.is_file()
                && !target_metadata
                    .as_ref()
                    .is_some_and(std::fs::Metadata::is_file)
            {
                continue;
            }
            let bytes = fs::read(&path)
                .map_err(|error| artifact_snapshot_failure(&path, error.to_string()))?;
            artifacts.insert(relative_path, Arc::new(bytes));
        }
    }

    Ok(Arc::new(artifacts))
}

fn artifact_snapshot_failure(path: &Path, detail: String) -> InfrastructureFailure {
    InfrastructureFailure::filesystem(
        CompilerStage::DevelopmentServer,
        path,
        format!("Failed to capture dev-server artifact snapshot: {detail}"),
    )
}

/// Constructs a host-operation failure.
pub fn dev_server_error(path: &Path, msg: impl Into<String>) -> InfrastructureFailure {
    InfrastructureFailure::development_server(path, msg.into())
}

#[cfg(test)]
#[path = "tests/build_loop_tests.rs"]
mod tests;
