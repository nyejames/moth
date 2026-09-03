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
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::display_messages::{
    print_compiler_messages, print_terse_compiler_messages,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::finish_command_timing;
use crate::projects::command_status::{
    CommandStatus, benchmark_diagnostic_counts, emit_benchmark_status,
};
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use saying::say;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckOptions {
    pub terse: bool,
    /// The explicit typed build-config inputs this check command started with.
    pub(crate) inputs: BuildConfigInputSet,
}

struct CheckOutcome {
    messages: CompilerMessages,
    status: CommandStatus,
}

pub(crate) fn run_check(path: &str, options: CheckOptions) -> CommandStatus {
    command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Check);
    let start = Instant::now();
    let outcome = execute_check(path, &options.inputs);
    let error_count = outcome.messages.error_count();
    let warning_count = outcome.messages.warning_count();
    let benchmark_counts = benchmark_diagnostic_counts(&outcome.messages);

    // Capture the single command duration before any terminal rendering.
    // Classification is already decided inside `execute_check` so the
    // captured boundary is execution + outcome + classification, not
    // execution + outcome alone.
    let duration =
        capture_command_duration!(crate::timing::TimingMetric::CommandCheckTotal, start,);

    let status = outcome.status;
    render_check_outcome(outcome, options, duration, error_count, warning_count);

    finish_command_timing!(timing_session, matches!(status, CommandStatus::Success));
    if let Some((error_count, warning_count)) = benchmark_counts {
        emit_benchmark_status(error_count, warning_count);
    }

    status
}

/// Render a completed check outcome after the command duration has been captured.
///
/// WHAT: owns all terminal presentation for check success and failure paths.
/// WHY:  presentation must never be interleaved with command work or outcome
///       classification, so the captured duration excludes rendering.
fn render_check_outcome(
    outcome: CheckOutcome,
    options: CheckOptions,
    duration: Duration,
    error_count: usize,
    warning_count: usize,
) {
    if options.terse {
        print_terse_compiler_messages(&outcome.messages);
        println!(
            "{}",
            format_terse_summary_line(duration, error_count, warning_count)
        );
    } else if error_count == 0 && warning_count == 0 {
        say!(Dark White "---------------------");
        say!(success_message(duration));
        say!(Bold Green "No errors or warnings");
    } else {
        print_compiler_messages(outcome.messages);
    }
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
    options: CheckOptions,
    duration: Duration,
    renderer: impl FnOnce(&CheckOutcome, Duration),
) -> (CommandStatus, Option<(usize, usize)>) {
    let error_count = outcome.messages.error_count();
    let warning_count = outcome.messages.warning_count();
    let benchmark_counts = benchmark_diagnostic_counts(&outcome.messages);
    let status = outcome.status;
    crate::timing::record_command_total_timing(
        crate::timing::TimingMetric::CommandCheckTotal,
        duration,
    );
    renderer(&outcome, duration);
    render_check_outcome(outcome, options, duration, error_count, warning_count);
    (status, benchmark_counts)
}

fn execute_check(path: &str, build_config_inputs: &BuildConfigInputSet) -> CheckOutcome {
    let normalized_path = normalize_entry_path(path);

    let mut path_string_table = StringTable::new();
    let valid_path = match check_if_valid_path(normalized_path, &mut path_string_table) {
        Ok(path) => path,
        Err(error) => {
            return CheckOutcome {
                messages: CompilerMessages::from_error(error, path_string_table),
                status: CommandStatus::Failure,
            };
        }
    };

    let project_builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
        build_config_inputs,
    } = match bootstrap_project_build(&project_builder, valid_path, build_config_inputs) {
        Ok(bootstrap) => bootstrap,
        Err(messages) => {
            return CheckOutcome {
                messages,
                status: CommandStatus::Failure,
            };
        }
    };

    let messages = match compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &build_config_inputs,
        FrontendCompilationMode::Check,
    ) {
        Ok(frontend) => {
            let facade_validation = validate_frontend_facade_boundaries(&frontend);
            let mut messages = frontend.into_render_messages(&mut string_table);
            if let Err(error) = facade_validation {
                let facade_messages = error.into_messages(&mut string_table);
                messages.string_table = string_table.clone();
                messages.append_messages_preserving_context(facade_messages);
            }
            messages
        }
        Err(messages) => messages,
    };

    let status = if messages.error_count() > 0 {
        CommandStatus::Failure
    } else {
        CommandStatus::Success
    };
    CheckOutcome { messages, status }
}

fn normalize_entry_path(path: &str) -> &str {
    if path.trim().is_empty() { "." } else { path }
}

fn format_terse_summary_line(
    duration: Duration,
    error_count: usize,
    warning_count: usize,
) -> String {
    if error_count == 0 && warning_count == 0 {
        return format!("{}. No errors or warnings.", success_message(duration));
    }

    format!("errors={error_count}, warnings={warning_count}.")
}

fn format_duration(duration: Duration) -> String {
    format!("{duration:?}")
}

fn success_message(duration: Duration) -> String {
    format!("Done in {}", format_duration(duration))
}

#[cfg(test)]
#[path = "tests/check_tests.rs"]
mod tests;
