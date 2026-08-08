//! Instrumentation regression tests.
//!
//! WHAT: protects the stable benchmark-counter path used by in-process frontend
//! benchmarks when detailed timer stdout is suppressed.
//! WHY: human-only counter output is not visible to the benchmark collector, so
//! AST counters must flow through the same stable machine path as frontend counters.

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
use super::{
    AstCounter, FrontendCounter, add_ast_counter, add_frontend_counter,
    capture_frontend_counters_for_test, log_ast_counters, log_frontend_counters,
    reset_ast_counters, reset_frontend_counters,
};

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
use crate::timing::start_benchmark_collection;

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
use std::sync::{Arc, Barrier};

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn ast_counters_record_stable_metrics_when_stdout_is_suppressed() {
    let _guard = super::lock_counter_test();

    reset_ast_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    add_ast_counter(AstCounter::ScopeContextsCreated, 3);

    // Template churn counters must also flow through the stable benchmark
    // metric path without panics.
    add_ast_counter(AstCounter::TemplateNestedTemplateParses, 7);
    add_ast_counter(AstCounter::TemplateBodyTokenVisits, 11);
    add_ast_counter(AstCounter::TemplateTextBytesParsed, 13);
    add_ast_counter(AstCounter::RuntimeTemplateHandoffsRefreshedForHir, 14);
    add_ast_counter(AstCounter::RuntimeSlotHandoffsMaterialized, 16);
    add_ast_counter(AstCounter::RuntimeSlotHandoffOwnedNodesMaterialized, 22);
    add_ast_counter(AstCounter::TemplateFoldOutputBytes, 17);
    add_ast_counter(AstCounter::TemplateEstimatedFoldOutputBytes, 18);
    add_ast_counter(AstCounter::TemplateFoldOutputEstimateMissBytes, 16);
    add_ast_counter(AstCounter::TemplateFoldStringInternCalls, 19);
    add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 23);
    add_ast_counter(AstCounter::TemplateFoldExpressionOwnedRewrites, 24);
    add_ast_counter(AstCounter::TemplateFoldBindingSubstitutions, 29);

    // Exact-view TIR attribution counters must flow through the stable metric path.
    add_ast_counter(AstCounter::TirFinalizationFoldAttempts, 313);
    add_ast_counter(AstCounter::TirFinalizationFoldSuccesses, 317);
    add_ast_counter(AstCounter::TirViewFoldsAttempted, 331);
    add_ast_counter(AstCounter::TirViewFoldOverlayEmpty, 337);
    add_ast_counter(AstCounter::TirViewFoldOverlayExpressionOnly, 341);
    add_ast_counter(AstCounter::TirViewFoldOverlaySlotOnly, 347);
    add_ast_counter(AstCounter::TirViewFoldOverlayExpressionAndSlot, 349);
    add_ast_counter(AstCounter::TirViewFoldWrapperContextPresent, 353);
    add_ast_counter(AstCounter::TirFoldCacheHits, 367);
    add_ast_counter(AstCounter::TirFoldCacheMisses, 373);
    add_ast_counter(AstCounter::TirPreparationAttempts, 409);
    add_ast_counter(AstCounter::TirPreparationNodesVisited, 419);

    // TIR wrapper-set counters must also flow through the stable metric path.
    add_ast_counter(AstCounter::TirWrapperSetReuseHits, 47);

    log_ast_counters();

    let observations = timing_session.finish();

    assert_counter_value(&observations.counters, "ast_scope_contexts_created", 3.0);
    assert_counter_value(
        &observations.counters,
        "ast_template_nested_template_parses",
        7.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_body_token_visits",
        11.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_text_bytes_parsed",
        13.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_runtime_template_handoffs_refreshed_for_hir",
        14.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_runtime_slot_handoffs_materialized",
        16.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_runtime_slot_handoff_owned_nodes_materialized",
        22.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_fold_output_bytes",
        17.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_estimated_fold_output_bytes",
        18.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_fold_output_estimate_miss_bytes",
        16.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_fold_string_intern_calls",
        19.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_fold_expression_clone_requests",
        23.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_fold_expression_owned_rewrites",
        24.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_template_fold_binding_substitutions",
        29.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_finalization_fold_attempts",
        313.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_finalization_fold_successes",
        317.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_view_folds_attempted",
        331.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_view_fold_overlay_empty",
        337.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_view_fold_overlay_expression_only",
        341.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_view_fold_overlay_slot_only",
        347.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_view_fold_overlay_expression_and_slot",
        349.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_view_fold_wrapper_context_present",
        353.0,
    );
    assert_counter_value(&observations.counters, "ast_tir_fold_cache_hits", 367.0);
    assert_counter_value(&observations.counters, "ast_tir_fold_cache_misses", 373.0);
    assert_counter_value(
        &observations.counters,
        "ast_tir_preparation_attempts",
        409.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_preparation_nodes_visited",
        419.0,
    );
    assert_counter_value(
        &observations.counters,
        "ast_tir_wrapper_set_reuse_hits",
        47.0,
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn ast_counters_are_isolated_per_thread() {
    let _guard = super::lock_counter_test();

    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    // Synchronize the two worker threads so each resets and adds its own value
    // before either thread logs. With process-global counters this would always
    // produce two identical contaminated values; with per-thread counters each
    // thread reports only the value it added.
    let barrier = Arc::new(Barrier::new(2));
    let barrier_a = Arc::clone(&barrier);
    let barrier_b = Arc::clone(&barrier);

    let thread_a = std::thread::spawn(move || {
        reset_ast_counters();
        add_ast_counter(AstCounter::ScopeContextsCreated, 3);
        barrier_a.wait();
        log_ast_counters();
    });

    let thread_b = std::thread::spawn(move || {
        reset_ast_counters();
        add_ast_counter(AstCounter::ScopeContextsCreated, 5);
        barrier_b.wait();
        log_ast_counters();
    });

    thread_a.join().expect("thread A panicked");
    thread_b.join().expect("thread B panicked");

    let observations = timing_session.finish();

    let scope_values: Vec<f64> = observations
        .counters
        .iter()
        .filter(|counter| counter.name == "ast_scope_contexts_created")
        .map(|counter| counter.value)
        .collect();

    assert_eq!(scope_values.len(), 2, "expected both threads to log");
    assert!(scope_values.contains(&3.0), "thread A value missing");
    assert!(scope_values.contains(&5.0), "thread B value missing");
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn frontend_counters_record_scheduling_metrics_when_stdout_is_suppressed() {
    let _guard = super::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();

    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, 1);
    add_frontend_counter(FrontendCounter::ModuleCompilationParallelTaskCount, 2);
    add_frontend_counter(FrontendCounter::FilePreparationSerialModuleCount, 3);
    add_frontend_counter(FrontendCounter::FilePreparationParallelModuleCount, 4);
    add_frontend_counter(FrontendCounter::FilePreparationStrategySmallSerialCount, 5);
    add_frontend_counter(
        FrontendCounter::FilePreparationStrategyByteThresholdSerialCount,
        6,
    );
    add_frontend_counter(FrontendCounter::FilePreparationStrategyParallelCount, 7);
    add_frontend_counter(
        FrontendCounter::FilePreparationStrategyParallelPerFileCount,
        8,
    );
    add_frontend_counter(FrontendCounter::FilePreparationStrategyChunkedCount, 9);
    add_frontend_counter(FrontendCounter::FilePreparationInputFileCount, 10);
    add_frontend_counter(FrontendCounter::FilePreparationInputByteCount, 11);
    add_frontend_counter(FrontendCounter::FilePreparationResultMergeCount, 12);
    add_frontend_counter(FrontendCounter::FilePreparationIdentityRemapCount, 13);
    add_frontend_counter(FrontendCounter::FilePreparationNonIdentityRemapCount, 14);
    add_frontend_counter(FrontendCounter::Stage0SourceCacheHitCount, 15);
    add_frontend_counter(FrontendCounter::Stage0SourceCacheMissCount, 16);
    add_frontend_counter(FrontendCounter::Stage0ParallelSourceLoadCount, 17);
    add_frontend_counter(FrontendCounter::Stage0SerialSourceLoadCount, 18);
    add_frontend_counter(FrontendCounter::Stage0SourceBytesLoaded, 19);

    log_frontend_counters();

    let observations = timing_session.finish();

    assert_counter_value(
        &observations.counters,
        "module_compilation_serial_count",
        1.0,
    );
    assert_counter_value(
        &observations.counters,
        "module_compilation_parallel_task_count",
        2.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_serial_module_count",
        3.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_parallel_module_count",
        4.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_strategy_small_serial_count",
        5.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_strategy_byte_threshold_serial_count",
        6.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_strategy_parallel_count",
        7.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_strategy_parallel_per_file_count",
        8.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_strategy_chunked_count",
        9.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_input_file_count",
        10.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_input_byte_count",
        11.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_result_merge_count",
        12.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_identity_remap_count",
        13.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_non_identity_remap_count",
        14.0,
    );
    assert_counter_value(
        &observations.counters,
        "stage0_source_cache_hit_count",
        15.0,
    );
    assert_counter_value(
        &observations.counters,
        "stage0_source_cache_miss_count",
        16.0,
    );
    assert_counter_value(
        &observations.counters,
        "stage0_parallel_source_load_count",
        17.0,
    );
    assert_counter_value(
        &observations.counters,
        "stage0_serial_source_load_count",
        18.0,
    );
    assert_counter_value(&observations.counters, "stage0_source_bytes_loaded", 19.0);
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn ast_body_root_and_scope_counters_record_stable_metrics() {
    let _guard = super::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();

    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    add_frontend_counter(FrontendCounter::AstFunctionBodyRootCount, 5);
    add_frontend_counter(FrontendCounter::AstStartBodyRootCount, 1);
    add_frontend_counter(FrontendCounter::AstConstTemplateFoldedCount, 3);
    add_frontend_counter(FrontendCounter::AstRootScopeArenaCount, 9);

    log_frontend_counters();

    let observations = timing_session.finish();

    assert_counter_value(&observations.counters, "ast_function_body_root_count", 5.0);
    assert_counter_value(&observations.counters, "ast_start_body_root_count", 1.0);
    assert_counter_value(
        &observations.counters,
        "ast_const_template_folded_count",
        3.0,
    );
    assert_counter_value(&observations.counters, "ast_root_scope_arena_count", 9.0);
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
fn assert_counter_value(
    counters: &[crate::timing::BenchmarkObservationMetric],
    name: &str,
    expected: f64,
) {
    let actual = counters
        .iter()
        .find(|counter| counter.name == name)
        .map(|counter| counter.value)
        .unwrap_or(-1.0);

    assert_eq!(actual, expected, "counter `{name}` did not match");
}
