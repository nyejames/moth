//! Enabled-collector tests for the timer facade.
//!
//! WHAT: proves that with `timers` active the facade macros record one
//!      observation per wrapped stage and never change production values.
//! WHY:  the enabled expansion must mirror the disabled one for control flow
//!       while adding collector evidence.

use crate::compiler_tests::test_support::assert_panics_with;
use crate::timing::{
    TimingCommandKind, TimingMetric, TimingMetricAggregate, start_benchmark_collection,
    start_raw_benchmark_collection,
};
use std::cell::Cell;

/// Serializes collector tests against every other timing test in the process.
/// The collector is one process-global scope, and the timing suite owns its
/// own test lock rather than borrowing the frontend counter-test lock.
fn collector_test_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::timing::tests::lock_timing_tests()
}

/// Non-zero dense rows for one metric, ignoring zero schema slots.
fn timings_named(
    snapshot: &crate::timing::BenchmarkObservationSnapshot,
    metric: TimingMetric,
) -> Vec<&crate::timing::TimingMetricAggregate> {
    snapshot
        .timings
        .iter()
        .filter(|aggregate| aggregate.metric == metric && aggregate.samples > 0)
        .collect()
}

fn wait_for_timing_flag(mut observed: impl FnMut() -> bool) -> bool {
    for _ in 0..1_000_000 {
        if observed() {
            return true;
        }
        std::thread::yield_now();
    }
    false
}

/// Join a spawned thread and surface its panic instead of discarding it.
///
/// WHAT: joins the handle and prints any panic payload to stderr before returning.
/// WHY: `let _ = handle.join()` silently discards a worker panic. In failure
///   paths where the test is about to panic anyway, the worker's panic must
///   still be visible so the root cause is not hidden.
fn surface_thread_panic<T>(name: &str, handle: std::thread::JoinHandle<T>) {
    if let Err(panic_payload) = handle.join() {
        let msg = panic_payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic_payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("{panic_payload:?}"));
        eprintln!("worker thread '{name}' panicked: {msg}");
    }
}

#[test]
fn final_benchmark_snapshot_uses_schema_order_and_omits_empty_rows() {
    let snapshot = crate::timing::BenchmarkObservationSnapshot {
        schema_version: crate::timing::TIMING_SCHEMA_VERSION,
        timings: vec![
            TimingMetricAggregate {
                metric: TimingMetric::CommandCheckTotal,
                total: std::time::Duration::from_millis(8),
                samples: 1,
            },
            TimingMetricAggregate {
                metric: TimingMetric::FrontendPrepare,
                total: std::time::Duration::from_millis(2),
                samples: 1,
            },
            TimingMetricAggregate {
                metric: TimingMetric::FrontendAstTotal,
                total: std::time::Duration::from_millis(0),
                samples: 0,
            },
        ],
        ..Default::default()
    };

    let lines = crate::timing::enabled::format_bench_timing_snapshot(&snapshot);

    assert_eq!(
        lines,
        vec![
            "MOTH_BENCH timing-schema 2",
            "MOTH_BENCH timing command.check.total=8ms",
            "MOTH_BENCH timing frontend.prepare=2ms",
        ]
    );
}

#[test]
fn timed_stage_records_one_observation_and_runs_expression_once() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    let runs = Cell::new(0);
    let value = timed_stage!(TimingMetric::FrontendPrepare, {
        runs.set(runs.get() + 1);
        42
    });

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
    let observations = timings_named(&snapshot, TimingMetric::FrontendPrepare);
    assert_eq!(observations.len(), 1);
}

#[test]
fn timed_stage_attributed_records_observation_and_passes_value_through() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let value = timed_stage_attributed!(
        TimingMetric::BoundaryInventory,
        Some(crate::timing::TimingContext::for_boundary(boundary)),
        7
    );

    let snapshot = timing_session.finish();

    assert_eq!(value, 7);
    let observations = timings_named(&snapshot, TimingMetric::BoundaryInventory);
    assert_eq!(observations.len(), 1);
}

#[test]
fn timing_guard_records_observation_when_scope_ends() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    {
        timing_scope!(timing_guard, TimingMetric::FrontendOrderDeclarations);
    }

    let snapshot = timing_session.finish();

    let observations = timings_named(&snapshot, TimingMetric::FrontendOrderDeclarations);
    assert_eq!(observations.len(), 1);
}

#[test]
fn timing_scope_records_started_stage() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    {
        timing_scope!(timing_guard, TimingMetric::FrontendAstTotal);
    }

    let snapshot = timing_session.finish();

    let observations = timings_named(&snapshot, TimingMetric::FrontendAstTotal);
    assert_eq!(observations.len(), 1);
}

#[test]
fn finish_timing_scope_records_a_guard_exactly_once() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    {
        timing_scope!(timing_guard, TimingMetric::FrontendAstTotal);
        finish_timing_scope!(timing_guard);
    }

    let snapshot = timing_session.finish();
    assert_eq!(
        timings_named(&snapshot, TimingMetric::FrontendAstTotal).len(),
        1
    );
}

#[test]
fn finished_snapshot_uses_dense_schema_order_with_zero_slots() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");
    record_timing_via_facade(TimingMetric::FrontendPrepare);

    let snapshot = timing_session.finish();

    assert_eq!(
        snapshot
            .timings
            .iter()
            .map(|aggregate| aggregate.metric)
            .collect::<Vec<_>>(),
        TimingMetric::ALL,
        "global snapshots must expose every metric in canonical schema order"
    );
    assert_eq!(
        timings_named(&snapshot, TimingMetric::FrontendPrepare)[0].samples,
        1
    );
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|aggregate| aggregate.metric == TimingMetric::CommandBuildTotal)
            .expect("every schema metric must have a slot")
            .samples,
        0
    );
}

#[test]
fn parallel_timing_records_sum_into_one_atomic_slot() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");
    const WORKERS: usize = 8;
    const RECORDS_PER_WORKER: usize = 100;
    let duration = std::time::Duration::from_nanos(1_000);

    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| {
                for _ in 0..RECORDS_PER_WORKER {
                    crate::timing::record_pipeline_timing(TimingMetric::FrontendPrepare, duration);
                }
            });
        }
    });

    let snapshot = timing_session.finish();
    let aggregate = timings_named(&snapshot, TimingMetric::FrontendPrepare)
        .into_iter()
        .next()
        .expect("parallel records should retain one aggregate row");
    assert_eq!(aggregate.samples, (WORKERS * RECORDS_PER_WORKER) as u64);
    assert_eq!(
        aggregate.total,
        duration * (WORKERS * RECORDS_PER_WORKER) as u32
    );
}

#[test]
fn admitted_attribution_policy_survives_session_drain() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let context = Some(crate::timing::TimingContext::for_boundary(boundary));
    let pause = crate::timing::enabled::runtime::pause_record_admission_for_test();

    let recorder = std::thread::spawn(move || {
        crate::timing::enabled::runtime::target_record_admission_pause_for_current_thread();
        crate::timing::record_pipeline_timing_attributed(
            TimingMetric::BoundaryInventory,
            std::time::Duration::from_millis(7),
            context,
        );
    });
    if !wait_for_timing_flag(|| {
        crate::timing::enabled::runtime::record_admission_reached_for_test()
    }) {
        pause.release();
        surface_thread_panic("recorder", recorder);
        panic!("the recorder should pause after admission");
    }

    let finisher = std::thread::spawn(move || timing_session.finish());
    if !wait_for_timing_flag(|| {
        crate::timing::enabled::runtime::record_session_deactivated_for_test()
    }) {
        pause.release();
        surface_thread_panic("recorder", recorder);
        surface_thread_panic("finisher", finisher);
        panic!("session finish should deactivate the fast-path bits before waiting");
    }

    pause.release();
    recorder
        .join()
        .expect("the admitted recorder should finish cleanly");
    let snapshot = finisher
        .join()
        .expect("session drain must not panic while an admitted recorder finishes");

    let global = timings_named(&snapshot, TimingMetric::BoundaryInventory)
        .into_iter()
        .next()
        .expect("the admitted global observation should be retained");
    let boundary_record = snapshot
        .boundaries
        .iter()
        .find(|record| record.id == boundary)
        .expect("the registered boundary should be retained");
    let attributed = boundary_record
        .timings
        .iter()
        .find(|aggregate| aggregate.metric == TimingMetric::BoundaryInventory)
        .expect("the attributed boundary row should be retained");

    assert_eq!(global.total, attributed.total);
    assert_eq!(global.samples, attributed.samples);
}

#[test]
fn attributed_duration_context_uses_admitted_session_policy() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let pause = crate::timing::enabled::runtime::pause_record_admission_for_test();
    let (context_sender, context_receiver) = std::sync::mpsc::channel();

    let recorder = std::thread::spawn(move || {
        crate::timing::enabled::runtime::target_record_admission_pause_for_current_thread();
        record_attributed_duration!(
            TimingMetric::BoundaryInventory,
            std::time::Duration::from_millis(7),
            {
                context_sender
                    .send(())
                    .expect("the context observer should remain available");
                Some(crate::timing::TimingContext::for_boundary(boundary))
            }
        )
    });
    if !wait_for_timing_flag(|| {
        crate::timing::enabled::runtime::record_admission_reached_for_test()
    }) {
        pause.release();
        surface_thread_panic("recorder", recorder);
        let _ = timing_session.finish();
        panic!("the direct-duration recorder should pause after admission");
    }
    let context_ran_before_admission = context_receiver.try_recv().is_ok();

    let (finish_sender, finish_receiver) = std::sync::mpsc::channel();
    let finisher = std::thread::spawn(move || {
        finish_sender
            .send(timing_session.finish())
            .expect("the finish receiver should remain available");
    });
    if !wait_for_timing_flag(|| {
        crate::timing::enabled::runtime::record_session_deactivated_for_test()
    }) {
        pause.release();
        surface_thread_panic("recorder", recorder);
        surface_thread_panic("finisher", finisher);
        panic!("session finish should deactivate the fast-path bits before waiting");
    }
    assert!(
        matches!(
            finish_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "finish must wait for the admitted direct-duration record"
    );

    pause.release();
    assert!(
        recorder
            .join()
            .expect("the admitted direct-duration recorder should finish cleanly"),
        "the direct-duration facade should return the captured output policy"
    );
    assert!(
        context_receiver.recv().is_ok(),
        "the context should evaluate after admission releases it"
    );
    finisher
        .join()
        .expect("session drain must not panic while direct duration finishes");
    let snapshot = finish_receiver
        .recv()
        .expect("the drained session should publish its snapshot");

    assert!(
        !context_ran_before_admission,
        "direct attributed duration must not evaluate context before admission"
    );
    let global = timings_named(&snapshot, TimingMetric::BoundaryInventory)
        .into_iter()
        .next()
        .expect("the admitted direct-duration observation should be retained");
    let boundary_record = snapshot
        .boundaries
        .iter()
        .find(|record| record.id == boundary)
        .expect("the registered boundary should be retained");
    let attributed = boundary_record
        .timings
        .iter()
        .find(|aggregate| aggregate.metric == TimingMetric::BoundaryInventory)
        .expect("the admitted direct-duration attribution should be retained");
    assert_eq!(global.samples, 1);
    assert_eq!(global.samples, attributed.samples);
}

#[test]
fn admitted_attribution_survives_session_drain() {
    let _test_guard = collector_test_guard();
    let timing_session = start_test_command_session(
        TimingCommandKind::Build,
        crate::timing::TimerOutputMode::Verbose,
    );
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "entry", 1, 128);
    let pause = crate::timing::enabled::runtime::pause_record_admission_for_test();

    let recorder = std::thread::spawn(move || {
        crate::timing::enabled::runtime::target_record_admission_pause_for_current_thread();
        let mut start = crate::timing::start_pipeline_timing(TimingMetric::FrontendBindHeaders);
        crate::timing::record_started_pipeline_timing_attributed(
            TimingMetric::FrontendBindHeaders,
            &mut start,
            Some(crate::timing::TimingContext::for_module(module)),
        )
    });
    if !wait_for_timing_flag(|| {
        crate::timing::enabled::runtime::record_admission_reached_for_test()
    }) {
        pause.release();
        surface_thread_panic("recorder", recorder);
        let _ = timing_session.finish();
        panic!("the recorder should pause after admission");
    }

    let (finish_sender, finish_receiver) = std::sync::mpsc::channel();
    let finisher = std::thread::spawn(move || {
        finish_sender
            .send(timing_session.finish())
            .expect("the finish receiver should remain available");
    });
    if !wait_for_timing_flag(|| {
        crate::timing::enabled::runtime::record_session_deactivated_for_test()
    }) {
        pause.release();
        surface_thread_panic("recorder", recorder);
        surface_thread_panic("finisher", finisher);
        panic!("session finish should deactivate the fast-path bits before waiting");
    }
    assert!(
        matches!(
            finish_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "finish must wait for the pre-clock admitted span"
    );

    pause.release();
    recorder
        .join()
        .expect("the admitted recorder should finish cleanly");
    finisher
        .join()
        .expect("session drain must not panic while an admitted recorder finishes");
    let snapshot = finish_receiver
        .recv()
        .expect("the drained session should publish its snapshot");

    assert_eq!(
        timings_named(&snapshot, TimingMetric::FrontendBindHeaders).len(),
        1
    );
    assert_eq!(
        snapshot
            .modules
            .iter()
            .flat_map(|module| module.timings.iter())
            .find(|aggregate| aggregate.metric == TimingMetric::FrontendBindHeaders)
            .expect("the pre-clock admitted module row should be retained")
            .samples,
        1
    );

    let next_session = start_test_command_session(TimingCommandKind::Build, timer_mode_summary());
    let next_boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "next-project".to_owned(),
    );
    let next_module = crate::timing::register_timing_module(next_boundary, 0, "entry", 1, 128);
    crate::timing::record_pipeline_timing_attributed(
        TimingMetric::FrontendPrepare,
        std::time::Duration::from_millis(3),
        Some(crate::timing::TimingContext::for_module(next_module)),
    );
    let _ = next_session.finish();
}

#[test]
fn attributed_slots_accept_only_registered_metric_kinds() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "entry", 1, 128);

    crate::timing::record_pipeline_timing_attributed(
        TimingMetric::BoundaryInventory,
        std::time::Duration::from_millis(2),
        Some(crate::timing::TimingContext::for_boundary(boundary)),
    );
    crate::timing::record_pipeline_timing_attributed(
        TimingMetric::BoundaryCompile,
        std::time::Duration::from_millis(3),
        Some(crate::timing::TimingContext::for_module(module)),
    );
    crate::timing::record_pipeline_timing_attributed(
        TimingMetric::FrontendPrepare,
        std::time::Duration::from_millis(4),
        Some(crate::timing::TimingContext::for_module(module)),
    );
    crate::timing::record_pipeline_timing_attributed(
        TimingMetric::FrontendPrepare,
        std::time::Duration::from_millis(5),
        Some(crate::timing::TimingContext::for_boundary(boundary)),
    );

    let snapshot = timing_session.finish();
    let boundary_record = snapshot
        .boundaries
        .iter()
        .find(|record| record.id == boundary)
        .expect("registered boundary should be snapshotted");
    let module_record = snapshot
        .modules
        .iter()
        .find(|record| record.key == module)
        .expect("registered module should be snapshotted");

    assert_eq!(
        boundary_record
            .timings
            .iter()
            .find(|aggregate| aggregate.metric == TimingMetric::BoundaryInventory)
            .expect("boundary metric should have a dense slot")
            .samples,
        1
    );
    assert!(boundary_record.timings.iter().all(|aggregate| {
        aggregate.metric != TimingMetric::BoundaryCompile || aggregate.samples == 0
    }));
    assert!(
        timings_named(&snapshot, TimingMetric::BoundaryCompile).is_empty(),
        "a wrong attribution context must not enter global storage"
    );
    assert_eq!(
        module_record
            .timings
            .iter()
            .find(|aggregate| aggregate.metric == TimingMetric::FrontendPrepare)
            .expect("module metric should have a dense slot")
            .samples,
        1
    );
    assert!(
        module_record
            .timings
            .iter()
            .all(|aggregate| aggregate.metric != TimingMetric::BoundaryCompile)
    );
}

#[test]
fn timing_scope_attributed_stores_context() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "", 2, 1024);
    {
        timing_scope_attributed!(
            timing_guard,
            TimingMetric::FrontendPrepare,
            Some(crate::timing::TimingContext::for_module(module)),
        );
    }

    let snapshot = timing_session.finish();

    let observations = timings_named(&snapshot, TimingMetric::FrontendPrepare);
    assert_eq!(observations.len(), 1);
    let module_record = snapshot
        .modules
        .iter()
        .find(|record| record.key == module)
        .expect("the registered module should be in the snapshot");
    assert!(module_record.timings.iter().any(|aggregate| {
        aggregate.metric == TimingMetric::FrontendPrepare && aggregate.samples == 1
    }));
    let global = timings_named(&snapshot, TimingMetric::FrontendPrepare)
        .into_iter()
        .next()
        .expect("global module metric should be recorded");
    let attributed = module_record
        .timings
        .iter()
        .find(|aggregate| aggregate.metric == TimingMetric::FrontendPrepare)
        .expect("module metric should have an attributed row");
    assert_eq!(global.total, attributed.total);
    assert_eq!(global.samples, attributed.samples);
}

#[test]
fn attributed_metric_without_context_is_rejected_when_attribution_is_active() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    crate::timing::record_pipeline_timing_attributed(
        TimingMetric::FrontendPrepare,
        std::time::Duration::from_millis(3),
        None,
    );

    let snapshot = timing_session.finish();
    assert!(
        timings_named(&snapshot, TimingMetric::FrontendPrepare).is_empty(),
        "an attributed metric without context must not enter global storage"
    );
}

#[test]
fn direct_facade_rejects_attributed_metrics_without_context() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let value = timed_stage!(TimingMetric::FrontendPrepare, 7);

    let snapshot = timing_session.finish();
    assert_eq!(value, 7);
    assert!(
        timings_named(&snapshot, TimingMetric::FrontendPrepare).is_empty(),
        "the direct facade must reject attributed metrics without context"
    );
}

#[test]
fn metric_only_raw_sessions_accept_global_attributed_metrics() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    let value = timed_stage!(TimingMetric::FrontendPrepare, 11);

    let snapshot = timing_session.finish();
    assert_eq!(value, 11);
    assert_eq!(
        timings_named(&snapshot, TimingMetric::FrontendPrepare).len(),
        1,
        "metric-only raw sessions must retain global attributed metrics"
    );
}

#[test]
fn sentinel_boundary_observations_are_dropped() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    {
        timing_scope_attributed!(
            timing_guard,
            TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(
                crate::timing::NO_TIMING_BOUNDARY
            )),
        );
    }

    let snapshot = timing_session.finish();

    assert!(
        timings_named(&snapshot, TimingMetric::BoundaryInventory).is_empty(),
        "late observations attributed to the sentinel boundary must be dropped"
    );
}

#[test]
fn finalizing_a_fallback_module_key_is_ignored_when_attribution_becomes_active() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let fallback_key = crate::timing::TimingModuleKey::new(boundary, 0);

    crate::timing::finalize_timing_module_source_facts(fallback_key, 1, 128);

    let snapshot = timing_session.finish();
    assert!(
        snapshot.modules.is_empty(),
        "a fallback key must not create or finalize an unregistered module"
    );
}

#[test]
fn finalizing_a_stale_registered_module_key_is_ignored_by_a_new_collection() {
    let _test_guard = collector_test_guard();
    let stale_session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "stale-project".to_owned(),
    );
    let stale_key = crate::timing::register_timing_module(boundary, 0, "entry", 0, 0);
    let stale_snapshot = stale_session.finish();
    assert_eq!(stale_snapshot.modules.len(), 1);

    let current_session = start_benchmark_collection(true).expect("timing session should start");
    crate::timing::finalize_timing_module_source_facts(stale_key, 1, 128);

    let current_snapshot = current_session.finish();
    assert!(
        current_snapshot.modules.is_empty(),
        "a stale module key must not mutate a later collection"
    );
}

#[test]
fn finalizing_after_the_owning_collection_drains_is_ignored() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "drained-project".to_owned(),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "entry", 0, 0);
    let snapshot = timing_session.finish();
    assert_eq!(snapshot.modules.len(), 1);

    crate::timing::finalize_timing_module_source_facts(module, 1, 128);
}

#[test]
fn timed_stage_records_observation_and_runs_expression_once() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let runs = Cell::new(0);
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let value = timed_stage_attributed!(
        TimingMetric::BoundaryCompile,
        Some(crate::timing::TimingContext::for_boundary(boundary)),
        {
            runs.set(runs.get() + 1);
            42
        },
    );

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
    let test_timings: Vec<_> = snapshot
        .timings
        .iter()
        .filter(|observation| observation.metric == TimingMetric::BoundaryCompile)
        .collect();
    assert_eq!(test_timings.len(), 1);
}

#[test]
fn timed_ast_stage_records_exactly_once_without_double_recording() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    let start = std::time::Instant::now();
    let duration = start.elapsed();
    record_timing_duration!(TimingMetric::FrontendAstTotal, duration);

    let snapshot = timing_session.finish();

    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.metric == TimingMetric::FrontendAstTotal)
            .count(),
        1,
        "the AST aggregate macro must record once even when detailed_timers is active"
    );
}

#[test]
fn boundary_and_module_registration_is_dense_and_deterministic() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let first = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::SourcePackage,
        || "@html".to_owned(),
    );
    let second = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::SourcePackage,
        || "@markdown".to_owned(),
    );
    let project = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "moth_docs".to_owned(),
    );

    let first_module = crate::timing::register_timing_module(first, 0, "", 1, 512);
    let second_module = crate::timing::register_timing_module(second, 0, "parser", 3, 2048);
    let project_module =
        crate::timing::register_timing_module(project, 7, "docs/progress", 1, 44_928);

    let snapshot = timing_session.finish();

    let own_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            matches!(
                boundary.display_name.as_str(),
                "@html" | "@markdown" | "moth_docs"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        own_boundaries
            .iter()
            .map(|boundary| boundary.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["@html", "@markdown", "moth_docs"]
    );
    assert_eq!(own_boundaries[0].module_count, 1);
    assert_eq!(own_boundaries[1].module_count, 1);
    assert_eq!(own_boundaries[2].module_count, 1);

    let own_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key == first_module
                || module.key == second_module
                || module.key == project_module
        })
        .collect::<Vec<_>>();
    assert_eq!(
        own_modules[0].logical_identity, "@html",
        "entry-root modules reuse the boundary display name"
    );
    assert_eq!(own_modules[1].logical_identity, "@markdown/parser");
    assert_eq!(own_modules[2].logical_identity, "moth_docs/docs/progress");

    assert_ne!(first_module, second_module);
    assert_ne!(first_module, project_module);
    assert_ne!(second_module, project_module);
    assert_eq!(own_modules[2].source_file_count, 1);
    assert_eq!(own_modules[2].source_byte_count, 44_928);
}

#[cfg(not(feature = "detailed_timers"))]
#[test]
fn timed_stage_is_recorded_without_detailed_timers() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    let value: u32 = timed_stage!(TimingMetric::FrontendHir, 42);

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert!(timings_named(&snapshot, TimingMetric::FrontendHir).len() == 1);
}

#[cfg(feature = "detailed_timers")]
#[test]
fn timed_stage_records_observation_when_detailed_timers_active() {
    let _test_guard = collector_test_guard();
    let timing_session = start_raw_benchmark_collection(true).expect("timing session should start");

    let value: u32 = timed_stage!(TimingMetric::FrontendHir, 42);

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.metric == TimingMetric::FrontendHir)
            .count(),
        1
    );
}

#[test]
fn nested_raw_start_returns_an_error_and_preserves_outer_session() {
    let _test_guard = collector_test_guard();
    let outer = start_raw_benchmark_collection(true).expect("timing session should start");
    record_timing_via_facade(TimingMetric::BuildFrontendTotal);

    assert!(
        matches!(
            start_raw_benchmark_collection(true),
            Err(crate::timing::TimingSessionStartError::CollectorBusy)
        ),
        "a nested raw start must fail instead of recording into the outer session"
    );

    let outer_snapshot = outer.finish();
    assert_eq!(
        outer_snapshot
            .timings
            .iter()
            .filter(|observation| observation.metric == TimingMetric::BuildFrontendTotal)
            .count(),
        1,
        "the outer session must keep every observation recorded before the nested start"
    );
}

#[test]
fn stale_finish_cannot_drain_another_session() {
    let _test_guard = collector_test_guard();
    let first = start_raw_benchmark_collection(true).expect("timing session should start");
    record_timing_via_facade(TimingMetric::BuildFrontendTotal);

    let stale_snapshot = crate::timing::enabled::collector::finish_session(
        crate::timing::enabled::session::TimingSessionId::from_raw(u64::MAX),
    );
    assert!(stale_snapshot.timings.is_empty());

    let first_snapshot = first.finish();
    assert_eq!(
        first_snapshot
            .timings
            .iter()
            .filter(|observation| observation.metric == TimingMetric::BuildFrontendTotal)
            .count(),
        1
    );
}

#[test]
fn dropped_unfinished_session_cleans_up_only_its_scope() {
    let _test_guard = collector_test_guard();
    {
        let abandoned = start_raw_benchmark_collection(true).expect("timing session should start");
        assert!(abandoned.is_active());
    }

    let next = start_raw_benchmark_collection(true).expect("timing session should start");
    assert!(
        next.is_active(),
        "dropping an unfinished session must release the collector scope"
    );
    let snapshot = next.finish();
    assert!(
        snapshot
            .timings
            .iter()
            .all(|aggregate| aggregate.samples == 0),
        "the abandoned session's observations must not leak into the next session"
    );
}

#[test]
fn stale_context_from_an_older_session_is_dropped() {
    let _test_guard = collector_test_guard();
    let first = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "first-project".to_owned(),
    );
    let _ = first.finish();

    let second = start_benchmark_collection(true).expect("timing session should start");
    {
        timing_scope_attributed!(
            timing_guard,
            TimingMetric::BoundaryInventory,
            Some(crate::timing::TimingContext::for_boundary(boundary)),
        );
    }

    let snapshot = second.finish();
    assert!(
        timings_named(&snapshot, TimingMetric::BoundaryInventory).is_empty(),
        "an observation attributed to a finished session must not pollute the next session"
    );
}

#[test]
fn duplicate_module_registration_is_idempotent_when_metadata_matches() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let first_key = crate::timing::register_timing_module(boundary, 0, "", 1, 512);
    let second_key = crate::timing::register_timing_module(boundary, 0, "", 1, 512);

    let snapshot = session.finish();

    assert_eq!(
        first_key, second_key,
        "duplicate registration returns the existing key"
    );
    assert_eq!(
        snapshot.modules.len(),
        1,
        "duplicate registration must not add a record"
    );
    assert_eq!(snapshot.boundaries[0].module_count, 1);
    assert_eq!(snapshot.modules[0].source_file_count, 1);
    assert_eq!(snapshot.modules[0].source_byte_count, 512);
}

#[test]
fn conflicting_module_registration_panics_without_mutating_the_record() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let first_key = crate::timing::register_timing_module(boundary, 0, "", 1, 512);

    // The exact rejection message proves the source-fact conflict was detected,
    // not some unrelated panic inside registration.
    assert_panics_with(
        "timing module registration changed source file count",
        || {
            crate::timing::register_timing_module(boundary, 0, "", 2, 2048);
        },
    );
    let snapshot = session.finish();

    assert_eq!(snapshot.modules.len(), 1);
    assert_eq!(snapshot.modules[0].key, first_key);
    assert_eq!(snapshot.modules[0].source_file_count, 1);
    assert_eq!(snapshot.modules[0].source_byte_count, 512);
}

#[test]
fn staged_module_registration_finalizes_source_facts_once() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true).expect("timing session should start");
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let key = crate::timing::register_timing_module_for_preparation(boundary, 0, "entry");

    crate::timing::finalize_timing_module_source_facts(key, 2, 1024);
    crate::timing::finalize_timing_module_source_facts(key, 2, 1024);

    // The exact rejection message proves the re-finalization conflict was
    // detected, not some unrelated panic inside finalization.
    assert_panics_with(
        "timing module finalization changed source file count",
        || {
            crate::timing::finalize_timing_module_source_facts(key, 3, 2048);
        },
    );
    let snapshot = session.finish();
    assert_eq!(snapshot.modules[0].source_file_count, 2);
    assert_eq!(snapshot.modules[0].source_byte_count, 1024);
}

#[test]
fn command_session_carries_an_explicit_command_kind() {
    let _test_guard = collector_test_guard();
    let session = start_test_command_session(TimingCommandKind::Dev, timer_mode_summary());
    assert!(session.is_active());
    assert_eq!(session.command(), Some(TimingCommandKind::Dev));
    assert!(
        session
            .configuration()
            .expect("active session must retain its configuration")
            .channels()
            .attribution()
    );
    let _ = session.finish();
}

fn start_test_command_session(
    command: TimingCommandKind,
    timer_mode: crate::timing::TimerOutputMode,
) -> crate::timing::TimingSession {
    crate::timing::start_command_session_with_configuration(
        command,
        crate::timing::enabled::runtime::TimingSessionConfiguration::for_test(timer_mode),
    )
}

fn timer_mode_summary() -> crate::timing::TimerOutputMode {
    crate::timing::TimerOutputMode::Summary
}

/// Record one timing observation through the public facade.
fn record_timing_via_facade(metric: TimingMetric) {
    crate::timing::record_pipeline_timing(metric, std::time::Duration::from_millis(1));
}

#[test]
fn bench_mode_command_session_collects_metrics_without_attribution() {
    let _test_guard = collector_test_guard();
    let session = start_test_command_session(
        TimingCommandKind::Build,
        crate::timing::TimerOutputMode::Bench,
    );
    assert!(
        session.is_active(),
        "bench mode must retain metric evidence"
    );
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || panic!("bench mode must not allocate boundary metadata"),
    );
    assert_eq!(boundary, crate::timing::NO_TIMING_BOUNDARY);
    record_timing_via_facade(TimingMetric::BuildFrontendTotal);
    record_timing_via_facade(TimingMetric::ConfigAstTotal);

    let snapshot = session.finish();
    assert_eq!(
        timings_named(&snapshot, TimingMetric::BuildFrontendTotal).len(),
        1
    );
    assert_eq!(
        timings_named(&snapshot, TimingMetric::ConfigAstTotal).len(),
        1,
        "bench mode must collect the schema's detailed evidence"
    );
    assert!(snapshot.boundaries.is_empty());
    assert!(snapshot.modules.is_empty());
}

#[test]
fn silent_mode_command_session_is_rejected_without_snapshot() {
    let _test_guard = collector_test_guard();
    let session = start_test_command_session(
        TimingCommandKind::Check,
        crate::timing::TimerOutputMode::Silent,
    );
    assert!(
        !session.is_active(),
        "silent mode must not build a command snapshot"
    );
    let snapshot = session.finish();
    assert!(snapshot.timings.is_empty());
}

#[cfg(feature = "detailed_timers")]
#[test]
fn summary_mode_command_session_collects_snapshot() {
    let _test_guard = collector_test_guard();
    let session = start_test_command_session(TimingCommandKind::Build, timer_mode_summary());
    assert!(
        session.is_active(),
        "summary mode must collect a command snapshot"
    );
    record_timing_via_facade(TimingMetric::BuildFrontendTotal);

    let snapshot = session.finish();
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.metric == TimingMetric::BuildFrontendTotal)
            .count(),
        1
    );
}

#[test]
fn summary_mode_does_not_clock_or_record_detailed_metrics() {
    let _test_guard = collector_test_guard();
    let session = start_test_command_session(TimingCommandKind::Check, timer_mode_summary());
    crate::timing::enabled::runtime::reset_timing_clock_reads_for_test();
    crate::timing::enabled::collector::reset_lock_acquisitions_for_test();

    let value = timed_stage!(TimingMetric::ConfigAstTotal, 42);

    assert_eq!(value, 42);
    assert_eq!(
        crate::timing::enabled::runtime::timing_clock_reads_for_test(),
        0,
        "summary mode must not clock detailed schema metrics"
    );
    assert_eq!(
        crate::timing::enabled::collector::lock_acquisitions_for_test(),
        0,
        "summary mode must not record detailed schema metrics"
    );

    let snapshot = session.finish();
    assert_eq!(
        snapshot
            .timings
            .iter()
            .find(|aggregate| aggregate.metric == TimingMetric::ConfigAstTotal)
            .expect("summary snapshots retain dense schema rows")
            .samples,
        0
    );
}

#[test]
fn command_admission_rejects_metrics_outside_command_scope() {
    let _test_guard = collector_test_guard();
    let rejected_records = [
        (TimingCommandKind::Build, TimingMetric::CommandCheckTotal),
        (TimingCommandKind::Check, TimingMetric::CommandBuildTotal),
        (TimingCommandKind::Build, TimingMetric::CommandDevBuildWrite),
        (TimingCommandKind::Check, TimingMetric::BuildBackendTotal),
    ];

    for (command, metric) in rejected_records {
        let session = start_test_command_session(command, timer_mode_summary());
        let outcome = crate::timing::enabled::collector::record_timing(
            metric,
            std::time::Duration::from_millis(1),
        );

        assert!(
            !outcome.recorded,
            "{metric:?} must be rejected for {command:?}"
        );
        let snapshot = session.finish();
        assert_eq!(
            snapshot
                .timings
                .iter()
                .find(|aggregate| aggregate.metric == metric)
                .expect("dense snapshots retain a row for every schema metric")
                .samples,
            0,
            "rejected {metric:?} must not mutate the global aggregate"
        );
    }
}

#[test]
fn raw_benchmark_without_attribution_skips_metadata_tables() {
    let _test_guard = collector_test_guard();
    let session = crate::timing::start_raw_benchmark_collection(true)
        .expect("raw timing session should start");

    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || panic!("boundary names must not be built without attribution"),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "entry", 1, 128);
    {
        timing_scope_attributed!(
            timing_guard,
            TimingMetric::FrontendPrepare,
            Some(crate::timing::TimingContext::for_module(module)),
        );
    }

    let snapshot = session.finish();

    assert!(
        snapshot.boundaries.is_empty(),
        "raw benchmarks without attribution must not allocate boundary records"
    );
    assert!(
        snapshot.modules.is_empty(),
        "raw benchmarks without attribution must not allocate module records"
    );
    assert_eq!(
        timings_named(&snapshot, TimingMetric::FrontendPrepare).len(),
        1,
        "raw benchmarks must still record every metric"
    );
}

#[test]
fn raw_benchmark_without_attribution_skips_facade_context_expressions() {
    let _test_guard = collector_test_guard();
    let session = crate::timing::start_raw_benchmark_collection(true)
        .expect("raw timing session should start");
    let context_evaluations = Cell::new(0);

    let value = timed_stage_attributed!(
        TimingMetric::BoundaryInventory,
        {
            context_evaluations.set(context_evaluations.get() + 1);
            None
        },
        7
    );
    let frontend_value = timed_stage_attributed!(
        TimingMetric::BoundaryCompile,
        {
            context_evaluations.set(context_evaluations.get() + 1);
            None
        },
        11,
    );
    let frontend_child_value = timed_stage_attributed!(
        TimingMetric::FrontendPrepare,
        {
            context_evaluations.set(context_evaluations.get() + 1);
            None
        },
        13,
    );
    {
        timing_scope_attributed!(timing_guard, TimingMetric::FrontendPrepare, {
            context_evaluations.set(context_evaluations.get() + 1);
            None
        },);
        timing_scope_attributed!(ast_timing_guard, TimingMetric::FrontendAstEnvironment, {
            context_evaluations.set(context_evaluations.get() + 1);
            None
        },);
    }
    record_attributed_duration!(TimingMetric::FrontendAstEmit, std::time::Duration::ZERO, {
        context_evaluations.set(context_evaluations.get() + 1);
        None
    });

    let snapshot = session.finish();

    assert_eq!(value, 7);
    assert_eq!(frontend_value, 11);
    assert_eq!(frontend_child_value, 13);
    assert_eq!(
        context_evaluations.get(),
        0,
        "metric-only raw sessions must not build unused attribution contexts"
    );
    assert!(snapshot.boundaries.is_empty() && snapshot.modules.is_empty());
}

#[test]
fn lazy_boundary_name_is_not_evaluated_without_a_session() {
    let _test_guard = collector_test_guard();
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || panic!("boundary name must not be evaluated without an active session"),
    );
    assert_eq!(boundary, crate::timing::NO_TIMING_BOUNDARY);
}

#[test]
fn inactive_metrics_skip_pipeline_clock_and_collector_lock() {
    let _test_guard = collector_test_guard();
    crate::timing::enabled::runtime::reset_timing_clock_reads_for_test();
    crate::timing::enabled::collector::reset_lock_acquisitions_for_test();

    let runs = Cell::new(0);
    let value = timed_stage!(TimingMetric::FrontendPrepare, {
        runs.set(runs.get() + 1);
        42
    });

    assert_eq!(value, 42);
    assert_eq!(
        runs.get(),
        1,
        "the production expression must still run once"
    );
    assert_eq!(
        crate::timing::enabled::runtime::timing_clock_reads_for_test(),
        0,
        "inactive metric timing must not read the pipeline clock"
    );
    assert_eq!(
        crate::timing::enabled::collector::lock_acquisitions_for_test(),
        0,
        "inactive metric timing must not lock the dense timing collector"
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn counter_metric_names_are_static_strings() {
    let _test_guard = collector_test_guard();
    let session = start_raw_benchmark_collection(true).expect("timing session should start");
    crate::timing::record_counter("test.counter", 3.0);

    let snapshot = session.finish();
    assert_eq!(snapshot.counters.len(), 1);
    assert_eq!(snapshot.counters[0].name, "test.counter");
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn counter_mode_parser_is_pure() {
    assert_eq!(
        crate::timing::CounterOutputMode::parse(Some("summary")),
        crate::timing::CounterOutputMode::Summary
    );
    assert_eq!(
        crate::timing::CounterOutputMode::parse(Some("full")),
        crate::timing::CounterOutputMode::Full
    );
    assert_eq!(
        crate::timing::CounterOutputMode::parse(Some("unknown")),
        crate::timing::CounterOutputMode::Off
    );
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn silent_counter_summary_session_collects_counters_without_metric_clocks() {
    let _test_guard = collector_test_guard();
    let session = crate::timing::start_command_session_with_configuration(
        TimingCommandKind::Check,
        crate::timing::enabled::runtime::TimingSessionConfiguration::for_test_with_counters(
            crate::timing::TimerOutputMode::Silent,
            crate::timing::CounterOutputMode::Summary,
        ),
    );
    assert!(session.is_active(), "counter-only mode must own a session");
    assert!(
        !session
            .configuration()
            .expect("active session must retain its configuration")
            .channels()
            .metrics()
    );

    crate::timing::enabled::runtime::reset_timing_clock_reads_for_test();
    crate::timing::enabled::collector::reset_lock_acquisitions_for_test();
    let value = timed_stage!(TimingMetric::FrontendPrepare, 42);
    crate::timing::record_counter("silent.counter_only", 3.0);

    assert_eq!(value, 42);
    assert_eq!(
        crate::timing::enabled::runtime::timing_clock_reads_for_test(),
        0,
        "counter-only sessions must not clock timer metrics"
    );
    assert_eq!(
        crate::timing::enabled::collector::lock_acquisitions_for_test(),
        1,
        "only the active counter record may lock the collector"
    );

    let snapshot = session.finish();
    assert!(
        snapshot
            .timings
            .iter()
            .all(|aggregate| aggregate.samples == 0)
    );
    assert_eq!(snapshot.counters.len(), 1);
    assert_eq!(snapshot.counters[0].name, "silent.counter_only");
}

#[test]
fn ast_stage_guard_records_on_drop_including_error_paths() {
    let _test_guard = collector_test_guard();
    let session = start_raw_benchmark_collection(true).expect("timing session should start");

    {
        timing_scope!(timing_guard, TimingMetric::FrontendAstFinalise);
    }

    let snapshot = session.finish();
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.metric == TimingMetric::FrontendAstFinalise)
            .count(),
        1,
        "the AST stage guard must record when the scope ends, including error paths"
    );
}

#[test]
fn capture_command_duration_records_total_and_returns_duration() {
    let _test_guard = collector_test_guard();
    let session = start_raw_benchmark_collection(true).expect("timing session should start");

    let start = std::time::Instant::now();
    let duration = capture_command_duration!(TimingMetric::CommandBuildTotal, start);

    let snapshot = session.finish();

    let observations = timings_named(&snapshot, TimingMetric::CommandBuildTotal);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].samples, 1);
    assert_eq!(observations[0].total, duration);
}

#[test]
#[should_panic(expected = "record_command_total_timing only accepts command-total metrics")]
fn record_command_total_timing_rejects_non_command_total_metrics() {
    let _test_guard = collector_test_guard();
    let _session = start_raw_benchmark_collection(true).expect("timing session should start");

    let _ = capture_command_duration!(TimingMetric::FrontendAstTotal, std::time::Instant::now());
}
