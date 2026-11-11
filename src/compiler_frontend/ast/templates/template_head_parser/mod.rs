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

mod child_decorators;
mod control_flow_suffix;
pub(crate) mod core_directives;
pub(crate) mod directive_args;
mod handler_directives;
mod head_expressions;
mod head_parser;

// -------------------------
//  Public API
// -------------------------

pub(crate) use child_decorators::finish_child_decorators;
pub(crate) use head_expressions::validate_template_head_value_type;
pub(crate) use head_parser::{ParsedTemplateHead, TemplateHeadParseRequest, parse_template_head};
