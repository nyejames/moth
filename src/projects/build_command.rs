//! Build command service for project tools.
//!
//! WHAT: runs the selected builder, plans and writes outputs, and publishes a completed report
//!       with the captured command duration and diagnostic counts.
//! WHY: build pipeline ownership stays in the compiler/build crate while the CLI supplies only
//!      terminal presentation at the report boundary.

use crate::build_system::build::{BuildResult, BuilderSelection, build_project};
use crate::build_system::output::{
    OutputPlan, SingleFileOutputPlan, WriteMode, WriteOptions, write_project_outputs,
};
use crate::capture_command_duration;
use crate::command_timing_scope;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::finish_command_timing;
use crate::projects::{ProjectCommandCompletion, ProjectCommandStatus, ProjectDiagnosticCounts};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One completed build command, ready for caller-owned terminal presentation.
pub struct BuildCommandReport {
    pub duration: Duration,
    pub outcome: CompilationResult<BuildCommandSuccess>,
}

pub struct BuildCommandSuccess {
    pub output_file_count: usize,
    pub output_warnings: Vec<String>,
}

impl BuildCommandReport {
    fn completion(outcome: &CompilationResult<BuildCommandSuccess>) -> ProjectCommandCompletion {
        match outcome {
            Ok(CompilationOutcome::Success { warnings, .. }) => ProjectCommandCompletion {
                status: ProjectCommandStatus::Success,
                diagnostic_counts: Some(ProjectDiagnosticCounts {
                    errors: 0,
                    warnings: warnings
                        .as_ref()
                        .map_or(0, |reports| reports.counts().warnings as usize),
                }),
            },
            Ok(CompilationOutcome::Diagnosed(reports)) => ProjectCommandCompletion {
                status: ProjectCommandStatus::Failure,
                diagnostic_counts: Some(ProjectDiagnosticCounts::from_reports(reports)),
            },
            Err(_) => ProjectCommandCompletion {
                status: ProjectCommandStatus::Failure,
                diagnostic_counts: None,
            },
        }
    }
}

enum BuildOutputFailure {
    Plan(InfrastructureFailure),
    Write(PreparedFailure),
}

/// Execute, classify and write one build before presenting its outcome to the caller.
///
/// The presentation callback runs after the single command duration is captured and before the
/// timing session is finished. This preserves the established order while keeping all build
/// orchestration and output ownership on the compiler side of the CLI boundary.
/// The caller resolves `entry_path` and the single-file `output_root` before invoking the service, so output
/// planning does not consult process-global current-directory state. Directory builds retain their
/// validated project output plan.
pub fn run_build_command(
    builder_selection: &BuilderSelection,
    entry_path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
    output_root: &Path,
    present: impl FnOnce(BuildCommandReport),
) -> ProjectCommandCompletion {
    run_build_command_with_output_plan(
        builder_selection,
        entry_path,
        flags,
        build_config_inputs,
        |build_result| create_build_output_plan(build_result, output_root),
        present,
    )
}

fn run_build_command_with_output_plan(
    builder_selection: &BuilderSelection,
    entry_path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, InfrastructureFailure>,
    present: impl FnOnce(BuildCommandReport),
) -> ProjectCommandCompletion {
    command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Build);
    let started = Instant::now();
    let outcome = build_command_outcome(
        builder_selection,
        entry_path,
        flags,
        build_config_inputs,
        output_plan_builder,
    );
    let completion = BuildCommandReport::completion(&outcome);
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
    builder_selection: &BuilderSelection,
    entry_path: &str,
    flags: &[Flag],
    build_config_inputs: &BuildConfigInputSet,
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, InfrastructureFailure>,
) -> CompilationResult<BuildCommandSuccess> {
    let build_result = match build_project(
        builder_selection.backend.as_ref(),
        entry_path,
        flags,
        build_config_inputs,
    )
    .unwrap_or_else(|never| match never {})?
    {
        CompilationOutcome::Success { value, .. } => value,
        CompilationOutcome::Diagnosed(reports) => {
            return Ok(CompilationOutcome::Diagnosed(reports));
        }
    };
    write_build_outputs_with_plan(build_result, output_plan_builder)
}

fn write_build_outputs_with_plan(
    build_result: BuildResult,
    output_plan_builder: impl FnOnce(&mut BuildResult) -> Result<OutputPlan, InfrastructureFailure>,
) -> CompilationResult<BuildCommandSuccess> {
    let mut build_result = build_result;
    let output_result = crate::timed_stage!(
        crate::timing::TimingMetric::BuildOutputTotal,
        (|| {
            let output_plan =
                output_plan_builder(&mut build_result).map_err(BuildOutputFailure::Plan)?;
            let mut diagnostic_strings =
                DiagnosticStringBuilder::new(build_result.warning_accumulator.string_table_mut());
            write_project_outputs(
                &mut build_result.project,
                &WriteOptions {
                    output_plan,
                    write_mode: WriteMode::AlwaysWrite,
                },
                &mut diagnostic_strings,
            )
            .map_err(BuildOutputFailure::Write)
        })()
    );

    match output_result {
        Ok(write_summary) => {
            let output_file_count = write_summary.emitted_count();
            let output_warnings = write_summary.into_warning_messages();
            let warnings = match build_result.finish_output(Ok(()))? {
                CompilationOutcome::Success { warnings, .. } => warnings,
                CompilationOutcome::Diagnosed(reports) => {
                    return Ok(CompilationOutcome::Diagnosed(reports));
                }
            };
            Ok(CompilationOutcome::Success {
                value: BuildCommandSuccess {
                    output_file_count,
                    output_warnings,
                },
                warnings,
            })
        }
        Err(BuildOutputFailure::Write(failure)) => {
            match build_result.finish_output(Err(failure))? {
                CompilationOutcome::Diagnosed(reports) => {
                    Ok(CompilationOutcome::Diagnosed(reports))
                }
                CompilationOutcome::Success { .. } => crate::compiler_bug!(
                    crate::compiler_frontend::compiler_messages::CompilerStage::Output,
                    "a failed output write unexpectedly finalized a successful build"
                ),
            }
        }
        Err(BuildOutputFailure::Plan(error)) => Err(error.into()),
    }
}

fn create_build_output_plan(
    build_result: &mut BuildResult,
    output_root: &Path,
) -> Result<OutputPlan, InfrastructureFailure> {
    if let Some(plan) = build_result.directory_output_plan.as_ref() {
        return Ok(OutputPlan::Directory(plan.clone()));
    }

    let project_root = single_file_project_entry_dir(&build_result.config.entry_dir)?;

    Ok(OutputPlan::SingleFile(SingleFileOutputPlan {
        output_root: output_root.to_path_buf(),
        project_root: Some(project_root),
        owner: build_result.output_owner.clone(),
    }))
}

fn single_file_project_entry_dir(entry_path: &Path) -> Result<PathBuf, InfrastructureFailure> {
    let Some(parent) = entry_path.parent() else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "Could not resolve containing directory for single-file build entry '{}'",
                entry_path.display()
            ),
        ));
    };

    if parent.as_os_str().is_empty() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "Could not resolve containing directory for single-file build entry '{}': its path must be absolute",
                entry_path.display()
            ),
        ));
    }

    // The directory can disappear or become unreadable after source validation, so this is host
    // filesystem state rather than a compiler invariant. The OS error names the exact cause.
    let problem = match std::fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => return Ok(parent.to_path_buf()),
        Ok(_) => String::from("it is not a directory"),
        Err(error) => error.to_string(),
    };
    Err(InfrastructureFailure::filesystem(
        CompilerStage::ProjectBuildAssembly,
        parent,
        format!(
            "Could not use containing directory '{}' for single-file build entry '{}': {problem}",
            parent.display(),
            entry_path.display()
        ),
    ))
}

#[cfg(test)]
#[path = "tests/build_command_tests.rs"]
mod tests;
