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
fn timed_manual_finish_attributed_stores_label_and_context() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        "test-project".to_owned(),
    );
    let module = crate::timing::register_timing_module(boundary, 0, "", 2, 1024);
    let start = crate::timing::start_pipeline_timing();
    timed_manual_finish_attributed!(
        "test.labeled_manual",
        start,
        Some("slowest module"),
        crate::timing::TimingModuleContext::for_module(module),
    );

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(snapshot.timings.len(), 1);
    assert_eq!(snapshot.timings[0].name, "test.labeled_manual");
    assert_eq!(snapshot.timings[0].label.as_deref(), Some("slowest module"));
    assert_eq!(snapshot.timings[0].boundary, Some(boundary));
    assert_eq!(snapshot.timings[0].module, Some(module));
}

#[test]
fn timed_frontend_stage_records_observation_and_runs_expression_once() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let runs = Cell::new(0);
    let value = timed_frontend_stage!(
        "frontend.test",
        "Test stage: ",
        None,
        crate::timing::TimingModuleContext::default(),
        {
            runs.set(runs.get() + 1);
            42
        },
    );

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
    let test_timings: Vec<_> = snapshot
        .timings
        .iter()
        .filter(|observation| observation.name == "frontend.test")
        .collect();
    assert_eq!(test_timings.len(), 1);
    assert_eq!(test_timings[0].label, None);
}

#[test]
fn boundary_and_module_registration_is_dense_and_deterministic() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let first = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::SourcePackage,
        "@html".to_owned(),
    );
    let second = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::SourcePackage,
        "@markdown".to_owned(),
    );
    let project = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        "moth_docs".to_owned(),
    );

    let first_module = crate::timing::register_timing_module(first, 0, "", 1, 512);
    let second_module = crate::timing::register_timing_module(second, 0, "parser", 3, 2048);
    let project_module =
        crate::timing::register_timing_module(project, 7, "docs/progress", 1, 44_928);

    let snapshot = stop_and_collect_benchmark_observations();

    assert_eq!(
        snapshot
            .boundaries
            .iter()
            .map(|boundary| boundary.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["@html", "@markdown", "moth_docs"]
    );
    assert_eq!(snapshot.boundaries[0].module_count, 1);
    assert_eq!(snapshot.boundaries[1].module_count, 1);
    assert_eq!(snapshot.boundaries[2].module_count, 1);

    assert_eq!(
        snapshot.modules[0].logical_identity, "@html",
        "entry-root modules reuse the boundary display name"
    );
    assert_eq!(snapshot.modules[1].logical_identity, "@markdown/parser");
    assert_eq!(
        snapshot.modules[2].logical_identity,
        "moth_docs/docs/progress"
    );

    assert_ne!(first_module, second_module);
    assert_ne!(first_module, project_module);
    assert_ne!(second_module, project_module);
    assert_eq!(snapshot.modules[2].source_file_count, 1);
    assert_eq!(snapshot.modules[2].source_byte_count, 44_928);
}

#[cfg(not(feature = "detailed_timers"))]
#[test]
fn timed_frontend_substep_is_a_direct_expression_without_detailed_timers() {
    let _test_guard = collector_test_guard();
    start_benchmark_collection(true);

    let value: u32 = timed_frontend_substep!("frontend.substep", "Substep: ", 42);

    let snapshot = stop_and_collect_benchmark_observations();

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
    start_benchmark_collection(true);

    let value: u32 = timed_frontend_substep!("frontend.substep", "Substep: ", 42);

    let snapshot = stop_and_collect_benchmark_observations();

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
