//! CLI mode parser for xtask
//!
//! WHAT: Parses the command-line mode string into a typed benchmark mode.
//! WHY: Keeps mode parsing testable and separate from main dispatch logic,
//!      and replaces raw string matching with a descriptive enum.

use crate::profile::{ProfileOptions, ProfileParseResult, parse_profile_args};

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
            "bench-report" => Some(BenchmarkMode::BenchReport),
            "bench-frontend" => Some(BenchmarkMode::BenchFrontend),
            "bench-frontend-check" => Some(BenchmarkMode::BenchFrontendCheck),
            "bench-validate" => Some(BenchmarkMode::BenchValidate),
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

    /// Legacy single-argument parse for backward compatibility with tests.
    ///
    /// Returns `Some(mode)` for simple modes, `None` for unknown modes or
    /// modes that require additional arguments (like `bench-profile`).
    #[allow(dead_code)]
    pub fn parse(mode_str: &str) -> Option<Self> {
        match mode_str {
            "bench" => Some(BenchmarkMode::Bench),
            "bench-check" => Some(BenchmarkMode::BenchCheck),
            "bench-report" => Some(BenchmarkMode::BenchReport),
            "bench-frontend" => Some(BenchmarkMode::BenchFrontend),
            "bench-frontend-check" => Some(BenchmarkMode::BenchFrontendCheck),
            "bench-validate" => Some(BenchmarkMode::BenchValidate),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
