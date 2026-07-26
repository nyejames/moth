//! Profile options and filter modes for the profiling workflow
//!
//! WHAT: Defines `ProfileOptions` and `ProfileFilterMode`, which control
//! profiling behavior such as hotspot filtering thresholds, case selection,
//! Samply sampling rate, and symbolication.
//!
//! WHY: Separating options from orchestration keeps the command parser
//! testable and makes the profiling workflow's configuration surface
//! explicit and readable.

/// Filter mode controlling how profiling hotspots are summarized.
///
/// Each mode defines different thresholds for which functions and cases
/// appear in agent-readable summaries. The raw profile is never deleted
/// or rewritten; filtering only affects derived metadata and summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileFilterMode {
    /// Agent-first default: top 8 functions, top 3 cases, tight thresholds.
    #[default]
    Terse,
    /// Human + agent investigation: top 20 functions, top 8 cases.
    Normal,
    /// Pre-refactor investigation: top 50 functions, all cases, includes edges.
    Deep,
    /// Artifact generation only: raw profile and observations, no hotspot parsing.
    RawIndex,
}

// Threshold methods used by Phase 4 hotspot extraction.
impl ProfileFilterMode {
    /// Parse a filter mode string, defaulting to `Terse` for empty input.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "" => Some(Self::Terse),
            "terse" => Some(Self::Terse),
            "normal" => Some(Self::Normal),
            "deep" => Some(Self::Deep),
            "raw-index" => Some(Self::RawIndex),
            _ => None,
        }
    }

    /// Maximum number of hot functions to keep per case summary.
    pub fn hot_function_limit(self) -> usize {
        match self {
            Self::Terse => 8,
            Self::Normal => 20,
            Self::Deep => 50,
            Self::RawIndex => 0,
        }
    }

    /// Maximum number of cases to include in the root agent summary.
    /// Used by Phase 5 summary generation.
    #[allow(dead_code)]
    pub fn root_case_limit(self) -> usize {
        match self {
            Self::Terse => 3,
            Self::Normal => 8,
            Self::Deep => usize::MAX,
            Self::RawIndex => 0,
        }
    }

    /// Minimum inclusive sample percent to keep a function.
    pub fn minimum_inclusive_pct(self) -> f64 {
        match self {
            Self::Terse => 2.0,
            Self::Normal => 1.0,
            Self::Deep => 0.25,
            Self::RawIndex => 0.0,
        }
    }

    /// Minimum self sample percent to keep a function.
    pub fn minimum_self_pct(self) -> f64 {
        match self {
            Self::Terse => 1.0,
            Self::Normal => 0.5,
            Self::Deep => 0.25,
            Self::RawIndex => 0.0,
        }
    }

    /// Whether to include caller/callee edge context in summaries.
    pub fn include_edges(self) -> bool {
        match self {
            Self::Terse | Self::Normal | Self::RawIndex => false,
            Self::Deep => true,
        }
    }

    /// Return a human-readable label for this filter mode.
    pub fn display_label(self) -> &'static str {
        match self {
            Self::Terse => "terse",
            Self::Normal => "normal",
            Self::Deep => "deep",
            Self::RawIndex => "raw-index",
        }
    }
}

/// Configuration for a single profiling run.
///
/// Controls case filtering, Samply sampling parameters, and output detail.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileOptions {
    /// Filter mode controlling hotspot summary thresholds.
    pub filter: ProfileFilterMode,
    /// Optional authored case ID filter; `None` profiles all cases.
    pub case_filter: Option<String>,
    /// Optional Samply sampling rate in Hz; `None` uses Samply's default.
    pub samply_rate_hz: Option<f64>,
    /// Whether to pass `--presymbolicate` to Samply.
    pub presymbolicate: bool,
}

impl ProfileOptions {
    /// Create default profile options (terse filter, all cases, Samply defaults).
    pub fn new() -> Self {
        Self {
            filter: ProfileFilterMode::default(),
            case_filter: None,
            samply_rate_hz: None,
            presymbolicate: false,
        }
    }
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of parsing profiling arguments.
///
/// Distinguishes between a help request, a successful parse, and an error.
pub enum ProfileParseResult {
    /// User requested `--help`; the contained string should be printed.
    Help(String),
    /// Successfully parsed profile options.
    Options(ProfileOptions),
    /// Parsing failed; the contained string is the error message.
    Error(String),
}

/// Parse the arguments following `bench-profile` into profile options.
///
/// Accepts these forms:
/// - `bench-profile` (default: terse, all cases)
/// - `bench-profile terse|normal|deep|raw-index` (positional filter)
/// - `bench-profile --filter <mode>`
/// - `bench-profile --case <case-id>`
/// - `bench-profile --rate <positive-number>`
/// - `bench-profile --presymbolicate`
/// - `bench-profile --help`
///
/// Flags can be combined. Unknown flags and duplicate options are rejected.
pub fn parse_profile_args(args: &[&str]) -> ProfileParseResult {
    let usage = profile_usage_message();

    if args.contains(&"--help") {
        return ProfileParseResult::Help(usage);
    }

    let mut options = ProfileOptions::new();
    let mut seen_filter = false;
    let mut seen_case = false;
    let mut seen_rate = false;
    let mut positional_count = 0;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index];

        match arg {
            "--filter" => {
                if seen_filter {
                    return ProfileParseResult::Error("Duplicate --filter flag.".to_string());
                }
                seen_filter = true;

                let value = match args.get(index + 1) {
                    Some(v) => *v,
                    None => {
                        return ProfileParseResult::Error(
                            "--filter requires a value (terse, normal, deep, raw-index)"
                                .to_string(),
                        );
                    }
                };

                match ProfileFilterMode::parse(value) {
                    Some(mode) => options.filter = mode,
                    None => {
                        return ProfileParseResult::Error(format!(
                            "Unknown filter '{}'. Valid filters: terse, normal, deep, raw-index",
                            value
                        ));
                    }
                }

                index += 2;
            }

            "--case" => {
                if seen_case {
                    return ProfileParseResult::Error("Duplicate --case flag.".to_string());
                }
                seen_case = true;

                let value = match args.get(index + 1) {
                    Some(v) => *v,
                    None => {
                        return ProfileParseResult::Error("--case requires a case ID".to_string());
                    }
                };

                options.case_filter = Some(value.to_string());
                index += 2;
            }

            "--rate" => {
                if seen_rate {
                    return ProfileParseResult::Error("Duplicate --rate flag.".to_string());
                }
                seen_rate = true;

                let value = match args.get(index + 1) {
                    Some(v) => *v,
                    None => {
                        return ProfileParseResult::Error(
                            "--rate requires a positive number (Hz)".to_string(),
                        );
                    }
                };

                match value.parse::<f64>() {
                    Ok(rate) if rate.is_finite() && rate > 0.0 => {
                        options.samply_rate_hz = Some(rate);
                    }
                    _ => {
                        return ProfileParseResult::Error(format!(
                            "Invalid sampling rate '{}'. Must be a positive finite number.",
                            value
                        ));
                    }
                }

                index += 2;
            }

            "--presymbolicate" => {
                options.presymbolicate = true;
                index += 1;
            }

            // Positional filter mode (e.g., `bench-profile terse`)
            other if !other.starts_with('-') => {
                positional_count += 1;

                if positional_count > 1 {
                    return ProfileParseResult::Error(format!(
                        "Unexpected argument '{}'. Only one positional filter mode is allowed.",
                        other
                    ));
                }

                // A positional filter is only valid as the first argument.
                if index != 0 {
                    return ProfileParseResult::Error(format!(
                        "Unexpected argument '{}'. Filter mode must be the first argument.",
                        other
                    ));
                }

                match ProfileFilterMode::parse(other) {
                    Some(mode) => options.filter = mode,
                    None => {
                        return ProfileParseResult::Error(format!(
                            "Unknown filter '{}'. Valid filters: terse, normal, deep, raw-index",
                            other
                        ));
                    }
                }

                index += 1;
            }

            other => {
                return ProfileParseResult::Error(format!(
                    "Unknown argument '{}'.\n\n{}",
                    other, usage
                ));
            }
        }
    }

    ProfileParseResult::Options(options)
}

/// Generate the usage message for `bench-profile`.
fn profile_usage_message() -> String {
    "\
Usage: xtask bench-profile [filter] [options]

Filters (positional or --filter):
  terse       Agent-first default (top 8 functions, top 3 cases)
  normal      Human + agent investigation (top 20 functions, top 8 cases)
  deep        Pre-refactor investigation (top 50 functions, all cases, edges)
  raw-index   Artifact generation only (raw profile, no hotspot parsing)

Options:
  --filter <mode>        Set filter mode (default: terse)
  --case <name>          Profile only the named case
  --rate <hz>            Samply sampling rate in Hz (must be positive)
  --presymbolicate       Pass --presymbolicate to Samply
  --help                 Show this help message"
        .to_string()
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
