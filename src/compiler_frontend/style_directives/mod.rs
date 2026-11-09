//! Frontend style-directive registry used by tokenization and template parsing.
//!
//! WHAT:
//! - `specs` defines the directive contract shared by core language directives and
//!   handler-based formatter directives.
//! - `compatibility` keeps template-head compatibility tags and policies out of parser
//!   branches.
//! - `builtins` owns frontend-provided directive data in stable diagnostic order.
//! - `registry` exposes the frontend-owned directive set for tokenizer/AST lookup.
//!
//! WHY:
//! - The frontend must know the directive set before backend lowering.
//! - A single frontend-owned registry avoids tokenizer/AST drift and keeps diagnostics
//!   consistent.
//!
//! Directive ownership policy:
//! - The frontend owns every template directive, including core template semantics and the
//!   `$md`, `$html`, `$css`, `$escape_html` and `$code` formatter directives.
//! - Every builder sees the same directive set.

mod builtins;
mod compatibility;
mod registry;
mod specs;

pub(crate) use crate::compiler_frontend::compiler_messages::TemplateHeadConflictReason;
pub use compatibility::{TemplateHeadCompatibility, TemplateHeadTag};
pub use registry::StyleDirectiveRegistry;
pub(crate) use specs::{
    AFTER_DIRECTIVE_NAME, BEFORE_DIRECTIVE_NAME, DOC_DIRECTIVE_NAME, INSERT_DIRECTIVE_NAME,
    SLOT_DIRECTIVE_NAME,
};
pub use specs::{
    CoreStyleDirectiveKind, StyleDirectiveArgumentType, StyleDirectiveArgumentValue,
    StyleDirectiveEffects, StyleDirectiveHandlerSpec, StyleDirectiveKind, StyleDirectiveSpec,
};

#[cfg(test)]
#[path = "tests/style_directives_tests.rs"]
mod style_directives_tests;
