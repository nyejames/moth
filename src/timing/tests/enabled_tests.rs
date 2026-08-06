//! Enabled-collector tests for the timer facade.
//!
//! WHAT: proves that with `timers` active the facade macros record one
//!      observation per wrapped stage and never change production values.
//! WHY:  the enabled expansion must mirror the disabled one for control flow
//!       while adding collector evidence.

use crate::timed_ast_stage;
use crate::timing::{TimingCommandKind, start_benchmark_collection};
use std::cell::Cell;

/// Serializes collector tests against every other timing test in the process.
/// The collector is one process-global scope, and the timing suite owns its
/// own test lock rather than borrowing the frontend counter-test lock.
fn collector_test_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::timing::tests::lock_timing_tests()
}

/// Observations for one metric, ignoring unrelated parallel-test pollution.
fn timings_named<'a>(
    snapshot: &'a crate::timing::BenchmarkObservationSnapshot,
    name: &'static str,
) -> Vec<&'a crate::timing::TimingObservation> {
    snapshot
        .timings
        .iter()
        .filter(|observation| observation.name == name)
        .collect()
}

#[test]
fn pipeline_timer_records_one_observation_and_runs_expression_once() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let runs = Cell::new(0);
    let value = pipeline_timer!("test.metric", {
        runs.set(runs.get() + 1);
        42
    });

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
    let observations = timings_named(&snapshot, "test.metric");
    assert_eq!(observations.len(), 1);
}

#[test]
fn labeled_pipeline_timer_records_observation_and_passes_value_through() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let value = labeled_pipeline_timer!("test.labeled", "prose label ", 7);

    let snapshot = timing_session.finish();

    assert_eq!(value, 7);
    let observations = timings_named(&snapshot, "test.labeled");
    assert_eq!(observations.len(), 1);
}

#[test]
fn timing_guard_records_observation_when_scope_ends() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    {
        timing_guard!("test.guard");
    }

    let snapshot = timing_session.finish();

    let observations = timings_named(&snapshot, "test.guard");
    assert_eq!(observations.len(), 1);
}

#[test]
fn timed_manual_finish_records_started_stage() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish!("test.manual", start);

    let snapshot = timing_session.finish();

    let observations = timings_named(&snapshot, "test.manual");
    assert_eq!(observations.len(), 1);
}

#[test]
fn timed_manual_finish_attributed_stores_context() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "", 2, 1024);
    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish_attributed!(
        "test.labeled_manual",
        start,
        Some(crate::timing::TimingContext::for_module(module)),
    );

    let snapshot = timing_session.finish();

    let observations = timings_named(&snapshot, "test.labeled_manual");
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].context,
        Some(crate::timing::TimingContext::for_module(module)),
    );
}

#[test]
fn sentinel_boundary_observations_are_dropped() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish_attributed!(
        "test.sentinel",
        start,
        Some(crate::timing::TimingContext::for_boundary(
            crate::timing::NO_TIMING_BOUNDARY
        )),
    );

    let snapshot = timing_session.finish();

    assert!(
        snapshot
            .timings
            .iter()
            .all(|observation| observation.name != "test.sentinel"),
        "late observations attributed to the sentinel boundary must be dropped"
    );
}

#[test]
fn timed_frontend_stage_records_observation_and_runs_expression_once() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let runs = Cell::new(0);
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let value = timed_frontend_stage!(
        "frontend.test",
        "Test stage: ",
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
        .filter(|observation| observation.name == "frontend.test")
        .collect();
    assert_eq!(test_timings.len(), 1);
}

#[test]
fn timed_ast_stage_records_exactly_once_without_double_recording() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let start = std::time::Instant::now();
    timed_ast_stage!(
        start,
        "ast.test_aggregate",
        "AST/test aggregate completed in: "
    );

    let snapshot = timing_session.finish();

    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "ast.test_aggregate")
            .count(),
        1,
        "the AST aggregate macro must record once even when detailed_timers is active"
    );
}

#[test]
fn boundary_and_module_registration_is_dense_and_deterministic() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

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
fn timed_frontend_substep_is_a_direct_expression_without_detailed_timers() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let value: u32 = timed_frontend_substep!("frontend.substep", "Substep: ", 42);

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert!(
        snapshot
            .timings
            .iter()
            .all(|observation| observation.name != "frontend.substep")
    );
}

#[cfg(feature = "detailed_timers")]
#[test]
fn timed_frontend_substep_records_observation_when_detailed_timers_active() {
    let _test_guard = collector_test_guard();
    let timing_session = start_benchmark_collection(true);

    let value: u32 = timed_frontend_substep!("frontend.substep", "Substep: ", 42);

    let snapshot = timing_session.finish();

    assert_eq!(value, 42);
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "frontend.substep")
            .count(),
        1
    );
}

#[test]
fn nested_start_is_rejected_and_preserves_outer_session() {
    let _test_guard = collector_test_guard();
    let outer = start_benchmark_collection(true);
    record_timing_via_facade("outer.metric");

    let inner = start_benchmark_collection(true);
    assert!(!inner.is_active(), "a nested start must be rejected");

    let inner_snapshot = inner.finish();
    assert!(
        inner_snapshot.timings.is_empty(),
        "a rejected session must never drain the outer session"
    );

    let outer_snapshot = outer.finish();
    assert_eq!(
        outer_snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "outer.metric")
            .count(),
        1,
        "the outer session must keep every observation recorded before the nested start"
    );
}

#[test]
fn mismatched_finish_cannot_drain_another_session() {
    let _test_guard = collector_test_guard();
    let first = start_benchmark_collection(true);
    record_timing_via_facade("first.metric");

    let second = start_benchmark_collection(true);
    assert!(!second.is_active());

    let second_snapshot = second.finish();
    assert!(second_snapshot.timings.is_empty());

    let first_snapshot = first.finish();
    assert_eq!(
        first_snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "first.metric")
            .count(),
        1
    );
}

#[test]
fn dropped_unfinished_session_cleans_up_only_its_scope() {
    let _test_guard = collector_test_guard();
    {
        let abandoned = start_benchmark_collection(true);
        assert!(abandoned.is_active());
    }

    let next = start_benchmark_collection(true);
    assert!(
        next.is_active(),
        "dropping an unfinished session must release the collector scope"
    );
    let snapshot = next.finish();
    assert!(
        snapshot.timings.is_empty(),
        "the abandoned session's observations must not leak into the next session"
    );
}

#[test]
fn stale_context_from_an_older_session_is_dropped() {
    let _test_guard = collector_test_guard();
    let first = start_benchmark_collection(true);
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "first-project".to_owned(),
    );
    let _ = first.finish();

    let second = start_benchmark_collection(true);
    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish_attributed!(
        "test.stale",
        start,
        Some(crate::timing::TimingContext::for_boundary(boundary)),
    );

    let snapshot = second.finish();
    assert!(
        snapshot
            .timings
            .iter()
            .all(|observation| observation.name != "test.stale"),
        "an observation attributed to a finished session must not pollute the next session"
    );
}

#[test]
fn duplicate_module_registration_is_ignored() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true);
    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || "test-project".to_owned(),
    );
    let first_key = crate::timing::register_timing_module(boundary, 0, "", 1, 512);
    let second_key = crate::timing::register_timing_module(boundary, 0, "", 2, 2048);

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
}

#[test]
fn command_session_carries_an_explicit_command_kind() {
    let _test_guard = collector_test_guard();
    crate::timing::enabled::mode::set_output_mode_for_test(
        crate::timing::enabled::mode::TimerOutputMode::Summary,
    );
    let session = crate::timing::start_command_session(TimingCommandKind::Dev);
    assert!(session.is_active());
    assert_eq!(session.command(), Some(TimingCommandKind::Dev));
    let _ = session.finish();
}

/// Record one timing observation through the public facade.
fn record_timing_via_facade(name: &'static str) {
    crate::timing::record_pipeline_timing(name, std::time::Duration::from_millis(1));
}

#[test]
fn bench_mode_command_session_is_rejected_without_snapshot() {
    let _test_guard = collector_test_guard();
    crate::timing::enabled::mode::set_output_mode_for_test(
        crate::timing::enabled::mode::TimerOutputMode::Bench,
    );

    let session = crate::timing::start_command_session(TimingCommandKind::Build);
    assert!(
        !session.is_active(),
        "bench mode must not build a command snapshot"
    );
    record_timing_via_facade("bench.metric");

    let snapshot = session.finish();
    assert!(
        snapshot.timings.is_empty(),
        "bench mode must not collect observations for a discarded snapshot"
    );
}

#[test]
fn silent_mode_command_session_is_rejected_without_snapshot() {
    let _test_guard = collector_test_guard();
    crate::timing::enabled::mode::set_output_mode_for_test(
        crate::timing::enabled::mode::TimerOutputMode::Silent,
    );

    let session = crate::timing::start_command_session(TimingCommandKind::Check);
    assert!(
        !session.is_active(),
        "silent mode must not build a command snapshot"
    );
    let snapshot = session.finish();
    assert!(snapshot.timings.is_empty());
}

#[test]
fn summary_mode_command_session_collects_snapshot() {
    let _test_guard = collector_test_guard();
    crate::timing::enabled::mode::set_output_mode_for_test(
        crate::timing::enabled::mode::TimerOutputMode::Summary,
    );

    let session = crate::timing::start_command_session(TimingCommandKind::Build);
    assert!(
        session.is_active(),
        "summary mode must collect a command snapshot"
    );
    record_timing_via_facade("summary.metric");

    let snapshot = session.finish();
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "summary.metric")
            .count(),
        1
    );
}

#[test]
fn raw_benchmark_without_attribution_skips_metadata_tables() {
    let _test_guard = collector_test_guard();
    let session = crate::timing::start_raw_benchmark_collection(true);

    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || panic!("boundary names must not be built without attribution"),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "entry", 1, 128);
    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish_attributed!(
        "raw.metric",
        start,
        Some(crate::timing::TimingContext::for_module(module)),
    );

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
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "raw.metric")
            .count(),
        1,
        "raw benchmarks must still record every metric"
    );
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

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn counter_metric_names_are_static_strings() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true);
    crate::timing::record_counter("test.counter", 3.0);

    let snapshot = session.finish();
    assert_eq!(snapshot.counters.len(), 1);
    assert_eq!(snapshot.counters[0].name, "test.counter");
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn counter_output_mode_can_be_overridden_for_tests() {
    let _test_guard = collector_test_guard();
    crate::timing::set_counter_output_mode_for_test(crate::timing::CounterOutputMode::Summary);
    assert_eq!(
        crate::timing::current_counter_output_mode(),
        crate::timing::CounterOutputMode::Summary
    );
}

#[test]
fn ast_stage_guard_records_on_drop_including_error_paths() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true);

    {
        timed_ast_stage_guard!(timing_guard, "ast.test_guard", None, "AST/test guard: ");
    }

    let snapshot = session.finish();
    assert_eq!(
        snapshot
            .timings
            .iter()
            .filter(|observation| observation.name == "ast.test_guard")
            .count(),
        1,
        "the AST stage guard must record when the scope ends, including error paths"
    );
}

#[test]
fn multi_record_uses_one_captured_duration() {
    let _test_guard = collector_test_guard();
    let session = start_benchmark_collection(true);

    crate::timing::record_pipeline_timing_multi(
        &[("shared.first", None), ("shared.second", None)],
        std::time::Duration::from_millis(7),
    );

    let snapshot = session.finish();
    let first = snapshot
        .timings
        .iter()
        .find(|observation| observation.name == "shared.first")
        .expect("first shared metric should be recorded");
    let second = snapshot
        .timings
        .iter()
        .find(|observation| observation.name == "shared.second")
        .expect("second shared metric should be recorded");
    assert_eq!(first.duration, std::time::Duration::from_millis(7));
    assert_eq!(second.duration, std::time::Duration::from_millis(7));
}
