//! Build command service for project tools.
//!
//! WHAT: runs the selected builder, plans and writes outputs, and publishes a completed report
//!       with the captured command duration and diagnostic counts.
//! WHY: build pipeline ownership stays in the compiler/build crate while the CLI supplies only
//!      terminal presentation at the report boundary.

use crate::build_system::build::{BuildResult, ProjectBuilder, build_project};
use crate::build_system::output::{
    OutputPlan, SingleFileOutputPlan, WriteMode, WriteOptions, write_project_outputs,
};
use crate::capture_command_duration;
use crate::command_timing_scope;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{BoundaryFailure, CompilerError, CompilerMessages};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::finish_command_timing;
use crate::projects::{ProjectCommandCompletion, ProjectCommandStatus, ProjectDiagnosticCounts};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One completed build command, ready for caller-owned terminal presentation.
pub struct BuildCommandReport {
    pub duration: Duration,
    pub outcome: BuildCommandOutcome,
}

/// The successful output summary or preserved diagnostic set from a build command.
pub enum BuildCommandOutcome {
    Success {
        output_file_count: usize,
        warning_count: usize,
        warnings: Option<BoundaryFailure>,
    },
    Failure {
        messages: BoundaryFailure,
        diagnostic_counts: Option<ProjectDiagnosticCounts>,
    },
}

impl BuildCommandOutcome {
    fn completion(&self) -> ProjectCommandCompletion {
        match self {
            BuildCommandOutcome::Success { warning_count, .. } => ProjectCommandCompletion {
                status: ProjectCommandStatus::Success,
                diagnostic_counts: Some(ProjectDiagnosticCounts {
                    errors: 0,
                    warnings: *warning_count,
                }),
            },
            BuildCommandOutcome::Failure {
                diagnostic_counts, ..
            } => ProjectCommandCompletion {
                status: ProjectCommandStatus::Failure,
                diagnostic_counts: *diagnostic_counts,
            },
        }
    }
}

enum BuildOutputFailure {
    Plan(CompilerError),
    Write(CompilerMessages),
}

/// Execute, classify and write one build before presenting its outcome to the caller.
///
/// The presentation callback runs after the single command duration is captured and before the
/// timing session is finished. This preserves the established order while keeping all build
/// orchestration and output ownership on the compiler side of the CLI boundary.
pub fn run_build_command(
    project_builder: &ProjectBuilder,
    entry_path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
    present: impl FnOnce(BuildCommandReport),
) -> ProjectCommandCompletion {
    run_build_command_with_output_plan(
        project_builder,
        entry_path,
        flags,
        build_config_inputs,
        create_build_output_plan,
        present,
    )
}

fn run_build_command_with_output_plan(
    project_builder: &ProjectBuilder,
    entry_path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, CompilerError>,
    present: impl FnOnce(BuildCommandReport),
) -> ProjectCommandCompletion {
    command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Build);
    let started = Instant::now();
    let outcome = build_command_outcome(
        project_builder,
        entry_path,
        flags,
        build_config_inputs,
        output_plan_builder,
    );
    let completion = outcome.completion();
    let duration =
        capture_command_duration!(crate::timing::TimingMetric::CommandBuildTotal, started,);
    let report = BuildCommandReport { duration, outcome };

    present(report);

    finish_command_timing!(
        timing_session,
        matches!(completion.status, ProjectCommandStatus::Success)
    );
    completion
}

fn build_command_outcome(
    project_builder: &ProjectBuilder,
    entry_path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, CompilerError>,
) -> BuildCommandOutcome {
    match build_project(project_builder, entry_path, flags, build_config_inputs) {
        Ok(mut build_result) => {
            write_build_outputs_with_plan(&mut build_result, output_plan_builder)
        }
        Err(messages) => {
            let diagnostic_counts = ProjectDiagnosticCounts::from_failure(&messages);
            BuildCommandOutcome::Failure {
                messages,
                diagnostic_counts,
            }
        }
    }
}

fn write_build_outputs_with_plan(
    build_result: &mut BuildResult,
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, CompilerError>,
) -> BuildCommandOutcome {
    let output_result = crate::timed_stage!(
        crate::timing::TimingMetric::BuildOutputTotal,
        (|| {
            let output_plan =
                output_plan_builder(build_result).map_err(BuildOutputFailure::Plan)?;
            write_project_outputs(
                &mut build_result.project,
                &WriteOptions {
                    output_plan,
                    write_mode: WriteMode::AlwaysWrite,
                },
                &mut build_result.string_table,
            )
            .map_err(BuildOutputFailure::Write)
        })()
    );

    match output_result {
        Ok(write_summary) => {
            let output_file_count = write_summary.emitted_count();
            let warning_count = build_result.warnings.len();
            let warnings = match build_result.take_warning_messages() {
                Some(messages) if messages.has_infrastructure_error() => {
                    return BuildCommandOutcome::Failure {
                        messages,
                        diagnostic_counts: None,
                    };
                }
                warnings => warnings,
            };
            BuildCommandOutcome::Success {
                output_file_count,
                warning_count,
                warnings,
            }
        }
        Err(BuildOutputFailure::Write(messages)) => BuildCommandOutcome::Failure {
            messages: build_result.take_output_failure_messages(messages),
            diagnostic_counts: None,
        },
        Err(BuildOutputFailure::Plan(error)) => {
            let messages = CompilerMessages::from_error(error, StringTable::new());
            BuildCommandOutcome::Failure {
                messages: build_result.take_output_failure_messages(messages),
                diagnostic_counts: None,
            }
        }
    }
}

fn create_build_output_plan(build_result: &mut BuildResult) -> Result<OutputPlan, CompilerError> {
    if let Some(plan) = build_result.directory_output_plan.as_ref() {
        return Ok(OutputPlan::Directory(plan.clone()));
    }

    let output_root = env::current_dir().map_err(|error| {
        CompilerError::compiler_error(format!(
            "Could not resolve current directory for build outputs: {error}"
        ))
    })?;
    let project_root = single_file_project_entry_dir(&build_result.config.entry_dir)?;

    Ok(OutputPlan::SingleFile(SingleFileOutputPlan {
        output_root,
        project_root: Some(project_root),
        owner: build_result.output_owner,
        setting_span: None,
    }))
}

fn single_file_project_entry_dir(entry_path: &Path) -> Result<PathBuf, CompilerError> {
    let Some(parent) = entry_path.parent() else {
        return Err(CompilerError::compiler_error(format!(
            "Could not resolve containing directory for single-file build entry '{}'",
            entry_path.display()
        )));
    };

    if parent.as_os_str().is_empty() {
        return env::current_dir().map_err(|error| {
            CompilerError::compiler_error(format!(
                "Could not resolve containing directory for single-file build entry '{}': {error}",
                entry_path.display()
            ))
        });
    }

    if parent.is_dir() {
        return Ok(parent.to_path_buf());
    }

    Err(CompilerError::compiler_error(format!(
        "Could not resolve containing directory for single-file build entry '{}': '{}' is not a directory",
        entry_path.display(),
        parent.display()
    )))
}

#[cfg(test)]
#[path = "tests/build_command_tests.rs"]
mod tests;
