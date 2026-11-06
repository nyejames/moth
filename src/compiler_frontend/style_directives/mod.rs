//! Frontend style-directive registry used by tokenization and template parsing.
//!
//! WHAT:
//! - `specs` defines the directive contract shared by core language directives and
//!   handler-based formatter directives.
//! - `compatibility` keeps template-head compatibility tags and policies out of parser
//!   branches.
//! - `builtins` owns frontend-provided directive data in stable diagnostic order.
//! - `registry` merges frontend and project-builder directives for tokenizer/AST lookup.
//!
//! WHY:
//! - The frontend must know the directive set before backend lowering.
//! - Project builders can register project-specific directives without changing parser code.
//! - A single merged registry avoids tokenizer/AST drift and keeps diagnostics consistent.
//!
//! Directive ownership policy:
//! - Frontend built-ins define language/template semantics and generic formatter directives
//!   such as `$md`.
//! - Project builders may only register additional project-owned directives such as the HTML
//!   project's `$html`, `$css`, `$escape_html` and `$code`.
//! - The frontend always executes directive handlers during parsing/folding, regardless of
//!   whether the directive itself is frontend-owned or project-owned.

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
