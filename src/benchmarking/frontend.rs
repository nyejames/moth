//! Frontend benchmark implementation.
//!
//! WHAT: measures the compiler frontend pipeline (Stage 0 through borrow
//! validation) for a single entry path, collecting total time and per-stage
//! timings when `timers` is enabled (counters additionally require
//! `benchmark_counters`).
//! WHY: avoids subprocess noise while reusing the exact same setup path as
//! `moth check`.

use std::path::PathBuf;
use std::time::Instant;

use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildBootstrap, ProjectBuilder, bootstrap_project_build};
use crate::build_system::create_project_modules::compile_project_frontend;
use crate::build_system::path_validation::check_if_valid_path;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::display_messages::format_terse_compiler_messages;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;

/// Build profile selector for frontend benchmarks.
///
/// WHAT: a narrow public selector that converts into the build-system `BuildProfile` at the
/// benchmark boundary without exposing internal compiler types in the benchmark API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendBenchmarkBuildProfile {
    Dev,
    Release,
}

/// Input options for a single frontend benchmark run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendBenchmarkOptions {
    pub entry_path: PathBuf,
    pub build_profile: FrontendBenchmarkBuildProfile,
}

/// Report produced by a successful frontend benchmark run.
#[derive(Debug, Clone)]
pub struct FrontendBenchmarkReport {
    pub total_ms: f64,
    pub warning_count: usize,
    pub warning_codes: Vec<String>,
    pub stages: Vec<FrontendBenchmarkStage>,
    pub counters: Vec<FrontendBenchmarkCounter>,
}

/// One named stage timing captured during frontend compilation.
#[derive(Debug, Clone)]
pub struct FrontendBenchmarkStage {
    pub name: String,
    pub duration_ms: f64,
}

/// One named counter value captured during frontend compilation.
#[derive(Debug, Clone)]
pub struct FrontendBenchmarkCounter {
    pub name: String,
    pub value: f64,
}

/// Error returned when a frontend benchmark fails.
///
/// The message is pre-rendered into a terse, multi-line string suitable for
/// direct display by xtask or other tooling.
#[derive(Debug, Clone)]
pub struct FrontendBenchmarkError {
    pub message: String,
}

impl std::fmt::Display for FrontendBenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FrontendBenchmarkError {}

/// Run one frontend benchmark for the given entry path.
///
/// WHAT: validates the path, bootstraps an HTML project build, compiles through
/// the frontend pipeline, and returns total plus per-stage timings.
/// WHY: this is the narrow dev-tooling entry point that keeps benchmark
/// orchestration out of the compiler frontend while reusing production setup.
///
/// Stage timings are populated when the `timers` feature is enabled and a
/// collection scope is active during compilation. Counters are additionally
/// populated when `benchmark_counters` is also enabled.
pub fn run_frontend_benchmark(
    options: FrontendBenchmarkOptions,
) -> Result<FrontendBenchmarkReport, FrontendBenchmarkError> {
    let start = Instant::now();

    let path = options
        .entry_path
        .to_str()
        .ok_or_else(|| FrontendBenchmarkError {
            message: format!(
                "Frontend benchmark path is not valid UTF-8: {}",
                options.entry_path.display()
            ),
        })?;
    let normalized = if path.trim().is_empty() { "." } else { path };

    let mut path_string_table = StringTable::new();
    let valid_path = match check_if_valid_path(normalized, &mut path_string_table) {
        Ok(path) => path,
        Err(error) => {
            let messages = CompilerMessages::from_error(error, path_string_table);

            return Err(FrontendBenchmarkError {
                message: format_compiler_messages(&messages),
            });
        }
    };

    let project_builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));

    #[cfg(feature = "timers")]
    let timing_session =
        crate::compiler_frontend::compiler_messages::compiler_dev_logging::start_raw_benchmark_collection(
            true,
        );

    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
    } = match bootstrap_project_build(&project_builder, valid_path) {
        Ok(bootstrap) => bootstrap,
        Err(messages) => {
            #[cfg(feature = "timers")]
            let _ = timing_session.finish();

            return Err(FrontendBenchmarkError {
                message: format_compiler_messages(&messages),
            });
        }
    };

    let build_profile = match options.build_profile {
        FrontendBenchmarkBuildProfile::Release => BuildProfile::Release,
        FrontendBenchmarkBuildProfile::Dev => BuildProfile::Dev,
    };

    let messages = match compile_project_frontend(
        &mut config,
        build_profile,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(frontend) => frontend.into_render_messages(&mut string_table),
        Err(messages) => messages,
    };

    #[cfg(feature = "timers")]
    let raw_observations = timing_session.finish();

    #[cfg(not(feature = "timers"))]
    let stages: Vec<FrontendBenchmarkStage> = Vec::new();

    #[cfg(not(all(feature = "timers", feature = "benchmark_counters")))]
    let counters: Vec<FrontendBenchmarkCounter> = Vec::new();

    let total_ms = start.elapsed().as_secs_f64() * 1000.0;

    if messages.error_count() > 0 {
        return Err(FrontendBenchmarkError {
            message: format_compiler_messages(&messages),
        });
    }

    let warning_count = messages.warning_count();
    let warning_codes = messages
        .warnings()
        .map(|warning| warning.kind.code().to_owned())
        .collect();

    #[cfg(feature = "timers")]
    let stages = raw_observations
        .timings
        .into_iter()
        .map(|observation| FrontendBenchmarkStage {
            name: observation.name.to_owned(),
            duration_ms: observation.duration.as_secs_f64() * 1000.0,
        })
        .collect();

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counters = raw_observations
        .counters
        .into_iter()
        .map(|metric| FrontendBenchmarkCounter {
            name: metric.name.to_owned(),
            value: metric.value,
        })
        .collect();

    Ok(FrontendBenchmarkReport {
        total_ms,
        warning_count,
        warning_codes,
        stages,
        counters,
    })
}

fn format_compiler_messages(messages: &CompilerMessages) -> String {
    let mut lines = format_terse_compiler_messages(messages);

    if lines.is_empty() {
        lines.push(format!("{} error(s) found", messages.error_count()));
    }

    lines.join("\n")
}
