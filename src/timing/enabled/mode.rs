//! Timer and counter output-mode selection.
//!
//! WHAT: owns parsing of the `MOTH_TIMERS` and `MOTH_COUNTERS` environment
//!      variables and the emission predicates shared by the collector and
//!      renderers.
//! WHY:  one small owner keeps mode policy out of collector state so output
//!       policy can change without touching observation storage.

/// Output mode controlling how timing information reaches the user.
///
/// Parsed from the `MOTH_TIMERS` environment variable. When unset,
/// `detailed_timers` defaults to `Verbose` (preserving existing behavior),
/// while `timers` alone defaults to `Summary`.
#[cfg(feature = "timers")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerOutputMode {
    /// No timing output at all.
    Silent,
    /// Concise human-readable summary printed after compilation.
    Summary,
    /// Stable machine-readable `MOTH_BENCH timing` lines for benchmark parsing.
    Bench,
    /// Both human prose and stable benchmark lines.
    Verbose,
}

#[cfg(feature = "timers")]
impl TimerOutputMode {
    /// Parse the output mode from the `MOTH_TIMERS` environment variable.
    ///
    /// When `MOTH_TIMERS` is unset, `detailed_timers` defaults to `Verbose`
    /// (backward compatible) and `timers` alone defaults to `Summary`.
    pub(crate) fn from_env() -> Self {
        match std::env::var("MOTH_TIMERS").as_deref() {
            Ok("silent") | Ok("none") | Ok("off") => Self::Silent,
            Ok("summary") => Self::Summary,
            Ok("bench") => Self::Bench,
            Ok("verbose") | Ok("full") => Self::Verbose,
            _ => {
                // Preserve existing detailed_timers behavior: verbose by default.
                // Timers-only builds default to a concise summary.
                #[cfg(feature = "detailed_timers")]
                {
                    Self::Verbose
                }
                #[cfg(not(feature = "detailed_timers"))]
                {
                    Self::Summary
                }
            }
        }
    }

    /// Whether stable `MOTH_BENCH timing` lines should be printed.
    pub(crate) fn emits_bench_lines(self) -> bool {
        matches!(self, Self::Bench | Self::Verbose)
    }

    /// Whether a human-readable summary should be printed.
    pub(crate) fn emits_summary(self) -> bool {
        matches!(self, Self::Summary | Self::Verbose)
    }

    /// Whether human timer prose should be printed inline during compilation.
    pub(crate) fn emits_human_prose(self) -> bool {
        matches!(self, Self::Verbose)
    }
}
