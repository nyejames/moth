//! Frontend-only `check` command orchestration.
//!
//! WHAT: compiles input through Stage 0 + frontend validation (including borrow checking)
//! without running backend lowering or writing output artifacts.
//! WHY: users and tooling need a fast diagnostic pass that validates source correctness while
//! remaining backend-agnostic.

use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BuildBootstrap, ProjectBuilder, bootstrap_project_build, finalize_project_assembly_failure,
    finish_successful_project_warnings, validate_frontend_facade_boundaries,
};
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, compile_project_frontend_with_inputs,
};
use crate::build_system::path_validation::check_if_valid_path;
use crate::capture_command_duration;
use crate::command_timing_scope;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::finish_command_timing;
use crate::projects::{ProjectCommandCompletion, ProjectCommandStatus, ProjectDiagnosticCounts};
use std::time::{Duration, Instant};

struct CheckOutcome {
    messages: CompilationResult<()>,
}

/// One completed frontend-only check, ready for caller-owned terminal presentation.
pub struct CheckCommandReport {
    pub duration: Duration,
    pub status: ProjectCommandStatus,
    pub error_count: usize,
    pub warning_count: usize,
    pub messages: CompilationResult<()>,
}

/// Execute one frontend-only check before presenting its result to the caller.
///
/// The presentation callback runs after the single command duration is captured and before the
/// timing session is finished. This leaves terse and ordinary terminal rendering with the CLI.
pub fn run_check_command(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
    present: impl FnOnce(CheckCommandReport),
) -> ProjectCommandCompletion {
    command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Check);
    let start = Instant::now();
    let outcome = execute_check(path, build_config_inputs, project_builder);
    let completion = check_completion(&outcome);
    let (error_count, warning_count) = diagnostic_counts(&outcome.messages);

    // Classification is decided inside `execute_check`, so the captured boundary includes it.
    let duration =
        capture_command_duration!(crate::timing::TimingMetric::CommandCheckTotal, start,);
    let report = CheckCommandReport {
        duration,
        status: completion.status,
        error_count,
        warning_count,
        messages: outcome.messages,
    };

    present(report);

    finish_command_timing!(
        timing_session,
        matches!(completion.status, ProjectCommandStatus::Success)
    );
    completion
}

/// Test-only boundary seam for proving check work, capture and rendering order.
///
/// WHAT: classifies a completed check outcome, records a scripted command
///       duration, hands the classified outcome to an injected renderer, then
///       renders it.
/// WHY:  focused tests need an exact duration and an observation point after
///       capture without introducing a general clock abstraction into
///       production.
#[cfg(all(test, feature = "timers"))]
fn run_check_for_tests(
    outcome: CheckOutcome,
    duration: Duration,
    renderer: impl FnOnce(&CheckCommandReport),
) -> ProjectCommandCompletion {
    let completion = check_completion(&outcome);
    let (error_count, warning_count) = diagnostic_counts(&outcome.messages);
    crate::timing::record_command_total_timing(
        crate::timing::TimingMetric::CommandCheckTotal,
        duration,
    );
    let report = CheckCommandReport {
        duration,
        status: completion.status,
        error_count,
        warning_count,
        messages: outcome.messages,
    };
    renderer(&report);
    completion
}

fn diagnostic_counts(messages: &CompilationResult<()>) -> (usize, usize) {
    match messages {
        Ok(CompilationOutcome::Success { warnings, .. }) => (
            0,
            warnings
                .as_ref()
                .map_or(0, |reports| reports.counts().warnings as usize),
        ),
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            let counts = ProjectDiagnosticCounts::from_reports(reports);
            (counts.errors, counts.warnings)
        }
        Err(_) => (1, 0),
    }
}

fn check_completion(outcome: &CheckOutcome) -> ProjectCommandCompletion {
    match &outcome.messages {
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

fn execute_check(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
) -> CheckOutcome {
    CheckOutcome {
        messages: execute_check_messages(path, build_config_inputs, project_builder),
    }
}

fn execute_check_messages(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
) -> CompilationResult<()> {
    execute_check_messages_with_warning_owner(
        path,
        build_config_inputs,
        project_builder,
        BoundaryAccumulator::with_root_string_table,
    )
}

fn execute_check_messages_with_warning_owner(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
    create_warning_owner: impl FnOnce(
        crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> BoundaryAccumulator,
) -> CompilationResult<()> {
    let valid_path = check_if_valid_path(normalize_entry_path(path))?;
    let bootstrap = match bootstrap_project_build(project_builder, valid_path, build_config_inputs)
    {
        Ok(bootstrap) => bootstrap,
        Err(failure) => {
            return failure
                .into_diagnostic_reports()
                .map(CompilationOutcome::Diagnosed);
        }
    };
    let BuildBootstrap {
        mut config,
        style_directives,
        string_table,
        mut frontend_surface,
        validated_directory_output_settings,
        mut project_source_builder,
        build_config_inputs,
    } = bootstrap;
    let mut project_source_files = None;
    let mut warning_owner = create_warning_owner(string_table);
    match compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &build_config_inputs,
        FrontendCompilationMode::Check,
    )? {
        CompilationOutcome::Diagnosed(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
        CompilationOutcome::Success { value, .. } => {
            let project_source_database = value
                .project_source_database
                .clone()
                .or(project_source_files.take());
            let validation = validate_frontend_facade_boundaries(&value);
            drop(value);
            match validation {
                Err(error) => {
                    finalize_project_assembly_failure(error, warning_owner, project_source_database)
                        .map(CompilationOutcome::Diagnosed)
                }
                Ok(()) => finish_successful_project_warnings(
                    &mut warning_owner,
                    project_source_database,
                    (),
                ),
            }
        }
    }
}
#[cfg(test)]
fn execute_check_messages_with_record_limit(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
    records: usize,
) -> CompilationResult<()> {
    execute_check_messages_with_warning_owner(
        path,
        build_config_inputs,
        project_builder,
        |string_table| {
            let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
            warning_owner.set_record_limit_for_test(records);
            warning_owner
        },
    )
}

fn normalize_entry_path(path: &str) -> &str {
    if path.trim().is_empty() { "." } else { path }
}

#[cfg(test)]
#[path = "tests/check_tests.rs"]
mod tests;
