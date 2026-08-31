//! HTML project backend components.
//!
//! WHAT: groups HTML document generation, routing/output planning, and backend-specific helpers.
//! WHY: HTML builds stitch together several focused subsystems around the shared frontend/HIR
//! pipeline.
//!
//! Resource placement and URL text are separate owners:
//! - `resource_output_plan`    — byte-free output placement and conflict preflight for planned
//!   resource origins, before any provider or resource reader runs
//! - `structural_url_renderer` — final context-sensitive URL text, rendered per consuming
//!   artefact once output planning has assigned that artefact a URL context

pub(crate) mod binding_packages;
pub(crate) mod compile_input;
pub(crate) mod diagnostics;
pub(crate) mod document_config;
pub(crate) mod document_shell;
pub(crate) mod external_js;
pub mod html_project_builder;
pub(crate) mod js_path;
pub(crate) mod moth_template;
pub mod new_html_project;
pub(crate) mod output_plan;
pub(crate) mod page_metadata;
pub(crate) mod path_policy;
pub(crate) mod resource_output_plan;
pub(crate) mod structural_url_renderer;
pub(crate) mod style_directives;
pub(crate) mod styles;
pub(crate) mod wasm;

#[cfg(test)]
pub(crate) mod tests;
