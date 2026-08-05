//! Compile-erasure tests for the timer facade.
//!
//! WHAT: proves that with `timers` disabled every facade macro discards its
//!      instrumentation arguments and the wrapped production expression runs
//!      exactly once.
//! WHY:  the zero-cost rule is enforced at the macro call site; these tests
//!       pin that contract before the Phase 2 binary audit exists.

#![cfg(not(feature = "timers"))]

use std::cell::Cell;
use std::rc::Rc;

fn evaluation_counter() -> Rc<Cell<usize>> {
    Rc::new(Cell::new(0))
}

#[test]
fn pipeline_timer_does_not_evaluate_metric_expression() {
    let metric_evaluated = evaluation_counter();
    let value = pipeline_timer!(
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "test.metric"
        },
        42
    );

    assert_eq!(value, 42);
    assert_eq!(metric_evaluated.get(), 0);
}

#[test]
fn labeled_pipeline_timer_does_not_evaluate_label_expression() {
    let label_evaluated = evaluation_counter();
    let value = labeled_pipeline_timer!(
        "test.metric",
        {
            label_evaluated.set(label_evaluated.get() + 1);
            "label"
        },
        42
    );

    assert_eq!(value, 42);
    assert_eq!(label_evaluated.get(), 0);
}

#[test]
fn timing_guard_does_not_evaluate_metric_expression() {
    let metric_evaluated = evaluation_counter();
    timing_guard!({
        metric_evaluated.set(metric_evaluated.get() + 1);
        "test.guard"
    });

    assert_eq!(metric_evaluated.get(), 0);
}

#[test]
fn timed_manual_finish_does_not_evaluate_arguments() {
    let metric_evaluated = evaluation_counter();
    let start_evaluated = evaluation_counter();
    timed_manual_finish!(
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "test.manual"
        },
        {
            start_evaluated.set(start_evaluated.get() + 1);
            ()
        }
    );

    assert_eq!(metric_evaluated.get(), 0);
    assert_eq!(start_evaluated.get(), 0);
}

#[test]
fn timed_manual_finish_labeled_does_not_evaluate_arguments() {
    let metric_evaluated = evaluation_counter();
    let start_evaluated = evaluation_counter();
    let label_evaluated = evaluation_counter();
    timed_manual_finish_labeled!(
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "test.manual"
        },
        {
            start_evaluated.set(start_evaluated.get() + 1);
            ()
        },
        {
            label_evaluated.set(label_evaluated.get() + 1);
            None
        }
    );

    assert_eq!(metric_evaluated.get(), 0);
    assert_eq!(start_evaluated.get(), 0);
    assert_eq!(label_evaluated.get(), 0);
}

#[test]
fn command_timing_macros_expand_to_nothing() {
    let mut runs = 0;
    for _ in 0..3 {
        command_timing_start!();
        runs += 1;
        command_timing_finish!(true);
    }

    assert_eq!(runs, 3);
}

#[test]
fn pipeline_timer_runs_wrapped_expression_exactly_once() {
    let runs = evaluation_counter();
    let value = pipeline_timer!("test.metric", {
        runs.set(runs.get() + 1);
        42
    });

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
}

#[test]
fn pipeline_timer_passes_ok_result_through_unchanged() {
    let result: Result<u32, &str> = pipeline_timer!("test.metric", Ok(7));

    assert_eq!(result, Ok(7));
}

#[test]
fn pipeline_timer_passes_error_through_unchanged() {
    let result: Result<u32, &str> = pipeline_timer!("test.metric", Err("boom"));

    assert_eq!(result, Err("boom"));
}

#[test]
fn timer_facade_sources_never_use_cfg_timer_macro() {
    let facade_sources = [
        include_str!("../../timing.rs"),
        include_str!("../enabled.rs"),
        include_str!("../enabled/mode.rs"),
        include_str!("../enabled/collector.rs"),
    ];

    for source in facade_sources {
        assert!(
            !source.contains("cfg!(feature = \"timers\")"),
            "timer facade must use #[cfg] macro definitions, not runtime cfg! checks"
        );
    }
}
