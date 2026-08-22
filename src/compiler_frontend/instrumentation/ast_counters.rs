//! Detailed AST build instrumentation.
//!
//! WHAT: tracks local-only AST churn counters for performance-sensitive parser, emitter, and
//! finalizer paths.
//! WHY: benchmark runs built with `benchmark_counters` need objective evidence for small timing
//! shifts, while normal compiler output must remain unchanged. Counter storage and logging are
//! gated by `benchmark_counters`, independent of `detailed_timers`; AST substage *timings*
//! remain gated by `detailed_timers`.

#[repr(usize)]
#[derive(Copy, Clone)]
// Counter variants are intentionally dormant in normal builds where storage is disabled.
#[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
pub(crate) enum AstCounter {
    // Scope-frame churn.
    ScopeContextsCreated,
    ScopeMaxFrameDepth,
    ScopeFrameLookupAncestorSteps,
    ScopeFrameRedeclarationAncestorChecks,
    ScopeLocalDeclarationsInserted,

    // Expression parser pressure.
    BoundedExpressionTokenWindows,
    BoundedExpressionTokenCopiesAvoided,

    // Module constant-resolution pass pressure.
    ConstantResolutionContextsCreated,
    ConstantsResolved,
    ModuleConstantDeclarationClones,

    // Expression ordering, operator typing and constant-folding pressure.
    ExpressionOrderingInputItems,
    ExpressionTypedStackItems,
    ExpressionFoldItems,
    ExpressionOperandClones,
    DiagnosticDataTypeMaterialisations,

    // Static Bool control-flow specialisation inputs.
    BranchLocalGenericRequests,

    // Template parsing and folding pressure.
    TemplateWrapperApplications,
    TemplateFoldLoopIterations,
    TemplateNormalizationNodesVisited,
    ModuleConstantNormalizationExpressionsVisited,
    TemplatesFoldedDuringFinalization,

    // TIR-native head-chain composition counters.
    TemplateTirHeadChainCompositionCalls,
    TemplateTirHeadChainCompositionHits,

    // TIR-native `$children(..)` wrapper application counters.
    TemplateTirChildWrapperCalls,
    TemplateTirChildWrapperHits,

    RuntimeTemplateHandoffsRefreshedForHir,
    RuntimeSlotHandoffsMaterialized,
    RuntimeSlotHandoffOwnedNodesMaterialized,
    RuntimeTemplateHandoffsMaterialized,

    // Additional template churn pressure.
    TemplateNestedTemplateParses,
    TemplateBodyTokenVisits,
    TemplateTextBytesParsed,
    TemplateFoldOutputBytes,
    TemplateEstimatedFoldOutputBytes,
    TemplateFoldOutputEstimateMissBytes,
    TemplateFoldStringInternCalls,
    TemplateFoldExpressionCloneRequests,
    TemplateFoldExpressionOwnedRewrites,
    TemplateFoldBindingSubstitutions,

    // AST environment/type-resolution pressure.
    TypeResolutionCalls,
    VisibleTypeLookupAttempts,
    VisibleTypeAliasLookupAttempts,
    VisibleSourceTypeLookupAttempts,
    ReceiverCatalogHeadersScanned,
    ReceiverMethodsRegistered,
    DeclarationReplacementsByPath,
    PublicSurfaceValidationChecks,

    // Field/receiver lowering pressure.
    PostfixReceiverNodesCopied,

    // Template IR (TIR) store, preparation, and folding pressure.
    TirTemplatesCreated,
    TirNodesCreated,
    TirTextNodesCreated,
    TirTextBytesRecorded,
    TirMaxDepth,
    TirWrapperSetsCreated,
    TirWrapperSetReuseHits,
    TirPreparationAttempts,
    TirPreparationNodesVisited,

    // TIR fold counters.
    TirFoldTemplatesFolded,
    TirFoldNodesVisited,
    TirFoldOutputBytes,
    TirFoldStringInternCalls,

    // Exact-view finalization attribution counters.
    //
    // WHAT: fine-grained attribution for module-store view folds and fold-cache
    // behavior.
    // WHY: broad TIR materialization counters cannot identify which finalization
    // paths drove view-fold volume.
    /// Finalization view fold attempt that passed reference and store
    /// validation and reached view construction.
    TirFinalizationFoldAttempts,

    /// Finalization view fold completed (folded or classified
    /// non-renderable through the view path, without falling back).
    TirFinalizationFoldSuccesses,

    /// Prepared exact-view fold entries from AST finalization, expression
    /// emission, and documentation-fragment callers.
    TirViewFoldsAttempted,

    /// A prepared exact-view fold ran with neither an expression nor a slot overlay.
    TirViewFoldOverlayEmpty,

    /// A prepared exact-view fold ran with an expression overlay but no slot overlay.
    TirViewFoldOverlayExpressionOnly,

    /// A prepared exact-view fold ran with a slot overlay but no expression overlay.
    TirViewFoldOverlaySlotOnly,

    /// A prepared exact-view fold ran with both an expression and a slot overlay.
    TirViewFoldOverlayExpressionAndSlot,

    /// A prepared exact-view fold ran with a wrapper-context overlay present
    /// (orthogonal to the expression/slot shape).
    TirViewFoldWrapperContextPresent,

    /// Prepared exact-view fold cache lookups that returned a cached emission.
    TirFoldCacheHits,

    /// Prepared exact-view fold cache lookups that missed and recomputed the fold.
    TirFoldCacheMisses,

    /// Top-level TIR subtree copy entries used by runtime planning and composition.
    TirCopyPasses,

    /// Slot schema or ordered-placeholder walks over TIR trees.
    TirSlotSchemaWalks,

    /// Public slot-contribution routing entries.
    TirContributionRoutingCalls,

    /// Keyed lookups inside expression, slot-resolution, or wrapper-context overlays.
    TirOverlayLookups,
}
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::compiler_messages::compiler_dev_logging::log_benchmark_counter;

#[cfg(feature = "benchmark_counters")]
mod detailed {
    use super::AstCounter;
    use super::log_benchmark_counter;
    use std::cell::RefCell;

    const COUNTER_COUNT: usize = AstCounter::TirOverlayLookups as usize + 1;

    thread_local! {
        /// Per-thread AST counter store.
        ///
        /// WHAT: each concurrently compiled module/task gets an isolated counter set
        /// so that reset/add/log cycles on one worker cannot corrupt another worker's
        /// snapshot.
        /// WHY: AST construction runs inside rayon worker threads; process-global
        /// atomics were reset by overlapping module builds, producing impossible
        /// detailed counter snapshots.
        static COUNTERS: RefCell<[usize; COUNTER_COUNT]> = const { RefCell::new([0; COUNTER_COUNT]) };
    }

    impl AstCounter {
        /// Stable dense index for this counter in the per-thread [`COUNTERS`] array.
        fn index(self) -> usize {
            self as usize
        }
    }

    pub(crate) fn reset_ast_counters() {
        COUNTERS.with(|counters| counters.borrow_mut().fill(0));
    }

    pub(crate) fn increment_ast_counter(counter: AstCounter) {
        add_ast_counter(counter, 1);
    }

    pub(crate) fn add_ast_counter(counter: AstCounter, amount: usize) {
        let index = counter.index();
        COUNTERS.with(|counters| counters.borrow_mut()[index] += amount);
    }

    pub(crate) fn record_ast_counter_max(counter: AstCounter, value: usize) {
        let index = counter.index();
        COUNTERS.with(|counters| {
            let mut array = counters.borrow_mut();
            if value > array[index] {
                array[index] = value;
            }
        });
    }

    pub(crate) fn log_ast_counters() {
        // With timers, counter call sites record only — stable MOTH_BENCH counter
        // lines and any human counter summary are emitted from the drained
        // snapshot after the command total. Without timers, log_benchmark_counter
        // emits directly.
        for &counter in all_counters() {
            let value = counter_value(counter);
            log_benchmark_counter(counter_metric_name(counter), value as f64);
        }
    }

    fn all_counters() -> &'static [AstCounter] {
        &[
            AstCounter::ScopeContextsCreated,
            AstCounter::ScopeMaxFrameDepth,
            AstCounter::ScopeFrameLookupAncestorSteps,
            AstCounter::ScopeFrameRedeclarationAncestorChecks,
            AstCounter::ScopeLocalDeclarationsInserted,
            AstCounter::BoundedExpressionTokenWindows,
            AstCounter::BoundedExpressionTokenCopiesAvoided,
            AstCounter::ConstantResolutionContextsCreated,
            AstCounter::ConstantsResolved,
            AstCounter::ModuleConstantDeclarationClones,
            AstCounter::ExpressionOrderingInputItems,
            AstCounter::ExpressionTypedStackItems,
            AstCounter::ExpressionFoldItems,
            AstCounter::ExpressionOperandClones,
            AstCounter::DiagnosticDataTypeMaterialisations,
            AstCounter::BranchLocalGenericRequests,
            AstCounter::TemplateWrapperApplications,
            AstCounter::TemplateFoldLoopIterations,
            AstCounter::TemplateNormalizationNodesVisited,
            AstCounter::ModuleConstantNormalizationExpressionsVisited,
            AstCounter::TemplatesFoldedDuringFinalization,
            AstCounter::TemplateTirHeadChainCompositionCalls,
            AstCounter::TemplateTirHeadChainCompositionHits,
            AstCounter::TemplateTirChildWrapperCalls,
            AstCounter::TemplateTirChildWrapperHits,
            AstCounter::RuntimeTemplateHandoffsRefreshedForHir,
            AstCounter::RuntimeSlotHandoffsMaterialized,
            AstCounter::RuntimeSlotHandoffOwnedNodesMaterialized,
            AstCounter::RuntimeTemplateHandoffsMaterialized,
            AstCounter::TemplateNestedTemplateParses,
            AstCounter::TemplateBodyTokenVisits,
            AstCounter::TemplateTextBytesParsed,
            AstCounter::TemplateFoldOutputBytes,
            AstCounter::TemplateEstimatedFoldOutputBytes,
            AstCounter::TemplateFoldOutputEstimateMissBytes,
            AstCounter::TemplateFoldStringInternCalls,
            AstCounter::TemplateFoldExpressionCloneRequests,
            AstCounter::TemplateFoldExpressionOwnedRewrites,
            AstCounter::TemplateFoldBindingSubstitutions,
            AstCounter::TypeResolutionCalls,
            AstCounter::VisibleTypeLookupAttempts,
            AstCounter::VisibleTypeAliasLookupAttempts,
            AstCounter::VisibleSourceTypeLookupAttempts,
            AstCounter::ReceiverCatalogHeadersScanned,
            AstCounter::ReceiverMethodsRegistered,
            AstCounter::DeclarationReplacementsByPath,
            AstCounter::PublicSurfaceValidationChecks,
            AstCounter::PostfixReceiverNodesCopied,
            AstCounter::TirTemplatesCreated,
            AstCounter::TirNodesCreated,
            AstCounter::TirTextNodesCreated,
            AstCounter::TirTextBytesRecorded,
            AstCounter::TirMaxDepth,
            AstCounter::TirWrapperSetsCreated,
            AstCounter::TirWrapperSetReuseHits,
            AstCounter::TirPreparationAttempts,
            AstCounter::TirPreparationNodesVisited,
            AstCounter::TirFoldTemplatesFolded,
            AstCounter::TirFoldNodesVisited,
            AstCounter::TirFoldOutputBytes,
            AstCounter::TirFoldStringInternCalls,
            AstCounter::TirFinalizationFoldAttempts,
            AstCounter::TirFinalizationFoldSuccesses,
            AstCounter::TirViewFoldsAttempted,
            AstCounter::TirViewFoldOverlayEmpty,
            AstCounter::TirViewFoldOverlayExpressionOnly,
            AstCounter::TirViewFoldOverlaySlotOnly,
            AstCounter::TirViewFoldOverlayExpressionAndSlot,
            AstCounter::TirViewFoldWrapperContextPresent,
            AstCounter::TirFoldCacheHits,
            AstCounter::TirFoldCacheMisses,
            AstCounter::TirCopyPasses,
            AstCounter::TirSlotSchemaWalks,
            AstCounter::TirContributionRoutingCalls,
            AstCounter::TirOverlayLookups,
        ]
    }

    fn counter_metric_name(counter: AstCounter) -> &'static str {
        match counter {
            AstCounter::ScopeContextsCreated => "ast_scope_contexts_created",
            AstCounter::ScopeMaxFrameDepth => "ast_scope_max_frame_depth",
            AstCounter::ScopeFrameLookupAncestorSteps => "ast_scope_frame_lookup_ancestor_steps",
            AstCounter::ScopeFrameRedeclarationAncestorChecks => {
                "ast_scope_frame_redeclaration_ancestor_checks"
            }
            AstCounter::ScopeLocalDeclarationsInserted => "ast_scope_local_declarations_inserted",
            AstCounter::BoundedExpressionTokenWindows => "ast_bounded_expression_token_windows",
            AstCounter::BoundedExpressionTokenCopiesAvoided => {
                "ast_bounded_expression_token_copies_avoided"
            }
            AstCounter::ConstantResolutionContextsCreated => {
                "ast_constant_resolution_contexts_created"
            }
            AstCounter::ConstantsResolved => "ast_constants_resolved",
            AstCounter::ModuleConstantDeclarationClones => "ast_module_constant_declaration_clones",

            AstCounter::ExpressionOrderingInputItems => "ast_expression_ordering_input_items",
            AstCounter::ExpressionTypedStackItems => "ast_expression_typed_stack_items",
            AstCounter::ExpressionFoldItems => "ast_expression_fold_items",
            AstCounter::ExpressionOperandClones => "ast_expression_operand_clones",
            AstCounter::DiagnosticDataTypeMaterialisations => {
                "ast_diagnostic_data_type_materialisations"
            }

            AstCounter::BranchLocalGenericRequests => "ast_branch_local_generic_requests",

            AstCounter::TemplateWrapperApplications => "ast_template_wrapper_applications",
            AstCounter::TemplateFoldLoopIterations => "ast_template_fold_loop_iterations",
            AstCounter::TemplateNormalizationNodesVisited => {
                "ast_template_normalization_nodes_visited"
            }
            AstCounter::ModuleConstantNormalizationExpressionsVisited => {
                "ast_module_constant_normalization_expressions_visited"
            }
            AstCounter::TemplatesFoldedDuringFinalization => {
                "ast_templates_folded_during_finalization"
            }

            AstCounter::TemplateTirHeadChainCompositionCalls => {
                "ast_template_tir_head_chain_composition_calls"
            }
            AstCounter::TemplateTirHeadChainCompositionHits => {
                "ast_template_tir_head_chain_composition_hits"
            }
            AstCounter::TemplateTirChildWrapperCalls => "ast_template_tir_child_wrapper_calls",
            AstCounter::TemplateTirChildWrapperHits => "ast_template_tir_child_wrapper_hits",

            AstCounter::RuntimeTemplateHandoffsRefreshedForHir => {
                "ast_runtime_template_handoffs_refreshed_for_hir"
            }
            AstCounter::RuntimeSlotHandoffsMaterialized => "ast_runtime_slot_handoffs_materialized",
            AstCounter::RuntimeSlotHandoffOwnedNodesMaterialized => {
                "ast_runtime_slot_handoff_owned_nodes_materialized"
            }
            AstCounter::RuntimeTemplateHandoffsMaterialized => {
                "ast_runtime_template_handoffs_materialized"
            }
            AstCounter::TemplateNestedTemplateParses => "ast_template_nested_template_parses",
            AstCounter::TemplateBodyTokenVisits => "ast_template_body_token_visits",
            AstCounter::TemplateTextBytesParsed => "ast_template_text_bytes_parsed",
            AstCounter::TemplateFoldOutputBytes => "ast_template_fold_output_bytes",
            AstCounter::TemplateEstimatedFoldOutputBytes => {
                "ast_template_estimated_fold_output_bytes"
            }
            AstCounter::TemplateFoldOutputEstimateMissBytes => {
                "ast_template_fold_output_estimate_miss_bytes"
            }
            AstCounter::TemplateFoldStringInternCalls => "ast_template_fold_string_intern_calls",
            AstCounter::TemplateFoldExpressionCloneRequests => {
                "ast_template_fold_expression_clone_requests"
            }
            AstCounter::TemplateFoldExpressionOwnedRewrites => {
                "ast_template_fold_expression_owned_rewrites"
            }
            AstCounter::TemplateFoldBindingSubstitutions => {
                "ast_template_fold_binding_substitutions"
            }

            AstCounter::TypeResolutionCalls => "ast_type_resolution_calls",
            AstCounter::VisibleTypeLookupAttempts => "ast_visible_type_lookup_attempts",
            AstCounter::VisibleTypeAliasLookupAttempts => "ast_visible_type_alias_lookup_attempts",
            AstCounter::VisibleSourceTypeLookupAttempts => {
                "ast_visible_source_type_lookup_attempts"
            }
            AstCounter::ReceiverCatalogHeadersScanned => "ast_receiver_catalog_headers_scanned",
            AstCounter::ReceiverMethodsRegistered => "ast_receiver_methods_registered",
            AstCounter::DeclarationReplacementsByPath => "ast_declaration_replacements_by_path",
            AstCounter::PublicSurfaceValidationChecks => "ast_public_surface_validation_checks",
            AstCounter::PostfixReceiverNodesCopied => "ast_postfix_receiver_nodes_copied",

            AstCounter::TirTemplatesCreated => "ast_tir_templates_created",
            AstCounter::TirNodesCreated => "ast_tir_nodes_created",
            AstCounter::TirTextNodesCreated => "ast_tir_text_nodes_created",
            AstCounter::TirTextBytesRecorded => "ast_tir_text_bytes_recorded",
            AstCounter::TirMaxDepth => "ast_tir_max_depth",
            AstCounter::TirWrapperSetsCreated => "ast_tir_wrapper_sets_created",
            AstCounter::TirWrapperSetReuseHits => "ast_tir_wrapper_set_reuse_hits",
            AstCounter::TirPreparationAttempts => "ast_tir_preparation_attempts",
            AstCounter::TirPreparationNodesVisited => "ast_tir_preparation_nodes_visited",

            AstCounter::TirFoldTemplatesFolded => "ast_tir_fold_templates_folded",
            AstCounter::TirFoldNodesVisited => "ast_tir_fold_nodes_visited",
            AstCounter::TirFoldOutputBytes => "ast_tir_fold_output_bytes",
            AstCounter::TirFoldStringInternCalls => "ast_tir_fold_string_intern_calls",

            AstCounter::TirFinalizationFoldAttempts => "ast_tir_finalization_fold_attempts",
            AstCounter::TirFinalizationFoldSuccesses => "ast_tir_finalization_fold_successes",
            AstCounter::TirViewFoldsAttempted => "ast_tir_view_folds_attempted",
            AstCounter::TirViewFoldOverlayEmpty => "ast_tir_view_fold_overlay_empty",
            AstCounter::TirViewFoldOverlayExpressionOnly => {
                "ast_tir_view_fold_overlay_expression_only"
            }
            AstCounter::TirViewFoldOverlaySlotOnly => "ast_tir_view_fold_overlay_slot_only",
            AstCounter::TirViewFoldOverlayExpressionAndSlot => {
                "ast_tir_view_fold_overlay_expression_and_slot"
            }
            AstCounter::TirViewFoldWrapperContextPresent => {
                "ast_tir_view_fold_wrapper_context_present"
            }
            AstCounter::TirFoldCacheHits => "ast_tir_fold_cache_hits",
            AstCounter::TirFoldCacheMisses => "ast_tir_fold_cache_misses",
            AstCounter::TirCopyPasses => "ast_tir_copy_passes",
            AstCounter::TirSlotSchemaWalks => "ast_tir_slot_schema_walks",
            AstCounter::TirContributionRoutingCalls => "ast_tir_contribution_routing_calls",
            AstCounter::TirOverlayLookups => "ast_tir_overlay_lookups",
        }
    }

    fn counter_value(counter: AstCounter) -> usize {
        let index = counter.index();
        COUNTERS.with(|counters| counters.borrow()[index])
    }

    /// Test-only readback for per-thread AST counter values.
    ///
    /// WHAT: lets unit tests assert that a specific production path incremented
    ///       the expected counter without relying on stdout or the benchmark
    ///       collector, which would need cross-test serialization.
    /// WHY: the public instrumentation API is intentionally write-only so normal
    ///      compiler code cannot read stale counter state.
    #[cfg(test)]
    pub(crate) fn test_read_ast_counter(counter: AstCounter) -> usize {
        counter_value(counter)
    }
}

#[cfg(feature = "benchmark_counters")]
pub(crate) use detailed::{
    add_ast_counter, increment_ast_counter, log_ast_counters, record_ast_counter_max,
    reset_ast_counters,
};

#[cfg(all(test, feature = "benchmark_counters"))]
pub(crate) use detailed::test_read_ast_counter;

// Stubs when detailed timers are disabled.
#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn reset_ast_counters() {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn increment_ast_counter(_counter: AstCounter) {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn add_ast_counter(_counter: AstCounter, _amount: usize) {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn record_ast_counter_max(_counter: AstCounter, _value: usize) {}

#[cfg(not(feature = "benchmark_counters"))]
pub(crate) fn log_ast_counters() {}
