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

    /// `DataType` spellings built for a resolved expression result.
    ///
    /// Operator typing decides on `TypeId` alone, so a fully folded expression materialises
    /// none. Only a partial fold, which needs a spelling for its runtime node, contributes.
    DiagnosticDataTypeMaterialisations,

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

    /// Slot schema or ordered-placeholder walks over TIR trees.
    TirSlotSchemaWalks,

    /// Public slot-contribution routing entries.
    TirContributionRoutingCalls,

    // Published-occurrence census. Additive values are per-AST snapshots; Max suffixes
    // are gauges. Sum snapshots and take maxima offline, including duplicate metric rows.
    CensusKindNoValue,
    CensusKindOptionNone,
    CensusKindRuntime,
    CensusKindInt,
    CensusKindUint,
    CensusKindFloat,
    CensusKindFixedScalar,
    CensusKindNumber,
    CensusKindStringSlice,
    CensusKindBool,
    CensusKindChar,
    CensusKindStructuralString,
    CensusKindReference,
    CensusKindCopy,
    CensusKindFunction,
    CensusKindFunctionCall,
    CensusKindFieldAccess,
    CensusKindMethodCall,
    CensusKindCollectionBuiltinCall,
    CensusKindMapBuiltinCall,
    CensusKindHandledFallibleFunctionCall,
    CensusKindHandledFallibleHostFunctionCall,
    CensusKindCast,
    CensusKindHandledFallibleExpression,
    CensusKindOptionPropagation,
    CensusKindHostFunctionCall,
    CensusKindTemplate,
    CensusKindRuntimeTemplateHandoff,
    CensusKindRuntimeSlotApplicationHandoff,
    CensusKindCollection,
    CensusKindMapLiteral,
    CensusKindStructDefinition,
    CensusKindStructInstance,
    CensusKindAnonymousConstRecord,
    CensusKindRange,
    CensusKindCoerced,
    CensusKindChoiceConstruct,
    CensusKindValueBlock,
    CensusCallArgsLists,
    CensusCallArgsLengthSum,
    CensusCallArgsLengthMax,
    CensusCallArgsCapacitySum,
    CensusCallArgsCapacityMax,
    CensusCallArgsEmpty,
    CensusCallArgsSingle,
    CensusCallArgsMultiple,
    CensusCallResultIdsLists,
    CensusCallResultIdsLengthSum,
    CensusCallResultIdsLengthMax,
    CensusCallResultIdsCapacitySum,
    CensusCallResultIdsCapacityMax,
    CensusCallResultIdsEmpty,
    CensusCallResultIdsSingle,
    CensusCallResultIdsMultiple,
    CensusCollectionItemsLists,
    CensusCollectionItemsLengthSum,
    CensusCollectionItemsLengthMax,
    CensusCollectionItemsCapacitySum,
    CensusCollectionItemsCapacityMax,
    CensusCollectionItemsEmpty,
    CensusCollectionItemsSingle,
    CensusCollectionItemsMultiple,
    CensusMapEntriesLists,
    CensusMapEntriesLengthSum,
    CensusMapEntriesLengthMax,
    CensusMapEntriesCapacitySum,
    CensusMapEntriesCapacityMax,
    CensusMapEntriesEmpty,
    CensusMapEntriesSingle,
    CensusMapEntriesMultiple,
    CensusFieldsLists,
    CensusFieldsLengthSum,
    CensusFieldsLengthMax,
    CensusFieldsCapacitySum,
    CensusFieldsCapacityMax,
    CensusFieldsEmpty,
    CensusFieldsSingle,
    CensusFieldsMultiple,
    CensusStructuralPiecesLists,
    CensusStructuralPiecesLengthSum,
    CensusStructuralPiecesLengthMax,
    CensusStructuralPiecesCapacitySum,
    CensusStructuralPiecesCapacityMax,
    CensusStructuralPiecesEmpty,
    CensusStructuralPiecesSingle,
    CensusStructuralPiecesMultiple,
    CensusRpnItemsLists,
    CensusRpnItemsLengthSum,
    CensusRpnItemsLengthMax,
    CensusRpnItemsCapacitySum,
    CensusRpnItemsCapacityMax,
    CensusRpnItemsEmpty,
    CensusRpnItemsSingle,
    CensusRpnItemsMultiple,
    CensusSignatureParametersLists,
    CensusSignatureParametersLengthSum,
    CensusSignatureParametersLengthMax,
    CensusSignatureParametersCapacitySum,
    CensusSignatureParametersCapacityMax,
    CensusSignatureParametersEmpty,
    CensusSignatureParametersSingle,
    CensusSignatureParametersMultiple,
    CensusSignatureReturnsLists,
    CensusSignatureReturnsLengthSum,
    CensusSignatureReturnsLengthMax,
    CensusSignatureReturnsCapacitySum,
    CensusSignatureReturnsCapacityMax,
    CensusSignatureReturnsEmpty,
    CensusSignatureReturnsSingle,
    CensusSignatureReturnsMultiple,
    CensusProducedValuesLists,
    CensusProducedValuesLengthSum,
    CensusProducedValuesLengthMax,
    CensusProducedValuesCapacitySum,
    CensusProducedValuesCapacityMax,
    CensusProducedValuesEmpty,
    CensusProducedValuesSingle,
    CensusProducedValuesMultiple,
    CensusBodyNodesLists,
    CensusBodyNodesLengthSum,
    CensusBodyNodesLengthMax,
    CensusBodyNodesCapacitySum,
    CensusBodyNodesCapacityMax,
    CensusBodyNodesEmpty,
    CensusBodyNodesSingle,
    CensusBodyNodesMultiple,
    CensusMatchArmsLists,
    CensusMatchArmsLengthSum,
    CensusMatchArmsLengthMax,
    CensusMatchArmsCapacitySum,
    CensusMatchArmsCapacityMax,
    CensusMatchArmsEmpty,
    CensusMatchArmsSingle,
    CensusMatchArmsMultiple,
    CensusColdImplicitFactsLists,
    CensusColdImplicitFactsLengthSum,
    CensusColdImplicitFactsLengthMax,
    CensusColdImplicitFactsCapacitySum,
    CensusColdImplicitFactsCapacityMax,
    CensusColdImplicitFactsEmpty,
    CensusColdImplicitFactsSingle,
    CensusColdImplicitFactsMultiple,
    CensusModules,
    CensusExpressions,
    CensusDiagnosticExpressions,
    CensusDeclarations,
    CensusStatements,
    CensusRpnOperands,
    CensusRpnOperators,
    CensusPendingNumericLiterals,
    CensusPendingGroups,
    CensusIncompleteViews,
    CensusPlaceRoots,
    CensusPlaceLocals,
    CensusPlaceFields,
    CensusPlaceDepthSum,
    CensusPlaceDepthMax,
    CensusFailurePresent,
    CensusReceiverPresent,
    CensusSpanAbsent,
    CensusConstRecordPresent,
    CensusDivisionPresent,
    CensusProvenancePresent,
    CensusFailureProvenance,
    CensusFailureDivision,
    CensusProvenanceDivision,
    CensusSpanAbsentSparse,
    CensusExpressionSizeMax,
    CensusExpressionAlignMax,
    CensusExpressionKindSizeMax,
    CensusExpressionKindAlignMax,
    CensusRpnItemSizeMax,
    CensusRpnItemAlignMax,
    CensusDeclarationSizeMax,
    CensusDeclarationAlignMax,
    CensusHirExpressionSizeMax,
    CensusHirExpressionAlignMax,
    CensusHirExpressionKindSizeMax,
    CensusHirExpressionKindAlignMax,
    CensusPlaceExpressionSizeMax,
    CensusPlaceExpressionAlignMax,
    CensusHirPlaceSizeMax,
    CensusHirPlaceAlignMax,

    CensusValueResultIdsLists,
    CensusValueResultIdsLengthSum,
    CensusValueResultIdsLengthMax,
    CensusValueResultIdsCapacitySum,
    CensusValueResultIdsCapacityMax,
    CensusValueResultIdsEmpty,
    CensusValueResultIdsSingle,
    CensusValueResultIdsMultiple,
    CensusMultiBindTargetsLists,
    CensusMultiBindTargetsLengthSum,
    CensusMultiBindTargetsLengthMax,
    CensusMultiBindTargetsCapacitySum,
    CensusMultiBindTargetsCapacityMax,
    CensusMultiBindTargetsEmpty,
    CensusMultiBindTargetsSingle,
    CensusMultiBindTargetsMultiple,
    CensusPatternCapturesLists,
    CensusPatternCapturesLengthSum,
    CensusPatternCapturesLengthMax,
    CensusPatternCapturesCapacitySum,
    CensusPatternCapturesCapacityMax,
    CensusPatternCapturesEmpty,
    CensusPatternCapturesSingle,
    CensusPatternCapturesMultiple,
    CensusHandoffNodesLists,
    CensusHandoffNodesLengthSum,
    CensusHandoffNodesLengthMax,
    CensusHandoffNodesCapacitySum,
    CensusHandoffNodesCapacityMax,
    CensusHandoffNodesEmpty,
    CensusHandoffNodesSingle,
    CensusHandoffNodesMultiple,
    CensusHandoffBranchesLists,
    CensusHandoffBranchesLengthSum,
    CensusHandoffBranchesLengthMax,
    CensusHandoffBranchesCapacitySum,
    CensusHandoffBranchesCapacityMax,
    CensusHandoffBranchesEmpty,
    CensusHandoffBranchesSingle,
    CensusHandoffBranchesMultiple,
    CensusHandoffSourcesLists,
    CensusHandoffSourcesLengthSum,
    CensusHandoffSourcesLengthMax,
    CensusHandoffSourcesCapacitySum,
    CensusHandoffSourcesCapacityMax,
    CensusHandoffSourcesEmpty,
    CensusHandoffSourcesSingle,
    CensusHandoffSourcesMultiple,
    CensusHandoffSitesLists,
    CensusHandoffSitesLengthSum,
    CensusHandoffSitesLengthMax,
    CensusHandoffSitesCapacitySum,
    CensusHandoffSitesCapacityMax,
    CensusHandoffSitesEmpty,
    CensusHandoffSitesSingle,
    CensusHandoffSitesMultiple,
    CensusOwnedStructuralPiecesLists,
    CensusOwnedStructuralPiecesLengthSum,
    CensusOwnedStructuralPiecesLengthMax,
    CensusOwnedStructuralPiecesCapacitySum,
    CensusOwnedStructuralPiecesCapacityMax,
    CensusOwnedStructuralPiecesEmpty,
    CensusOwnedStructuralPiecesSingle,
    CensusOwnedStructuralPiecesMultiple,
    CensusTirChildIdsLists,
    CensusTirChildIdsLengthSum,
    CensusTirChildIdsLengthMax,
    CensusTirChildIdsCapacitySum,
    CensusTirChildIdsCapacityMax,
    CensusTirChildIdsEmpty,
    CensusTirChildIdsSingle,
    CensusTirChildIdsMultiple,
    CensusTirBranchesLists,
    CensusTirBranchesLengthSum,
    CensusTirBranchesLengthMax,
    CensusTirBranchesCapacitySum,
    CensusTirBranchesCapacityMax,
    CensusTirBranchesEmpty,
    CensusTirBranchesSingle,
    CensusTirBranchesMultiple,
    CensusHandoffNodeOccurrences,
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
        let census_start = AstCounter::CensusKindNoValue as usize;
        let census_end = AstCounter::CensusHandoffNodeOccurrences as usize;
        for &counter in all_counters() {
            let index = counter.index();
            if (census_start..=census_end).contains(&index) {
                continue;
            }
            let value = counter_value(counter);
            log_benchmark_counter(counter_metric_name(counter), value as f64);
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
        let census_start = AstCounter::CensusKindNoValue as usize;
        let census_end = AstCounter::CensusHandoffNodeOccurrences as usize;
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
            let value = counter_value(counter);
            log_benchmark_counter(counter_metric_name(counter), value as f64);
        }
        COUNTERS.with(|counters| {
            let mut array = counters.borrow_mut();
            array[census_start..=census_end].fill(0);
            array[overlay_index] = saved_overlay;
        });
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
            AstCounter::DiagnosticDataTypeMaterialisations,
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
            AstCounter::TirSlotSchemaWalks,
            AstCounter::TirContributionRoutingCalls,
            AstCounter::CensusKindNoValue,
            AstCounter::CensusKindOptionNone,
            AstCounter::CensusKindRuntime,
            AstCounter::CensusKindInt,
            AstCounter::CensusKindUint,
            AstCounter::CensusKindFloat,
            AstCounter::CensusKindFixedScalar,
            AstCounter::CensusKindNumber,
            AstCounter::CensusKindStringSlice,
            AstCounter::CensusKindBool,
            AstCounter::CensusKindChar,
            AstCounter::CensusKindStructuralString,
            AstCounter::CensusKindReference,
            AstCounter::CensusKindCopy,
            AstCounter::CensusKindFunction,
            AstCounter::CensusKindFunctionCall,
            AstCounter::CensusKindFieldAccess,
            AstCounter::CensusKindMethodCall,
            AstCounter::CensusKindCollectionBuiltinCall,
            AstCounter::CensusKindMapBuiltinCall,
            AstCounter::CensusKindHandledFallibleFunctionCall,
            AstCounter::CensusKindHandledFallibleHostFunctionCall,
            AstCounter::CensusKindCast,
            AstCounter::CensusKindHandledFallibleExpression,
            AstCounter::CensusKindOptionPropagation,
            AstCounter::CensusKindHostFunctionCall,
            AstCounter::CensusKindTemplate,
            AstCounter::CensusKindRuntimeTemplateHandoff,
            AstCounter::CensusKindRuntimeSlotApplicationHandoff,
            AstCounter::CensusKindCollection,
            AstCounter::CensusKindMapLiteral,
            AstCounter::CensusKindStructDefinition,
            AstCounter::CensusKindStructInstance,
            AstCounter::CensusKindAnonymousConstRecord,
            AstCounter::CensusKindRange,
            AstCounter::CensusKindCoerced,
            AstCounter::CensusKindChoiceConstruct,
            AstCounter::CensusKindValueBlock,
            AstCounter::CensusCallArgsLists,
            AstCounter::CensusCallArgsLengthSum,
            AstCounter::CensusCallArgsLengthMax,
            AstCounter::CensusCallArgsCapacitySum,
            AstCounter::CensusCallArgsCapacityMax,
            AstCounter::CensusCallArgsEmpty,
            AstCounter::CensusCallArgsSingle,
            AstCounter::CensusCallArgsMultiple,
            AstCounter::CensusCallResultIdsLists,
            AstCounter::CensusCallResultIdsLengthSum,
            AstCounter::CensusCallResultIdsLengthMax,
            AstCounter::CensusCallResultIdsCapacitySum,
            AstCounter::CensusCallResultIdsCapacityMax,
            AstCounter::CensusCallResultIdsEmpty,
            AstCounter::CensusCallResultIdsSingle,
            AstCounter::CensusCallResultIdsMultiple,
            AstCounter::CensusCollectionItemsLists,
            AstCounter::CensusCollectionItemsLengthSum,
            AstCounter::CensusCollectionItemsLengthMax,
            AstCounter::CensusCollectionItemsCapacitySum,
            AstCounter::CensusCollectionItemsCapacityMax,
            AstCounter::CensusCollectionItemsEmpty,
            AstCounter::CensusCollectionItemsSingle,
            AstCounter::CensusCollectionItemsMultiple,
            AstCounter::CensusMapEntriesLists,
            AstCounter::CensusMapEntriesLengthSum,
            AstCounter::CensusMapEntriesLengthMax,
            AstCounter::CensusMapEntriesCapacitySum,
            AstCounter::CensusMapEntriesCapacityMax,
            AstCounter::CensusMapEntriesEmpty,
            AstCounter::CensusMapEntriesSingle,
            AstCounter::CensusMapEntriesMultiple,
            AstCounter::CensusFieldsLists,
            AstCounter::CensusFieldsLengthSum,
            AstCounter::CensusFieldsLengthMax,
            AstCounter::CensusFieldsCapacitySum,
            AstCounter::CensusFieldsCapacityMax,
            AstCounter::CensusFieldsEmpty,
            AstCounter::CensusFieldsSingle,
            AstCounter::CensusFieldsMultiple,
            AstCounter::CensusStructuralPiecesLists,
            AstCounter::CensusStructuralPiecesLengthSum,
            AstCounter::CensusStructuralPiecesLengthMax,
            AstCounter::CensusStructuralPiecesCapacitySum,
            AstCounter::CensusStructuralPiecesCapacityMax,
            AstCounter::CensusStructuralPiecesEmpty,
            AstCounter::CensusStructuralPiecesSingle,
            AstCounter::CensusStructuralPiecesMultiple,
            AstCounter::CensusRpnItemsLists,
            AstCounter::CensusRpnItemsLengthSum,
            AstCounter::CensusRpnItemsLengthMax,
            AstCounter::CensusRpnItemsCapacitySum,
            AstCounter::CensusRpnItemsCapacityMax,
            AstCounter::CensusRpnItemsEmpty,
            AstCounter::CensusRpnItemsSingle,
            AstCounter::CensusRpnItemsMultiple,
            AstCounter::CensusSignatureParametersLists,
            AstCounter::CensusSignatureParametersLengthSum,
            AstCounter::CensusSignatureParametersLengthMax,
            AstCounter::CensusSignatureParametersCapacitySum,
            AstCounter::CensusSignatureParametersCapacityMax,
            AstCounter::CensusSignatureParametersEmpty,
            AstCounter::CensusSignatureParametersSingle,
            AstCounter::CensusSignatureParametersMultiple,
            AstCounter::CensusSignatureReturnsLists,
            AstCounter::CensusSignatureReturnsLengthSum,
            AstCounter::CensusSignatureReturnsLengthMax,
            AstCounter::CensusSignatureReturnsCapacitySum,
            AstCounter::CensusSignatureReturnsCapacityMax,
            AstCounter::CensusSignatureReturnsEmpty,
            AstCounter::CensusSignatureReturnsSingle,
            AstCounter::CensusSignatureReturnsMultiple,
            AstCounter::CensusProducedValuesLists,
            AstCounter::CensusProducedValuesLengthSum,
            AstCounter::CensusProducedValuesLengthMax,
            AstCounter::CensusProducedValuesCapacitySum,
            AstCounter::CensusProducedValuesCapacityMax,
            AstCounter::CensusProducedValuesEmpty,
            AstCounter::CensusProducedValuesSingle,
            AstCounter::CensusProducedValuesMultiple,
            AstCounter::CensusBodyNodesLists,
            AstCounter::CensusBodyNodesLengthSum,
            AstCounter::CensusBodyNodesLengthMax,
            AstCounter::CensusBodyNodesCapacitySum,
            AstCounter::CensusBodyNodesCapacityMax,
            AstCounter::CensusBodyNodesEmpty,
            AstCounter::CensusBodyNodesSingle,
            AstCounter::CensusBodyNodesMultiple,
            AstCounter::CensusMatchArmsLists,
            AstCounter::CensusMatchArmsLengthSum,
            AstCounter::CensusMatchArmsLengthMax,
            AstCounter::CensusMatchArmsCapacitySum,
            AstCounter::CensusMatchArmsCapacityMax,
            AstCounter::CensusMatchArmsEmpty,
            AstCounter::CensusMatchArmsSingle,
            AstCounter::CensusMatchArmsMultiple,
            AstCounter::CensusColdImplicitFactsLists,
            AstCounter::CensusColdImplicitFactsLengthSum,
            AstCounter::CensusColdImplicitFactsLengthMax,
            AstCounter::CensusColdImplicitFactsCapacitySum,
            AstCounter::CensusColdImplicitFactsCapacityMax,
            AstCounter::CensusColdImplicitFactsEmpty,
            AstCounter::CensusColdImplicitFactsSingle,
            AstCounter::CensusColdImplicitFactsMultiple,
            AstCounter::CensusModules,
            AstCounter::CensusExpressions,
            AstCounter::CensusDiagnosticExpressions,
            AstCounter::CensusDeclarations,
            AstCounter::CensusStatements,
            AstCounter::CensusRpnOperands,
            AstCounter::CensusRpnOperators,
            AstCounter::CensusPendingNumericLiterals,
            AstCounter::CensusPendingGroups,
            AstCounter::CensusIncompleteViews,
            AstCounter::CensusPlaceRoots,
            AstCounter::CensusPlaceLocals,
            AstCounter::CensusPlaceFields,
            AstCounter::CensusPlaceDepthSum,
            AstCounter::CensusPlaceDepthMax,
            AstCounter::CensusFailurePresent,
            AstCounter::CensusReceiverPresent,
            AstCounter::CensusSpanAbsent,
            AstCounter::CensusConstRecordPresent,
            AstCounter::CensusDivisionPresent,
            AstCounter::CensusProvenancePresent,
            AstCounter::CensusFailureProvenance,
            AstCounter::CensusFailureDivision,
            AstCounter::CensusProvenanceDivision,
            AstCounter::CensusSpanAbsentSparse,
            AstCounter::CensusExpressionSizeMax,
            AstCounter::CensusExpressionAlignMax,
            AstCounter::CensusExpressionKindSizeMax,
            AstCounter::CensusExpressionKindAlignMax,
            AstCounter::CensusRpnItemSizeMax,
            AstCounter::CensusRpnItemAlignMax,
            AstCounter::CensusDeclarationSizeMax,
            AstCounter::CensusDeclarationAlignMax,
            AstCounter::CensusHirExpressionSizeMax,
            AstCounter::CensusHirExpressionAlignMax,
            AstCounter::CensusHirExpressionKindSizeMax,
            AstCounter::CensusHirExpressionKindAlignMax,
            AstCounter::CensusPlaceExpressionSizeMax,
            AstCounter::CensusPlaceExpressionAlignMax,
            AstCounter::CensusHirPlaceSizeMax,
            AstCounter::CensusHirPlaceAlignMax,
            AstCounter::CensusValueResultIdsLists,
            AstCounter::CensusValueResultIdsLengthSum,
            AstCounter::CensusValueResultIdsLengthMax,
            AstCounter::CensusValueResultIdsCapacitySum,
            AstCounter::CensusValueResultIdsCapacityMax,
            AstCounter::CensusValueResultIdsEmpty,
            AstCounter::CensusValueResultIdsSingle,
            AstCounter::CensusValueResultIdsMultiple,
            AstCounter::CensusMultiBindTargetsLists,
            AstCounter::CensusMultiBindTargetsLengthSum,
            AstCounter::CensusMultiBindTargetsLengthMax,
            AstCounter::CensusMultiBindTargetsCapacitySum,
            AstCounter::CensusMultiBindTargetsCapacityMax,
            AstCounter::CensusMultiBindTargetsEmpty,
            AstCounter::CensusMultiBindTargetsSingle,
            AstCounter::CensusMultiBindTargetsMultiple,
            AstCounter::CensusPatternCapturesLists,
            AstCounter::CensusPatternCapturesLengthSum,
            AstCounter::CensusPatternCapturesLengthMax,
            AstCounter::CensusPatternCapturesCapacitySum,
            AstCounter::CensusPatternCapturesCapacityMax,
            AstCounter::CensusPatternCapturesEmpty,
            AstCounter::CensusPatternCapturesSingle,
            AstCounter::CensusPatternCapturesMultiple,
            AstCounter::CensusHandoffNodesLists,
            AstCounter::CensusHandoffNodesLengthSum,
            AstCounter::CensusHandoffNodesLengthMax,
            AstCounter::CensusHandoffNodesCapacitySum,
            AstCounter::CensusHandoffNodesCapacityMax,
            AstCounter::CensusHandoffNodesEmpty,
            AstCounter::CensusHandoffNodesSingle,
            AstCounter::CensusHandoffNodesMultiple,
            AstCounter::CensusHandoffBranchesLists,
            AstCounter::CensusHandoffBranchesLengthSum,
            AstCounter::CensusHandoffBranchesLengthMax,
            AstCounter::CensusHandoffBranchesCapacitySum,
            AstCounter::CensusHandoffBranchesCapacityMax,
            AstCounter::CensusHandoffBranchesEmpty,
            AstCounter::CensusHandoffBranchesSingle,
            AstCounter::CensusHandoffBranchesMultiple,
            AstCounter::CensusHandoffSourcesLists,
            AstCounter::CensusHandoffSourcesLengthSum,
            AstCounter::CensusHandoffSourcesLengthMax,
            AstCounter::CensusHandoffSourcesCapacitySum,
            AstCounter::CensusHandoffSourcesCapacityMax,
            AstCounter::CensusHandoffSourcesEmpty,
            AstCounter::CensusHandoffSourcesSingle,
            AstCounter::CensusHandoffSourcesMultiple,
            AstCounter::CensusHandoffSitesLists,
            AstCounter::CensusHandoffSitesLengthSum,
            AstCounter::CensusHandoffSitesLengthMax,
            AstCounter::CensusHandoffSitesCapacitySum,
            AstCounter::CensusHandoffSitesCapacityMax,
            AstCounter::CensusHandoffSitesEmpty,
            AstCounter::CensusHandoffSitesSingle,
            AstCounter::CensusHandoffSitesMultiple,
            AstCounter::CensusOwnedStructuralPiecesLists,
            AstCounter::CensusOwnedStructuralPiecesLengthSum,
            AstCounter::CensusOwnedStructuralPiecesLengthMax,
            AstCounter::CensusOwnedStructuralPiecesCapacitySum,
            AstCounter::CensusOwnedStructuralPiecesCapacityMax,
            AstCounter::CensusOwnedStructuralPiecesEmpty,
            AstCounter::CensusOwnedStructuralPiecesSingle,
            AstCounter::CensusOwnedStructuralPiecesMultiple,
            AstCounter::CensusTirChildIdsLists,
            AstCounter::CensusTirChildIdsLengthSum,
            AstCounter::CensusTirChildIdsLengthMax,
            AstCounter::CensusTirChildIdsCapacitySum,
            AstCounter::CensusTirChildIdsCapacityMax,
            AstCounter::CensusTirChildIdsEmpty,
            AstCounter::CensusTirChildIdsSingle,
            AstCounter::CensusTirChildIdsMultiple,
            AstCounter::CensusTirBranchesLists,
            AstCounter::CensusTirBranchesLengthSum,
            AstCounter::CensusTirBranchesLengthMax,
            AstCounter::CensusTirBranchesCapacitySum,
            AstCounter::CensusTirBranchesCapacityMax,
            AstCounter::CensusTirBranchesEmpty,
            AstCounter::CensusTirBranchesSingle,
            AstCounter::CensusTirBranchesMultiple,
            AstCounter::CensusHandoffNodeOccurrences,
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
            AstCounter::ConstFactEnvironmentClones => "ast_const_fact_environment_clones",
            AstCounter::ConstFactEnvironmentEntriesCloned => {
                "ast_const_fact_environment_entries_cloned"
            }

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
            AstCounter::TirSlotSchemaWalks => "ast_tir_slot_schema_walks",
            AstCounter::TirContributionRoutingCalls => "ast_tir_contribution_routing_calls",
            AstCounter::CensusKindNoValue => "ast_census_kind_no_value",
            AstCounter::CensusKindOptionNone => "ast_census_kind_option_none",
            AstCounter::CensusKindRuntime => "ast_census_kind_runtime",
            AstCounter::CensusKindInt => "ast_census_kind_int",
            AstCounter::CensusKindUint => "ast_census_kind_uint",
            AstCounter::CensusKindFloat => "ast_census_kind_float",
            AstCounter::CensusKindFixedScalar => "ast_census_kind_fixed_scalar",
            AstCounter::CensusKindNumber => "ast_census_kind_number",
            AstCounter::CensusKindStringSlice => "ast_census_kind_string_slice",
            AstCounter::CensusKindBool => "ast_census_kind_bool",
            AstCounter::CensusKindChar => "ast_census_kind_char",
            AstCounter::CensusKindStructuralString => "ast_census_kind_structural_string",
            AstCounter::CensusKindReference => "ast_census_kind_reference",
            AstCounter::CensusKindCopy => "ast_census_kind_copy",
            AstCounter::CensusKindFunction => "ast_census_kind_function",
            AstCounter::CensusKindFunctionCall => "ast_census_kind_function_call",
            AstCounter::CensusKindFieldAccess => "ast_census_kind_field_access",
            AstCounter::CensusKindMethodCall => "ast_census_kind_method_call",
            AstCounter::CensusKindCollectionBuiltinCall => {
                "ast_census_kind_collection_builtin_call"
            }
            AstCounter::CensusKindMapBuiltinCall => "ast_census_kind_map_builtin_call",
            AstCounter::CensusKindHandledFallibleFunctionCall => {
                "ast_census_kind_handled_fallible_function_call"
            }
            AstCounter::CensusKindHandledFallibleHostFunctionCall => {
                "ast_census_kind_handled_fallible_host_function_call"
            }
            AstCounter::CensusKindCast => "ast_census_kind_cast",
            AstCounter::CensusKindHandledFallibleExpression => {
                "ast_census_kind_handled_fallible_expression"
            }
            AstCounter::CensusKindOptionPropagation => "ast_census_kind_option_propagation",
            AstCounter::CensusKindHostFunctionCall => "ast_census_kind_host_function_call",
            AstCounter::CensusKindTemplate => "ast_census_kind_template",
            AstCounter::CensusKindRuntimeTemplateHandoff => {
                "ast_census_kind_runtime_template_handoff"
            }
            AstCounter::CensusKindRuntimeSlotApplicationHandoff => {
                "ast_census_kind_runtime_slot_application_handoff"
            }
            AstCounter::CensusKindCollection => "ast_census_kind_collection",
            AstCounter::CensusKindMapLiteral => "ast_census_kind_map_literal",
            AstCounter::CensusKindStructDefinition => "ast_census_kind_struct_definition",
            AstCounter::CensusKindStructInstance => "ast_census_kind_struct_instance",
            AstCounter::CensusKindAnonymousConstRecord => "ast_census_kind_anonymous_const_record",
            AstCounter::CensusKindRange => "ast_census_kind_range",
            AstCounter::CensusKindCoerced => "ast_census_kind_coerced",
            AstCounter::CensusKindChoiceConstruct => "ast_census_kind_choice_construct",
            AstCounter::CensusKindValueBlock => "ast_census_kind_value_block",
            AstCounter::CensusCallArgsLists => "ast_census_call_args_lists",
            AstCounter::CensusCallArgsLengthSum => "ast_census_call_args_length_sum",
            AstCounter::CensusCallArgsLengthMax => "ast_census_call_args_length_max",
            AstCounter::CensusCallArgsCapacitySum => "ast_census_call_args_capacity_sum",
            AstCounter::CensusCallArgsCapacityMax => "ast_census_call_args_capacity_max",
            AstCounter::CensusCallArgsEmpty => "ast_census_call_args_empty",
            AstCounter::CensusCallArgsSingle => "ast_census_call_args_single",
            AstCounter::CensusCallArgsMultiple => "ast_census_call_args_multiple",
            AstCounter::CensusCallResultIdsLists => "ast_census_call_result_ids_lists",
            AstCounter::CensusCallResultIdsLengthSum => "ast_census_call_result_ids_length_sum",
            AstCounter::CensusCallResultIdsLengthMax => "ast_census_call_result_ids_length_max",
            AstCounter::CensusCallResultIdsCapacitySum => "ast_census_call_result_ids_capacity_sum",
            AstCounter::CensusCallResultIdsCapacityMax => "ast_census_call_result_ids_capacity_max",
            AstCounter::CensusCallResultIdsEmpty => "ast_census_call_result_ids_empty",
            AstCounter::CensusCallResultIdsSingle => "ast_census_call_result_ids_single",
            AstCounter::CensusCallResultIdsMultiple => "ast_census_call_result_ids_multiple",
            AstCounter::CensusCollectionItemsLists => "ast_census_collection_items_lists",
            AstCounter::CensusCollectionItemsLengthSum => "ast_census_collection_items_length_sum",
            AstCounter::CensusCollectionItemsLengthMax => "ast_census_collection_items_length_max",
            AstCounter::CensusCollectionItemsCapacitySum => {
                "ast_census_collection_items_capacity_sum"
            }
            AstCounter::CensusCollectionItemsCapacityMax => {
                "ast_census_collection_items_capacity_max"
            }
            AstCounter::CensusCollectionItemsEmpty => "ast_census_collection_items_empty",
            AstCounter::CensusCollectionItemsSingle => "ast_census_collection_items_single",
            AstCounter::CensusCollectionItemsMultiple => "ast_census_collection_items_multiple",
            AstCounter::CensusMapEntriesLists => "ast_census_map_entries_lists",
            AstCounter::CensusMapEntriesLengthSum => "ast_census_map_entries_length_sum",
            AstCounter::CensusMapEntriesLengthMax => "ast_census_map_entries_length_max",
            AstCounter::CensusMapEntriesCapacitySum => "ast_census_map_entries_capacity_sum",
            AstCounter::CensusMapEntriesCapacityMax => "ast_census_map_entries_capacity_max",
            AstCounter::CensusMapEntriesEmpty => "ast_census_map_entries_empty",
            AstCounter::CensusMapEntriesSingle => "ast_census_map_entries_single",
            AstCounter::CensusMapEntriesMultiple => "ast_census_map_entries_multiple",
            AstCounter::CensusFieldsLists => "ast_census_fields_lists",
            AstCounter::CensusFieldsLengthSum => "ast_census_fields_length_sum",
            AstCounter::CensusFieldsLengthMax => "ast_census_fields_length_max",
            AstCounter::CensusFieldsCapacitySum => "ast_census_fields_capacity_sum",
            AstCounter::CensusFieldsCapacityMax => "ast_census_fields_capacity_max",
            AstCounter::CensusFieldsEmpty => "ast_census_fields_empty",
            AstCounter::CensusFieldsSingle => "ast_census_fields_single",
            AstCounter::CensusFieldsMultiple => "ast_census_fields_multiple",
            AstCounter::CensusStructuralPiecesLists => "ast_census_structural_pieces_lists",
            AstCounter::CensusStructuralPiecesLengthSum => {
                "ast_census_structural_pieces_length_sum"
            }
            AstCounter::CensusStructuralPiecesLengthMax => {
                "ast_census_structural_pieces_length_max"
            }
            AstCounter::CensusStructuralPiecesCapacitySum => {
                "ast_census_structural_pieces_capacity_sum"
            }
            AstCounter::CensusStructuralPiecesCapacityMax => {
                "ast_census_structural_pieces_capacity_max"
            }
            AstCounter::CensusStructuralPiecesEmpty => "ast_census_structural_pieces_empty",
            AstCounter::CensusStructuralPiecesSingle => "ast_census_structural_pieces_single",
            AstCounter::CensusStructuralPiecesMultiple => "ast_census_structural_pieces_multiple",
            AstCounter::CensusRpnItemsLists => "ast_census_rpn_items_lists",
            AstCounter::CensusRpnItemsLengthSum => "ast_census_rpn_items_length_sum",
            AstCounter::CensusRpnItemsLengthMax => "ast_census_rpn_items_length_max",
            AstCounter::CensusRpnItemsCapacitySum => "ast_census_rpn_items_capacity_sum",
            AstCounter::CensusRpnItemsCapacityMax => "ast_census_rpn_items_capacity_max",
            AstCounter::CensusRpnItemsEmpty => "ast_census_rpn_items_empty",
            AstCounter::CensusRpnItemsSingle => "ast_census_rpn_items_single",
            AstCounter::CensusRpnItemsMultiple => "ast_census_rpn_items_multiple",
            AstCounter::CensusSignatureParametersLists => "ast_census_signature_parameters_lists",
            AstCounter::CensusSignatureParametersLengthSum => {
                "ast_census_signature_parameters_length_sum"
            }
            AstCounter::CensusSignatureParametersLengthMax => {
                "ast_census_signature_parameters_length_max"
            }
            AstCounter::CensusSignatureParametersCapacitySum => {
                "ast_census_signature_parameters_capacity_sum"
            }
            AstCounter::CensusSignatureParametersCapacityMax => {
                "ast_census_signature_parameters_capacity_max"
            }
            AstCounter::CensusSignatureParametersEmpty => "ast_census_signature_parameters_empty",
            AstCounter::CensusSignatureParametersSingle => "ast_census_signature_parameters_single",
            AstCounter::CensusSignatureParametersMultiple => {
                "ast_census_signature_parameters_multiple"
            }
            AstCounter::CensusSignatureReturnsLists => "ast_census_signature_returns_lists",
            AstCounter::CensusSignatureReturnsLengthSum => {
                "ast_census_signature_returns_length_sum"
            }
            AstCounter::CensusSignatureReturnsLengthMax => {
                "ast_census_signature_returns_length_max"
            }
            AstCounter::CensusSignatureReturnsCapacitySum => {
                "ast_census_signature_returns_capacity_sum"
            }
            AstCounter::CensusSignatureReturnsCapacityMax => {
                "ast_census_signature_returns_capacity_max"
            }
            AstCounter::CensusSignatureReturnsEmpty => "ast_census_signature_returns_empty",
            AstCounter::CensusSignatureReturnsSingle => "ast_census_signature_returns_single",
            AstCounter::CensusSignatureReturnsMultiple => "ast_census_signature_returns_multiple",
            AstCounter::CensusProducedValuesLists => "ast_census_produced_values_lists",
            AstCounter::CensusProducedValuesLengthSum => "ast_census_produced_values_length_sum",
            AstCounter::CensusProducedValuesLengthMax => "ast_census_produced_values_length_max",
            AstCounter::CensusProducedValuesCapacitySum => {
                "ast_census_produced_values_capacity_sum"
            }
            AstCounter::CensusProducedValuesCapacityMax => {
                "ast_census_produced_values_capacity_max"
            }
            AstCounter::CensusProducedValuesEmpty => "ast_census_produced_values_empty",
            AstCounter::CensusProducedValuesSingle => "ast_census_produced_values_single",
            AstCounter::CensusProducedValuesMultiple => "ast_census_produced_values_multiple",
            AstCounter::CensusBodyNodesLists => "ast_census_body_nodes_lists",
            AstCounter::CensusBodyNodesLengthSum => "ast_census_body_nodes_length_sum",
            AstCounter::CensusBodyNodesLengthMax => "ast_census_body_nodes_length_max",
            AstCounter::CensusBodyNodesCapacitySum => "ast_census_body_nodes_capacity_sum",
            AstCounter::CensusBodyNodesCapacityMax => "ast_census_body_nodes_capacity_max",
            AstCounter::CensusBodyNodesEmpty => "ast_census_body_nodes_empty",
            AstCounter::CensusBodyNodesSingle => "ast_census_body_nodes_single",
            AstCounter::CensusBodyNodesMultiple => "ast_census_body_nodes_multiple",
            AstCounter::CensusMatchArmsLists => "ast_census_match_arms_lists",
            AstCounter::CensusMatchArmsLengthSum => "ast_census_match_arms_length_sum",
            AstCounter::CensusMatchArmsLengthMax => "ast_census_match_arms_length_max",
            AstCounter::CensusMatchArmsCapacitySum => "ast_census_match_arms_capacity_sum",
            AstCounter::CensusMatchArmsCapacityMax => "ast_census_match_arms_capacity_max",
            AstCounter::CensusMatchArmsEmpty => "ast_census_match_arms_empty",
            AstCounter::CensusMatchArmsSingle => "ast_census_match_arms_single",
            AstCounter::CensusMatchArmsMultiple => "ast_census_match_arms_multiple",
            AstCounter::CensusColdImplicitFactsLists => "ast_census_cold_implicit_facts_lists",
            AstCounter::CensusColdImplicitFactsLengthSum => {
                "ast_census_cold_implicit_facts_length_sum"
            }
            AstCounter::CensusColdImplicitFactsLengthMax => {
                "ast_census_cold_implicit_facts_length_max"
            }
            AstCounter::CensusColdImplicitFactsCapacitySum => {
                "ast_census_cold_implicit_facts_capacity_sum"
            }
            AstCounter::CensusColdImplicitFactsCapacityMax => {
                "ast_census_cold_implicit_facts_capacity_max"
            }
            AstCounter::CensusColdImplicitFactsEmpty => "ast_census_cold_implicit_facts_empty",
            AstCounter::CensusColdImplicitFactsSingle => "ast_census_cold_implicit_facts_single",
            AstCounter::CensusColdImplicitFactsMultiple => {
                "ast_census_cold_implicit_facts_multiple"
            }
            AstCounter::CensusModules => "ast_census_modules",
            AstCounter::CensusExpressions => "ast_census_expressions",
            AstCounter::CensusDiagnosticExpressions => "ast_census_diagnostic_expressions",
            AstCounter::CensusDeclarations => "ast_census_declarations",
            AstCounter::CensusStatements => "ast_census_statements",
            AstCounter::CensusRpnOperands => "ast_census_rpn_operands",
            AstCounter::CensusRpnOperators => "ast_census_rpn_operators",
            AstCounter::CensusPendingNumericLiterals => "ast_census_pending_numeric_literals",
            AstCounter::CensusPendingGroups => "ast_census_pending_groups",
            AstCounter::CensusIncompleteViews => "ast_census_incomplete_views",
            AstCounter::CensusPlaceRoots => "ast_census_place_roots",
            AstCounter::CensusPlaceLocals => "ast_census_place_locals",
            AstCounter::CensusPlaceFields => "ast_census_place_fields",
            AstCounter::CensusPlaceDepthSum => "ast_census_place_depth_sum",
            AstCounter::CensusPlaceDepthMax => "ast_census_place_depth_max",
            AstCounter::CensusFailurePresent => "ast_census_failure_present",
            AstCounter::CensusReceiverPresent => "ast_census_receiver_present",
            AstCounter::CensusSpanAbsent => "ast_census_span_absent",
            AstCounter::CensusConstRecordPresent => "ast_census_const_record_present",
            AstCounter::CensusDivisionPresent => "ast_census_division_present",
            AstCounter::CensusProvenancePresent => "ast_census_provenance_present",
            AstCounter::CensusFailureProvenance => "ast_census_failure_provenance",
            AstCounter::CensusFailureDivision => "ast_census_failure_division",
            AstCounter::CensusProvenanceDivision => "ast_census_provenance_division",
            AstCounter::CensusSpanAbsentSparse => "ast_census_span_absent_sparse",
            AstCounter::CensusExpressionSizeMax => "ast_census_expression_size_max",
            AstCounter::CensusExpressionAlignMax => "ast_census_expression_align_max",
            AstCounter::CensusExpressionKindSizeMax => "ast_census_expression_kind_size_max",
            AstCounter::CensusExpressionKindAlignMax => "ast_census_expression_kind_align_max",
            AstCounter::CensusRpnItemSizeMax => "ast_census_rpn_item_size_max",
            AstCounter::CensusRpnItemAlignMax => "ast_census_rpn_item_align_max",
            AstCounter::CensusDeclarationSizeMax => "ast_census_declaration_size_max",
            AstCounter::CensusDeclarationAlignMax => "ast_census_declaration_align_max",
            AstCounter::CensusHirExpressionSizeMax => "ast_census_hir_expression_size_max",
            AstCounter::CensusHirExpressionAlignMax => "ast_census_hir_expression_align_max",
            AstCounter::CensusHirExpressionKindSizeMax => "ast_census_hir_expression_kind_size_max",
            AstCounter::CensusHirExpressionKindAlignMax => {
                "ast_census_hir_expression_kind_align_max"
            }
            AstCounter::CensusPlaceExpressionSizeMax => "ast_census_place_expression_size_max",
            AstCounter::CensusPlaceExpressionAlignMax => "ast_census_place_expression_align_max",
            AstCounter::CensusHirPlaceSizeMax => "ast_census_hir_place_size_max",
            AstCounter::CensusHirPlaceAlignMax => "ast_census_hir_place_align_max",
            AstCounter::CensusValueResultIdsLists => "ast_census_value_result_ids_lists",
            AstCounter::CensusValueResultIdsLengthSum => "ast_census_value_result_ids_length_sum",
            AstCounter::CensusValueResultIdsLengthMax => "ast_census_value_result_ids_length_max",
            AstCounter::CensusValueResultIdsCapacitySum => {
                "ast_census_value_result_ids_capacity_sum"
            }
            AstCounter::CensusValueResultIdsCapacityMax => {
                "ast_census_value_result_ids_capacity_max"
            }
            AstCounter::CensusValueResultIdsEmpty => "ast_census_value_result_ids_empty",
            AstCounter::CensusValueResultIdsSingle => "ast_census_value_result_ids_single",
            AstCounter::CensusValueResultIdsMultiple => "ast_census_value_result_ids_multiple",
            AstCounter::CensusMultiBindTargetsLists => "ast_census_multi_bind_targets_lists",
            AstCounter::CensusMultiBindTargetsLengthSum => {
                "ast_census_multi_bind_targets_length_sum"
            }
            AstCounter::CensusMultiBindTargetsLengthMax => {
                "ast_census_multi_bind_targets_length_max"
            }
            AstCounter::CensusMultiBindTargetsCapacitySum => {
                "ast_census_multi_bind_targets_capacity_sum"
            }
            AstCounter::CensusMultiBindTargetsCapacityMax => {
                "ast_census_multi_bind_targets_capacity_max"
            }
            AstCounter::CensusMultiBindTargetsEmpty => "ast_census_multi_bind_targets_empty",
            AstCounter::CensusMultiBindTargetsSingle => "ast_census_multi_bind_targets_single",
            AstCounter::CensusMultiBindTargetsMultiple => "ast_census_multi_bind_targets_multiple",
            AstCounter::CensusPatternCapturesLists => "ast_census_pattern_captures_lists",
            AstCounter::CensusPatternCapturesLengthSum => "ast_census_pattern_captures_length_sum",
            AstCounter::CensusPatternCapturesLengthMax => "ast_census_pattern_captures_length_max",
            AstCounter::CensusPatternCapturesCapacitySum => {
                "ast_census_pattern_captures_capacity_sum"
            }
            AstCounter::CensusPatternCapturesCapacityMax => {
                "ast_census_pattern_captures_capacity_max"
            }
            AstCounter::CensusPatternCapturesEmpty => "ast_census_pattern_captures_empty",
            AstCounter::CensusPatternCapturesSingle => "ast_census_pattern_captures_single",
            AstCounter::CensusPatternCapturesMultiple => "ast_census_pattern_captures_multiple",
            AstCounter::CensusHandoffNodesLists => "ast_census_handoff_nodes_lists",
            AstCounter::CensusHandoffNodesLengthSum => "ast_census_handoff_nodes_length_sum",
            AstCounter::CensusHandoffNodesLengthMax => "ast_census_handoff_nodes_length_max",
            AstCounter::CensusHandoffNodesCapacitySum => "ast_census_handoff_nodes_capacity_sum",
            AstCounter::CensusHandoffNodesCapacityMax => "ast_census_handoff_nodes_capacity_max",
            AstCounter::CensusHandoffNodesEmpty => "ast_census_handoff_nodes_empty",
            AstCounter::CensusHandoffNodesSingle => "ast_census_handoff_nodes_single",
            AstCounter::CensusHandoffNodesMultiple => "ast_census_handoff_nodes_multiple",
            AstCounter::CensusHandoffBranchesLists => "ast_census_handoff_branches_lists",
            AstCounter::CensusHandoffBranchesLengthSum => "ast_census_handoff_branches_length_sum",
            AstCounter::CensusHandoffBranchesLengthMax => "ast_census_handoff_branches_length_max",
            AstCounter::CensusHandoffBranchesCapacitySum => {
                "ast_census_handoff_branches_capacity_sum"
            }
            AstCounter::CensusHandoffBranchesCapacityMax => {
                "ast_census_handoff_branches_capacity_max"
            }
            AstCounter::CensusHandoffBranchesEmpty => "ast_census_handoff_branches_empty",
            AstCounter::CensusHandoffBranchesSingle => "ast_census_handoff_branches_single",
            AstCounter::CensusHandoffBranchesMultiple => "ast_census_handoff_branches_multiple",
            AstCounter::CensusHandoffSourcesLists => "ast_census_handoff_sources_lists",
            AstCounter::CensusHandoffSourcesLengthSum => "ast_census_handoff_sources_length_sum",
            AstCounter::CensusHandoffSourcesLengthMax => "ast_census_handoff_sources_length_max",
            AstCounter::CensusHandoffSourcesCapacitySum => {
                "ast_census_handoff_sources_capacity_sum"
            }
            AstCounter::CensusHandoffSourcesCapacityMax => {
                "ast_census_handoff_sources_capacity_max"
            }
            AstCounter::CensusHandoffSourcesEmpty => "ast_census_handoff_sources_empty",
            AstCounter::CensusHandoffSourcesSingle => "ast_census_handoff_sources_single",
            AstCounter::CensusHandoffSourcesMultiple => "ast_census_handoff_sources_multiple",
            AstCounter::CensusHandoffSitesLists => "ast_census_handoff_sites_lists",
            AstCounter::CensusHandoffSitesLengthSum => "ast_census_handoff_sites_length_sum",
            AstCounter::CensusHandoffSitesLengthMax => "ast_census_handoff_sites_length_max",
            AstCounter::CensusHandoffSitesCapacitySum => "ast_census_handoff_sites_capacity_sum",
            AstCounter::CensusHandoffSitesCapacityMax => "ast_census_handoff_sites_capacity_max",
            AstCounter::CensusHandoffSitesEmpty => "ast_census_handoff_sites_empty",
            AstCounter::CensusHandoffSitesSingle => "ast_census_handoff_sites_single",
            AstCounter::CensusHandoffSitesMultiple => "ast_census_handoff_sites_multiple",
            AstCounter::CensusOwnedStructuralPiecesLists => {
                "ast_census_owned_structural_pieces_lists"
            }
            AstCounter::CensusOwnedStructuralPiecesLengthSum => {
                "ast_census_owned_structural_pieces_length_sum"
            }
            AstCounter::CensusOwnedStructuralPiecesLengthMax => {
                "ast_census_owned_structural_pieces_length_max"
            }
            AstCounter::CensusOwnedStructuralPiecesCapacitySum => {
                "ast_census_owned_structural_pieces_capacity_sum"
            }
            AstCounter::CensusOwnedStructuralPiecesCapacityMax => {
                "ast_census_owned_structural_pieces_capacity_max"
            }
            AstCounter::CensusOwnedStructuralPiecesEmpty => {
                "ast_census_owned_structural_pieces_empty"
            }
            AstCounter::CensusOwnedStructuralPiecesSingle => {
                "ast_census_owned_structural_pieces_single"
            }
            AstCounter::CensusOwnedStructuralPiecesMultiple => {
                "ast_census_owned_structural_pieces_multiple"
            }
            AstCounter::CensusTirChildIdsLists => "ast_census_tir_child_ids_lists",
            AstCounter::CensusTirChildIdsLengthSum => "ast_census_tir_child_ids_length_sum",
            AstCounter::CensusTirChildIdsLengthMax => "ast_census_tir_child_ids_length_max",
            AstCounter::CensusTirChildIdsCapacitySum => "ast_census_tir_child_ids_capacity_sum",
            AstCounter::CensusTirChildIdsCapacityMax => "ast_census_tir_child_ids_capacity_max",
            AstCounter::CensusTirChildIdsEmpty => "ast_census_tir_child_ids_empty",
            AstCounter::CensusTirChildIdsSingle => "ast_census_tir_child_ids_single",
            AstCounter::CensusTirChildIdsMultiple => "ast_census_tir_child_ids_multiple",
            AstCounter::CensusTirBranchesLists => "ast_census_tir_branches_lists",
            AstCounter::CensusTirBranchesLengthSum => "ast_census_tir_branches_length_sum",
            AstCounter::CensusTirBranchesLengthMax => "ast_census_tir_branches_length_max",
            AstCounter::CensusTirBranchesCapacitySum => "ast_census_tir_branches_capacity_sum",
            AstCounter::CensusTirBranchesCapacityMax => "ast_census_tir_branches_capacity_max",
            AstCounter::CensusTirBranchesEmpty => "ast_census_tir_branches_empty",
            AstCounter::CensusTirBranchesSingle => "ast_census_tir_branches_single",
            AstCounter::CensusTirBranchesMultiple => "ast_census_tir_branches_multiple",
            AstCounter::CensusHandoffNodeOccurrences => "ast_census_handoff_node_occurrences",
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
