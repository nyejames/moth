//! HTML project backend components.
//!
//! WHAT: groups HTML document generation, routing/output planning, and backend-specific helpers.
//! WHY: HTML builds stitch together several focused subsystems around the shared frontend/HIR
//! pipeline.

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
