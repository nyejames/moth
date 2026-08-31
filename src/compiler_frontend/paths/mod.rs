//! Moth path syntax, project-aware resolution, and rendered-path tracking.
//!
//! WHAT: keeps path parsing (`const_paths`), project/dependency resolution (`path_resolution`),
//! compile-time path values (`compile_time_paths`), public/runtime formatting (`path_format`),
//! module-root discovery (`module_roots`), normalization helpers (`path_normalization`),
//! rendered usage collection (`rendered_path_usage`), structural file references
//! (`file_references`), stable resource identity (`resource_identity`), the module-local
//! resource origin table (`module_resources`) and site-root URL rendering (`site_root`)
//! behind one frontend module map.
//! WHY: paths cross Stage 0, header parsing, AST folding, diagnostics, and backend builders.
//! This module should expose those owners without letting dependency semantics, path literal values,
//! and rendered output formatting collapse into one implementation path.
//!
//! This module must not own module/dependency visibility policy. Header dependency preparation and Stage 0
//! project discovery consume the path helpers, then apply their own stage-specific rules.

pub(crate) mod compile_time_paths;
pub(crate) mod const_paths;
pub(crate) mod dependency_resolution;
pub(crate) mod file_references;
pub(crate) mod module_resources;
pub(crate) mod module_roots;
pub(crate) mod path_format;
pub(crate) mod path_normalization;
pub(crate) mod path_resolution;
pub(crate) mod path_syntax;
pub(crate) mod rendered_path_usage;
pub(crate) mod resource_identity;
pub(crate) mod site_root;
