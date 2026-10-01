//! Compiler-owned frontend options.
//!
//! WHAT: the exact settings the frontend consumes while compiling one module: the compilation
//!       boundary's `NumericProfile` and how far a compile-time template loop may run.
//! WHY:  the frontend must not read the project tool's configuration container to compile source.
//!       Callers translate their own configuration and their boundary's numeric profile into this
//!       value, so only settings the compiler actually uses cross the boundary.

use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;

/// Default iteration ceiling for a compile-time template loop.
///
/// WHY: the limit is a compiler semantic guard against non-terminating const template folding, so
///      the compiler owns its default. Project configuration may lower it through
///      [`FrontendOptions`], and the build system owns the config key and its accepted maximum.
pub(crate) const DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 10_000;

/// Settings one frontend instance consumes.
#[derive(Clone, Debug)]
pub(crate) struct FrontendOptions {
    /// The `Int` width and `Float` precision every numeric type in this compilation boundary uses.
    ///
    /// WHAT: the selected builder's profile, carried unchanged through the build entry points.
    /// WHY: numeric typing is one boundary-wide fact, so the frontend reads it here instead of
    ///      choosing a representation per service.
    pub(crate) numeric_profile: NumericProfile,
    /// Iteration ceiling for compile-time template loops.
    pub(crate) template_const_loop_iteration_limit: usize,
}

#[cfg(test)]
impl Default for FrontendOptions {
    /// The standard-profile settings a standalone test fixture compiles under.
    ///
    /// WHY: only fixtures compile one standalone source with no configured origin, so only they
    ///      need a ready-made value. Production callers must project the selected boundary profile
    ///      through `Config::frontend_options`, and a shipping `Default` would let a caller compile
    ///      numbers under widths no builder selected.
    fn default() -> Self {
        Self {
            numeric_profile: NumericProfile::STANDARD,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        }
    }
}
