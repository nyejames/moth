//! Enabled-collector tests for the timer facade.
//!
//! WHAT: proves that with `timers` active the facade macros record one
//!      observation per wrapped stage and never change production values.
//! WHY:  the enabled expansion must mirror the disabled one for control flow
//!       while adding collector evidence.

use crate::timing::{start_benchmark_collection, stop_and_collect_benchmark_observations};
use std::cell::Cell;

/// Serializes collector tests against every other collector test in the
/// process: the collector is one process-global scope, so parallel tests would
/// replace each other's in-flight collections.
fn collector_test_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::compiler_frontend::instrumentation::lock_counter_test()
}

#[test]
fn pipeline_timer_records_one_observation_and_runs_expression_once() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let runs = Cell::new(0);
    let value = pipeline_timer!("test.metric", {
        runs.set(runs.get() + 1);
        42
    });

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
    assert_eq!(snapshot.timings.len(), 1);
    assert_eq!(snapshot.timings[0].name, "test.metric");
}

#[test]
fn labeled_pipeline_timer_records_observation_and_passes_value_through() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let value = labeled_pipeline_timer!("test.labeled", "prose label ", 7);

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(value, 7);
    assert_eq!(snapshot.timings.len(), 1);
    assert_eq!(snapshot.timings[0].name, "test.labeled");
}

#[test]
fn timing_guard_records_observation_when_scope_ends() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    {
        timing_guard!("test.guard");
    }

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(snapshot.timings.len(), 1);
    assert_eq!(snapshot.timings[0].name, "test.guard");
}

#[test]
fn timed_manual_finish_records_started_stage() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish!("test.manual", start);

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(snapshot.timings.len(), 1);
    assert_eq!(snapshot.timings[0].name, "test.manual");
}

#[test]
fn timed_manual_finish_labeled_stores_attribution() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish_labeled!("test.labeled_manual", start, Some("slowest module"));

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(snapshot.timings.len(), 1);
    assert_eq!(snapshot.timings[0].name, "test.labeled_manual");
    assert_eq!(snapshot.timings[0].label.as_deref(), Some("slowest module"));
}
