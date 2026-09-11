//! Runtime timing configuration and lock-free active-channel state.
//!
//! WHAT: parses process-level timing settings once, derives explicit session
//! channels, and exposes the atomic predicates used by timing hot paths.
//! WHY: timing mode policy must not require a mutex or an environment lookup
//! for every stage, and disabled channels must avoid both clock reads and
//! collector locks while preserving the production expression.

#[cfg(feature = "benchmark_counters")]
use crate::timing::CounterOutputMode;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, Ordering};

use super::schema::{TimingLevel, TimingMetric};
use super::session::TimingCommandKind;
// WHAT: the test build routes admission observation into a child module that can reach private
//       runtime atomics; the shipping fallback keeps the hot path branch-free.
// WHY: `admitted_attribution_*` tests must pause one admitted recorder without exposing
//      synchronization state as production API.
#[cfg(test)]
#[path = "runtime_test_support.rs"]
pub(crate) mod test_support;

#[cfg(not(test))]
mod test_support {
    #[inline(always)]
    pub(super) fn observe_record_admission() {}
}

/// Output mode controlling how timing information reaches the user.
///
/// `MOTH_TIMERS` selects this mode once per process. The parsing function
/// accepts the raw environment value directly so tests do not mutate global
/// process configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerOutputMode {
    /// No timing collection or timing output.
    Silent,
    /// A concise human-readable report after a command completes.
    Summary,
    /// Stable machine-readable timing lines without human timing prose.
    Bench,
    /// Detailed substage evidence, stable benchmark lines and the concise report.
    Verbose,
}

impl TimerOutputMode {
    /// Parse one optional `MOTH_TIMERS` value without reading process state.
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("silent" | "none" | "off") => Self::Silent,
            Some("summary") => Self::Summary,
            Some("bench") => Self::Bench,
            Some("verbose" | "full") => Self::Verbose,
            _ => {
                // Preserve detailed-timer developer output while timers-only
                // builds default to the concise report.
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

    /// Read and parse `MOTH_TIMERS` once while process configuration starts.
    fn from_environment() -> Self {
        let value = std::env::var("MOTH_TIMERS").ok();
        Self::parse(value.as_deref())
    }

    /// Whether the command renders the timing report after collection.
    pub(crate) fn emits_summary(self) -> bool {
        matches!(self, Self::Summary | Self::Verbose)
    }
}

/// Explicit collection and presentation channels for one session.
///
/// The representation remains a compact bitset so the enabled hot path can
/// read each predicate atomically without allocating or locking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TimingChannels {
    bits: u16,
}

impl TimingChannels {
    const METRICS: u16 = 1 << 0;
    const COUNTERS: u16 = 1 << 1;
    const ATTRIBUTION: u16 = 1 << 2;
    const DETAILED: u16 = 1 << 3;
    const BENCH_OUTPUT: u16 = 1 << 4;
    const HUMAN_SUMMARY: u16 = 1 << 5;

    const fn empty() -> Self {
        Self { bits: 0 }
    }

    const fn with(self, bit: u16) -> Self {
        Self {
            bits: self.bits | bit,
        }
    }

    /// Whether stable timing metrics are collected.
    pub(crate) const fn metrics(self) -> bool {
        self.bits & Self::METRICS != 0
    }

    /// Whether benchmark counters are collected.
    pub(crate) const fn counters(self) -> bool {
        self.bits & Self::COUNTERS != 0
    }

    /// Whether boundary and module metadata is retained.
    pub(crate) const fn attribution(self) -> bool {
        self.bits & Self::ATTRIBUTION != 0
    }

    /// Whether detailed schema metric evidence is active.
    #[cfg(test)]
    pub(crate) const fn detailed(self) -> bool {
        self.bits & Self::DETAILED != 0
    }

    /// Whether timing benchmark lines are emitted.
    pub(crate) const fn bench_output(self) -> bool {
        self.bits & Self::BENCH_OUTPUT != 0
    }

    /// Whether any human summary is requested after the command.
    pub(crate) const fn human_summary(self) -> bool {
        self.bits & Self::HUMAN_SUMMARY != 0
    }

    /// Whether any event collection channel is active.
    pub(crate) const fn has_collection(self) -> bool {
        self.metrics() || self.counters()
    }

    /// Whether this captured channel policy collects the supplied schema metric.
    const fn metric_active(self, metric: TimingMetric) -> bool {
        if !self.metrics() {
            return false;
        }

        match metric.descriptor().level {
            TimingLevel::Basic => true,
            TimingLevel::Detailed => self.detailed_enabled(),
        }
    }

    const fn detailed_enabled(self) -> bool {
        self.bits & Self::DETAILED != 0
    }
}

/// Stable policy captured when one recorder enters the active session.
///
/// Finish deactivates the process-wide fast-path bits before waiting for
/// admitted recorders. Carrying the session policy through the admission
/// window prevents a recorder that already passed the generation check from
/// observing the drained session's cleared command, attribution or output
/// bits.
#[derive(Debug)]
pub(crate) struct TimingRecordAdmission {
    session: u64,
    channels: TimingChannels,
    command: Option<TimingCommandKind>,
    output_suppressed: bool,
}

impl TimingRecordAdmission {
    pub(crate) const fn session(&self) -> u64 {
        self.session
    }

    pub(crate) const fn metric_active(&self, metric: TimingMetric) -> bool {
        if !self.channels.metric_active(metric) {
            return false;
        }

        match self.command {
            Some(command) => metric.applies_to(command),
            None => true,
        }
    }

    pub(crate) const fn attribution_active(&self) -> bool {
        self.channels.attribution()
    }

    pub(crate) const fn output_suppressed(&self) -> bool {
        self.output_suppressed
    }
}

impl Drop for TimingRecordAdmission {
    fn drop(&mut self) {
        end_record();
    }
}

/// Immutable mode and channel configuration owned by one timing session.
///
/// Command sessions receive the cached process configuration. Raw benchmark
/// callers construct an explicit configuration so they never depend on or
/// mutate global output-mode state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimingSessionConfiguration {
    channels: TimingChannels,
    timer_mode: TimerOutputMode,
    #[cfg(feature = "benchmark_counters")]
    counter_mode: CounterOutputMode,
    suppress_output: bool,
}

impl TimingSessionConfiguration {
    /// Derive a command configuration from the process-wide parsed settings.
    fn for_command(runtime: TimingRuntimeConfig) -> Self {
        let timer_channels = match runtime.timer_mode {
            TimerOutputMode::Silent => TimingChannels::empty(),
            TimerOutputMode::Summary => TimingChannels::empty()
                .with(TimingChannels::METRICS)
                .with(TimingChannels::ATTRIBUTION)
                .with(TimingChannels::HUMAN_SUMMARY),
            TimerOutputMode::Bench => TimingChannels::empty()
                .with(TimingChannels::METRICS)
                .with(TimingChannels::DETAILED)
                .with(TimingChannels::BENCH_OUTPUT),
            TimerOutputMode::Verbose => TimingChannels::empty()
                .with(TimingChannels::METRICS)
                .with(TimingChannels::ATTRIBUTION)
                .with(TimingChannels::DETAILED)
                .with(TimingChannels::BENCH_OUTPUT)
                .with(TimingChannels::HUMAN_SUMMARY),
        };

        #[cfg(feature = "benchmark_counters")]
        let channels = {
            if runtime.counter_mode == CounterOutputMode::Off {
                timer_channels
            } else {
                let channels = timer_channels.with(TimingChannels::COUNTERS);
                if runtime.counter_mode.emits_counter_summary() {
                    channels.with(TimingChannels::HUMAN_SUMMARY)
                } else {
                    channels
                }
            }
        };

        #[cfg(not(feature = "benchmark_counters"))]
        let channels = timer_channels;

        Self {
            channels,
            timer_mode: runtime.timer_mode,
            #[cfg(feature = "benchmark_counters")]
            counter_mode: runtime.counter_mode,
            suppress_output: false,
        }
    }

    /// Configure a caller-owned raw benchmark collection.
    pub(crate) fn raw_benchmark(suppress_output: bool, attribution: bool) -> Self {
        let mut channels = TimingChannels::empty()
            .with(TimingChannels::METRICS)
            .with(TimingChannels::DETAILED);
        if attribution {
            channels = channels.with(TimingChannels::ATTRIBUTION);
        }
        if !suppress_output {
            channels = channels.with(TimingChannels::BENCH_OUTPUT);
        }

        #[cfg(feature = "benchmark_counters")]
        {
            channels = channels.with(TimingChannels::COUNTERS);
        }

        Self {
            channels,
            timer_mode: if suppress_output {
                TimerOutputMode::Silent
            } else {
                TimerOutputMode::Bench
            },
            #[cfg(feature = "benchmark_counters")]
            counter_mode: if suppress_output {
                CounterOutputMode::Off
            } else {
                current_runtime_configuration().counter_mode
            },
            suppress_output,
        }
    }

    /// The channels active for the session.
    pub(crate) const fn channels(self) -> TimingChannels {
        self.channels
    }

    /// Whether this session owns any collector storage.
    pub(crate) const fn has_collection(self) -> bool {
        self.channels.has_collection()
    }

    /// The timer output mode that owns command summary rendering.
    pub(crate) const fn timer_mode(self) -> TimerOutputMode {
        self.timer_mode
    }

    /// Whether the session suppresses terminal output for caller-owned APIs.
    pub(crate) const fn suppress_output(self) -> bool {
        self.suppress_output
    }

    #[cfg(feature = "benchmark_counters")]
    /// The counter output policy active for this session.
    pub(crate) const fn counter_mode(self) -> CounterOutputMode {
        self.counter_mode
    }

    #[cfg(test)]
    pub(crate) fn for_test(timer_mode: TimerOutputMode) -> Self {
        Self::for_command(TimingRuntimeConfig {
            timer_mode,
            #[cfg(feature = "benchmark_counters")]
            counter_mode: CounterOutputMode::Off,
        })
    }

    #[cfg(all(test, feature = "benchmark_counters"))]
    pub(crate) fn for_test_with_counters(
        timer_mode: TimerOutputMode,
        counter_mode: CounterOutputMode,
    ) -> Self {
        Self::for_command(TimingRuntimeConfig {
            timer_mode,
            counter_mode,
        })
    }

    fn active_bits(self) -> u16 {
        #[cfg(feature = "benchmark_counters")]
        {
            let mut bits = self.channels.bits;
            if self.counter_mode.emits_bench_counter_lines() {
                bits |= ACTIVE_COUNTER_BENCH_OUTPUT;
            }
            bits
        }

        #[cfg(not(feature = "benchmark_counters"))]
        self.channels.bits
    }
}

/// Parsed process configuration reused by ordinary command sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimingRuntimeConfig {
    timer_mode: TimerOutputMode,
    #[cfg(feature = "benchmark_counters")]
    counter_mode: CounterOutputMode,
}

impl TimingRuntimeConfig {
    fn from_environment() -> Self {
        Self {
            timer_mode: TimerOutputMode::from_environment(),
            #[cfg(feature = "benchmark_counters")]
            counter_mode: CounterOutputMode::from_environment(),
        }
    }
}

static PROCESS_CONFIGURATION: OnceLock<TimingRuntimeConfig> = OnceLock::new();

/// Return the immutable process configuration after one environment parse.
fn current_runtime_configuration() -> TimingRuntimeConfig {
    *PROCESS_CONFIGURATION.get_or_init(TimingRuntimeConfig::from_environment)
}

/// Build the session configuration for one ordinary command invocation.
pub(crate) fn command_session_configuration() -> TimingSessionConfiguration {
    TimingSessionConfiguration::for_command(current_runtime_configuration())
}

#[cfg(feature = "benchmark_counters")]
const ACTIVE_COUNTER_BENCH_OUTPUT: u16 = 1 << 7;

static ACTIVE_CHANNEL_BITS: AtomicU16 = AtomicU16::new(0);
static ACTIVE_COMMAND_KIND: AtomicU8 = AtomicU8::new(0);
static ACTIVE_SESSION_ID: AtomicU64 = AtomicU64::new(0);
static ACTIVE_RECORDERS: AtomicU64 = AtomicU64::new(0);
static ACTIVE_OUTPUT_SUPPRESSED: AtomicBool = AtomicBool::new(false);

/// Publish a newly owned session's active channels before compiler work begins.
pub(crate) fn activate_session(
    id: u64,
    command: Option<TimingCommandKind>,
    configuration: TimingSessionConfiguration,
) {
    ACTIVE_COMMAND_KIND.store(command_code(command), Ordering::Release);
    ACTIVE_OUTPUT_SUPPRESSED.store(configuration.suppress_output(), Ordering::Relaxed);
    ACTIVE_CHANNEL_BITS.store(configuration.active_bits(), Ordering::Release);
    ACTIVE_SESSION_ID.store(id, Ordering::Release);
}

/// Clear active channels after the matching session stops or drops.
pub(crate) fn deactivate_session() {
    ACTIVE_SESSION_ID.store(0, Ordering::Release);
    ACTIVE_COMMAND_KIND.store(0, Ordering::Release);
    ACTIVE_CHANNEL_BITS.store(0, Ordering::Release);
    ACTIVE_OUTPUT_SUPPRESSED.store(false, Ordering::Relaxed);
}

/// Begin one lock-free record admission window for the current session.
pub(crate) fn begin_record() -> Option<TimingRecordAdmission> {
    loop {
        let session = ACTIVE_SESSION_ID.load(Ordering::Acquire);
        if session == 0 {
            return None;
        }

        let channel_bits = ACTIVE_CHANNEL_BITS.load(Ordering::Acquire);
        let command_code = ACTIVE_COMMAND_KIND.load(Ordering::Acquire);
        let output_suppressed = ACTIVE_OUTPUT_SUPPRESSED.load(Ordering::Relaxed);
        ACTIVE_RECORDERS.fetch_add(1, Ordering::Acquire);
        if ACTIVE_SESSION_ID.load(Ordering::Acquire) == session
            && ACTIVE_CHANNEL_BITS.load(Ordering::Acquire) == channel_bits
            && ACTIVE_COMMAND_KIND.load(Ordering::Acquire) == command_code
        {
            let admission = TimingRecordAdmission {
                session,
                channels: TimingChannels { bits: channel_bits },
                command: command_from_code(command_code),
                output_suppressed,
            };
            // The child observes this exact post-validation point; its shipping counterpart is a
            // no-op, so admission remains a lock-free production path.
            test_support::observe_record_admission();
            return Some(admission);
        }

        ACTIVE_RECORDERS.fetch_sub(1, Ordering::Release);
        if ACTIVE_SESSION_ID.load(Ordering::Acquire) == 0 {
            return None;
        }
    }
}

/// Admit one metric before its timing clock starts.
pub(crate) fn begin_metric_record(metric: TimingMetric) -> Option<TimingRecordAdmission> {
    if !metric_active(metric) {
        return None;
    }

    let admission = begin_record()?;
    if admission.metric_active(metric) {
        Some(admission)
    } else {
        None
    }
}

/// End one lock-free record admission window.
pub(crate) fn end_record() {
    ACTIVE_RECORDERS.fetch_sub(1, Ordering::Release);
}

/// Wait until records admitted before deactivation have completed.
pub(crate) fn wait_for_records() {
    while ACTIVE_RECORDERS.load(Ordering::Acquire) != 0 {
        std::hint::spin_loop();
    }
}

fn channel_active(bit: u16) -> bool {
    ACTIVE_CHANNEL_BITS.load(Ordering::Acquire) & bit != 0
}

/// Whether a counter needs collector storage.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn counters_active() -> bool {
    channel_active(TimingChannels::COUNTERS)
}

/// Whether boundary or module metadata should be registered.
pub(crate) fn attribution_active() -> bool {
    channel_active(TimingChannels::ATTRIBUTION)
}

/// Whether the active session collects the supplied schema metric.
pub(crate) fn metric_active(metric: TimingMetric) -> bool {
    let channels = TimingChannels {
        bits: ACTIVE_CHANNEL_BITS.load(Ordering::Acquire),
    };
    if !channels.metric_active(metric) {
        return false;
    }

    match command_from_code(ACTIVE_COMMAND_KIND.load(Ordering::Acquire)) {
        Some(command) => metric.applies_to(command),
        None => true,
    }
}

const fn command_code(command: Option<TimingCommandKind>) -> u8 {
    match command {
        None => 0,
        Some(TimingCommandKind::Build) => 1,
        Some(TimingCommandKind::Check) => 2,
        Some(TimingCommandKind::Dev) => 3,
    }
}

fn command_from_code(code: u8) -> Option<TimingCommandKind> {
    match code {
        0 => None,
        1 => Some(TimingCommandKind::Build),
        2 => Some(TimingCommandKind::Check),
        3 => Some(TimingCommandKind::Dev),
        _ => unreachable!("invalid active timing command code"),
    }
}

/// Whether a counter output mode requests stable benchmark counter lines.
/// Only used in counter-only builds (no timers); with timers, counter lines
/// are emitted from the drained snapshot.
#[cfg(all(feature = "benchmark_counters", not(feature = "timers")))]
pub(crate) fn counter_bench_output_active() -> bool {
    channel_active(ACTIVE_COUNTER_BENCH_OUTPUT)
}

/// Observe one timing clock read for optimization tests.
///
/// WHY: the enabled guard reaches this narrow bridge immediately after admission; keeping the
/// counter storage in the test child module preserves the no-clock-on-inactive-metric assertion
/// without adding test state to the runtime.
#[cfg(test)]
pub(crate) fn observe_timing_clock_read() {
    test_support::record_timing_clock_read();
}

#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod runtime_tests;
