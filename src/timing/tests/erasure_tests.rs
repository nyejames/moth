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
fn timed_stage_does_not_evaluate_metric_expression() {
    let metric_evaluated = evaluation_counter();
    let value = timed_stage!(
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
fn timed_stage_attributed_does_not_evaluate_context_expression() {
    let context_evaluated = evaluation_counter();
    let value = timed_stage_attributed!(
        "test.metric",
        {
            context_evaluated.set(context_evaluated.get() + 1);
            None
        },
        42
    );

    assert_eq!(value, 42);
    assert_eq!(context_evaluated.get(), 0);
}

#[test]
fn timing_scope_does_not_evaluate_metric_expression() {
    let metric_evaluated = evaluation_counter();
    timing_scope!(timing_guard, {
        metric_evaluated.set(metric_evaluated.get() + 1);
        "test.guard"
    });

    assert_eq!(metric_evaluated.get(), 0);
}

#[test]
fn finish_timing_scope_does_not_evaluate_binding_expression() {
    finish_timing_scope!({ panic!("disabled timing scope must not evaluate its binding") });
}

#[test]
fn record_timing_duration_does_not_evaluate_arguments() {
    let metric_evaluated = evaluation_counter();
    let duration_evaluated = evaluation_counter();
    record_timing_duration!(
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "test.manual"
        },
        {
            duration_evaluated.set(duration_evaluated.get() + 1);
            std::time::Duration::ZERO
        }
    );

    assert_eq!(metric_evaluated.get(), 0);
    assert_eq!(duration_evaluated.get(), 0);
}

#[test]
fn record_attributed_duration_does_not_evaluate_arguments() {
    let metric_evaluated = evaluation_counter();
    let duration_evaluated = evaluation_counter();
    let context_evaluated = evaluation_counter();
    record_attributed_duration!(
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "test.manual"
        },
        {
            duration_evaluated.set(duration_evaluated.get() + 1);
            std::time::Duration::ZERO
        },
        {
            context_evaluated.set(context_evaluated.get() + 1);
            None
        }
    );

    assert_eq!(metric_evaluated.get(), 0);
    assert_eq!(duration_evaluated.get(), 0);
    assert_eq!(context_evaluated.get(), 0);
}

#[test]
fn command_timing_macros_expand_to_nothing() {
    let mut runs = 0;
    for _ in 0..3 {
        command_timing_scope!(timing_session, crate::timing::TimingCommandKind::Build);
        runs += 1;
        finish_command_timing!(timing_session, true);
    }

    assert_eq!(runs, 3);
}

#[test]
fn finish_command_timing_discards_success_expression() {
    let succeeded_evaluated = evaluation_counter();
    finish_command_timing!(timing_session, {
        succeeded_evaluated.set(succeeded_evaluated.get() + 1);
        true
    });

    assert_eq!(succeeded_evaluated.get(), 0);
}

#[test]
fn timed_stage_attributed_expands_to_production_expression() {
    let metric_evaluated = evaluation_counter();
    let context_evaluated = evaluation_counter();

    // A non-callable value only compiles when the disabled expansion is the
    // production expression itself rather than a closure invocation.
    let value: u32 = timed_stage_attributed!(
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "frontend.test"
        },
        {
            context_evaluated.set(context_evaluated.get() + 1);
            None
        },
        42
    );

    assert_eq!(value, 42);
    assert_eq!(metric_evaluated.get(), 0);
    assert_eq!(context_evaluated.get(), 0);
}

#[test]
fn timing_scope_attributed_erases_metric_and_context_expressions() {
    let metric_evaluated = evaluation_counter();
    timing_scope_attributed!(
        timing_guard,
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            "frontend.substep"
        },
        {
            metric_evaluated.set(metric_evaluated.get() + 1);
            None
        }
    );

    assert_eq!(metric_evaluated.get(), 0);
}

#[test]
fn timed_stage_runs_wrapped_expression_exactly_once() {
    let runs = evaluation_counter();
    let value = timed_stage!(
        {
            let _ = "test.metric";
            "test.metric"
        },
        {
            runs.set(runs.get() + 1);
            42
        }
    );

    assert_eq!(value, 42);
    assert_eq!(runs.get(), 1);
}

#[test]
fn timed_stage_passes_ok_result_through_unchanged() {
    let result: Result<u32, &str> = timed_stage!("test.metric", Ok(7));

    assert_eq!(result, Ok(7));
}

#[test]
fn timed_stage_passes_error_through_unchanged() {
    let result: Result<u32, &str> = timed_stage!("test.metric", Err("boom"));

    assert_eq!(result, Err("boom"));
}

#[test]
fn timer_facade_sources_never_use_cfg_timer_macro() {
    let facade_sources = [
        include_str!("../../timing.rs"),
        include_str!("../enabled/mod.rs"),
        include_str!("../enabled/runtime.rs"),
        include_str!("../enabled/collector.rs"),
    ];

    for source in facade_sources {
        assert!(
            !source.contains("cfg!(feature = \"timers\")"),
            "timer facade must use #[cfg] macro definitions, not runtime cfg! checks"
        );
    }
}

#[test]
fn command_timing_scope_does_not_evaluate_command_expression() {
    let command_evaluated = evaluation_counter();
    command_timing_scope!(timing_session, {
        command_evaluated.set(command_evaluated.get() + 1);
        crate::timing::TimingCommandKind::Build
    });

    assert_eq!(command_evaluated.get(), 0);
}
