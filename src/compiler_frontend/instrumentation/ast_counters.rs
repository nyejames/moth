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
    // AST-local failure-summary convergence pressure.
    FailureSummaryBodiesClassified,
    /// Top-level statement nodes in bodies reopened by AST-local summary convergence.
    /// Flat unrelated-body fixtures vary this count independently of call-chain length.
    FailureSummaryBodyNodesClassified,

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

    /// Advisory const-fact environments copied for a function body or nested lexical scope.
    ConstFactEnvironmentClones,

    /// Entries copied by those environment clones, summed.
    ConstFactEnvironmentEntriesCloned,

    // Expression ordering, operator typing and constant-folding pressure.
    ExpressionOrderingInputItems,
    ExpressionTypedStackItems,
    ExpressionFoldItems,

    /// RPN items copied whole so a caller can keep its pre-fold input.
    ///
    /// Folding itself consumes its input and copies nothing, so the ordinary arithmetic path
    /// contributes zero. The only contributors are the two template callers whose non-folding
    /// outcome rebuilds a runtime node from the items as they stood before the fold.
    ExpressionOperandClones,

    // Static Bool control-flow specialisation inputs.
    BranchLocalGenericRequests,

    // Template parsing and folding pressure.
    TemplateWrapperApplications,
    TemplateFoldLoopIterations,
    TemplateNormalizationNodesVisited,
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

    /// Top-level TIR subtree copy entries used by runtime planning and composition.
    TirCopyPasses,

    /// Pending bound-range header rows inspected through their counter's ordered queue.
    TirNumericCounterHeadersInspected,

    // Numeric binding-origin query pressure.
    NumericAliasOriginSteps,
    // Numeric exact-receiver selection pressure.
    NumericReceiverWitnessesInspected,

    // Declared numeric substitution lookup and identity-merge pressure.
    NumericDeclaredBindingComparisons,

    // Known-shape DAG traversal pressure.
    NumericShapePinNodesVisited,

    /// Registered numeric intake, header and pattern rows inspected while copying a TIR node.
    TirNumericCopyRowsInspected,

    /// Slot schema or ordered-placeholder walks over TIR trees.
    TirSlotSchemaWalks,

    /// Public slot-contribution routing entries.
    TirContributionRoutingCalls,

    // Published-occurrence census. Additive values are per-AST snapshots; Max suffixes
    // are gauges. Sum snapshots and take maxima offline, including duplicate metric rows.
    CensusModules,
    CensusExpressionRowSizeMax,
    CensusExpressionRowAlignMax,
    CensusExpressionOpSizeMax,
    CensusExpressionOpAlignMax,
    CensusExpressionRows,
    CensusExpressionRowCapacity,
    CensusExpressionRowBytes,
    CensusExpressionRowCapacityBytes,
    CensusExpressionSideEntries,
    CensusExpressionSideCapacity,
    CensusExpressionSideBytes,
    CensusExpressionSideCapacityBytes,
    CensusHirExpressionSizeMax,
    CensusHirExpressionAlignMax,
    CensusHirExpressionKindSizeMax,
    CensusHirExpressionKindAlignMax,
    CensusHirPlaceSizeMax,
    CensusHirPlaceAlignMax,

    /// Keyed lookups inside expression, slot-resolution, or wrapper-context overlays.
    TirOverlayLookups,
}
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::compiler_messages::compiler_dev_logging::log_benchmark_counter;

#[cfg(feature = "benchmark_counters")]
mod detailed {
    use super::AstCounter;
    use super::log_benchmark_counter;
    use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
    use crate::compiler_frontend::hir::places::HirPlace;
    use std::cell::RefCell;
    use std::mem::{align_of, size_of};

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
        // Census snapshots self-emit at each finalizer call (including generated
        // sidecars), so the module-scope log skips that range and cannot duplicate it.
        let census_start = AstCounter::CensusModules as usize;
        let census_end = AstCounter::CensusHirPlaceAlignMax as usize;
        for &counter in all_counters() {
            let index = counter.index();
            if (census_start..=census_end).contains(&index) {
                continue;
            }
            log_ast_counter(counter);
        }
    }

    /// Runs one published census collect and emits only its snapshot.
    ///
    /// WHAT: saves the existing `TirOverlayLookups` traffic, clears only the
    ///       contiguous census range, runs the observer, emits only that range,
    ///       then clears it and restores the saved overlay traffic.
    /// WHY: generated sidecars finalize after the requester already logged, so each
    ///      finalizer call must emit its own snapshot instead of leaving census rows
    ///      for the module-scope log to duplicate or erase. Restoring the overlay
    ///      count keeps observer reads out of the existing semantic metric.
    pub(crate) fn with_expression_census_counters(collect: impl FnOnce()) {
        let census_start = AstCounter::CensusModules as usize;
        let census_end = AstCounter::CensusHirPlaceAlignMax as usize;
        let overlay_index = AstCounter::TirOverlayLookups as usize;
        let saved_overlay = COUNTERS.with(|counters| counters.borrow()[overlay_index]);
        COUNTERS.with(|counters| {
            let mut array = counters.borrow_mut();
            array[census_start..=census_end].fill(0);
        });
        collect();
        record_hir_layout_census();
        for &counter in all_counters() {
            let index = counter.index();
            if !(census_start..=census_end).contains(&index) {
                continue;
            }
            log_ast_counter(counter);
        }
        COUNTERS.with(|counters| {
            let mut array = counters.borrow_mut();
            array[census_start..=census_end].fill(0);
            array[overlay_index] = saved_overlay;
        });
    }

    /// Logs one AST counter into the active collection session.
    fn log_ast_counter(counter: AstCounter) {
        // Under test, only threads that opted into capture reach the shared collector, so an
        // unrelated compile test cannot add AST rows to a guarded test's collection session.
        #[cfg(test)]
        if !crate::compiler_frontend::instrumentation::test_counter_capture_active() {
            return;
        }

        log_benchmark_counter(counter_metric_name(counter), counter_value(counter) as f64);
    }

    /// Constant downstream-layout gauges recorded with each census snapshot.
    ///
    /// WHAT: records the six HIR size/align gauges under the existing census
    ///       `AstCounter` variants after the AST observer collect.
    /// WHY: neutral instrumentation can measure both stages without an AST-to-HIR
    ///      dependency. Every snapshot retains the same eight size/alignment pairs.
    fn record_hir_layout_census() {
        record_ast_counter_max(
            AstCounter::CensusHirExpressionSizeMax,
            size_of::<HirExpression>(),
        );
        record_ast_counter_max(
            AstCounter::CensusHirExpressionAlignMax,
            align_of::<HirExpression>(),
        );
        record_ast_counter_max(
            AstCounter::CensusHirExpressionKindSizeMax,
            size_of::<HirExpressionKind>(),
        );
        record_ast_counter_max(
            AstCounter::CensusHirExpressionKindAlignMax,
            align_of::<HirExpressionKind>(),
        );
        record_ast_counter_max(AstCounter::CensusHirPlaceSizeMax, size_of::<HirPlace>());
        record_ast_counter_max(AstCounter::CensusHirPlaceAlignMax, align_of::<HirPlace>());
    }

    fn all_counters() -> &'static [AstCounter] {
        &[
            AstCounter::FailureSummaryBodiesClassified,
            AstCounter::FailureSummaryBodyNodesClassified,
            AstCounter::ScopeContextsCreated,
            AstCounter::ScopeMaxFrameDepth,
            AstCounter::ScopeFrameLookupAncestorSteps,
            AstCounter::ScopeFrameRedeclarationAncestorChecks,
            AstCounter::ScopeLocalDeclarationsInserted,
            AstCounter::BoundedExpressionTokenWindows,
            AstCounter::BoundedExpressionTokenCopiesAvoided,
            AstCounter::ConstantResolutionContextsCreated,
            AstCounter::ConstantsResolved,
            AstCounter::ConstFactEnvironmentClones,
            AstCounter::ConstFactEnvironmentEntriesCloned,
            AstCounter::ExpressionOrderingInputItems,
            AstCounter::ExpressionTypedStackItems,
            AstCounter::ExpressionFoldItems,
            AstCounter::ExpressionOperandClones,
            AstCounter::BranchLocalGenericRequests,
            AstCounter::TemplateWrapperApplications,
            AstCounter::TemplateFoldLoopIterations,
            AstCounter::TemplateNormalizationNodesVisited,
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
            AstCounter::TirCopyPasses,
            AstCounter::TirNumericCounterHeadersInspected,
            AstCounter::NumericAliasOriginSteps,
            AstCounter::NumericReceiverWitnessesInspected,
            AstCounter::NumericDeclaredBindingComparisons,
            AstCounter::NumericShapePinNodesVisited,
            AstCounter::TirNumericCopyRowsInspected,
            AstCounter::TirSlotSchemaWalks,
            AstCounter::TirContributionRoutingCalls,
            AstCounter::CensusModules,
            AstCounter::CensusExpressionRowSizeMax,
            AstCounter::CensusExpressionRowAlignMax,
            AstCounter::CensusExpressionOpSizeMax,
            AstCounter::CensusExpressionOpAlignMax,
            AstCounter::CensusExpressionRows,
            AstCounter::CensusExpressionRowCapacity,
            AstCounter::CensusExpressionRowBytes,
            AstCounter::CensusExpressionRowCapacityBytes,
            AstCounter::CensusExpressionSideEntries,
            AstCounter::CensusExpressionSideCapacity,
            AstCounter::CensusExpressionSideBytes,
            AstCounter::CensusExpressionSideCapacityBytes,
            AstCounter::CensusHirExpressionSizeMax,
            AstCounter::CensusHirExpressionAlignMax,
            AstCounter::CensusHirExpressionKindSizeMax,
            AstCounter::CensusHirExpressionKindAlignMax,
            AstCounter::CensusHirPlaceSizeMax,
            AstCounter::CensusHirPlaceAlignMax,
            AstCounter::TirOverlayLookups,
        ]
    }

    fn counter_metric_name(counter: AstCounter) -> &'static str {
        match counter {
            AstCounter::FailureSummaryBodiesClassified => "ast_failure_summary_bodies_classified",
            AstCounter::FailureSummaryBodyNodesClassified => {
                "ast_failure_summary_body_nodes_classified"
            }
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
            AstCounter::ConstFactEnvironmentClones => "ast_const_fact_environment_clones",
            AstCounter::ConstFactEnvironmentEntriesCloned => {
                "ast_const_fact_environment_entries_cloned"
            }

            AstCounter::ExpressionOrderingInputItems => "ast_expression_ordering_input_items",
            AstCounter::ExpressionTypedStackItems => "ast_expression_typed_stack_items",
            AstCounter::ExpressionFoldItems => "ast_expression_fold_items",
            AstCounter::ExpressionOperandClones => "ast_expression_operand_clones",

            AstCounter::BranchLocalGenericRequests => "ast_branch_local_generic_requests",

            AstCounter::TemplateWrapperApplications => "ast_template_wrapper_applications",
            AstCounter::TemplateFoldLoopIterations => "ast_template_fold_loop_iterations",
            AstCounter::TemplateNormalizationNodesVisited => {
                "ast_template_normalization_nodes_visited"
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
            AstCounter::TirCopyPasses => "ast_tir_copy_passes",
            AstCounter::NumericAliasOriginSteps => "ast_numeric_alias_origin_steps",
            AstCounter::NumericReceiverWitnessesInspected => {
                "ast_numeric_receiver_witnesses_inspected"
            }
            AstCounter::NumericDeclaredBindingComparisons => {
                "ast_numeric_declared_binding_comparisons"
            }
            AstCounter::NumericShapePinNodesVisited => "ast_numeric_shape_pin_nodes_visited",
            AstCounter::TirNumericCounterHeadersInspected => {
                "ast_tir_numeric_counter_headers_inspected"
            }
            AstCounter::TirNumericCopyRowsInspected => "ast_tir_numeric_copy_rows_inspected",
            AstCounter::TirSlotSchemaWalks => "ast_tir_slot_schema_walks",
            AstCounter::TirContributionRoutingCalls => "ast_tir_contribution_routing_calls",
            AstCounter::CensusModules => "ast_census_modules",
            AstCounter::CensusExpressionRowSizeMax => "ast_census_expression_row_size_max",
            AstCounter::CensusExpressionRowAlignMax => "ast_census_expression_row_align_max",
            AstCounter::CensusExpressionOpSizeMax => "ast_census_expression_op_size_max",
            AstCounter::CensusExpressionOpAlignMax => "ast_census_expression_op_align_max",
            AstCounter::CensusExpressionRows => "ast_census_expression_rows",
            AstCounter::CensusExpressionRowCapacity => "ast_census_expression_row_capacity",
            AstCounter::CensusExpressionRowBytes => "ast_census_expression_row_bytes",
            AstCounter::CensusExpressionRowCapacityBytes => {
                "ast_census_expression_row_capacity_bytes"
            }
            AstCounter::CensusExpressionSideEntries => "ast_census_expression_side_entries",
            AstCounter::CensusExpressionSideCapacity => "ast_census_expression_side_capacity",
            AstCounter::CensusExpressionSideBytes => "ast_census_expression_side_bytes",
            AstCounter::CensusExpressionSideCapacityBytes => {
                "ast_census_expression_side_capacity_bytes"
            }
            AstCounter::CensusHirExpressionSizeMax => "ast_census_hir_expression_size_max",
            AstCounter::CensusHirExpressionAlignMax => "ast_census_hir_expression_align_max",
            AstCounter::CensusHirExpressionKindSizeMax => "ast_census_hir_expression_kind_size_max",
            AstCounter::CensusHirExpressionKindAlignMax => {
                "ast_census_hir_expression_kind_align_max"
            }
            AstCounter::CensusHirPlaceSizeMax => "ast_census_hir_place_size_max",
            AstCounter::CensusHirPlaceAlignMax => "ast_census_hir_place_align_max",
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
    reset_ast_counters, with_expression_census_counters,
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

/// Physical typed buffer census, excluding nested payload allocations and allocator overhead.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn record_expression_storage_census(
    statistics: crate::compiler_frontend::ast::expressions::expression_store::ExpressionStorageStatistics,
) {
    add_ast_counter(AstCounter::CensusExpressionRows, statistics.rows);
    add_ast_counter(
        AstCounter::CensusExpressionRowCapacity,
        statistics.row_capacity,
    );
    add_ast_counter(AstCounter::CensusExpressionRowBytes, statistics.row_bytes);
    add_ast_counter(
        AstCounter::CensusExpressionRowCapacityBytes,
        statistics.row_capacity_bytes,
    );
    add_ast_counter(
        AstCounter::CensusExpressionSideEntries,
        statistics.side_entries,
    );
    add_ast_counter(
        AstCounter::CensusExpressionSideCapacity,
        statistics.side_capacity,
    );
    add_ast_counter(AstCounter::CensusExpressionSideBytes, statistics.side_bytes);
    add_ast_counter(
        AstCounter::CensusExpressionSideCapacityBytes,
        statistics.side_capacity_bytes,
    );
}
