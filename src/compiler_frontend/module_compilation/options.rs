//! Compiler-owned frontend options.
//!
//! WHAT: the exact settings the frontend consumes while compiling one module: how rendered paths
//!       are spelled and how far a compile-time template loop may run.
//! WHY:  the frontend must not read the project tool's configuration container to compile source.
//!       Callers translate their own configuration into this value, so only settings the compiler
//!       actually uses cross the boundary.

use crate::compiler_frontend::paths::path_format::{OutputPathStyle, PathStringFormatConfig};

/// Default iteration ceiling for a compile-time template loop.
///
/// WHY: the limit is a compiler semantic guard against non-terminating const template folding, so
///      the compiler owns its default. Project configuration may lower it through
///      [`FrontendOptions`], and the build system owns the config key and its accepted maximum.
pub const DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS: usize = 10_000;

/// Settings one frontend instance consumes.
#[derive(Clone, Debug)]
pub(crate) struct FrontendOptions {
    /// How resolved paths are rendered into source-visible strings.
    pub(crate) path_format_config: PathStringFormatConfig,
    /// Iteration ceiling for compile-time template loops.
    pub(crate) template_const_loop_iteration_limit: usize,
}

impl FrontendOptions {
    /// Build options for a caller that only needs to override the rendered path origin.
    ///
    /// WHY: every current caller derives portable output paths from one origin string and keeps
    ///      the compiler's own loop ceiling, so this is the whole boundary surface today.
    pub(crate) fn from_origin(origin: String) -> Self {
        Self {
            path_format_config: PathStringFormatConfig {
                origin,
                output_style: OutputPathStyle::Portable,
            },
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        }
    }
}

impl Default for FrontendOptions {
    fn default() -> Self {
        Self::from_origin(String::from("/"))
    }
}
