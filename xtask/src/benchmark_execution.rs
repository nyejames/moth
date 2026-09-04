//! Shared execution authority for one typed benchmark manifest case.
//!
//! WHAT: runs one CLI or in-process frontend case, validates its diagnostic and
//! timing facts, and returns one common execution shape.
//! WHY: preflight, measurement and profiling must agree on what counts as a

//! successful benchmark run without reconstructing commands or diagnostics.
// TEMPORARY VALIDATION BRIDGE: `BenchmarkCaseFailure` is currently a 224-byte benchmark
// failure record, so Rust 1.95 Clippy reports `result_large_err` at the existing benchmark
// execution `Result` boundaries. The data-layout plan's final workspace Clippy gate must remove
// this allowance and fix the underlying failure representation.
#![allow(clippy::result_large_err)]

use std::fmt::{Display, Formatter};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::bench_observations::{average_observations, parse_stdout_observations};
use crate::bench_types::BenchmarkCaseObservations;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkManifest,
    BenchmarkManifestError, BenchmarkRunner, CliBenchmarkCommand, CliBenchmarkInvocation,
};
use crate::benchmark_status::{BenchmarkDiagnosticStatus, BenchmarkStatusError};
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use crate::frontend_bench::{report_to_observations, run_one_frontend_case};
use crate::process_runner::{ProcessRun, run_moth_command};

const MAX_FAILURE_EVIDENCE_CHARS: usize = 2_000;

/// Immutable inputs shared by every case in one benchmark execution boundary.
#[derive(Debug)]
pub(crate) struct BenchmarkExecutionContext<'a> {
    manifest: &'a BenchmarkManifest,
    compiler_binary: Option<&'a Path>,
    workspace: &'a BenchmarkExecutionWorkspace,
}

impl<'a> BenchmarkExecutionContext<'a> {
    pub(crate) fn new(
        manifest: &'a BenchmarkManifest,
        compiler_binary: &'a Path,
        workspace: &'a BenchmarkExecutionWorkspace,
    ) -> Self {
        Self {
            manifest,
            compiler_binary: Some(compiler_binary),
            workspace,
        }
    }

    pub(crate) fn frontend(
        manifest: &'a BenchmarkManifest,
        workspace: &'a BenchmarkExecutionWorkspace,
    ) -> Self {
        Self {
            manifest,
            compiler_binary: None,
            workspace,
        }
    }

    /// Resolve one CLI invocation through the run workspace.
    ///
    /// File-entry cases receive an isolated working directory below
    /// `target/benchmark-work/`. Directory-entry cases keep the repository root.
    pub(crate) fn resolve_cli_invocation(
        &self,
        case: &BenchmarkCase,
    ) -> Result<CliBenchmarkInvocation, BenchmarkManifestError> {
        self.workspace.resolve_cli_invocation(self.manifest, case)
    }
}

/// Validated facts produced by one successful benchmark case execution.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkCaseExecution {
    pub(crate) case_id: String,
    pub(crate) workload_id: String,
    pub(crate) runner: BenchmarkRunner,
    pub(crate) total_duration_ms: f64,
    pub(crate) benchmark_status: BenchmarkDiagnosticStatus,
    pub(crate) observations: BenchmarkCaseObservations,
    pub(crate) stdout: Option<String>,
    pub(crate) stderr: Option<String>,
}

/// One manifest case failure with typed cause and bounded channel evidence.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkCaseFailure {
    pub(crate) case_id: String,
    pub(crate) workload_id: String,
    pub(crate) runner: BenchmarkRunner,
    pub(crate) entry: PathBuf,
    pub(crate) kind: BenchmarkFailureKind,
    pub(crate) exit_code: Option<i32>,
    pub(crate) benchmark_status: Option<BenchmarkDiagnosticStatus>,
    pub(crate) stdout_evidence: Option<String>,
    pub(crate) stderr_evidence: Option<String>,
}

/// Stable failure categories shared by preflight and later measurement callers.
#[derive(Debug, Clone)]
pub(crate) enum BenchmarkFailureKind {
    ProcessSpawnFailure {
        message: String,
    },
    NonZeroProcessStatus,
    InvalidMachineStatus {
        error: BenchmarkStatusError,
    },
    CleanExpectationErrors {
        error_count: usize,
    },
    CleanExpectationWarnings {
        warning_count: usize,
        warning_codes: Vec<String>,
    },
    FrontendCompilationFailure,
    InvalidTotalDuration {
        duration_ms: f64,
    },
    ObservationInfrastructureFailure {
        message: String,
    },
    WorkloadInfrastructureFailure {
        message: String,
    },
}

#[derive(Debug, Clone, Default)]
struct BenchmarkFailureEvidence {
    exit_code: Option<i32>,
    benchmark_status: Option<BenchmarkDiagnosticStatus>,
    stdout: Option<String>,
    stderr: Option<String>,
}

/// Execute every selected case once, preserving selection order.
///
/// Ordinary case failures are accumulated so a caller can report the complete
/// broken set. This function performs no history, summary or artifact writes.
pub(crate) fn preflight_cases(
    context: &BenchmarkExecutionContext<'_>,
    cases: &[BenchmarkCase],
) -> Result<Vec<BenchmarkCaseExecution>, Vec<BenchmarkCaseFailure>> {
    let mut executions = Vec::with_capacity(cases.len());
    let mut failures = Vec::new();

    for case in cases {
        announce_preflight_case(case);
        match execute_case(context, case) {
            Ok(execution) => executions.push(execution),
            Err(failure) => failures.push(failure),
        }
    }

    if failures.is_empty() {
        Ok(executions)
    } else {
        Err(failures)
    }
}

/// Report the case before entering compiler code.
///
/// Stderr is unbuffered in normal process execution, but flush explicitly so
/// the identity survives a compiler stack overflow while the preflight is
/// still in progress.
fn announce_preflight_case(case: &BenchmarkCase) {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "Preflighting benchmark case '{}'", case.id)
        .expect("benchmark preflight case identity should be writable");
    stderr
        .flush()
        .expect("benchmark preflight case identity should be flushable");
}

pub(crate) fn format_case_failures(
    execution_phase: &str,
    failures: &[BenchmarkCaseFailure],
) -> String {
    let mut report = format!(
        "{} benchmark case(s) failed {execution_phase}:",
        failures.len()
    );

    for failure in failures {
        report.push_str("\n\n");
        report.push_str(&failure.to_string());
    }

    report
}

/// Execute and validate one typed manifest case.
pub(crate) fn execute_case(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
) -> Result<BenchmarkCaseExecution, BenchmarkCaseFailure> {
    match case.runner {
        BenchmarkRunner::Cli { .. } => execute_cli_case(context, case),
        BenchmarkRunner::Frontend { .. } => execute_frontend_case(context, case),
    }
}

/// Validate and average one case's complete measured observation collection.
///
/// Cross-iteration drift uses the same typed observation-failure lane as an
/// invalid individual execution, so measurement still stops before completion.
pub(crate) fn average_case_observations(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    observations: &[BenchmarkCaseObservations],
) -> Result<BenchmarkCaseObservations, BenchmarkCaseFailure> {
    average_observations(observations).map_err(|error| {
        case_failure(
            context,
            case,
            BenchmarkFailureKind::ObservationInfrastructureFailure {
                message: error.to_string(),
            },
            BenchmarkFailureEvidence::default(),
        )
    })
}

fn execute_cli_case(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
) -> Result<BenchmarkCaseExecution, BenchmarkCaseFailure> {
    let compiler_binary = context.compiler_binary.ok_or_else(|| {
        infrastructure_failure(
            context,
            case,
            "CLI benchmark execution requires a compiler binary".to_owned(),
        )
    })?;
    let invocation = context
        .resolve_cli_invocation(case)
        .map_err(|error| infrastructure_failure(context, case, error.to_string()))?;
    let run = run_moth_command(
        compiler_binary,
        &invocation.current_directory,
        invocation.command.as_str(),
        &invocation.args,
    )
    .map_err(|message| {
        case_failure(
            context,
            case,
            BenchmarkFailureKind::ProcessSpawnFailure { message },
            BenchmarkFailureEvidence::default(),
        )
    })?;

    if !run.status.success {
        let benchmark_status = BenchmarkDiagnosticStatus::try_from(run.stdout.as_str()).ok();

        return Err(process_failure(
            context,
            case,
            BenchmarkFailureKind::NonZeroProcessStatus,
            &run,
            benchmark_status,
        ));
    }

    let benchmark_status =
        BenchmarkDiagnosticStatus::try_from(run.stdout.as_str()).map_err(|error| {
            process_failure(
                context,
                case,
                BenchmarkFailureKind::InvalidMachineStatus { error },
                &run,
                None,
            )
        })?;

    validate_clean_expectation(context, case, benchmark_status, Vec::new(), Some(&run))?;
    validate_total_duration(context, case, run.duration_ms, Some(&run), benchmark_status)?;

    let observations =
        parse_stdout_observations(&run.stdout, invocation.command).map_err(|error| {
            process_failure(
                context,
                case,
                BenchmarkFailureKind::ObservationInfrastructureFailure {
                    message: error.to_string(),
                },
                &run,
                Some(benchmark_status),
            )
        })?;

    // A successful directory build must not leave an undeclared output
    // manifest behind. The bounded recursive scan runs once at finalisation.
    if invocation.command == CliBenchmarkCommand::Build
        && let Some(workload) = context.manifest.workload_for(case)
        && workload.entry_kind == BenchmarkEntryKind::Directory
    {
        let entry_path = context.manifest.repository_root.join(&workload.entry);
        context
            .workspace
            .check_directory_build_output(&entry_path)
            .map_err(|error| {
                infrastructure_failure(
                    context,
                    case,
                    format!("workspace output check failed: {error}"),
                )
            })?;
    }

    Ok(successful_execution(
        context,
        case,
        run.duration_ms,
        benchmark_status,
        observations,
        Some(run.stdout),
        Some(run.stderr),
    ))
}

fn execute_frontend_case(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
) -> Result<BenchmarkCaseExecution, BenchmarkCaseFailure> {
    // Frontend cases register no generated output roots: the in-process
    // frontend API performs no repository output writing.
    let report = run_one_frontend_case(context.manifest, case).map_err(|message| {
        case_failure(
            context,
            case,
            BenchmarkFailureKind::FrontendCompilationFailure,
            BenchmarkFailureEvidence {
                stderr: bounded_evidence(&message),
                ..BenchmarkFailureEvidence::default()
            },
        )
    })?;
    let benchmark_status = BenchmarkDiagnosticStatus {
        error_count: 0,
        warning_count: report.warning_count,
    };

    validate_clean_expectation(
        context,
        case,
        benchmark_status,
        report.warning_codes.clone(),
        None,
    )?;
    validate_total_duration(context, case, report.total_ms, None, benchmark_status)?;

    let observations = report_to_observations(&report).map_err(|error| {
        case_failure(
            context,
            case,
            BenchmarkFailureKind::ObservationInfrastructureFailure {
                message: error.to_string(),
            },
            BenchmarkFailureEvidence {
                benchmark_status: Some(benchmark_status),
                ..BenchmarkFailureEvidence::default()
            },
        )
    })?;

    Ok(successful_execution(
        context,
        case,
        report.total_ms,
        benchmark_status,
        observations,
        None,
        None,
    ))
}

fn validate_clean_expectation(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    benchmark_status: BenchmarkDiagnosticStatus,
    warning_codes: Vec<String>,
    process_run: Option<&ProcessRun>,
) -> Result<(), BenchmarkCaseFailure> {
    match case.expectation {
        BenchmarkExpectation::Clean if benchmark_status.error_count > 0 => {
            Err(failure_with_optional_process(
                context,
                case,
                BenchmarkFailureKind::CleanExpectationErrors {
                    error_count: benchmark_status.error_count,
                },
                process_run,
                Some(benchmark_status),
            ))
        }
        BenchmarkExpectation::Clean if benchmark_status.warning_count > 0 => {
            Err(failure_with_optional_process(
                context,
                case,
                BenchmarkFailureKind::CleanExpectationWarnings {
                    warning_count: benchmark_status.warning_count,
                    warning_codes,
                },
                process_run,
                Some(benchmark_status),
            ))
        }
        BenchmarkExpectation::Clean => Ok(()),
    }
}

fn validate_total_duration(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    duration_ms: f64,
    process_run: Option<&ProcessRun>,
    benchmark_status: BenchmarkDiagnosticStatus,
) -> Result<(), BenchmarkCaseFailure> {
    if duration_ms.is_finite() && duration_ms > 0.0 {
        return Ok(());
    }

    Err(failure_with_optional_process(
        context,
        case,
        BenchmarkFailureKind::InvalidTotalDuration { duration_ms },
        process_run,
        Some(benchmark_status),
    ))
}

fn successful_execution(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    total_duration_ms: f64,
    benchmark_status: BenchmarkDiagnosticStatus,
    observations: BenchmarkCaseObservations,
    stdout: Option<String>,
    stderr: Option<String>,
) -> BenchmarkCaseExecution {
    let workload_id = context
        .manifest
        .workload_for(case)
        .map(|workload| workload.id.clone())
        .unwrap_or_else(|| unresolved_workload_id(case));

    BenchmarkCaseExecution {
        case_id: case.id.clone(),
        workload_id,
        runner: case.runner.clone(),
        total_duration_ms,
        benchmark_status,
        observations,
        stdout,
        stderr,
    }
}

fn infrastructure_failure(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    message: String,
) -> BenchmarkCaseFailure {
    case_failure(
        context,
        case,
        BenchmarkFailureKind::WorkloadInfrastructureFailure { message },
        BenchmarkFailureEvidence::default(),
    )
}

fn process_failure(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    kind: BenchmarkFailureKind,
    run: &ProcessRun,
    benchmark_status: Option<BenchmarkDiagnosticStatus>,
) -> BenchmarkCaseFailure {
    case_failure(
        context,
        case,
        kind,
        BenchmarkFailureEvidence {
            exit_code: run.status.code,
            benchmark_status,
            stdout: bounded_evidence(&run.stdout),
            stderr: bounded_evidence(&run.stderr),
        },
    )
}

fn failure_with_optional_process(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    kind: BenchmarkFailureKind,
    process_run: Option<&ProcessRun>,
    benchmark_status: Option<BenchmarkDiagnosticStatus>,
) -> BenchmarkCaseFailure {
    match process_run {
        Some(run) => process_failure(context, case, kind, run, benchmark_status),
        None => case_failure(
            context,
            case,
            kind,
            BenchmarkFailureEvidence {
                benchmark_status,
                ..BenchmarkFailureEvidence::default()
            },
        ),
    }
}

fn case_failure(
    context: &BenchmarkExecutionContext<'_>,
    case: &BenchmarkCase,
    kind: BenchmarkFailureKind,
    evidence: BenchmarkFailureEvidence,
) -> BenchmarkCaseFailure {
    let (workload_id, entry) = match context.manifest.workload_for(case) {
        Some(workload) => (workload.id.clone(), workload.entry.clone()),
        None => (unresolved_workload_id(case), PathBuf::from("<unresolved>")),
    };

    BenchmarkCaseFailure {
        case_id: case.id.clone(),
        workload_id,
        runner: case.runner.clone(),
        entry,
        kind,
        exit_code: evidence.exit_code,
        benchmark_status: evidence.benchmark_status,
        stdout_evidence: evidence.stdout,
        stderr_evidence: evidence.stderr,
    }
}

fn unresolved_workload_id(case: &BenchmarkCase) -> String {
    format!("<workload-index-{}>", case.workload_index)
}

fn bounded_evidence(output: &str) -> Option<String> {
    let output = output.trim();
    if output.is_empty() {
        return None;
    }

    let mut chars = output.chars();
    let excerpt: String = chars.by_ref().take(MAX_FAILURE_EVIDENCE_CHARS).collect();
    if chars.next().is_some() {
        Some(format!("{excerpt}\n[output truncated]"))
    } else {
        Some(excerpt)
    }
}

impl Display for BenchmarkCaseFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            formatter,
            "case '{}' (workload '{}', {}, entry '{}')",
            self.case_id,
            self.workload_id,
            RunnerDisplay(&self.runner),
            self.entry.display()
        )?;
        write!(formatter, "  failure: {}", self.kind)?;

        if let Some(exit_code) = self.exit_code {
            write!(formatter, "\n  exit code: {exit_code}")?;
        }

        if let Some(status) = self.benchmark_status {
            write!(
                formatter,
                "\n  diagnostic status: errors={} warnings={}",
                status.error_count, status.warning_count
            )?;
        }

        if let Some(stdout) = &self.stdout_evidence {
            write!(formatter, "\n  stdout:\n{stdout}")?;
        }

        if let Some(stderr) = &self.stderr_evidence {
            write!(formatter, "\n  stderr:\n{stderr}")?;
        }

        Ok(())
    }
}

impl Display for BenchmarkCaseExecution {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let observation_count =
            self.observations.stage_timings.len() + self.observations.counters.len();

        write!(
            formatter,
            "case '{}' (workload '{}', {}) passed in {:.3}ms: errors={} warnings={}, {} observation(s)",
            self.case_id,
            self.workload_id,
            RunnerDisplay(&self.runner),
            self.total_duration_ms,
            self.benchmark_status.error_count,
            self.benchmark_status.warning_count,
            observation_count
        )
    }
}

impl Display for BenchmarkFailureKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcessSpawnFailure { message } => {
                write!(
                    formatter,
                    "process spawn failed: {}",
                    bounded_inline(message)
                )
            }
            Self::NonZeroProcessStatus => write!(formatter, "process exited unsuccessfully"),
            Self::InvalidMachineStatus { error } => {
                write!(
                    formatter,
                    "invalid machine status: {}",
                    bounded_inline(&error.to_string())
                )
            }
            Self::CleanExpectationErrors { error_count } => {
                write!(
                    formatter,
                    "clean expectation found {error_count} diagnostic error(s)"
                )
            }
            Self::CleanExpectationWarnings {
                warning_count,
                warning_codes,
            } => {
                write!(
                    formatter,
                    "clean expectation found {warning_count} warning(s)"
                )?;
                if !warning_codes.is_empty() {
                    write!(formatter, ": {}", bounded_inline(&warning_codes.join(", ")))?;
                }
                Ok(())
            }
            Self::FrontendCompilationFailure => write!(formatter, "frontend compilation failed"),
            Self::InvalidTotalDuration { duration_ms } => {
                write!(
                    formatter,
                    "total duration must be positive and finite, got {duration_ms}"
                )
            }
            Self::ObservationInfrastructureFailure { message } => {
                write!(
                    formatter,
                    "benchmark observations failed: {}",
                    bounded_inline(message)
                )
            }
            Self::WorkloadInfrastructureFailure { message } => {
                write!(
                    formatter,
                    "benchmark workload infrastructure failed: {}",
                    bounded_inline(message)
                )
            }
        }
    }
}

struct RunnerDisplay<'a>(&'a BenchmarkRunner);

impl Display for RunnerDisplay<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            BenchmarkRunner::Cli { command, args } => {
                write!(formatter, "CLI {}", command.as_str())?;
                if !args.is_empty() {
                    write!(formatter, " args={args:?}")?;
                }
                Ok(())
            }
            BenchmarkRunner::Frontend { profile } => {
                write!(formatter, "frontend {}", profile.as_str())
            }
        }
    }
}

fn bounded_inline(message: &str) -> String {
    let mut chars = message.chars();
    let excerpt: String = chars.by_ref().take(MAX_FAILURE_EVIDENCE_CHARS).collect();

    if chars.next().is_some() {
        format!("{excerpt} [truncated]")
    } else {
        excerpt
    }
}

#[cfg(test)]
mod tests;
