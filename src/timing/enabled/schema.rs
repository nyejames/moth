//! Timing schema v1 and the typed metric registry.
//!
//! WHAT: owns the immutable contract every timing observation, snapshot,
//!      benchmark line and human summary obeys. One dense registry maps every
//!      `TimingMetric` to its stable name, presentation level, measurement
//!      relation, attribution kind, owner and command scope.
//! WHY:  before collector and call-site work, metric identity must be settled
//!       once. A dense typed registry replaces provisional string names with a
//!       single account-of-record, keeps the descriptor table parallel to the
//!       enum, and lets later phases build dense aggregate storage, snapshot
//!       ordering and benchmark fingerprints from the same table.
//!
//! Compatibility: timing data recorded before schema v1 is legacy and
//! non-comparable. This module performs no numeric migration and carries no
//! aliases for provisional names.
//!
//! This module is timer-only infrastructure, exactly like the rest of
//! `enabled`. It must not import build-system, frontend, analysis, IR or
//! backend compiler modules.

// The `enabled` module broad-allow is removed in Phase 3; until the registry
// has a live production consumer, a targeted allow keeps `cargo check
// --features timers` quiet for helpers exercised only by tests today.
#![cfg_attr(feature = "timers", allow(dead_code))]

/// One versioned contract for timing metric identity and meaning.
///
/// Increment this only when a change alters:
/// - a stable metric name
/// - a metric's semantic start or end
/// - wall/accumulated/nested classification
/// - parent or accounting ownership
/// - attribution meaning
/// - aggregate output semantics
///
/// Adding an independent metric that preserves every existing meaning may stay
/// within the same schema; record that decision in the plan checkpoint.
pub(crate) const TIMING_SCHEMA_VERSION: u32 = 1;

/// The session command kind reused for command applicability and command-total
/// ownership, so the registry never duplicates the session's command enum.
pub(crate) use super::session::TimingCommandKind as TimingCommand;

/// Whether a metric belongs to the concise human report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingLevel {
    /// Shown in the concise human report.
    Basic,
    /// Shown only in verbose or bench output.
    Detailed,
}

/// How a metric's duration relates to wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingRelation {
    /// One contiguous wall-clock span.
    WallSpan,
    /// Sum of repeated module or boundary observations.
    Accumulated,
    /// Evidence measured inside a parent row's span; never added separately.
    NestedEvidence,
}

/// Which attribution context a metric may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingAttributionKind {
    /// Not attributed; a single process-wide value.
    None,
    /// Attributed to one compilation boundary (source package or main project).
    Boundary,
    /// Attributed to one source module registered inside a boundary.
    Module,
}

/// Under which commands a metric may be recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingCommandScope {
    /// Every command.
    Universal,
    /// Only the build command.
    BuildOnly,
    /// Only the check command.
    CheckOnly,
    /// Only the dev command.
    DevOnly,
    /// Build and dev; check never runs backend or output phases.
    BuildOrDev,
}

/// The accountable owner of a metric's semantic boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingMetricOwner {
    /// Command-level orchestration.
    Command,
    /// Build-system orchestration: bootstrap, orchestrate, output.
    BuildSystem,
    /// Stage 0 directory and source discovery.
    Stage0,
    /// Frontend module semantics.
    Frontend,
    /// Backend compilation and rendering.
    Backend,
}

/// A virtual aggregate row used by the concise timing summary.
///
/// These groups intentionally have no measured parent span. They only group
/// disjoint accumulated evidence for presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TimingSummaryGroup {
    /// Pre-HIR projection and post-borrow interface closure.
    PublicInterface,
    /// Initial and converged direct borrow validation.
    BorrowValidation,
    /// Generated materialisation, generated AST work and rechecks.
    GeneratedFunctions,
}

/// The typed semantic parent of a timing metric.
///
/// A metric parent represents a measured span that contains this metric. A
/// summary group represents only a human aggregate of disjoint evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingParent {
    Metric(TimingMetric),
    SummaryGroup(TimingSummaryGroup),
}

/// A unique command-pipeline segment used to construct command accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TimingPipelineStage {
    Bootstrap,
    Frontend,
    Backend,
    Output,
}

/// Whether a metric supplies command-accounting wall time or only evidence.
///
/// The concise summary consumes these typed roles instead of maintaining a
/// second list of raw metric names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingAccountingRole {
    /// The command's headline total.
    CommandTotal,
    /// One disjoint top-level command-pipeline segment.
    Pipeline(TimingPipelineStage),
    /// Nested or accumulated evidence that never enters command accounting.
    Evidence,
}

/// One immutable description of a metric in schema order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimingMetricDescriptor {
    /// The contractually stable dotted name, e.g. `frontend.ast.total`.
    pub(crate) stable_name: &'static str,
    pub(crate) level: TimingLevel,
    pub(crate) relation: TimingRelation,
    pub(crate) attribution: TimingAttributionKind,
    pub(crate) command_scope: TimingCommandScope,
    pub(crate) owner: TimingMetricOwner,
    /// The measured parent span or typed virtual summary group.
    pub(crate) parent: Option<TimingParent>,
    /// The metric's command-accounting role.
    pub(crate) accounting: TimingAccountingRole,
}

/// Metrical registry built from one declarative list.
///
/// The table expands to both the dense enum and the parallel descriptor array,
/// so enum/name/descriptor drift is impossible by construction.
macro_rules! timing_metrics {
    ($($variant:ident, $name:literal, $level:ident, $relation:ident, $attribution:ident, $scope:ident, $owner:ident, $parent:expr, $accounting:expr;)*) => {
        /// Dense metric identity; the variant order is the canonical schema
        /// order used for storage, snapshots and benchmark output.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub(crate) enum TimingMetric {
            $($variant,)*
        }

        impl TimingMetric {
            /// Every metric in canonical schema order.
            pub(crate) const ALL: &'static [TimingMetric] = &[$(TimingMetric::$variant,)*];
        }

        pub(crate) const TIMING_METRIC_DESCRIPTORS: &[TimingMetricDescriptor] = &[
            $(TimingMetricDescriptor {
                stable_name: $name,
                level: TimingLevel::$level,
                relation: TimingRelation::$relation,
                attribution: TimingAttributionKind::$attribution,
                command_scope: TimingCommandScope::$scope,
                owner: TimingMetricOwner::$owner,
                parent: $parent,
                accounting: $accounting,
            },)*
        ];
    };
}

timing_metrics! {
// `command.build.total`: complete build command work through required
// output write, excluding timer rendering.
    CommandBuildTotal, "command.build.total", Basic, WallSpan, None, BuildOnly, Command,
        None, TimingAccountingRole::CommandTotal;
// `command.check.total`: complete check command work through diagnostic
// rendering, preserving the chosen command contract.
    CommandCheckTotal, "command.check.total", Basic, WallSpan, None, CheckOnly, Command,
        None, TimingAccountingRole::CommandTotal;
// `command.dev.build_write`: one dev executor build and output write.
    CommandDevBuildWrite, "command.dev.build_write", Basic, WallSpan, None, DevOnly, Command,
        None, TimingAccountingRole::CommandTotal;
// `command.dev.cycle`: detailed full dev cycle, including state and
// broadcast work.
    CommandDevCycle, "command.dev.cycle", Detailed, WallSpan, None, DevOnly, Command,
        None, TimingAccountingRole::Evidence;
// `build.bootstrap.total`: complete build bootstrap.
    BuildBootstrapTotal, "build.bootstrap.total", Basic, WallSpan, None, Universal, BuildSystem,
        None, TimingAccountingRole::Pipeline(TimingPipelineStage::Bootstrap);
// `build.frontend.total`: complete Stage 0 plus project frontend compile.
    BuildFrontendTotal, "build.frontend.total", Basic, WallSpan, None, Universal, BuildSystem,
        None, TimingAccountingRole::Pipeline(TimingPipelineStage::Frontend);
// `build.backend.total`: complete selected backend build.
    BuildBackendTotal, "build.backend.total", Basic, WallSpan, None, BuildOrDev, BuildSystem,
        None, TimingAccountingRole::Pipeline(TimingPipelineStage::Backend);
// `build.output.total`: complete output orchestration.
    BuildOutputTotal, "build.output.total", Basic, WallSpan, None, BuildOrDev, BuildSystem,
        None, TimingAccountingRole::Pipeline(TimingPipelineStage::Output);
// `stage0.directory.inventory`: directory graph, source and module
// inventory work.
    Stage0DirectoryInventory, "stage0.directory.inventory", Basic, NestedEvidence, None, Universal, Stage0,
        Some(TimingParent::Metric(TimingMetric::BuildFrontendTotal)), TimingAccountingRole::Evidence;
// `stage0.directory.compile`: package and project module compilation.
    Stage0DirectoryCompile, "stage0.directory.compile", Basic, NestedEvidence, None, Universal, Stage0,
        Some(TimingParent::Metric(TimingMetric::BuildFrontendTotal)), TimingAccountingRole::Evidence;
// `stage0.single_file.total`: complete single-file Stage 0/frontend
// orchestration.
    Stage0SingleFileTotal, "stage0.single_file.total", Basic, NestedEvidence, None, Universal, Stage0,
        Some(TimingParent::Metric(TimingMetric::BuildFrontendTotal)), TimingAccountingRole::Evidence;
// `boundary.inventory`: accumulated inventory work for one package or
// the main project.
    BoundaryInventory, "boundary.inventory", Basic, Accumulated, Boundary, Universal, Stage0,
        None, TimingAccountingRole::Evidence;
// `boundary.compile`: accumulated compile work for one source package or
// the main project.
    BoundaryCompile, "boundary.compile", Basic, Accumulated, Boundary, Universal, Stage0,
        None, TimingAccountingRole::Evidence;
// `frontend.prepare`: source preparation owned by one module.
    FrontendPrepare, "frontend.prepare", Basic, Accumulated, Module, Universal, Frontend,
        None, TimingAccountingRole::Evidence;
// `frontend.bind_headers`: provider-dependent header binding.
    FrontendBindHeaders, "frontend.bind_headers", Basic, Accumulated, Module, Universal, Frontend,
        None, TimingAccountingRole::Evidence;
// `frontend.order_declarations`: dependency ordering and sorted
// declaration preparation.
    FrontendOrderDeclarations, "frontend.order_declarations", Basic, Accumulated, Module, Universal, Frontend,
        None, TimingAccountingRole::Evidence;
// `frontend.ast.total`: complete module AST construction.
    FrontendAstTotal, "frontend.ast.total", Basic, Accumulated, Module, Universal, Frontend,
        None, TimingAccountingRole::Evidence;
// `frontend.ast.environment`: complete environment construction including
// final environment assembly.
    FrontendAstEnvironment, "frontend.ast.environment", Basic, NestedEvidence, Module, Universal, Frontend,
        Some(TimingParent::Metric(TimingMetric::FrontendAstTotal)), TimingAccountingRole::Evidence;
// `frontend.ast.emit`: AstEmitter production of emitted AST state.
    FrontendAstEmit, "frontend.ast.emit", Basic, NestedEvidence, Module, Universal, Frontend,
        Some(TimingParent::Metric(TimingMetric::FrontendAstTotal)), TimingAccountingRole::Evidence;
// `frontend.ast.finalise`: AstFinalizer production of the final AST.
    FrontendAstFinalise, "frontend.ast.finalise", Basic, NestedEvidence, Module, Universal, Frontend,
        Some(TimingParent::Metric(TimingMetric::FrontendAstTotal)), TimingAccountingRole::Evidence;
// `frontend.public_interface.project`: pre-HIR public-interface
// projection.
    FrontendPublicInterfaceProject,
        "frontend.public_interface.project", Basic, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::PublicInterface)), TimingAccountingRole::Evidence;
// `frontend.hir`: module AST to HIR lowering.
    FrontendHir, "frontend.hir", Basic, Accumulated, Module, Universal, Frontend,
        None, TimingAccountingRole::Evidence;
// `frontend.borrow.initial`: initial direct borrow analysis.
    FrontendBorrowInitial, "frontend.borrow.initial", Basic, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::BorrowValidation)), TimingAccountingRole::Evidence;
// `frontend.borrow.converge`: repeated direct call-summary borrow
// convergence.
    FrontendBorrowConverge, "frontend.borrow.converge", Basic, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::BorrowValidation)), TimingAccountingRole::Evidence;
// `frontend.generated.materialise`: generated-function materialisation.
    FrontendGeneratedMaterialise,
        "frontend.generated.materialise", Basic, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::GeneratedFunctions)), TimingAccountingRole::Evidence;
// `frontend.generated.borrow_recheck`: generated sidecar borrow rechecks.
    FrontendGeneratedBorrowRecheck,
        "frontend.generated.borrow_recheck", Basic, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::GeneratedFunctions)), TimingAccountingRole::Evidence;
// `frontend.public_interface.finalise`: post-borrow public-interface
// closure.
    FrontendPublicInterfaceFinalise,
        "frontend.public_interface.finalise", Basic, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::PublicInterface)), TimingAccountingRole::Evidence;
// `frontend.module.semantic_total`: complete provider-dependent semantic
// module compilation.
    FrontendModuleSemanticTotal,
        "frontend.module.semantic_total", Basic, Accumulated, Module, Universal, Frontend,
        None, TimingAccountingRole::Evidence;
// `config.ast.total`: complete config AST construction.
    ConfigAstTotal, "config.ast.total", Detailed, NestedEvidence, None, Universal, BuildSystem,
        Some(TimingParent::Metric(TimingMetric::BuildBootstrapTotal)), TimingAccountingRole::Evidence;
// `config.ast.environment`: detailed config AST environment.
    ConfigAstEnvironment, "config.ast.environment", Detailed, NestedEvidence, None, Universal, BuildSystem,
        Some(TimingParent::Metric(TimingMetric::ConfigAstTotal)), TimingAccountingRole::Evidence;
// `config.ast.emit`: detailed config AST emission.
    ConfigAstEmit, "config.ast.emit", Detailed, NestedEvidence, None, Universal, BuildSystem,
        Some(TimingParent::Metric(TimingMetric::ConfigAstTotal)), TimingAccountingRole::Evidence;
// `config.ast.finalise`: detailed config AST finalisation.
    ConfigAstFinalise, "config.ast.finalise", Detailed, NestedEvidence, None, Universal, BuildSystem,
        Some(TimingParent::Metric(TimingMetric::ConfigAstTotal)), TimingAccountingRole::Evidence;
// `frontend.generated.ast.total`: generated materialisation AST work.
    FrontendGeneratedAstTotal, "frontend.generated.ast.total", Detailed, Accumulated, Module, Universal, Frontend,
        Some(TimingParent::SummaryGroup(TimingSummaryGroup::GeneratedFunctions)), TimingAccountingRole::Evidence;
// `frontend.generated.ast.environment`: detailed generated AST
// environment.
    FrontendGeneratedAstEnvironment,
        "frontend.generated.ast.environment", Detailed, NestedEvidence, Module, Universal, Frontend,
        Some(TimingParent::Metric(TimingMetric::FrontendGeneratedAstTotal)), TimingAccountingRole::Evidence;
// `frontend.generated.ast.emit`: detailed generated AST emission.
    FrontendGeneratedAstEmit, "frontend.generated.ast.emit", Detailed, NestedEvidence, Module, Universal, Frontend,
        Some(TimingParent::Metric(TimingMetric::FrontendGeneratedAstTotal)), TimingAccountingRole::Evidence;
// `frontend.generated.ast.finalise`: detailed generated AST finalisation.
    FrontendGeneratedAstFinalise,
        "frontend.generated.ast.finalise", Detailed, NestedEvidence, Module, Universal, Frontend,
        Some(TimingParent::Metric(TimingMetric::FrontendGeneratedAstTotal)), TimingAccountingRole::Evidence;
// `backend.js.lower_entry`: entry-module HIR to JS lowering.
    BackendJsLowerEntry, "backend.js.lower_entry", Basic, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal)), TimingAccountingRole::Evidence;
// `backend.js.lower_linked`: linked-module HIR to JS lowering.
    BackendJsLowerLinked, "backend.js.lower_linked", Basic, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal)), TimingAccountingRole::Evidence;
// `backend.html.render`: HTML document rendering.
    BackendHtmlRender, "backend.html.render", Basic, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal)), TimingAccountingRole::Evidence;
// `backend.wasm.total`: complete HTML-Wasm route build.
    BackendWasmTotal, "backend.wasm.total", Basic, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal)), TimingAccountingRole::Evidence;
// `backend.wasm.lower`: Wasm lowering only.
    BackendWasmLower, "backend.wasm.lower", Detailed, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BackendWasmTotal)), TimingAccountingRole::Evidence;
// `backend.wasm.artifacts`: Wasm artifact and bootstrap assembly.
    BackendWasmArtifacts, "backend.wasm.artifacts", Detailed, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BackendWasmTotal)), TimingAccountingRole::Evidence;
// `backend.assets.plan`: tracked/runtime asset planning.
    BackendAssetsPlan, "backend.assets.plan", Basic, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal)), TimingAccountingRole::Evidence;
// `backend.assets.emit`: tracked/runtime asset emission.
    BackendAssetsEmit, "backend.assets.emit", Basic, NestedEvidence, None, BuildOrDev, Backend,
        Some(TimingParent::Metric(TimingMetric::BuildBackendTotal)), TimingAccountingRole::Evidence;
// `output.write.total`: complete output file write orchestration.
    OutputWriteTotal, "output.write.total", Basic, NestedEvidence, None, BuildOrDev, BuildSystem,
        Some(TimingParent::Metric(TimingMetric::BuildOutputTotal)), TimingAccountingRole::Evidence;
}

impl TimingMetric {
    /// The descriptor for this metric.
    pub(crate) const fn descriptor(self) -> &'static TimingMetricDescriptor {
        &TIMING_METRIC_DESCRIPTORS[self as usize]
    }

    /// The canonical schema index, equal to the position in `ALL`.
    ///
    /// The enum discriminant is the position in `ALL` because both derive
    /// from the same declarative table.
    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The metric at a dense index, when the index is in range.
    pub(crate) fn from_index(index: usize) -> Option<TimingMetric> {
        TimingMetric::ALL.get(index).copied()
    }

    /// Look up a metric by its exact stable name.
    pub(crate) fn from_name(name: &str) -> Option<TimingMetric> {
        TimingMetric::ALL
            .iter()
            .find(|metric| metric.descriptor().stable_name == name)
            .copied()
    }

    /// Whether this metric answers a command's reported total.
    pub(crate) const fn is_command_total(self) -> bool {
        matches!(
            self,
            TimingMetric::CommandBuildTotal
                | TimingMetric::CommandCheckTotal
                | TimingMetric::CommandDevBuildWrite
        )
    }

    /// The one metric that owns a command's total, when the command has one.
    ///
    /// The schema guarantees exactly one total per command, so command reports
    /// never guess or collide on their headline duration.
    pub(crate) const fn command_total(command: TimingCommand) -> Option<TimingMetric> {
        match command {
            TimingCommand::Build => Some(TimingMetric::CommandBuildTotal),
            TimingCommand::Check => Some(TimingMetric::CommandCheckTotal),
            TimingCommand::Dev => Some(TimingMetric::CommandDevBuildWrite),
        }
    }

    /// Whether a command may record this metric under its scope.
    pub(crate) const fn applies_to(self, command: TimingCommand) -> bool {
        match self.descriptor().command_scope {
            TimingCommandScope::Universal => true,
            TimingCommandScope::BuildOnly => matches!(command, TimingCommand::Build),
            TimingCommandScope::CheckOnly => matches!(command, TimingCommand::Check),
            TimingCommandScope::DevOnly => matches!(command, TimingCommand::Dev),
            TimingCommandScope::BuildOrDev => {
                matches!(command, TimingCommand::Build | TimingCommand::Dev)
            }
        }
    }
}
