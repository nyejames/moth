//! CLI mode parser for xtask
//!
//! WHAT: Parses the command-line mode string into a typed benchmark mode.
//! WHY: Keeps mode parsing testable and separate from main dispatch logic,
//!      and replaces raw string matching with a descriptive enum.

use crate::profile::{ProfileOptions, ProfileParseResult, parse_profile_args};
use crate::stress::DEFAULT_STRESS_REPEATS;

pub(crate) const TOP_LEVEL_USAGE: &str = "\
Usage: xtask <mode> [options]

Modes:
  bench                Run the full benchmark suite and update local/public summaries
  bench-check          Run the full benchmark suite without writing benchmark history
  bench-ci             Preflight all cases, then measure the quick subset without recording
  bench-report         Print a local-only benchmark drilldown report
  bench-frontend-check Run the focused frontend benchmark suite without writing history
  bench-frontend       Run the focused frontend benchmark suite and record
  bench-validate       Validate all benchmark cases compile without errors
  bench-profile        Run Samply-backed profiling (use --help for options)
  stress               Repeat the unit and integration suites across thread counts
                       (use --repeats <n>; default 3)
  timers-erasure-check Build a no-timer release binary and verify zero-cost erasure
  feature-matrix       Run every curated feature lane and report the outcome table
  feature-lane-check   Check feature-lane coverage and write the coverage report
  source-audit         Apply the broad-source architecture bans and write their report";

/// Distinguishes the supported xtask benchmark modes.
///
/// WHAT: Each variant represents a valid CLI mode the user can pass to xtask.
/// WHY: Using an enum prevents silent typos in dispatch code and makes the
///      set of supported modes explicit to readers and tests.
///
/// `Copy` is not derived because `BenchProfile` carries `ProfileOptions`,
/// which owns heap-allocated strings.
#[derive(Debug, Clone, PartialEq)]
pub enum BenchmarkMode {
    /// Run the full benchmark suite and update local/public summaries.
    Bench,
    /// Run the full benchmark suite without writing benchmark history.
    BenchCheck,
    /// Run the bounded all-preflight, quick-measurement development gate.
    BenchCi,
    /// Read local benchmark history and print a drilldown report.
    BenchReport,
    /// Run the focused frontend benchmark suite and record.
    BenchFrontend,
    /// Run the focused frontend benchmark suite without writing history.
    BenchFrontendCheck,
    /// Validate all benchmark cases compile without errors (no timing).
    BenchValidate,
    /// Run Samply-backed profiling on benchmark cases.
    BenchProfile(ProfileOptions),
    /// Prove that a no-timer release binary contains no timer-only markers.
    TimersErasureCheck,
    /// Run every curated feature lane and report the complete outcome table.
    FeatureMatrix,
    /// Check feature-lane coverage without running a lane.
    FeatureLaneCheck,
    /// Apply the broad-source architecture bans across the workspace.
    SourceAudit,
    /// Repeat the unit and integration suites across the stress thread counts.
    Stress { repeats: u32 },
}

/// Result of parsing the full xtask command line.
///
/// Distinguishes between a successful mode parse and different failure shapes
/// so `main.rs` can print the right error or help message.
pub enum ModeParseResult {
    /// Successfully parsed a benchmark mode.
    Mode(BenchmarkMode),
    /// `bench-profile` was requested with `--help`; print the contained message.
    ProfileHelp(String),
    /// Parsing failed; print the contained error message.
    Error(String),
}

impl BenchmarkMode {
    /// Parse the full xtask command-line arguments into a typed mode.
    ///
    /// For single-argument modes (`bench`, `bench-check`, etc.), `args` should
    /// contain exactly one element. For `bench-profile`, `args` may contain
    /// additional flags and values after the mode name.
    ///
    /// Returns a `ModeParseResult` so callers can distinguish help requests
    /// from hard errors.
    pub fn parse_args(args: &[String]) -> ModeParseResult {
        if args.is_empty() {
            return ModeParseResult::Error("No mode specified.".to_string());
        }

        let mode_str = &args[0];

        // Single-argument modes: accept exactly one argument.
        let single_mode = match mode_str.as_str() {
            "bench" => Some(BenchmarkMode::Bench),
            "bench-check" => Some(BenchmarkMode::BenchCheck),
            "bench-ci" => Some(BenchmarkMode::BenchCi),
            "bench-report" => Some(BenchmarkMode::BenchReport),
            "bench-frontend" => Some(BenchmarkMode::BenchFrontend),
            "bench-frontend-check" => Some(BenchmarkMode::BenchFrontendCheck),
            "bench-validate" => Some(BenchmarkMode::BenchValidate),
            "timers-erasure-check" => Some(BenchmarkMode::TimersErasureCheck),
            "feature-matrix" => Some(BenchmarkMode::FeatureMatrix),
            "feature-lane-check" => Some(BenchmarkMode::FeatureLaneCheck),
            "source-audit" => Some(BenchmarkMode::SourceAudit),
            _ => None,
        };

        if let Some(mode) = single_mode {
            if args.len() > 1 {
                return ModeParseResult::Error(format!(
                    "Mode '{}' does not accept additional arguments.",
                    mode_str
                ));
            }
            return ModeParseResult::Mode(mode);
        }

        if mode_str == "stress" {
            return match parse_stress_repeats(&args[1..]) {
                Ok(repeats) => ModeParseResult::Mode(BenchmarkMode::Stress { repeats }),
                Err(error) => ModeParseResult::Error(error),
            };
        }

        // bench-profile: variable arguments parsed by the profile module.
        if mode_str == "bench-profile" {
            let remaining: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

            return match parse_profile_args(&remaining) {
                ProfileParseResult::Help(help) => ModeParseResult::ProfileHelp(help),
                ProfileParseResult::Options(options) => {
                    ModeParseResult::Mode(BenchmarkMode::BenchProfile(options))
                }
                ProfileParseResult::Error(error) => ModeParseResult::Error(error),
            };
        }

        ModeParseResult::Error(format!("Unknown mode '{}'", mode_str))
    }
}

/// Parse the optional `--repeats <n>` argument for the stress mode.
fn parse_stress_repeats(args: &[String]) -> Result<u32, String> {
    match args {
        [] => Ok(DEFAULT_STRESS_REPEATS),
        [flag, value] if flag == "--repeats" => value
            .parse::<u32>()
            .ok()
            .filter(|repeats| *repeats > 0)
            .ok_or_else(|| format!("--repeats must be a positive integer, got '{value}'")),
        [flag] if flag == "--repeats" => Err("--repeats requires a value.".to_string()),
        _ => Err("Mode 'stress' accepts only '--repeats <n>'.".to_string()),
    }
}

#[cfg(test)]
mod tests;
