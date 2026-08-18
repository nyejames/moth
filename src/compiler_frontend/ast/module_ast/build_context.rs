//! Shared AST build inputs.
//!
//! WHAT: groups long-lived frontend services and per-build settings used by the AST phases.
//! WHY: environment building, node emission, and finalization all need the same build services,
//!      but each phase owns its own mutable state and must borrow the `StringTable` independently.
//!
//! ## Phase separation
//!
//! `AstBuildContext` carries the full context including a mutable `StringTable` reference.
//! `AstPhaseContext` is a narrowed view that omits the `StringTable` so each phase can borrow
//! it mutably while still accessing the shared immutable services.
//!
//! The entry point creates one `AstBuildContext`, then each phase narrows to `AstPhaseContext`
//! and re-borrows the `StringTable` as needed.
//!
//! ## TIR store ownership
//!
//! `AstPhaseContext` allocates one module-local `TemplateIrStore`. All production
//! parser and semantic contexts receive clones of that shared handle.

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
#[cfg(feature = "timers")]
use crate::timing::TimingMetric;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Exact current-schema metric family for one AST construction path.
///
/// Frontend modules, `config.moth`, and generated materialisation all use the
/// same AST implementation but have deliberately distinct timing identities.
/// Keeping that choice in the AST context prevents a config or generated AST
/// span from being misreported as module-attributed frontend work.
#[cfg(feature = "timers")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AstTimingMetricFamily {
    Frontend,
    Config,
    Generated,
}

#[cfg(feature = "timers")]
impl AstTimingMetricFamily {
    pub(crate) const fn total(self) -> TimingMetric {
        match self {
            Self::Frontend => TimingMetric::FrontendAstTotal,
            Self::Config => TimingMetric::ConfigAstTotal,
            Self::Generated => TimingMetric::FrontendGeneratedAstTotal,
        }
    }

    pub(crate) const fn environment(self) -> TimingMetric {
        match self {
            Self::Frontend => TimingMetric::FrontendAstEnvironment,
            Self::Config => TimingMetric::ConfigAstEnvironment,
            Self::Generated => TimingMetric::FrontendGeneratedAstEnvironment,
        }
    }

    pub(crate) const fn emit(self) -> TimingMetric {
        match self {
            Self::Frontend => TimingMetric::FrontendAstEmit,
            Self::Config => TimingMetric::ConfigAstEmit,
            Self::Generated => TimingMetric::FrontendGeneratedAstEmit,
        }
    }

    pub(crate) const fn finalise(self) -> TimingMetric {
        match self {
            Self::Frontend => TimingMetric::FrontendAstFinalise,
            Self::Config => TimingMetric::ConfigAstFinalise,
            Self::Generated => TimingMetric::FrontendGeneratedAstFinalise,
        }
    }

    /// Constant-header semantic resolution metric, nested inside the
    /// environment pass. These four constant-sensitive metrics are
    /// frontend-module-attributed evidence only, so `None` is returned for
    /// Config and Generated.
    pub(crate) const fn constant_header_resolution(self) -> Option<TimingMetric> {
        match self {
            Self::Frontend => Some(TimingMetric::FrontendAstEnvironmentConstantHeaderResolution),
            Self::Config => None,
            Self::Generated => None,
        }
    }

    /// Const-template parsing metric, nested inside the emission pass.
    /// Frontend-module-attributed evidence only, so `None` is returned
    /// for Config and Generated.
    pub(crate) const fn const_template_parse(self) -> Option<TimingMetric> {
        match self {
            Self::Frontend => Some(TimingMetric::FrontendAstEmitConstTemplateParse),
            Self::Config => None,
            Self::Generated => None,
        }
    }

    /// Const-template folding metric, nested inside the emission pass.
    /// Frontend-module-attributed evidence only, so `None` is returned
    /// for Config and Generated.
    pub(crate) const fn const_template_fold(self) -> Option<TimingMetric> {
        match self {
            Self::Frontend => Some(TimingMetric::FrontendAstEmitConstTemplateFold),
            Self::Config => None,
            Self::Generated => None,
        }
    }

    /// Module-constant finalisation metric, nested inside the finalisation
    /// pass. Frontend-module-attributed evidence only, so `None` is
    /// returned for Config and Generated.
    pub(crate) const fn module_constant(self) -> Option<TimingMetric> {
        match self {
            Self::Frontend => Some(TimingMetric::FrontendAstFinaliseModuleConstant),
            Self::Config => None,
            Self::Generated => None,
        }
    }
}

/// Shared dependencies and configuration required to build one module AST.
///
/// WHAT: holds all immutable registries, the mutable string table, and path/build settings
///       that remain constant for the lifetime of a single module compilation.
/// WHY: centralises service ownership so the environment builder, emitter, and finalizer
///      do not need to track these individually.
pub struct AstBuildContext<'a> {
    /// Backend-provided virtual package metadata and external symbol registry.
    pub external_package_registry: Arc<ExternalPackageRegistry>,

    /// Merged frontend + builder style directive registry used by tokenizer and template parsing.
    pub style_directives: &'a StyleDirectiveRegistry,

    /// Mutable string table for interning paths, symbols, and diagnostic strings.
    pub string_table: &'a mut StringTable,

    /// Canonical path of the module entry directory.
    pub entry_dir: InternedPath,

    /// Graph-owned semantic role of the active module root.
    pub root_role: ModuleRootRole,

    /// Current build profile (dev/release) affecting optimization and diagnostic levels.
    pub build_profile: FrontendBuildProfile,

    /// Optional project-relative path resolver for source-backed package and dependency resolution.
    pub project_path_resolver: Option<ProjectPathResolver>,

    /// Formatting rules for rendering interned paths in diagnostics and output.
    pub path_format_config: PathStringFormatConfig,

    /// Per-loop expansion limit for compile-time template loops.
    pub template_const_loop_iteration_limit: usize,

    /// Module-level frontend arena capacity policy gathered before AST construction.
    pub capacity_estimate: FrontendArenaCapacityEstimate,

    /// Timer-only attribution context for this AST build.
    ///
    /// Module AST construction carries the module key; config and generated
    /// materialisation pass `None` so their raw observations never appear as
    /// module AST children in the basic report.
    #[cfg(feature = "timers")]
    pub(crate) timing_context: Option<crate::timing::TimingContext>,

    /// Selects the current-schema identity family for this AST construction.
    #[cfg(feature = "timers")]
    pub(crate) timing_metric_family: AstTimingMetricFamily,
}

/// Narrowed phase-local view of `AstBuildContext` without the mutable `StringTable`.
///
/// WHAT: allows a phase to borrow the `StringTable` mutably while retaining access to all
///       other shared build services.
/// WHY: prevents simultaneous mutable borrows of the string table and the context struct
///      when both are passed through recursive parsing calls.
pub(crate) struct AstPhaseContext<'a> {
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) entry_dir: InternedPath,
    pub(crate) root_role: ModuleRootRole,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) project_path_resolver: Option<ProjectPathResolver>,
    pub(crate) path_format_config: PathStringFormatConfig,
    pub(crate) template_const_loop_iteration_limit: usize,
    pub(crate) capacity_estimate: FrontendArenaCapacityEstimate,

    /// Shared module-local TIR store for this AST phase.
    pub(crate) template_ir_store: Rc<RefCell<TemplateIrStore>>,

    /// Timer-only attribution context for this AST phase.
    #[cfg(feature = "timers")]
    pub(crate) timing_context: Option<crate::timing::TimingContext>,

    /// Exact stable metric names selected by the owning AST path.
    #[cfg(feature = "timers")]
    pub(crate) timing_metric_family: AstTimingMetricFamily,
}

impl<'a> AstPhaseContext<'a> {
    /// Split the full build context into its phase-local view and the mutable string table.
    ///
    /// WHAT: extracts all fields except `string_table` into `AstPhaseContext` and returns
    ///       the table as a separate mutable reference.
    /// WHY: lets the caller pass the phase context and string table independently,
    ///      resolving Rust's borrow checker constraints across phase boundaries.
    pub(crate) fn from_build_context(context: AstBuildContext<'a>) -> (Self, &'a mut StringTable) {
        let AstBuildContext {
            external_package_registry,
            style_directives,
            string_table,
            entry_dir,
            root_role,
            build_profile,
            project_path_resolver,
            path_format_config,
            template_const_loop_iteration_limit,
            capacity_estimate,
            #[cfg(feature = "timers")]
            timing_context,
            #[cfg(feature = "timers")]
            timing_metric_family,
        } = context;

        let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::with_capacity_estimate(
            capacity_estimate,
        )));

        (
            Self {
                external_package_registry,
                style_directives,
                entry_dir,
                root_role,
                build_profile,
                project_path_resolver,
                path_format_config,
                template_const_loop_iteration_limit,
                capacity_estimate,
                template_ir_store,
                #[cfg(feature = "timers")]
                timing_context,
                #[cfg(feature = "timers")]
                timing_metric_family,
            },
            string_table,
        )
    }
}
