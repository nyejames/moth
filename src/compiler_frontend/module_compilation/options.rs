//! Compiler-owned frontend options.
//!
//! WHAT: the exact settings the frontend consumes while compiling one module: how far a
//!       compile-time template loop may run.
//! WHY:  the frontend must not read the project tool's configuration container to compile source.
//!       Callers translate their own configuration into this value, so only settings the compiler
//!       actually uses cross the boundary.

/// Default iteration ceiling for a compile-time template loop.
///
/// WHY: the limit is a compiler semantic guard against non-terminating const template folding, so
///      the compiler owns its default. Project configuration may lower it through
///      [`FrontendOptions`], and the build system owns the config key and its accepted maximum.
pub(crate) const DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 10_000;

/// Settings one frontend instance consumes.
#[derive(Clone, Debug)]
pub(crate) struct FrontendOptions {
    /// Iteration ceiling for compile-time template loops.
    pub(crate) template_const_loop_iteration_limit: usize,
}

impl Default for FrontendOptions {
    /// The settings a caller with no project configuration compiles under.
    ///
    /// WHY: the direct Moth template service and every fixture compile one standalone source, so
    ///      there is no configured origin and no reason to lower the compiler's own loop ceiling.
    ///      `Config::frontend_options` is the projection that may override the ceiling.
    fn default() -> Self {
        Self {
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        }
    }
}
