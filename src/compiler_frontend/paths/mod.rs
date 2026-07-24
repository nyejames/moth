//! Moth path syntax, project-aware resolution, and rendered-path tracking.
//!
//! WHAT: keeps path parsing (`const_paths`), project/import resolution (`path_resolution`),
//! compile-time path values (`compile_time_paths`), public/runtime formatting (`path_format`),
//! module-root discovery (`module_roots`), normalization helpers (`path_normalization`), and
//! rendered usage collection (`rendered_path_usage`) behind one frontend module map.
//! WHY: paths cross Stage 0, header parsing, AST folding, diagnostics, and backend builders.
//! This module should expose those owners without letting import semantics, path literal values,
//! and rendered output formatting collapse into one implementation path.
//!
//! This module must not own module/import visibility policy. Header import preparation and Stage 0
//! project discovery consume the path helpers, then apply their own stage-specific rules.

pub(crate) mod compile_time_paths;
pub(crate) mod const_paths;
pub(crate) mod import_resolution;
pub(crate) mod module_roots;
pub(crate) mod path_format;
pub(crate) mod path_normalization;
pub(crate) mod path_resolution;
pub(crate) mod rendered_path_usage;
