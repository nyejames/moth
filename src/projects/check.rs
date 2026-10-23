//! Frontend-only `check` command orchestration.
//!
//! WHAT: compiles input through Stage 0 + frontend validation (including borrow checking)
//! without running backend lowering or writing output artifacts.
//! WHY: users and tooling need a fast diagnostic pass that validates source correctness while
//! remaining backend-agnostic.

use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BuildBootstrap, ProjectBuilder, bootstrap_project_build, validate_frontend_facade_boundaries,
};
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, compile_project_frontend_with_inputs,
};
use crate::build_system::path_validation::check_if_valid_path;
use crate::capture_command_duration;
use crate::command_timing_scope;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::BoundaryFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::finish_command_timing;
use crate::projects::{ProjectCommandCompletion, ProjectCommandStatus, ProjectDiagnosticCounts};
use std::time::{Duration, Instant};

struct CheckOutcome {
    messages: BoundaryFailure,
    status: ProjectCommandStatus,
}

/// One completed frontend-only check, ready for caller-owned terminal presentation.
pub struct CheckCommandReport {
    pub duration: Duration,
    pub status: ProjectCommandStatus,
    pub error_count: usize,
    pub warning_count: usize,
    pub messages: BoundaryFailure,
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
    let error_count = outcome.messages.error_count();
    let warning_count = outcome.messages.warning_count();

    // Classification is decided inside `execute_check`, so the captured boundary includes it.
    let duration =
        capture_command_duration!(crate::timing::TimingMetric::CommandCheckTotal, start,);
    let report = CheckCommandReport {
        duration,
        status: outcome.status,
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
    let error_count = outcome.messages.error_count();
    let warning_count = outcome.messages.warning_count();
    crate::timing::record_command_total_timing(
        crate::timing::TimingMetric::CommandCheckTotal,
        duration,
    );
    let report = CheckCommandReport {
        duration,
        status: outcome.status,
        error_count,
        warning_count,
        messages: outcome.messages,
    };
    renderer(&report);
    completion
}

fn check_completion(outcome: &CheckOutcome) -> ProjectCommandCompletion {
    ProjectCommandCompletion {
        status: outcome.status,
        diagnostic_counts: ProjectDiagnosticCounts::from_failure(&outcome.messages),
    }
}

fn execute_check(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
) -> CheckOutcome {
    let messages = execute_check_messages(path, build_config_inputs, project_builder);
    let status = if messages.has_errors() {
        ProjectCommandStatus::Failure
    } else {
        ProjectCommandStatus::Success
    };
    CheckOutcome { messages, status }
}

fn execute_check_messages(
    path: &str,
    build_config_inputs: &BuildConfigInputSet,
    project_builder: &ProjectBuilder,
) -> BoundaryFailure {
    let normalized_path = normalize_entry_path(path);

    let valid_path = match check_if_valid_path(normalized_path) {
        Ok(path) => path,
        Err(error) => {
            let mut accumulator = BoundaryAccumulator::with_root_string_table(&StringTable::new());
            accumulator.append_infrastructure(error.into(), None);
            return accumulator.finish(InstallMode::Unfrozen);
        }
    };

    let bootstrap = match bootstrap_project_build(project_builder, valid_path, build_config_inputs)
    {
        Ok(bootstrap) => bootstrap,
        Err(failure) => return failure.into_boundary_failure(),
    };
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
        mut project_source_files,
        build_config_inputs,
    } = bootstrap;

    match compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Check,
    ) {
        Ok(frontend) => {
            let facade_failure = validate_frontend_facade_boundaries(&frontend).err();
            frontend.into_boundary_failure(
                &mut string_table,
                project_source_files.take(),
                facade_failure,
            )
        }
        Err(failure) => failure,
    }
}

fn normalize_entry_path(path: &str) -> &str {
    if path.trim().is_empty() { "." } else { path }
}

#[cfg(test)]
#[path = "tests/check_tests.rs"]
mod tests;
