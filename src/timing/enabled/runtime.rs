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
    #[cfg(test)]
    note_session_deactivated_for_test(false);
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
    #[cfg(test)]
    note_session_deactivated_for_test(true);
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
            #[cfg(test)]
            pause_after_record_admission_for_test();
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

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

#[cfg(test)]
static TIMING_CLOCK_READS: AtomicUsize = AtomicUsize::new(0);

/// Record one timer-clock read in tests without adding production work.
#[cfg(test)]
pub(crate) fn record_timing_clock_read_for_test() {
    TIMING_CLOCK_READS.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn reset_timing_clock_reads_for_test() {
    TIMING_CLOCK_READS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn timing_clock_reads_for_test() -> usize {
    TIMING_CLOCK_READS.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
//  Drain-synchronization seam for collector tests
// ---------------------------------------------------------------------------
//
// Session drain has two transient orderings a test must observe: a recorder
// that has been admitted but has not published its observation, and a session
// that has cleared its fast-path bits but has not finished draining. A test
// stops the targeted recorder inside that window and waits for the matching
// state.
//
// The seam is a condition variable, not atomics plus `yield_now`: a spin count
// is a CPU-speed timeout rather than synchronization, and it reports nothing
// about the state it gave up on. The wait deadline below exists only so a
// broken ordering fails instead of hanging the suite.

#[cfg(test)]
use std::sync::{Condvar, Mutex};
#[cfg(test)]
use std::time::Duration;

/// The transient drain state a collector test can wait on.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordAdmissionState {
    /// The targeted recorder must stop after admission until the test releases it.
    pub(crate) paused: bool,
    /// The targeted recorder reached the pause point inside its admission window.
    pub(crate) admission_reached: bool,
    /// A session cleared its fast-path bits and is now draining admitted recorders.
    pub(crate) session_deactivated: bool,
}

#[cfg(test)]
impl std::fmt::Display for RecordAdmissionState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "paused={}, admission_reached={}, session_deactivated={}",
            self.paused, self.admission_reached, self.session_deactivated
        )
    }
}

#[cfg(test)]
static RECORD_ADMISSION_STATE: Mutex<RecordAdmissionState> = Mutex::new(RecordAdmissionState {
    paused: false,
    admission_reached: false,
    session_deactivated: false,
});

#[cfg(test)]
static RECORD_ADMISSION_SIGNAL: Condvar = Condvar::new();

/// Deadlock protection for the drain waits, never the synchronization itself.
#[cfg(test)]
const RECORD_ADMISSION_WAIT_DEADLINE: Duration = Duration::from_secs(30);

#[cfg(test)]
thread_local! {
    /// Marks the one recorder thread selected by a drain-synchronization test.
    ///
    /// A process-global pause can accidentally capture unrelated parallel tests that also record
    /// timings. Keeping targeting thread-local makes the synchronization hook observational for
    /// every other test thread.
    static RECORD_ADMISSION_PAUSE_TARGET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn lock_record_admission_state() -> std::sync::MutexGuard<'static, RecordAdmissionState> {
    RECORD_ADMISSION_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Apply one state change and wake every waiter.
#[cfg(test)]
fn publish_record_admission_state(update: impl FnOnce(&mut RecordAdmissionState)) {
    update(&mut lock_record_admission_state());
    RECORD_ADMISSION_SIGNAL.notify_all();
}

/// Wait until `reached` holds, or report the state the wait gave up on.
#[cfg(test)]
fn wait_for_record_admission_state(
    reached: impl Fn(&RecordAdmissionState) -> bool,
) -> Result<(), RecordAdmissionState> {
    let (state, wait_result) = RECORD_ADMISSION_SIGNAL
        .wait_timeout_while(
            lock_record_admission_state(),
            RECORD_ADMISSION_WAIT_DEADLINE,
            |state| !reached(state),
        )
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if wait_result.timed_out() {
        Err(*state)
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct RecordAdmissionPauseGuard;

#[cfg(test)]
impl RecordAdmissionPauseGuard {
    pub(crate) fn release(&self) {
        publish_record_admission_state(|state| state.paused = false);
    }
}

#[cfg(test)]
impl Drop for RecordAdmissionPauseGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
pub(crate) fn pause_record_admission_for_test() -> RecordAdmissionPauseGuard {
    publish_record_admission_state(|state| {
        *state = RecordAdmissionState {
            paused: true,
            admission_reached: false,
            session_deactivated: false,
        };
    });
    RecordAdmissionPauseGuard
}

/// Block until the targeted recorder is parked inside its admission window.
#[cfg(test)]
pub(crate) fn wait_for_paused_record_admission_for_test() -> Result<(), RecordAdmissionState> {
    wait_for_record_admission_state(|state| state.admission_reached)
}

/// Block until a session has cleared its fast-path bits and started draining.
#[cfg(test)]
pub(crate) fn wait_for_session_deactivation_for_test() -> Result<(), RecordAdmissionState> {
    wait_for_record_admission_state(|state| state.session_deactivated)
}

#[cfg(test)]
fn note_session_deactivated_for_test(deactivated: bool) {
    publish_record_admission_state(|state| state.session_deactivated = deactivated);
}

#[cfg(test)]
pub(crate) fn target_record_admission_pause_for_current_thread() {
    RECORD_ADMISSION_PAUSE_TARGET.with(|target| target.set(true));
}

/// Park the targeted recorder after admission until the owning test releases it.
#[cfg(test)]
fn pause_after_record_admission_for_test() {
    if !RECORD_ADMISSION_PAUSE_TARGET.with(|target| target.replace(false)) {
        return;
    }

    let mut state = lock_record_admission_state();
    if !state.paused {
        return;
    }

    state.admission_reached = true;
    RECORD_ADMISSION_SIGNAL.notify_all();
    while state.paused {
        state = RECORD_ADMISSION_SIGNAL
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

#[cfg(test)]
mod tests {
    use super::{TimerOutputMode, TimingSessionConfiguration};

    #[test]
    fn timer_mode_parser_maps_known_values_and_defaults() {
        assert_eq!(
            TimerOutputMode::parse(Some("silent")),
            TimerOutputMode::Silent
        );
        assert_eq!(
            TimerOutputMode::parse(Some("none")),
            TimerOutputMode::Silent
        );
        assert_eq!(TimerOutputMode::parse(Some("off")), TimerOutputMode::Silent);
        assert_eq!(
            TimerOutputMode::parse(Some("summary")),
            TimerOutputMode::Summary
        );
        assert_eq!(
            TimerOutputMode::parse(Some("bench")),
            TimerOutputMode::Bench
        );
        assert_eq!(
            TimerOutputMode::parse(Some("verbose")),
            TimerOutputMode::Verbose
        );
        assert_eq!(
            TimerOutputMode::parse(Some("full")),
            TimerOutputMode::Verbose
        );

        #[cfg(feature = "detailed_timers")]
        assert_eq!(TimerOutputMode::parse(None), TimerOutputMode::Verbose);
        #[cfg(not(feature = "detailed_timers"))]
        assert_eq!(TimerOutputMode::parse(None), TimerOutputMode::Summary);
    }

    #[test]
    fn timer_only_command_channels_follow_the_mode_matrix() {
        let summary = TimingSessionConfiguration::for_test(TimerOutputMode::Summary);
        assert!(summary.channels().metrics());
        assert!(summary.channels().attribution());
        assert!(summary.channels().human_summary());
        assert!(!summary.channels().detailed());
        assert!(!summary.channels().bench_output());

        let verbose = TimingSessionConfiguration::for_test(TimerOutputMode::Verbose);
        assert!(verbose.channels().metrics());
        assert!(verbose.channels().attribution());
        assert!(verbose.channels().detailed());
        assert!(verbose.channels().bench_output());
        assert!(verbose.channels().human_summary());

        let bench = TimingSessionConfiguration::for_test(TimerOutputMode::Bench);
        assert!(bench.channels().metrics());
        assert!(bench.channels().detailed());
        assert!(bench.channels().bench_output());
        assert!(!bench.channels().attribution());
        assert!(!bench.channels().human_summary());

        let silent = TimingSessionConfiguration::for_test(TimerOutputMode::Silent);
        assert!(!silent.has_collection());
        assert!(!silent.channels().metrics());
        assert!(!silent.channels().counters());
    }

    #[test]
    fn raw_benchmark_configuration_owns_metrics_without_attribution() {
        let raw = TimingSessionConfiguration::raw_benchmark(true, false);
        assert!(raw.channels().metrics());
        assert!(raw.channels().detailed());
        assert!(!raw.channels().attribution());
        assert!(raw.suppress_output());
        assert!(!raw.channels().bench_output());
    }

    #[cfg(feature = "benchmark_counters")]
    #[test]
    fn counter_modes_extend_the_session_channel_matrix() {
        use crate::timing::CounterOutputMode;

        let summary = TimingSessionConfiguration::for_test_with_counters(
            TimerOutputMode::Summary,
            CounterOutputMode::Summary,
        );
        assert!(summary.channels().metrics());
        assert!(summary.channels().counters());
        assert!(summary.channels().attribution());
        assert!(summary.channels().human_summary());
        assert_eq!(summary.counter_mode(), CounterOutputMode::Summary);

        let bench = TimingSessionConfiguration::for_test_with_counters(
            TimerOutputMode::Bench,
            CounterOutputMode::Summary,
        );
        assert!(bench.channels().metrics());
        assert!(bench.channels().counters());
        assert!(bench.channels().bench_output());
        assert!(bench.channels().human_summary());
        assert!(!bench.channels().attribution());

        let counter_only = TimingSessionConfiguration::for_test_with_counters(
            TimerOutputMode::Silent,
            CounterOutputMode::Summary,
        );
        assert!(!counter_only.channels().metrics());
        assert!(counter_only.channels().counters());
        assert!(counter_only.channels().human_summary());
        assert!(!counter_only.channels().attribution());

        let full = TimingSessionConfiguration::for_test_with_counters(
            TimerOutputMode::Silent,
            CounterOutputMode::Full,
        );
        assert!(!full.channels().metrics());
        assert!(full.channels().counters());
        assert!(full.channels().human_summary());
        assert_eq!(full.counter_mode(), CounterOutputMode::Full);
    }
}
