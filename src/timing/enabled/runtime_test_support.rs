//! Test-only synchronization and clock-observation support for the timing runtime.
//!
//! WHAT: observes the one production admission hook, coordinates drain-window tests, and counts
//!       clock reads without placing test state in the shipping runtime.
//! WHY: admission and clock assertions need access to private runtime state, while production must
//!      retain only a zero-cost observer seam and no test-owned storage.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

static TIMING_CLOCK_READS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn record_timing_clock_read() {
    TIMING_CLOCK_READS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn reset_timing_clock_reads_for_test() {
    TIMING_CLOCK_READS.store(0, Ordering::Relaxed);
}

pub(crate) fn timing_clock_reads_for_test() -> usize {
    TIMING_CLOCK_READS.load(Ordering::Relaxed)
}

/// The transient drain state a collector test can wait on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordAdmissionState {
    /// The targeted recorder must stop after admission until the test releases it.
    pub(crate) paused: bool,
    /// The targeted recorder reached the pause point inside its admission window.
    pub(crate) admission_reached: bool,
    /// A session cleared its fast-path bits and is now draining admitted recorders.
    pub(crate) session_deactivated: bool,
}

impl std::fmt::Display for RecordAdmissionState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "paused={}, admission_reached={}, session_deactivated={}",
            self.paused, self.admission_reached, self.session_deactivated
        )
    }
}

static RECORD_ADMISSION_STATE: Mutex<RecordAdmissionState> = Mutex::new(RecordAdmissionState {
    paused: false,
    admission_reached: false,
    session_deactivated: false,
});

static RECORD_ADMISSION_SIGNAL: Condvar = Condvar::new();

/// Deadlock protection for the drain waits, never the synchronization itself.
const RECORD_ADMISSION_WAIT_DEADLINE: Duration = Duration::from_secs(30);

thread_local! {
    /// Marks the one recorder thread selected by a drain-synchronization test.
    ///
    /// A process-global pause can accidentally capture unrelated parallel tests that also record
    /// timings. Keeping targeting thread-local makes the synchronization hook observational for
    /// every other test thread.
    static RECORD_ADMISSION_PAUSE_TARGET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn lock_record_admission_state() -> std::sync::MutexGuard<'static, RecordAdmissionState> {
    RECORD_ADMISSION_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Apply one state change and wake every waiter.
fn publish_record_admission_state(update: impl FnOnce(&mut RecordAdmissionState)) {
    update(&mut lock_record_admission_state());
    RECORD_ADMISSION_SIGNAL.notify_all();
}

/// Wait until `reached` holds, or report the state the wait gave up on.
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

pub(crate) struct RecordAdmissionPauseGuard;

impl RecordAdmissionPauseGuard {
    pub(crate) fn release(&self) {
        publish_record_admission_state(|state| state.paused = false);
    }
}

impl Drop for RecordAdmissionPauseGuard {
    fn drop(&mut self) {
        self.release();
    }
}

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
pub(crate) fn wait_for_paused_record_admission_for_test() -> Result<(), RecordAdmissionState> {
    wait_for_record_admission_state(|state| state.admission_reached)
}

/// Block until a session has cleared its fast-path bits and started draining.
///
/// The production deactivation transition is an atomic fast-path write with no test notification.
/// This child module can observe that private generation directly, so the shipping path needs no
/// second synchronization callback. Short condition-variable waits avoid an unbounded spin while
/// retaining a bounded failure for a broken ordering.
pub(crate) fn wait_for_session_deactivation_for_test() -> Result<(), RecordAdmissionState> {
    let deadline = Instant::now() + RECORD_ADMISSION_WAIT_DEADLINE;
    loop {
        if super::ACTIVE_SESSION_ID.load(Ordering::Acquire) == 0 {
            publish_record_admission_state(|state| state.session_deactivated = true);
            return Ok(());
        }

        let state = lock_record_admission_state();
        if state.session_deactivated {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(*state);
        }
        let wait_for = remaining.min(Duration::from_millis(5));
        let (state, wait_result) = RECORD_ADMISSION_SIGNAL
            .wait_timeout(state, wait_for)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if wait_result.timed_out() && Instant::now() >= deadline {
            return Err(*state);
        }
    }
}

/// Mark the recorder on the current thread as the one selected by a drain test.
pub(crate) fn target_record_admission_pause_for_current_thread() {
    RECORD_ADMISSION_PAUSE_TARGET.with(|target| target.set(true));
}

/// Observe one successful production admission and park the selected recorder if requested.
pub(super) fn observe_record_admission() {
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
