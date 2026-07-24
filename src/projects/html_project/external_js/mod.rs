//! HTML JavaScript external binding module support.
//!
//! WHAT: parses single-file JavaScript binding modules annotated with Moth `@moth.*`
//!       metadata into a structured, parser-owned data model, and tracks builder-owned
//!       core JS runtime modules such as `@moth/runtime`.
//! WHY: project-local `.js` imports and built-in JS-backed packages such as `@web/canvas`
//!      need a typed surface before they can be fed into the compiler frontend.
//!
//! This module is intentionally isolated from compiler-stage machinery.
//! The JS external import provider and built-in JS-backed packages convert
//! `ParsedJsModule` into `ExternalPackageRegistry` entries before the frontend
//! consumes package visibility.
//!
//! ## Module layout
//!
//! - `parser/`: the `@moth.*` annotation scanner, export scanner, and signature parser.
//! - `runtime_module_registry`: builder-owned registry of allowed JS runtime module imports
//!   and their authored source.

pub(crate) mod js_import_provider;
pub(crate) mod package_registration;
pub(crate) mod parser;
pub(crate) mod path_identity;
pub(crate) mod runtime_assets;
pub(crate) mod runtime_emission_plan;
pub(crate) mod runtime_glue;
pub(crate) mod runtime_module_registry;
