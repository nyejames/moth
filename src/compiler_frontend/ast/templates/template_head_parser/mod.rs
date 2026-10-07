//! Template head parsing split into focused modules.
//!
//! WHAT:
//! - Exposes the stable template-head parsing entrypoints.
//! - Routes directive parsing and head expression handling to focused submodules.
//!
//! WHY:
//! - Keeps each piece of head parsing responsible for one concern.
//! - Makes parser control-flow and directive ownership boundaries easier to maintain.

// -------------------------
//  Submodules
// -------------------------

mod children_directive;
mod control_flow_suffix;
mod core_directives;
pub(crate) mod directive_args;
mod handler_directives;
mod head_expressions;
mod head_parser;

// -------------------------
//  Public API
// -------------------------

pub(crate) use core_directives::apply_doc_comment_defaults;
pub(crate) use head_parser::{ParsedTemplateHead, TemplateHeadParseRequest, parse_template_head};
