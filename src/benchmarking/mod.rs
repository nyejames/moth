//! Public compiler benchmarking API for dev tooling.
//!
//! WHAT: provides in-process benchmark entry points that reuse production
//! compiler setup without duplicating project discovery or builder logic.
//! WHY: xtask and other tooling need focused compiler-stage measurements
//! without subprocess overhead.

pub mod frontend;

/// Stable timing observation schema exposed by the in-process benchmark API.
#[cfg(feature = "timers")]
pub use crate::timing::TIMING_SCHEMA_VERSION;

/// Stable metric names in the timing schema's canonical output order.
#[cfg(feature = "timers")]
pub const TIMING_SCHEMA_METRIC_NAMES: &[&str] = crate::timing::TIMING_SCHEMA_METRIC_NAMES;

/// Schema-owned pipeline rows used for build command wall accounting.
#[cfg(feature = "timers")]
pub const TIMING_BUILD_PIPELINE_METRIC_NAMES: &[&str] =
    crate::timing::TIMING_BUILD_PIPELINE_METRIC_NAMES;

/// Schema-owned pipeline rows used for check command wall accounting.
#[cfg(feature = "timers")]
pub const TIMING_CHECK_PIPELINE_METRIC_NAMES: &[&str] =
    crate::timing::TIMING_CHECK_PIPELINE_METRIC_NAMES;

/// Schema-owned command-total identities used by CLI benchmark parsing.
#[cfg(feature = "timers")]
pub const TIMING_COMMAND_BUILD_TOTAL_NAME: &str = crate::timing::TIMING_COMMAND_BUILD_TOTAL_NAME;
#[cfg(feature = "timers")]
pub const TIMING_COMMAND_CHECK_TOTAL_NAME: &str = crate::timing::TIMING_COMMAND_CHECK_TOTAL_NAME;

#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_PREPARE_NAME: &str = crate::timing::TIMING_FRONTEND_PREPARE_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_ORDER_DECLARATIONS_NAME: &str =
    crate::timing::TIMING_FRONTEND_ORDER_DECLARATIONS_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_AST_TOTAL_NAME: &str = crate::timing::TIMING_FRONTEND_AST_TOTAL_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_AST_ENVIRONMENT_NAME: &str =
    crate::timing::TIMING_FRONTEND_AST_ENVIRONMENT_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_AST_EMIT_NAME: &str = crate::timing::TIMING_FRONTEND_AST_EMIT_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_AST_FINALISE_NAME: &str =
    crate::timing::TIMING_FRONTEND_AST_FINALISE_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_HIR_NAME: &str = crate::timing::TIMING_FRONTEND_HIR_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_BORROW_INITIAL_NAME: &str =
    crate::timing::TIMING_FRONTEND_BORROW_INITIAL_NAME;
#[cfg(feature = "timers")]
pub const TIMING_FRONTEND_BORROW_CONVERGE_NAME: &str =
    crate::timing::TIMING_FRONTEND_BORROW_CONVERGE_NAME;

/// Resolve a stable schema name to the concise label used by benchmark
/// summaries. Unknown or legacy names are returned unchanged.
#[cfg(feature = "timers")]
pub fn timing_metric_label(name: &str) -> &str {
    crate::timing::benchmark_label_for_name(name)
}

pub use frontend::{
    FrontendBenchmarkBuildProfile, FrontendBenchmarkCounter, FrontendBenchmarkError,
    FrontendBenchmarkOptions, FrontendBenchmarkReport, FrontendBenchmarkStage,
    run_frontend_benchmark,
};

#[cfg(test)]
mod tests;
