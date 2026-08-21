//! Local-only frontend performance instrumentation.
//!
//! WHAT: exposes counters for clone-heavy, cache-sensitive, and remap-heavy frontend paths.
//! WHY: benchmark runs built with `benchmark_counters` need enough local evidence to
//! interpret small end-to-end timing changes, while normal compiler builds must not
//! pay for or print this diagnostic data. Counter storage and logging are gated by
//! `benchmark_counters`, independent of `detailed_timers`.

/// Stable local benchmark counters grouped by the compiler stage that owns the work.
///
/// These counters are diagnostic evidence for benchmark reports. They deliberately stay in one
/// enum so the logging path has one current implementation and metric names remain stable.
#[derive(Clone, Copy)]
pub(crate) enum FrontendCounter {
    // Stage 0 and per-file preparation volume.
    ModuleCount,
    SourceFileCount,
    SourceByteCount,
    PreparedFileCount,
    FilePreparationPassCount,
    TokenCount,
    HeaderCount,
    PathSyntaxRowCount,
    PersistentGenericPathSyntaxSubsetCopyCount,
    PersistentGenericPathSyntaxRowCopyCount,
    DependencyClauseCount,
    DependencySelectionCount,
    RetainedShellCount,
    ResolvedSourcePackageClauseCount,
    ResolvedProviderClauseCount,
    BoundNamespaceClauseCount,
    BoundSelectedNameCount,
    // Structural zero: Stage 0 consumes retained clause facts, so a raw token rescan
    // is unrepresentable. The counter exists so benchmark summaries can assert zero.
    #[allow(dead_code)]
    TokenRescanCount,
    TopLevelDeclarationCount,
    ModuleCompilationSerialCount,
    // Retained for benchmark-history schema stability while boundary generated-function
    // publication keeps module jobs serial. File preparation still uses Rayon independently.
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ModuleCompilationParallelTaskCount,
    FilePreparationSerialModuleCount,
    FilePreparationParallelModuleCount,
    FilePreparationStrategySmallSerialCount,
    FilePreparationStrategyByteThresholdSerialCount,
    FilePreparationStrategyParallelCount,
    FilePreparationStrategyParallelPerFileCount,
    FilePreparationStrategyChunkedCount,
    FilePreparationInputFileCount,
    FilePreparationInputByteCount,
    FilePreparationResultMergeCount,
    FilePreparationIdentityRemapCount,
    FilePreparationNonIdentityRemapCount,
    Stage0SourceCacheHitCount,
    Stage0SourceCacheMissCount,
    Stage0ParallelSourceLoadCount,
    Stage0SerialSourceLoadCount,
    Stage0SourceBytesLoaded,

    // Dependency sorting volume.
    DependencyHeaderCount,
    DependencyEdgeCount,
    DependencyVisitCount,

    // AST construction and compile-time evaluation volume.
    AstHeaderCount,
    AstFunctionCount,
    AstStructCount,
    AstChoiceCount,
    AstConstantCount,
    AstTraitDeclarationCount,
    AstTraitConformanceCount,
    AstReceiverMethodCount,
    AstGenericTemplateCount,
    AstGenericInstanceCount,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    AstFunctionBodyRootCount,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    AstStartBodyRootCount,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    AstConstTemplateFoldedCount,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    AstRootScopeArenaCount,
    ConstantFoldAttemptCount,
    ConstantFoldSuccessCount,
    TemplateCount,
    ConstTemplateCount,
    RuntimeTemplateCount,

    // HIR and borrow-validation volume.
    HirBlockCount,
    HirStatementCount,
    HirFunctionCount,
    BorrowFunctionCount,
    BorrowBlockCount,
    BorrowConflictCheckCount,
    BorrowStateSnapshotCount,
    BorrowStatementVisitCount,
    BorrowTerminatorVisitCount,
    BorrowWorklistIterationCount,
    BorrowStateJoinCount,
    BorrowPlaceAccessCount,
    BorrowStatementFactCount,
    BorrowTerminatorFactCount,
    BorrowValueFactCount,
    ConvergenceInitialBaseBorrowPasses,
    ConvergenceBaseBorrowPasses,
    ConvergenceGeneratedSidecarBorrowPasses,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ConvergenceCompleteGeneratedSummaryMapBuilds,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ConvergenceGeneratedSummaryMapClones,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ConvergencePrivateSummaryMapRebuilds,
    ConvergenceSummaryComparisons,
    ConvergenceSummaryChanges,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ConvergenceStableSidecarsRechecked,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ConvergenceMaxIterations,

    // Implementation-pressure counters from shared frontend data structures.
    TypeEnvironmentFieldsForQueries,
    TypeEnvironmentFieldsReturned,
    TypeEnvironmentVariantsForQueries,
    TypeEnvironmentVariantsReturned,
    TypeEnvironmentSubstituteTypeIdCalls,
    TypeEnvironmentSubstitutionCacheLookups,
    TypeEnvironmentSubstitutionCacheHits,
    TypeEnvironmentSubstitutionCacheMisses,
    TypeCompatibilityCacheLookups,
    TypeCompatibilityCacheHits,
    TypeCompatibilityCacheMisses,
    StringTableFullClones,
    StringTableMergeFromSourceEntriesScanned,
    StringTableDeltaMergeCalls,
    StringTableDeltaEntriesScanned,
    // These identity/non-identity counters are emitted only by explicit
    // benchmark-counter identity scans so default builds avoid extra remap
    // traversal cost.
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    StringTableDeltaIdentityRemaps,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    StringTableDeltaNonIdentityRemaps,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    StringTableDeltaNonIdentityEntries,
    ModuleRemapStringIdsCalls,
    FilePrepareOutputRemapCalls,
    FilePrepareErrorRemapCalls,
    AlreadyGlobalPreparedOutputRemapSkipCount,
    PreparedFileInvariantValidationCount,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    FilePrepareNonIdentityPayloadRemaps,

    // Arena capacity-estimate counters (Phase 1).
    EstimatedScopeFrames,
    ActualScopeFrames,
    ScopeArenaCapacity,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ScopeFrameEstimateToActualBasisPoints,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ScopeArenaCapacityToActualBasisPoints,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ScopeFrameUnderEstimateCount,
    #[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
    ScopeFrameOverEstimateCount,
    CappedCapacityEstimates,

    // External package metadata clone-pressure counters (Phase 3).
    ExternalPackageRegistryCloneCount,
    ExternalPackageDefinitionCloneCount,
    ExternalFunctionDefinitionCloneCount,
    ExternalSymbolPathCloneCount,
    ExternalAbiParameterCloneCount,
}

#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::compiler_messages::compiler_dev_logging::log_benchmark_counter;

#[cfg(feature = "benchmark_counters")]
mod detailed {
    use super::FrontendCounter;
    use super::log_benchmark_counter;
    #[cfg(test)]
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TYPE_ENVIRONMENT_FIELDS_FOR_QUERIES: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_FIELDS_RETURNED: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_VARIANTS_FOR_QUERIES: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_VARIANTS_RETURNED: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_SUBSTITUTE_TYPE_ID_CALLS: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_SUBSTITUTION_CACHE_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_SUBSTITUTION_CACHE_HITS: AtomicUsize = AtomicUsize::new(0);
    static TYPE_ENVIRONMENT_SUBSTITUTION_CACHE_MISSES: AtomicUsize = AtomicUsize::new(0);
    static TYPE_COMPATIBILITY_CACHE_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static TYPE_COMPATIBILITY_CACHE_HITS: AtomicUsize = AtomicUsize::new(0);
    static TYPE_COMPATIBILITY_CACHE_MISSES: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_FULL_CLONES: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_MERGE_FROM_SOURCE_ENTRIES_SCANNED: AtomicUsize = AtomicUsize::new(0);
    static MODULE_REMAP_STRING_IDS_CALLS: AtomicUsize = AtomicUsize::new(0);
    static ESTIMATED_SCOPE_FRAMES: AtomicUsize = AtomicUsize::new(0);
    static ACTUAL_SCOPE_FRAMES: AtomicUsize = AtomicUsize::new(0);
    static SCOPE_ARENA_CAPACITY: AtomicUsize = AtomicUsize::new(0);
    static SCOPE_FRAME_ESTIMATE_TO_ACTUAL_BASIS_POINTS: AtomicUsize = AtomicUsize::new(0);
    static SCOPE_ARENA_CAPACITY_TO_ACTUAL_BASIS_POINTS: AtomicUsize = AtomicUsize::new(0);
    static SCOPE_FRAME_UNDER_ESTIMATE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static SCOPE_FRAME_OVER_ESTIMATE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CAPPED_CAPACITY_ESTIMATES: AtomicUsize = AtomicUsize::new(0);
    static EXTERNAL_PACKAGE_REGISTRY_CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static EXTERNAL_PACKAGE_DEFINITION_CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static EXTERNAL_FUNCTION_DEFINITION_CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static EXTERNAL_SYMBOL_PATH_CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static EXTERNAL_ABI_PARAMETER_CLONE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static MODULE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static SOURCE_FILE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static SOURCE_BYTE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static PREPARED_FILE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_PASS_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TOKEN_COUNT: AtomicUsize = AtomicUsize::new(0);
    static HEADER_COUNT: AtomicUsize = AtomicUsize::new(0);
    static PATH_SYNTAX_ROW_COUNT: AtomicUsize = AtomicUsize::new(0);
    static PERSISTENT_GENERIC_PATH_SYNTAX_SUBSET_COPY_COUNT: AtomicUsize = AtomicUsize::new(0);
    static PERSISTENT_GENERIC_PATH_SYNTAX_ROW_COPY_COUNT: AtomicUsize = AtomicUsize::new(0);
    static DEPENDENCY_CLAUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static DEPENDENCY_SELECTION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RETAINED_SHELL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RESOLVED_SOURCE_PACKAGE_CLAUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RESOLVED_PROVIDER_CLAUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BOUND_NAMESPACE_CLAUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BOUND_SELECTED_NAME_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TOKEN_RESCAN_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TOP_LEVEL_DECLARATION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static MODULE_COMPILATION_SERIAL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static MODULE_COMPILATION_PARALLEL_TASK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_SERIAL_MODULE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_PARALLEL_MODULE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_STRATEGY_SMALL_SERIAL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_STRATEGY_BYTE_THRESHOLD_SERIAL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_STRATEGY_PARALLEL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_STRATEGY_PARALLEL_PER_FILE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_STRATEGY_CHUNKED_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_INPUT_FILE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_INPUT_BYTE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_RESULT_MERGE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_IDENTITY_REMAP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARATION_NON_IDENTITY_REMAP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static STAGE0_SOURCE_CACHE_HIT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static STAGE0_SOURCE_CACHE_MISS_COUNT: AtomicUsize = AtomicUsize::new(0);
    static STAGE0_PARALLEL_SOURCE_LOAD_COUNT: AtomicUsize = AtomicUsize::new(0);
    static STAGE0_SERIAL_SOURCE_LOAD_COUNT: AtomicUsize = AtomicUsize::new(0);
    static STAGE0_SOURCE_BYTES_LOADED: AtomicUsize = AtomicUsize::new(0);
    static DEPENDENCY_HEADER_COUNT: AtomicUsize = AtomicUsize::new(0);
    static DEPENDENCY_EDGE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static DEPENDENCY_VISIT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_HEADER_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_FUNCTION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_STRUCT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_CHOICE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_CONSTANT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_TRAIT_DECLARATION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_TRAIT_CONFORMANCE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_RECEIVER_METHOD_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_GENERIC_TEMPLATE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_GENERIC_INSTANCE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_FUNCTION_BODY_ROOT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_START_BODY_ROOT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_CONST_TEMPLATE_FOLDED_COUNT: AtomicUsize = AtomicUsize::new(0);
    static AST_ROOT_SCOPE_ARENA_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CONSTANT_FOLD_ATTEMPT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CONSTANT_FOLD_SUCCESS_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TEMPLATE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CONST_TEMPLATE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static RUNTIME_TEMPLATE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static HIR_BLOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static HIR_STATEMENT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static HIR_FUNCTION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_FUNCTION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_BLOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_CONFLICT_CHECK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_STATE_SNAPSHOT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_STATEMENT_VISIT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_TERMINATOR_VISIT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_WORKLIST_ITERATION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_STATE_JOIN_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_PLACE_ACCESS_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_STATEMENT_FACT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_TERMINATOR_FACT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static BORROW_VALUE_FACT_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_INITIAL_BASE_BORROW_PASSES: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_BASE_BORROW_PASSES: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_GENERATED_SIDECAR_BORROW_PASSES: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_COMPLETE_GENERATED_SUMMARY_MAP_BUILDS: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_GENERATED_SUMMARY_MAP_CLONES: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_PRIVATE_SUMMARY_MAP_REBUILDS: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_SUMMARY_COMPARISONS: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_SUMMARY_CHANGES: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_STABLE_SIDECARS_RECHECKED: AtomicUsize = AtomicUsize::new(0);
    static CONVERGENCE_MAX_ITERATIONS: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_DELTA_MERGE_CALLS: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_DELTA_ENTRIES_SCANNED: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_DELTA_IDENTITY_REMAPS: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_DELTA_NON_IDENTITY_REMAPS: AtomicUsize = AtomicUsize::new(0);
    static STRING_TABLE_DELTA_NON_IDENTITY_ENTRIES: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARE_OUTPUT_REMAP_CALLS: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARE_ERROR_REMAP_CALLS: AtomicUsize = AtomicUsize::new(0);
    static ALREADY_GLOBAL_PREPARED_OUTPUT_REMAP_SKIP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static PREPARED_FILE_INVARIANT_VALIDATION_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FILE_PREPARE_NON_IDENTITY_PAYLOAD_REMAPS: AtomicUsize = AtomicUsize::new(0);

    #[cfg(test)]
    thread_local! {
        /// Whether this test thread is intentionally capturing global frontend counters.
        ///
        /// WHAT: exact counter tests opt in before mutating the process-global
        ///      frontend counter atomics.
        /// WHY: most unit tests compile frontend snippets under `benchmark_counters`
        ///      without caring about counters. Letting those unrelated tests update
        ///      the same atomics makes reset/read assertions race under parallel
        ///      `cargo test`.
        static TEST_COUNTER_CAPTURE_ACTIVE: Cell<bool> = const { Cell::new(false) };
    }

    /// Gated on `timers` as well as this module's `benchmark_counters` because every test that
    /// opts in also opens a timing session through `start_benchmark_collection`, which only
    /// exists with `timers`. Without the extra gate the counters-only lane compiles a guard no
    /// caller in that configuration can reach, and reports it as dead code.
    #[cfg(all(test, feature = "timers"))]
    pub(crate) struct FrontendCounterTestCaptureGuard {
        previous: bool,
    }

    #[cfg(all(test, feature = "timers"))]
    impl Drop for FrontendCounterTestCaptureGuard {
        fn drop(&mut self) {
            TEST_COUNTER_CAPTURE_ACTIVE.with(|active| active.set(self.previous));
        }
    }

    #[cfg(all(test, feature = "timers"))]
    pub(crate) fn capture_frontend_counters_for_test() -> FrontendCounterTestCaptureGuard {
        let previous = TEST_COUNTER_CAPTURE_ACTIVE.with(|active| {
            let previous = active.get();
            active.set(true);
            previous
        });

        FrontendCounterTestCaptureGuard { previous }
    }

    pub(crate) fn reset_frontend_counters() {
        for &counter in all_counters() {
            atomic_counter(counter).store(0, Ordering::Relaxed);
        }
    }

    pub(crate) fn increment_frontend_counter(counter: FrontendCounter) {
        add_frontend_counter(counter, 1);
    }

    pub(crate) fn add_frontend_counter(counter: FrontendCounter, amount: usize) {
        #[cfg(test)]
        if !test_counter_capture_active() {
            return;
        }

        atomic_counter(counter).fetch_add(amount, Ordering::Relaxed);
    }

    pub(crate) fn log_frontend_counters() {
        // With timers, counter call sites record only — stable MOTH_BENCH counter
        // lines and any human counter summary are emitted from the drained
        // snapshot after the command total. Without timers, log_benchmark_counter
        // emits directly.
        update_scope_capacity_derived_counters();

        for &counter in all_counters() {
            let value = counter_value(counter);
            log_benchmark_counter(counter_metric_name(counter), value as f64);
        }
    }

    fn all_counters() -> &'static [FrontendCounter] {
        &[
            FrontendCounter::ModuleCount,
            FrontendCounter::SourceFileCount,
            FrontendCounter::SourceByteCount,
            FrontendCounter::PreparedFileCount,
            FrontendCounter::FilePreparationPassCount,
            FrontendCounter::TokenCount,
            FrontendCounter::HeaderCount,
            FrontendCounter::PathSyntaxRowCount,
            FrontendCounter::PersistentGenericPathSyntaxSubsetCopyCount,
            FrontendCounter::PersistentGenericPathSyntaxRowCopyCount,
            FrontendCounter::DependencyClauseCount,
            FrontendCounter::DependencySelectionCount,
            FrontendCounter::RetainedShellCount,
            FrontendCounter::ResolvedSourcePackageClauseCount,
            FrontendCounter::ResolvedProviderClauseCount,
            FrontendCounter::BoundNamespaceClauseCount,
            FrontendCounter::BoundSelectedNameCount,
            FrontendCounter::TokenRescanCount,
            FrontendCounter::TopLevelDeclarationCount,
            FrontendCounter::ModuleCompilationSerialCount,
            FrontendCounter::ModuleCompilationParallelTaskCount,
            FrontendCounter::FilePreparationSerialModuleCount,
            FrontendCounter::FilePreparationParallelModuleCount,
            FrontendCounter::FilePreparationStrategySmallSerialCount,
            FrontendCounter::FilePreparationStrategyByteThresholdSerialCount,
            FrontendCounter::FilePreparationStrategyParallelCount,
            FrontendCounter::FilePreparationStrategyParallelPerFileCount,
            FrontendCounter::FilePreparationStrategyChunkedCount,
            FrontendCounter::FilePreparationInputFileCount,
            FrontendCounter::FilePreparationInputByteCount,
            FrontendCounter::FilePreparationResultMergeCount,
            FrontendCounter::FilePreparationIdentityRemapCount,
            FrontendCounter::FilePreparationNonIdentityRemapCount,
            FrontendCounter::Stage0SourceCacheHitCount,
            FrontendCounter::Stage0SourceCacheMissCount,
            FrontendCounter::Stage0ParallelSourceLoadCount,
            FrontendCounter::Stage0SerialSourceLoadCount,
            FrontendCounter::Stage0SourceBytesLoaded,
            FrontendCounter::DependencyHeaderCount,
            FrontendCounter::DependencyEdgeCount,
            FrontendCounter::DependencyVisitCount,
            FrontendCounter::AstHeaderCount,
            FrontendCounter::AstFunctionCount,
            FrontendCounter::AstStructCount,
            FrontendCounter::AstChoiceCount,
            FrontendCounter::AstConstantCount,
            FrontendCounter::AstTraitDeclarationCount,
            FrontendCounter::AstTraitConformanceCount,
            FrontendCounter::AstReceiverMethodCount,
            FrontendCounter::AstGenericTemplateCount,
            FrontendCounter::AstGenericInstanceCount,
            FrontendCounter::AstFunctionBodyRootCount,
            FrontendCounter::AstStartBodyRootCount,
            FrontendCounter::AstConstTemplateFoldedCount,
            FrontendCounter::AstRootScopeArenaCount,
            FrontendCounter::ConstantFoldAttemptCount,
            FrontendCounter::ConstantFoldSuccessCount,
            FrontendCounter::TemplateCount,
            FrontendCounter::ConstTemplateCount,
            FrontendCounter::RuntimeTemplateCount,
            FrontendCounter::HirBlockCount,
            FrontendCounter::HirStatementCount,
            FrontendCounter::HirFunctionCount,
            FrontendCounter::BorrowFunctionCount,
            FrontendCounter::BorrowBlockCount,
            FrontendCounter::BorrowConflictCheckCount,
            FrontendCounter::BorrowStateSnapshotCount,
            FrontendCounter::BorrowStatementVisitCount,
            FrontendCounter::BorrowTerminatorVisitCount,
            FrontendCounter::BorrowWorklistIterationCount,
            FrontendCounter::BorrowStateJoinCount,
            FrontendCounter::BorrowPlaceAccessCount,
            FrontendCounter::BorrowStatementFactCount,
            FrontendCounter::BorrowTerminatorFactCount,
            FrontendCounter::BorrowValueFactCount,
            FrontendCounter::ConvergenceInitialBaseBorrowPasses,
            FrontendCounter::ConvergenceBaseBorrowPasses,
            FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses,
            FrontendCounter::ConvergenceCompleteGeneratedSummaryMapBuilds,
            FrontendCounter::ConvergenceGeneratedSummaryMapClones,
            FrontendCounter::ConvergencePrivateSummaryMapRebuilds,
            FrontendCounter::ConvergenceSummaryComparisons,
            FrontendCounter::ConvergenceSummaryChanges,
            FrontendCounter::ConvergenceStableSidecarsRechecked,
            FrontendCounter::ConvergenceMaxIterations,
            FrontendCounter::TypeEnvironmentFieldsForQueries,
            FrontendCounter::TypeEnvironmentFieldsReturned,
            FrontendCounter::TypeEnvironmentVariantsForQueries,
            FrontendCounter::TypeEnvironmentVariantsReturned,
            FrontendCounter::TypeEnvironmentSubstituteTypeIdCalls,
            FrontendCounter::TypeEnvironmentSubstitutionCacheLookups,
            FrontendCounter::TypeEnvironmentSubstitutionCacheHits,
            FrontendCounter::TypeEnvironmentSubstitutionCacheMisses,
            FrontendCounter::TypeCompatibilityCacheLookups,
            FrontendCounter::TypeCompatibilityCacheHits,
            FrontendCounter::TypeCompatibilityCacheMisses,
            FrontendCounter::StringTableFullClones,
            FrontendCounter::StringTableMergeFromSourceEntriesScanned,
            FrontendCounter::StringTableDeltaMergeCalls,
            FrontendCounter::StringTableDeltaEntriesScanned,
            FrontendCounter::StringTableDeltaIdentityRemaps,
            FrontendCounter::StringTableDeltaNonIdentityRemaps,
            FrontendCounter::StringTableDeltaNonIdentityEntries,
            FrontendCounter::ModuleRemapStringIdsCalls,
            FrontendCounter::FilePrepareOutputRemapCalls,
            FrontendCounter::FilePrepareErrorRemapCalls,
            FrontendCounter::AlreadyGlobalPreparedOutputRemapSkipCount,
            FrontendCounter::PreparedFileInvariantValidationCount,
            FrontendCounter::FilePrepareNonIdentityPayloadRemaps,
            FrontendCounter::EstimatedScopeFrames,
            FrontendCounter::ActualScopeFrames,
            FrontendCounter::ScopeArenaCapacity,
            FrontendCounter::ScopeFrameEstimateToActualBasisPoints,
            FrontendCounter::ScopeArenaCapacityToActualBasisPoints,
            FrontendCounter::ScopeFrameUnderEstimateCount,
            FrontendCounter::ScopeFrameOverEstimateCount,
            FrontendCounter::CappedCapacityEstimates,
            FrontendCounter::ExternalPackageRegistryCloneCount,
            FrontendCounter::ExternalPackageDefinitionCloneCount,
            FrontendCounter::ExternalFunctionDefinitionCloneCount,
            FrontendCounter::ExternalSymbolPathCloneCount,
            FrontendCounter::ExternalAbiParameterCloneCount,
        ]
    }

    #[cfg(test)]
    fn test_counter_capture_active() -> bool {
        TEST_COUNTER_CAPTURE_ACTIVE.with(Cell::get)
    }

    fn atomic_counter(counter: FrontendCounter) -> &'static AtomicUsize {
        match counter {
            FrontendCounter::ModuleCount => &MODULE_COUNT,

            FrontendCounter::SourceFileCount => &SOURCE_FILE_COUNT,

            FrontendCounter::SourceByteCount => &SOURCE_BYTE_COUNT,

            FrontendCounter::PreparedFileCount => &PREPARED_FILE_COUNT,

            FrontendCounter::FilePreparationPassCount => &FILE_PREPARATION_PASS_COUNT,

            FrontendCounter::TokenCount => &TOKEN_COUNT,

            FrontendCounter::HeaderCount => &HEADER_COUNT,

            FrontendCounter::PathSyntaxRowCount => &PATH_SYNTAX_ROW_COUNT,

            FrontendCounter::PersistentGenericPathSyntaxSubsetCopyCount => {
                &PERSISTENT_GENERIC_PATH_SYNTAX_SUBSET_COPY_COUNT
            }

            FrontendCounter::PersistentGenericPathSyntaxRowCopyCount => {
                &PERSISTENT_GENERIC_PATH_SYNTAX_ROW_COPY_COUNT
            }

            FrontendCounter::DependencyClauseCount => &DEPENDENCY_CLAUSE_COUNT,

            FrontendCounter::DependencySelectionCount => &DEPENDENCY_SELECTION_COUNT,

            FrontendCounter::RetainedShellCount => &RETAINED_SHELL_COUNT,

            FrontendCounter::ResolvedSourcePackageClauseCount => {
                &RESOLVED_SOURCE_PACKAGE_CLAUSE_COUNT
            }

            FrontendCounter::ResolvedProviderClauseCount => &RESOLVED_PROVIDER_CLAUSE_COUNT,

            FrontendCounter::BoundNamespaceClauseCount => &BOUND_NAMESPACE_CLAUSE_COUNT,

            FrontendCounter::BoundSelectedNameCount => &BOUND_SELECTED_NAME_COUNT,

            FrontendCounter::TokenRescanCount => &TOKEN_RESCAN_COUNT,

            FrontendCounter::TopLevelDeclarationCount => &TOP_LEVEL_DECLARATION_COUNT,

            FrontendCounter::ModuleCompilationSerialCount => &MODULE_COMPILATION_SERIAL_COUNT,

            FrontendCounter::ModuleCompilationParallelTaskCount => {
                &MODULE_COMPILATION_PARALLEL_TASK_COUNT
            }

            FrontendCounter::FilePreparationSerialModuleCount => {
                &FILE_PREPARATION_SERIAL_MODULE_COUNT
            }

            FrontendCounter::FilePreparationParallelModuleCount => {
                &FILE_PREPARATION_PARALLEL_MODULE_COUNT
            }

            FrontendCounter::FilePreparationStrategySmallSerialCount => {
                &FILE_PREPARATION_STRATEGY_SMALL_SERIAL_COUNT
            }

            FrontendCounter::FilePreparationStrategyByteThresholdSerialCount => {
                &FILE_PREPARATION_STRATEGY_BYTE_THRESHOLD_SERIAL_COUNT
            }

            FrontendCounter::FilePreparationStrategyParallelCount => {
                &FILE_PREPARATION_STRATEGY_PARALLEL_COUNT
            }

            FrontendCounter::FilePreparationStrategyParallelPerFileCount => {
                &FILE_PREPARATION_STRATEGY_PARALLEL_PER_FILE_COUNT
            }

            FrontendCounter::FilePreparationStrategyChunkedCount => {
                &FILE_PREPARATION_STRATEGY_CHUNKED_COUNT
            }

            FrontendCounter::FilePreparationInputFileCount => &FILE_PREPARATION_INPUT_FILE_COUNT,

            FrontendCounter::FilePreparationInputByteCount => &FILE_PREPARATION_INPUT_BYTE_COUNT,

            FrontendCounter::FilePreparationResultMergeCount => {
                &FILE_PREPARATION_RESULT_MERGE_COUNT
            }

            FrontendCounter::FilePreparationIdentityRemapCount => {
                &FILE_PREPARATION_IDENTITY_REMAP_COUNT
            }

            FrontendCounter::FilePreparationNonIdentityRemapCount => {
                &FILE_PREPARATION_NON_IDENTITY_REMAP_COUNT
            }

            FrontendCounter::Stage0SourceCacheHitCount => &STAGE0_SOURCE_CACHE_HIT_COUNT,

            FrontendCounter::Stage0SourceCacheMissCount => &STAGE0_SOURCE_CACHE_MISS_COUNT,

            FrontendCounter::Stage0ParallelSourceLoadCount => &STAGE0_PARALLEL_SOURCE_LOAD_COUNT,

            FrontendCounter::Stage0SerialSourceLoadCount => &STAGE0_SERIAL_SOURCE_LOAD_COUNT,

            FrontendCounter::Stage0SourceBytesLoaded => &STAGE0_SOURCE_BYTES_LOADED,

            FrontendCounter::DependencyHeaderCount => &DEPENDENCY_HEADER_COUNT,

            FrontendCounter::DependencyEdgeCount => &DEPENDENCY_EDGE_COUNT,

            FrontendCounter::DependencyVisitCount => &DEPENDENCY_VISIT_COUNT,

            FrontendCounter::AstHeaderCount => &AST_HEADER_COUNT,

            FrontendCounter::AstFunctionCount => &AST_FUNCTION_COUNT,

            FrontendCounter::AstStructCount => &AST_STRUCT_COUNT,

            FrontendCounter::AstChoiceCount => &AST_CHOICE_COUNT,

            FrontendCounter::AstConstantCount => &AST_CONSTANT_COUNT,

            FrontendCounter::AstTraitDeclarationCount => &AST_TRAIT_DECLARATION_COUNT,

            FrontendCounter::AstTraitConformanceCount => &AST_TRAIT_CONFORMANCE_COUNT,

            FrontendCounter::AstReceiverMethodCount => &AST_RECEIVER_METHOD_COUNT,

            FrontendCounter::AstGenericTemplateCount => &AST_GENERIC_TEMPLATE_COUNT,

            FrontendCounter::AstGenericInstanceCount => &AST_GENERIC_INSTANCE_COUNT,

            FrontendCounter::AstFunctionBodyRootCount => &AST_FUNCTION_BODY_ROOT_COUNT,

            FrontendCounter::AstStartBodyRootCount => &AST_START_BODY_ROOT_COUNT,

            FrontendCounter::AstConstTemplateFoldedCount => &AST_CONST_TEMPLATE_FOLDED_COUNT,

            FrontendCounter::AstRootScopeArenaCount => &AST_ROOT_SCOPE_ARENA_COUNT,

            FrontendCounter::ConstantFoldAttemptCount => &CONSTANT_FOLD_ATTEMPT_COUNT,

            FrontendCounter::ConstantFoldSuccessCount => &CONSTANT_FOLD_SUCCESS_COUNT,

            FrontendCounter::TemplateCount => &TEMPLATE_COUNT,

            FrontendCounter::ConstTemplateCount => &CONST_TEMPLATE_COUNT,

            FrontendCounter::RuntimeTemplateCount => &RUNTIME_TEMPLATE_COUNT,

            FrontendCounter::HirBlockCount => &HIR_BLOCK_COUNT,

            FrontendCounter::HirStatementCount => &HIR_STATEMENT_COUNT,

            FrontendCounter::HirFunctionCount => &HIR_FUNCTION_COUNT,

            FrontendCounter::BorrowFunctionCount => &BORROW_FUNCTION_COUNT,

            FrontendCounter::BorrowBlockCount => &BORROW_BLOCK_COUNT,

            FrontendCounter::BorrowConflictCheckCount => &BORROW_CONFLICT_CHECK_COUNT,

            FrontendCounter::BorrowStateSnapshotCount => &BORROW_STATE_SNAPSHOT_COUNT,

            FrontendCounter::BorrowStatementVisitCount => &BORROW_STATEMENT_VISIT_COUNT,

            FrontendCounter::BorrowTerminatorVisitCount => &BORROW_TERMINATOR_VISIT_COUNT,

            FrontendCounter::BorrowWorklistIterationCount => &BORROW_WORKLIST_ITERATION_COUNT,

            FrontendCounter::BorrowStateJoinCount => &BORROW_STATE_JOIN_COUNT,

            FrontendCounter::BorrowPlaceAccessCount => &BORROW_PLACE_ACCESS_COUNT,

            FrontendCounter::BorrowStatementFactCount => &BORROW_STATEMENT_FACT_COUNT,

            FrontendCounter::BorrowTerminatorFactCount => &BORROW_TERMINATOR_FACT_COUNT,

            FrontendCounter::BorrowValueFactCount => &BORROW_VALUE_FACT_COUNT,

            FrontendCounter::ConvergenceInitialBaseBorrowPasses => {
                &CONVERGENCE_INITIAL_BASE_BORROW_PASSES
            }

            FrontendCounter::ConvergenceBaseBorrowPasses => &CONVERGENCE_BASE_BORROW_PASSES,

            FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses => {
                &CONVERGENCE_GENERATED_SIDECAR_BORROW_PASSES
            }

            FrontendCounter::ConvergenceCompleteGeneratedSummaryMapBuilds => {
                &CONVERGENCE_COMPLETE_GENERATED_SUMMARY_MAP_BUILDS
            }

            FrontendCounter::ConvergenceGeneratedSummaryMapClones => {
                &CONVERGENCE_GENERATED_SUMMARY_MAP_CLONES
            }

            FrontendCounter::ConvergencePrivateSummaryMapRebuilds => {
                &CONVERGENCE_PRIVATE_SUMMARY_MAP_REBUILDS
            }

            FrontendCounter::ConvergenceSummaryComparisons => &CONVERGENCE_SUMMARY_COMPARISONS,

            FrontendCounter::ConvergenceSummaryChanges => &CONVERGENCE_SUMMARY_CHANGES,

            FrontendCounter::ConvergenceStableSidecarsRechecked => {
                &CONVERGENCE_STABLE_SIDECARS_RECHECKED
            }

            FrontendCounter::ConvergenceMaxIterations => &CONVERGENCE_MAX_ITERATIONS,

            FrontendCounter::TypeEnvironmentFieldsForQueries => {
                &TYPE_ENVIRONMENT_FIELDS_FOR_QUERIES
            }

            FrontendCounter::TypeEnvironmentFieldsReturned => &TYPE_ENVIRONMENT_FIELDS_RETURNED,

            FrontendCounter::TypeEnvironmentVariantsForQueries => {
                &TYPE_ENVIRONMENT_VARIANTS_FOR_QUERIES
            }

            FrontendCounter::TypeEnvironmentVariantsReturned => &TYPE_ENVIRONMENT_VARIANTS_RETURNED,

            FrontendCounter::TypeEnvironmentSubstituteTypeIdCalls => {
                &TYPE_ENVIRONMENT_SUBSTITUTE_TYPE_ID_CALLS
            }

            FrontendCounter::TypeEnvironmentSubstitutionCacheLookups => {
                &TYPE_ENVIRONMENT_SUBSTITUTION_CACHE_LOOKUPS
            }

            FrontendCounter::TypeEnvironmentSubstitutionCacheHits => {
                &TYPE_ENVIRONMENT_SUBSTITUTION_CACHE_HITS
            }

            FrontendCounter::TypeEnvironmentSubstitutionCacheMisses => {
                &TYPE_ENVIRONMENT_SUBSTITUTION_CACHE_MISSES
            }

            FrontendCounter::TypeCompatibilityCacheLookups => &TYPE_COMPATIBILITY_CACHE_LOOKUPS,

            FrontendCounter::TypeCompatibilityCacheHits => &TYPE_COMPATIBILITY_CACHE_HITS,

            FrontendCounter::TypeCompatibilityCacheMisses => &TYPE_COMPATIBILITY_CACHE_MISSES,

            FrontendCounter::StringTableFullClones => &STRING_TABLE_FULL_CLONES,

            FrontendCounter::StringTableMergeFromSourceEntriesScanned => {
                &STRING_TABLE_MERGE_FROM_SOURCE_ENTRIES_SCANNED
            }

            FrontendCounter::StringTableDeltaMergeCalls => &STRING_TABLE_DELTA_MERGE_CALLS,

            FrontendCounter::StringTableDeltaEntriesScanned => &STRING_TABLE_DELTA_ENTRIES_SCANNED,

            FrontendCounter::StringTableDeltaIdentityRemaps => &STRING_TABLE_DELTA_IDENTITY_REMAPS,

            FrontendCounter::StringTableDeltaNonIdentityRemaps => {
                &STRING_TABLE_DELTA_NON_IDENTITY_REMAPS
            }

            FrontendCounter::StringTableDeltaNonIdentityEntries => {
                &STRING_TABLE_DELTA_NON_IDENTITY_ENTRIES
            }

            FrontendCounter::ModuleRemapStringIdsCalls => &MODULE_REMAP_STRING_IDS_CALLS,

            FrontendCounter::FilePrepareOutputRemapCalls => &FILE_PREPARE_OUTPUT_REMAP_CALLS,

            FrontendCounter::FilePrepareErrorRemapCalls => &FILE_PREPARE_ERROR_REMAP_CALLS,

            FrontendCounter::AlreadyGlobalPreparedOutputRemapSkipCount => {
                &ALREADY_GLOBAL_PREPARED_OUTPUT_REMAP_SKIP_COUNT
            }

            FrontendCounter::PreparedFileInvariantValidationCount => {
                &PREPARED_FILE_INVARIANT_VALIDATION_COUNT
            }

            FrontendCounter::FilePrepareNonIdentityPayloadRemaps => {
                &FILE_PREPARE_NON_IDENTITY_PAYLOAD_REMAPS
            }

            FrontendCounter::EstimatedScopeFrames => &ESTIMATED_SCOPE_FRAMES,

            FrontendCounter::ActualScopeFrames => &ACTUAL_SCOPE_FRAMES,

            FrontendCounter::ScopeArenaCapacity => &SCOPE_ARENA_CAPACITY,

            FrontendCounter::ScopeFrameEstimateToActualBasisPoints => {
                &SCOPE_FRAME_ESTIMATE_TO_ACTUAL_BASIS_POINTS
            }

            FrontendCounter::ScopeArenaCapacityToActualBasisPoints => {
                &SCOPE_ARENA_CAPACITY_TO_ACTUAL_BASIS_POINTS
            }

            FrontendCounter::ScopeFrameUnderEstimateCount => &SCOPE_FRAME_UNDER_ESTIMATE_COUNT,

            FrontendCounter::ScopeFrameOverEstimateCount => &SCOPE_FRAME_OVER_ESTIMATE_COUNT,

            FrontendCounter::CappedCapacityEstimates => &CAPPED_CAPACITY_ESTIMATES,

            FrontendCounter::ExternalPackageRegistryCloneCount => {
                &EXTERNAL_PACKAGE_REGISTRY_CLONE_COUNT
            }

            FrontendCounter::ExternalPackageDefinitionCloneCount => {
                &EXTERNAL_PACKAGE_DEFINITION_CLONE_COUNT
            }

            FrontendCounter::ExternalFunctionDefinitionCloneCount => {
                &EXTERNAL_FUNCTION_DEFINITION_CLONE_COUNT
            }

            FrontendCounter::ExternalSymbolPathCloneCount => &EXTERNAL_SYMBOL_PATH_CLONE_COUNT,

            FrontendCounter::ExternalAbiParameterCloneCount => &EXTERNAL_ABI_PARAMETER_CLONE_COUNT,
        }
    }

    fn counter_metric_name(counter: FrontendCounter) -> &'static str {
        match counter {
            FrontendCounter::ModuleCount => "module_count",

            FrontendCounter::SourceFileCount => "source_file_count",

            FrontendCounter::SourceByteCount => "source_byte_count",

            FrontendCounter::PreparedFileCount => "prepared_file_count",

            FrontendCounter::FilePreparationPassCount => "file_preparation_pass_count",

            FrontendCounter::TokenCount => "token_count",

            FrontendCounter::HeaderCount => "header_count",

            FrontendCounter::PathSyntaxRowCount => "path_syntax_row_count",

            FrontendCounter::PersistentGenericPathSyntaxSubsetCopyCount => {
                "persistent_generic_path_syntax_subset_copy_count"
            }

            FrontendCounter::PersistentGenericPathSyntaxRowCopyCount => {
                "persistent_generic_path_syntax_row_copy_count"
            }

            FrontendCounter::DependencyClauseCount => "dependency_clause_count",

            FrontendCounter::DependencySelectionCount => "dependency_selection_count",

            FrontendCounter::RetainedShellCount => "retained_shell_count",

            FrontendCounter::ResolvedSourcePackageClauseCount => {
                "resolved_source_package_clause_count"
            }

            FrontendCounter::ResolvedProviderClauseCount => "resolved_provider_clause_count",

            FrontendCounter::BoundNamespaceClauseCount => "bound_namespace_clause_count",

            FrontendCounter::BoundSelectedNameCount => "bound_selected_name_count",

            FrontendCounter::TokenRescanCount => "token_rescan_count",

            FrontendCounter::TopLevelDeclarationCount => "top_level_declaration_count",

            FrontendCounter::ModuleCompilationSerialCount => "module_compilation_serial_count",

            FrontendCounter::ModuleCompilationParallelTaskCount => {
                "module_compilation_parallel_task_count"
            }

            FrontendCounter::FilePreparationSerialModuleCount => {
                "file_preparation_serial_module_count"
            }

            FrontendCounter::FilePreparationParallelModuleCount => {
                "file_preparation_parallel_module_count"
            }

            FrontendCounter::FilePreparationStrategySmallSerialCount => {
                "file_preparation_strategy_small_serial_count"
            }

            FrontendCounter::FilePreparationStrategyByteThresholdSerialCount => {
                "file_preparation_strategy_byte_threshold_serial_count"
            }

            FrontendCounter::FilePreparationStrategyParallelCount => {
                "file_preparation_strategy_parallel_count"
            }

            FrontendCounter::FilePreparationStrategyParallelPerFileCount => {
                "file_preparation_strategy_parallel_per_file_count"
            }

            FrontendCounter::FilePreparationStrategyChunkedCount => {
                "file_preparation_strategy_chunked_count"
            }

            FrontendCounter::FilePreparationInputFileCount => "file_preparation_input_file_count",

            FrontendCounter::FilePreparationInputByteCount => "file_preparation_input_byte_count",

            FrontendCounter::FilePreparationResultMergeCount => {
                "file_preparation_result_merge_count"
            }

            FrontendCounter::FilePreparationIdentityRemapCount => {
                "file_preparation_identity_remap_count"
            }

            FrontendCounter::FilePreparationNonIdentityRemapCount => {
                "file_preparation_non_identity_remap_count"
            }

            FrontendCounter::Stage0SourceCacheHitCount => "stage0_source_cache_hit_count",

            FrontendCounter::Stage0SourceCacheMissCount => "stage0_source_cache_miss_count",

            FrontendCounter::Stage0ParallelSourceLoadCount => "stage0_parallel_source_load_count",

            FrontendCounter::Stage0SerialSourceLoadCount => "stage0_serial_source_load_count",

            FrontendCounter::Stage0SourceBytesLoaded => "stage0_source_bytes_loaded",

            FrontendCounter::DependencyHeaderCount => "dependency_header_count",

            FrontendCounter::DependencyEdgeCount => "dependency_edge_count",

            FrontendCounter::DependencyVisitCount => "dependency_visit_count",

            FrontendCounter::AstHeaderCount => "ast_header_count",

            FrontendCounter::AstFunctionCount => "ast_function_count",

            FrontendCounter::AstStructCount => "ast_struct_count",

            FrontendCounter::AstChoiceCount => "ast_choice_count",

            FrontendCounter::AstConstantCount => "ast_constant_count",

            FrontendCounter::AstTraitDeclarationCount => "ast_trait_declaration_count",

            FrontendCounter::AstTraitConformanceCount => "ast_trait_conformance_count",

            FrontendCounter::AstReceiverMethodCount => "ast_receiver_method_count",

            FrontendCounter::AstGenericTemplateCount => "ast_generic_template_count",

            FrontendCounter::AstGenericInstanceCount => "ast_generic_instance_count",

            FrontendCounter::AstFunctionBodyRootCount => "ast_function_body_root_count",

            FrontendCounter::AstStartBodyRootCount => "ast_start_body_root_count",

            FrontendCounter::AstConstTemplateFoldedCount => "ast_const_template_folded_count",

            FrontendCounter::AstRootScopeArenaCount => "ast_root_scope_arena_count",

            FrontendCounter::ConstantFoldAttemptCount => "constant_fold_attempt_count",

            FrontendCounter::ConstantFoldSuccessCount => "constant_fold_success_count",

            FrontendCounter::TemplateCount => "template_count",

            FrontendCounter::ConstTemplateCount => "const_template_count",

            FrontendCounter::RuntimeTemplateCount => "runtime_template_count",

            FrontendCounter::HirBlockCount => "hir_block_count",

            FrontendCounter::HirStatementCount => "hir_statement_count",

            FrontendCounter::HirFunctionCount => "hir_function_count",

            FrontendCounter::BorrowFunctionCount => "borrow_function_count",

            FrontendCounter::BorrowBlockCount => "borrow_block_count",

            FrontendCounter::BorrowConflictCheckCount => "borrow_conflict_check_count",

            FrontendCounter::BorrowStateSnapshotCount => "borrow_state_snapshot_count",

            FrontendCounter::BorrowStatementVisitCount => "borrow_statement_visit_count",

            FrontendCounter::BorrowTerminatorVisitCount => "borrow_terminator_visit_count",

            FrontendCounter::BorrowWorklistIterationCount => "borrow_worklist_iteration_count",

            FrontendCounter::BorrowStateJoinCount => "borrow_state_join_count",

            FrontendCounter::BorrowPlaceAccessCount => "borrow_place_access_count",

            FrontendCounter::BorrowStatementFactCount => "borrow_statement_fact_count",

            FrontendCounter::BorrowTerminatorFactCount => "borrow_terminator_fact_count",

            FrontendCounter::BorrowValueFactCount => "borrow_value_fact_count",

            FrontendCounter::ConvergenceInitialBaseBorrowPasses => {
                "convergence_initial_base_borrow_passes"
            }

            FrontendCounter::ConvergenceBaseBorrowPasses => "convergence_base_borrow_passes",

            FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses => {
                "convergence_generated_sidecar_borrow_passes"
            }

            FrontendCounter::ConvergenceCompleteGeneratedSummaryMapBuilds => {
                "convergence_complete_generated_summary_map_builds"
            }

            FrontendCounter::ConvergenceGeneratedSummaryMapClones => {
                "convergence_generated_summary_map_clones"
            }

            FrontendCounter::ConvergencePrivateSummaryMapRebuilds => {
                "convergence_private_summary_map_rebuilds"
            }

            FrontendCounter::ConvergenceSummaryComparisons => "convergence_summary_comparisons",

            FrontendCounter::ConvergenceSummaryChanges => "convergence_summary_changes",

            FrontendCounter::ConvergenceStableSidecarsRechecked => {
                "convergence_stable_sidecars_rechecked"
            }

            FrontendCounter::ConvergenceMaxIterations => "convergence_max_iterations",

            FrontendCounter::TypeEnvironmentFieldsForQueries => {
                "type_environment_fields_for_queries"
            }

            FrontendCounter::TypeEnvironmentFieldsReturned => "type_environment_fields_returned",

            FrontendCounter::TypeEnvironmentVariantsForQueries => {
                "type_environment_variants_for_queries"
            }

            FrontendCounter::TypeEnvironmentVariantsReturned => {
                "type_environment_variants_returned"
            }

            FrontendCounter::TypeEnvironmentSubstituteTypeIdCalls => {
                "type_environment_substitute_type_id_calls"
            }

            FrontendCounter::TypeEnvironmentSubstitutionCacheLookups => {
                "type_environment_substitution_cache_lookups"
            }

            FrontendCounter::TypeEnvironmentSubstitutionCacheHits => {
                "type_environment_substitution_cache_hits"
            }

            FrontendCounter::TypeEnvironmentSubstitutionCacheMisses => {
                "type_environment_substitution_cache_misses"
            }

            FrontendCounter::TypeCompatibilityCacheLookups => "type_compatibility_cache_lookups",

            FrontendCounter::TypeCompatibilityCacheHits => "type_compatibility_cache_hits",

            FrontendCounter::TypeCompatibilityCacheMisses => "type_compatibility_cache_misses",

            FrontendCounter::StringTableFullClones => "string_table_full_clones",

            FrontendCounter::StringTableMergeFromSourceEntriesScanned => {
                "string_table_merge_source_entries_scanned"
            }

            FrontendCounter::StringTableDeltaMergeCalls => "string_table_delta_merge_calls",

            FrontendCounter::StringTableDeltaEntriesScanned => "string_table_delta_entries_scanned",

            FrontendCounter::StringTableDeltaIdentityRemaps => "string_table_delta_identity_remaps",

            FrontendCounter::StringTableDeltaNonIdentityRemaps => {
                "string_table_delta_non_identity_remaps"
            }

            FrontendCounter::StringTableDeltaNonIdentityEntries => {
                "string_table_delta_non_identity_entries"
            }

            FrontendCounter::ModuleRemapStringIdsCalls => "module_remap_string_ids_calls",

            FrontendCounter::FilePrepareOutputRemapCalls => "file_prepare_output_remap_calls",

            FrontendCounter::FilePrepareErrorRemapCalls => "file_prepare_error_remap_calls",

            FrontendCounter::AlreadyGlobalPreparedOutputRemapSkipCount => {
                "already_global_prepared_output_remap_skip_count"
            }

            FrontendCounter::PreparedFileInvariantValidationCount => {
                "prepared_file_invariant_validation_count"
            }

            FrontendCounter::FilePrepareNonIdentityPayloadRemaps => {
                "file_prepare_non_identity_payload_remaps"
            }

            FrontendCounter::EstimatedScopeFrames => "estimated_scope_frames",

            FrontendCounter::ActualScopeFrames => "actual_scope_frames",

            FrontendCounter::ScopeArenaCapacity => "scope_arena_capacity",

            FrontendCounter::ScopeFrameEstimateToActualBasisPoints => {
                "scope_frame_estimate_to_actual_bps"
            }

            FrontendCounter::ScopeArenaCapacityToActualBasisPoints => {
                "scope_arena_capacity_to_actual_bps"
            }

            FrontendCounter::ScopeFrameUnderEstimateCount => "scope_frame_under_estimate_count",

            FrontendCounter::ScopeFrameOverEstimateCount => "scope_frame_over_estimate_count",

            FrontendCounter::CappedCapacityEstimates => "capped_capacity_estimates",

            FrontendCounter::ExternalPackageRegistryCloneCount => {
                "external_package_registry_clone_count"
            }

            FrontendCounter::ExternalPackageDefinitionCloneCount => {
                "external_package_definition_clone_count"
            }

            FrontendCounter::ExternalFunctionDefinitionCloneCount => {
                "external_function_definition_clone_count"
            }

            FrontendCounter::ExternalSymbolPathCloneCount => "external_symbol_path_clone_count",

            FrontendCounter::ExternalAbiParameterCloneCount => "external_abi_parameter_clone_count",
        }
    }

    fn counter_value(counter: FrontendCounter) -> usize {
        atomic_counter(counter).load(Ordering::Relaxed)
    }

    fn update_scope_capacity_derived_counters() {
        let estimated = ESTIMATED_SCOPE_FRAMES.load(Ordering::Relaxed);
        let actual = ACTUAL_SCOPE_FRAMES.load(Ordering::Relaxed);
        let capacity = SCOPE_ARENA_CAPACITY.load(Ordering::Relaxed);

        SCOPE_FRAME_ESTIMATE_TO_ACTUAL_BASIS_POINTS
            .store(ratio_basis_points(estimated, actual), Ordering::Relaxed);
        SCOPE_ARENA_CAPACITY_TO_ACTUAL_BASIS_POINTS
            .store(ratio_basis_points(capacity, actual), Ordering::Relaxed);

        if actual > estimated {
            SCOPE_FRAME_UNDER_ESTIMATE_COUNT.store(actual - estimated, Ordering::Relaxed);
            SCOPE_FRAME_OVER_ESTIMATE_COUNT.store(0, Ordering::Relaxed);
        } else {
            SCOPE_FRAME_UNDER_ESTIMATE_COUNT.store(0, Ordering::Relaxed);
            SCOPE_FRAME_OVER_ESTIMATE_COUNT.store(estimated - actual, Ordering::Relaxed);
        }
    }

    fn ratio_basis_points(numerator: usize, denominator: usize) -> usize {
        numerator
            .saturating_mul(10_000)
            .checked_div(denominator)
            .unwrap_or(0)
    }
}

#[cfg(feature = "benchmark_counters")]
pub(crate) use detailed::{
    add_frontend_counter, increment_frontend_counter, log_frontend_counters,
    reset_frontend_counters,
};

#[cfg(all(test, feature = "benchmark_counters", feature = "timers"))]
pub(crate) use detailed::capture_frontend_counters_for_test;

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn reset_frontend_counters() {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn increment_frontend_counter(_counter: FrontendCounter) {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn add_frontend_counter(_counter: FrontendCounter, _amount: usize) {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn log_frontend_counters() {}
