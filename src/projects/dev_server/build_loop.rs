//! Build execution and watch-triggered rebuild coordination for the dev server.
//!
//! This module delegates compilation and artifact writing to the core build APIs, then translates
//! build outcomes into dev-server state updates and SSE reload broadcasts.

use crate::build_system::build::{self, BuildResult, ProjectBuilder};
use crate::build_system::output::{
    OutputPlan, SingleFileOutputPlan, WriteMode, WriteOptions, write_project_outputs,
};
use crate::command_timing_start;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::display_messages::print_compiler_messages;
use crate::projects::dev_server::error_page::{
    format_compiler_messages, render_compiler_error_page, render_runtime_error_page,
};
use crate::projects::dev_server::sse;
use crate::projects::dev_server::state::DevServerState;
use crate::projects::dev_server::watch;
use crate::projects::routing::{HtmlSiteConfig, parse_html_site_config};
#[cfg(feature = "detailed_timers")]
use crate::timed_manual_finish;
use crate::timing_guard;
use saying::say;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub struct BuildCycleReport {
    pub version: u64,
    pub build_ok: bool,
    pub clients_notified: usize,
    pub watch_scope: Option<watch::WatchScope>,
    /// Structured warnings for successful dev builds, carried to the rebuild loop so terminal
    /// output can print full diagnostics after the success summary.
    pub success_messages: Option<CompilerMessages>,
    /// Drained timing observations for this cycle, rendered by the caller
    /// after its own status line.
    #[cfg(feature = "timers")]
    pub timing_snapshot: Option<crate::timing::BenchmarkObservationSnapshot>,
}

#[derive(Debug, Clone)]
pub struct RebuildRunReport {
    pub watch_scope: Option<watch::WatchScope>,
}

struct BuildOutcome {
    build_succeeded: bool,
    entry_page_rel: Option<PathBuf>,
    html_site_config: Option<HtmlSiteConfig>,
    diagnostics_summary: String,
    /// Full structured warnings for successful dev builds.
    ///
    /// WHAT: carries the same `CompilerDiagnostic` payloads that would be printed on a CLI build
    /// so the dev-server terminal can show complete warnings after a successful rebuild.
    /// WHY: `diagnostics_summary` is intentionally brief for SSE/state consumers; terminal output
    /// needs the full diagnostic cards without rebuilding them from summary titles.
    success_messages: Option<CompilerMessages>,
    failed_build: Option<BuildFailure>,
    watch_scope: Option<watch::WatchScope>,
    output_dir: Option<PathBuf>,
}

enum BuildFailure {
    CompilerMessages(CompilerMessages),
    RuntimeError { title: String, details: String },
}

/// Adapter for build execution used by the dev loop.
///
/// Keeping this contract small makes the watch/build coordination testable while still delegating
/// real work to the core build APIs.
pub trait DevBuildExecutor: Send {
    fn build_and_write(
        &mut self,
        entry_file: &Path,
        flags: &[Flag],
    ) -> Result<BuildResult, CompilerMessages>;
}

pub struct ProjectBuildExecutor {
    builder: ProjectBuilder,
}

impl ProjectBuildExecutor {
    pub fn new(builder: ProjectBuilder) -> Self {
        Self { builder }
    }
}

impl DevBuildExecutor for ProjectBuildExecutor {
    fn build_and_write(
        &mut self,
        entry_file: &Path,
        flags: &[Flag],
    ) -> Result<BuildResult, CompilerMessages> {
        let entry_path = entry_file.to_str().ok_or_else(|| {
            dev_server_error_messages(
                entry_file,
                "Dev server entry path contains invalid UTF-8 and cannot be compiled.",
            )
        })?;

        let mut build_result = build::build_project(&self.builder, entry_path, flags)?;
        let output_plan = if let Some(plan) = build_result.directory_output_plan.as_ref() {
            OutputPlan::Directory(plan.clone())
        } else {
            let project_root = entry_file
                .parent()
                .filter(|parent| parent.is_dir())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| entry_file.to_path_buf());
            OutputPlan::SingleFile(SingleFileOutputPlan {
                output_root: project_root.join("dev"),
                project_root: Some(project_root),
                owner: build_result.output_owner,
                setting_location: SourceLocation::from_path(
                    entry_file,
                    &mut build_result.string_table,
                ),
            })
        };
        if let Err(mut messages) = write_project_outputs(
            &build_result.project,
            &WriteOptions {
                output_plan,
                write_mode: WriteMode::SkipUnchanged,
            },
            &build_result.string_table,
        ) {
            messages.extend_diagnostics(build_result.warnings);
            return Err(messages);
        }

        Ok(build_result)
    }
}

pub fn run_single_build_cycle(
    state: &Arc<DevServerState>,
    executor: &mut dyn DevBuildExecutor,
    entry_file: &Path,
    flags: &[Flag],
) -> BuildCycleReport {
    command_timing_start!(timing_session, crate::timing::TimingCommandKind::Dev);
    #[cfg(feature = "detailed_timers")]
    let cycle_start = crate::timing::start_pipeline_timing();
    let build_outcome = build_once(executor, entry_file, flags);
    let project_root = dev_server_project_root(entry_file);
    let BuildOutcome {
        build_succeeded,
        entry_page_rel,
        html_site_config,
        diagnostics_summary,
        success_messages,
        failed_build,
        watch_scope,
        output_dir,
    } = build_outcome;

    let version = {
        // If a previous dev-server task panicked while holding the lock, keep the latest state and
        // continue serving rebuild results instead of crashing the entire watcher loop.
        let mut build_state = match state.build_state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                say!(
                    Yellow "Dev server state warning: recovering from a poisoned build-state lock after a previous panic."
                );
                poisoned.into_inner()
            }
        };
        build_state.last_build_version = build_state.last_build_version.saturating_add(1);
        build_state.last_build_ok = build_succeeded;
        build_state.last_build_messages_summary = diagnostics_summary;

        if build_succeeded {
            build_state.last_error_html = None;
            build_state.entry_page_rel = entry_page_rel;
            if let Some(output_dir) = output_dir {
                build_state.output_dir = output_dir;
            }
            if let Some(html_site_config) = html_site_config {
                build_state.html_site_config = html_site_config;
            }
        } else {
            // Render compiler diagnostics only after the version increments so the error page and
            // the SSE reload event always point at the same build number.
            build_state.last_error_html = Some(match failed_build {
                Some(BuildFailure::CompilerMessages(messages)) => render_compiler_error_page(
                    &messages,
                    &project_root,
                    &build_state.html_site_config.origin,
                    build_state.last_build_version,
                ),
                Some(BuildFailure::RuntimeError { title, details }) => render_runtime_error_page(
                    &title,
                    &details,
                    &build_state.html_site_config.origin,
                    build_state.last_build_version,
                ),
                None => render_runtime_error_page(
                    "Build Failed",
                    "The latest build failed, but no diagnostics were stored.",
                    &build_state.html_site_config.origin,
                    build_state.last_build_version,
                ),
            });
        }

        build_state.last_build_version
    };

    let clients_notified = sse::broadcast_reload(state, version);
    #[cfg(feature = "detailed_timers")]
    timed_manual_finish!("command.dev.cycle", cycle_start);
    #[cfg(feature = "timers")]
    let timing_snapshot = timing_session.finish();
    BuildCycleReport {
        version,
        build_ok: build_succeeded,
        clients_notified,
        watch_scope,
        success_messages,
        #[cfg(feature = "timers")]
        timing_snapshot: Some(timing_snapshot),
    }
}

/// Maximum consecutive rebuilds before the loop stops to prevent infinite rebuild cycles.
/// If the build itself modifies watched files (e.g. through file-system side effects),
/// the fingerprint check would trigger indefinitely without this limit.
const MAX_CONSECUTIVE_REBUILDS: usize = 5;

pub fn run_builds_until_stable(
    state: &Arc<DevServerState>,
    executor: &mut dyn DevBuildExecutor,
    entry_file: &Path,
    flags: &[Flag],
    watch_session: &watch::WatchSession,
) -> io::Result<RebuildRunReport> {
    let mut build_count = 0usize;
    let latest_watch_scope = loop {
        let build_start_revision = watch_session.current_revision();

        let timer = std::time::Instant::now();
        let report = run_single_build_cycle(state, executor, entry_file, flags);
        let build_duration = timer.elapsed();

        build_count += 1;
        let report_watch_scope = report.watch_scope.clone();

        if report.build_ok {
            say!(
                "Dev build ",
                Blue "#", report.version,
                Reset " done in ",
                Green build_duration.as_millis(), " ms ",
                Reset "- Broadcast to ",
                Blue report.clients_notified,
                Reset " clients."
            );
            if let Some(messages) = report.success_messages {
                print_compiler_messages(messages);
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

    Ok(RebuildRunReport {
        watch_scope: latest_watch_scope,
    })
}

pub fn run_watch_build_loop(
    state: Arc<DevServerState>,
    mut executor: Box<dyn DevBuildExecutor>,
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

        let rebuild_report = match run_builds_until_stable(
            &state,
            executor.as_mut(),
            &entry_file,
            &flags,
            &watch_session,
        ) {
            Ok(report) => report,
            Err(error) => {
                say!(
                    Yellow "Dev server watch warning: rebuild cycle failed: ",
                    Yellow error.to_string()
                );
                last_seen_revision = watch_session.current_revision().max(seen_revision);
                continue;
            }
        };

        last_seen_revision = watch_session.current_revision().max(seen_revision);

        if let Some(next_watch_scope) = rebuild_report.watch_scope
            && next_watch_scope != *watch_session.scope()
        {
            watch_session = watch::WatchSession::start(next_watch_scope, poll_interval);
            last_seen_revision = watch_session.current_revision();
        }
    }
}

fn build_once(
    executor: &mut dyn DevBuildExecutor,
    entry_file: &Path,
    flags: &[Flag],
) -> BuildOutcome {
    let mut build_result = {
        // The dev total is owned by the orchestration around the executor trait
        // call, so every DevBuildExecutor implementation receives the same metric.
        timing_guard!("command.dev.build_and_write");
        match executor.build_and_write(entry_file, flags) {
            Ok(build_result) => build_result,
            Err(messages) => {
                return BuildOutcome {
                    build_succeeded: false,
                    entry_page_rel: None,
                    html_site_config: None,
                    diagnostics_summary: format_compiler_messages(&messages),
                    success_messages: None,
                    failed_build: Some(BuildFailure::CompilerMessages(messages)),
                    watch_scope: None,
                    output_dir: None,
                };
            }
        }
    };
    let output_dir = build_result
        .directory_output_plan
        .as_ref()
        .map(|plan| canonical_output_dir(&plan.output_root))
        .unwrap_or_else(|| {
            canonical_output_dir(
                &entry_file
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("dev"),
            )
        });
    let watch_scope = watch::WatchScope::derive(
        &build_result.config.entry_dir,
        Some(&build_result.config),
        &output_dir,
    );

    let html_site_config =
        match parse_html_site_config(&build_result.config, &mut build_result.string_table) {
            Ok(config) => config,
            Err(error) => {
                let messages = error.into_messages(build_result.string_table.clone());
                return BuildOutcome {
                    build_succeeded: false,
                    entry_page_rel: None,
                    html_site_config: None,
                    diagnostics_summary: format_compiler_messages(&messages),
                    success_messages: None,
                    failed_build: Some(BuildFailure::CompilerMessages(messages)),
                    watch_scope: Some(watch_scope),
                    output_dir: Some(output_dir),
                };
            }
        };

    let warnings_summary = build_result
        .warnings
        .iter()
        .map(|warning| warning.kind.descriptor().title.to_string())
        .collect::<Vec<String>>()
        .join("\n");

    if let Some(entry_page_rel) = build_result.project.entry_page_rel.clone() {
        let diagnostics_summary = if warnings_summary.is_empty() {
            String::from("Build succeeded.")
        } else {
            format!("Build succeeded with warnings:\n{warnings_summary}")
        };

        let success_messages = if build_result.warnings.is_empty() {
            None
        } else {
            Some(CompilerMessages::from_diagnostics(
                build_result.warnings,
                build_result.string_table,
            ))
        };

        BuildOutcome {
            build_succeeded: true,
            entry_page_rel: Some(entry_page_rel),
            html_site_config: Some(html_site_config),
            diagnostics_summary,
            success_messages,
            failed_build: None,
            watch_scope: Some(watch_scope),
            output_dir: Some(output_dir),
        }
    } else {
        BuildOutcome {
            build_succeeded: false,
            entry_page_rel: None,
            html_site_config: None,
            diagnostics_summary: String::from(
                "Build completed, but the project builder did not declare a dev entry page.",
            ),
            success_messages: None,
            failed_build: Some(BuildFailure::RuntimeError {
                title: String::from("Missing Dev Entry"),
                details: String::from(
                    "Build completed, but the project builder did not declare a dev entry page.",
                ),
            }),
            watch_scope: Some(watch_scope),
            output_dir: Some(output_dir),
        }
    }
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

pub fn dev_server_error_messages(path: &Path, msg: impl Into<String>) -> CompilerMessages {
    let mut string_table = Default::default();
    let error = CompilerError::file_error(path, msg.into(), &mut string_table)
        .with_error_type(ErrorType::DevServer);
    CompilerMessages::from_error(error, string_table)
}

#[cfg(test)]
#[path = "tests/build_loop_tests.rs"]
mod tests;
