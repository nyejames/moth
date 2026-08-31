//! Moth path syntax, project-aware resolution, and structural file references.
//!
//! WHAT: keeps path parsing (`const_paths`), project/dependency resolution (`path_resolution`),
//! compile-time path bases (`compile_time_paths`), module-root discovery (`module_roots`),
//! normalization helpers (`path_normalization`), structural file references (`file_references`),
//! stable resource identity (`resource_identity`) and the module-local resource origin table
//! (`module_resources`) behind one frontend module map.
//! WHY: paths cross Stage 0, header parsing, AST folding, diagnostics, and backend builders.
//! This module should expose those owners without letting dependency semantics, path literal values,
//! and structural output pieces collapse into one implementation path.
//!
//! This module must not own module/dependency visibility policy. Header dependency preparation and Stage 0
//! project discovery consume the path helpers, then apply their own stage-specific rules.

pub(crate) mod compile_time_paths;
pub(crate) mod const_paths;
pub(crate) mod dependency_resolution;
pub(crate) mod file_references;
pub(crate) mod module_resources;
pub(crate) mod module_roots;
pub(crate) mod path_normalization;
pub(crate) mod path_resolution;
pub(crate) mod path_syntax;
pub(crate) mod resource_identity;
