//! Compiler-owned frontend options.
//!
//! WHAT: the exact settings the frontend consumes while compiling one module: how far a
//!       compile-time template loop may run and how much diagnostic recovery it may attempt.
//! WHY:  the frontend must not read the project tool's configuration container to compile source.
//!       Callers translate their own configuration into this value, so only settings the compiler
//!       actually uses cross the boundary.

use std::num::NonZeroU32;

/// Default iteration ceiling for a compile-time template loop.
///
/// WHY: the limit is a compiler semantic guard against non-terminating const template folding, so
///      the compiler owns its default. Project configuration may lower it through
///      [`FrontendOptions`], and the build system owns the config key and its accepted maximum.
pub(crate) const DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 10_000;

/// Default primary-error allowance retained for each module.
///
/// WHY: the allowance bounds the frontend's own diagnostic recovery, so the compiler owns its
///      default and project configuration overrides it through [`FrontendOptions`]. The project
///      config omits the setting to accept this default and supplies an explicit absence to disable
///      the allowance, so the default is a value rather than the absence of the limit.
pub(crate) const DEFAULT_MAX_ERRORS_PER_MODULE: NonZeroU32 =
    NonZeroU32::new(5).expect("the default module error allowance is non-zero");

/// Tokens of additional body syntax one module may admit after its first body diagnosis.
///
/// WHAT: the compiler-owned recovery-work allowance. Tests supply smaller synthetic values through
///       [`AstRecoveryPolicy`]; production always uses this constant.
/// WHY: recovery may parse another body only within a deterministic token budget that no project
///      setting can raise or disable. 4096 tokens covers about 80 mean-sized ordinary bodies, enough
///      to reach the default five-error allowance across ordinary spacing, while a smaller budget
///      lost independent errors and a larger one only added work. The benchmark report owns the
///      measurements.
pub(crate) const AST_RECOVERY_WORK_TOKENS: u32 = 4_096;

/// Default work steps one const call may consume.
pub(crate) const DEFAULT_CONST_EXECUTION_WORK_LIMIT: usize = 1_000_000;
/// Hard ceiling a project setting cannot raise for one const call.
pub(crate) const MAX_CONST_EXECUTION_WORK_LIMIT: usize = 16_000_000;
/// Default retained folded-text bytes one const call may produce.
pub(crate) const DEFAULT_CONST_EXECUTION_MEMORY_LIMIT_BYTES: usize = 16 * 1024 * 1024;
/// Hard ceiling a project setting cannot raise for retained folded memory.
pub(crate) const MAX_CONST_EXECUTION_MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
/// Aggregate work and memory allow this many attempts' worth of the per-call allowance.
pub(crate) const CONST_EXECUTION_AGGREGATE_MULTIPLIER: usize = 8;

/// Typed work and memory allowances for const execution.
///
/// WHAT: carries one per-attempt allowance and the larger module-wide safeguards selected from
///       project configuration.
/// WHY:  every const call and bounded template fold charges the same budget owner. Keeping the
///       limits typed prevents a template loop setting or a raw numeric field from becoming a
///       general function-work budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConstExecutionLimits {
    pub(crate) max_attempt_steps: usize,
    pub(crate) max_total_steps: usize,
    pub(crate) max_attempt_memory_bytes: usize,
    pub(crate) max_total_memory_bytes: usize,
}

impl Default for ConstExecutionLimits {
    fn default() -> Self {
        Self {
            max_attempt_steps: DEFAULT_CONST_EXECUTION_WORK_LIMIT,
            max_total_steps: DEFAULT_CONST_EXECUTION_WORK_LIMIT
                * CONST_EXECUTION_AGGREGATE_MULTIPLIER,
            max_attempt_memory_bytes: DEFAULT_CONST_EXECUTION_MEMORY_LIMIT_BYTES,
            max_total_memory_bytes: DEFAULT_CONST_EXECUTION_MEMORY_LIMIT_BYTES
                * CONST_EXECUTION_AGGREGATE_MULTIPLIER,
        }
    }
}

impl ConstExecutionLimits {
    pub(crate) fn from_configured(work_limit: usize, memory_limit_bytes: usize) -> Self {
        Self {
            max_attempt_steps: work_limit.min(MAX_CONST_EXECUTION_WORK_LIMIT),
            max_total_steps: work_limit
                .min(MAX_CONST_EXECUTION_WORK_LIMIT)
                .saturating_mul(CONST_EXECUTION_AGGREGATE_MULTIPLIER),
            max_attempt_memory_bytes: memory_limit_bytes
                .min(MAX_CONST_EXECUTION_MEMORY_LIMIT_BYTES),
            max_total_memory_bytes: memory_limit_bytes
                .min(MAX_CONST_EXECUTION_MEMORY_LIMIT_BYTES)
                .saturating_mul(CONST_EXECUTION_AGGREGATE_MULTIPLIER),
        }
    }
}

/// Settings one frontend instance consumes.
#[derive(Clone, Debug)]
pub(crate) struct FrontendOptions {
    /// Iteration ceiling for compile-time template loops.
    pub(crate) template_const_loop_iteration_limit: usize,
    /// Primary errors retained per module before optional diagnostic recovery stops.
    ///
    /// `None` disables only this count limit; the recovery-work guard still applies.
    pub(crate) max_errors_per_module: Option<NonZeroU32>,
    /// Work and retained-memory allowances for const execution.
    pub(crate) const_execution_limits: ConstExecutionLimits,
}

/// The recovery policy one AST build consumes.
///
/// WHAT: the module primary-error allowance and the compiler-owned additional-body token budget.
/// WHY: the emitter owns where recovery stops, but the project configuration owns the count limit,
///      so the two values travel beside the AST input rather than inside a scope or diagnostic.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AstRecoveryPolicy {
    pub(crate) module_error_allowance: Option<NonZeroU32>,
    pub(crate) body_work_allowance: u32,
}

impl AstRecoveryPolicy {
    /// The policy a frontend instance projects from its configured options.
    pub(crate) fn from_frontend_options(options: &FrontendOptions) -> Self {
        Self {
            module_error_allowance: options.max_errors_per_module,
            body_work_allowance: AST_RECOVERY_WORK_TOKENS,
        }
    }

    /// The policy for a build with no project configuration source.
    ///
    /// WHY: direct template and config builds still construct an AST, and they use the same
    ///      omitted-config defaults a project without either diagnostic key would receive.
    pub(crate) fn configured_defaults() -> Self {
        Self {
            module_error_allowance: Some(DEFAULT_MAX_ERRORS_PER_MODULE),
            body_work_allowance: AST_RECOVERY_WORK_TOKENS,
        }
    }
}

#[cfg(test)]
impl Default for FrontendOptions {
    /// The settings a standalone test fixture compiles under.
    ///
    /// WHY: fixtures build their options directly instead of loading a project, so they need a
    ///      ready-made value. Production callers project their configuration through
    ///      `Config::frontend_options`, or use [`AstRecoveryPolicy::configured_defaults`] when a
    ///      direct template or config build has no project configuration source. A shipping
    ///      `Default` would let a caller compile without the project's configured settings.
    fn default() -> Self {
        Self {
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            max_errors_per_module: Some(DEFAULT_MAX_ERRORS_PER_MODULE),
            const_execution_limits: ConstExecutionLimits::default(),
        }
    }
}
