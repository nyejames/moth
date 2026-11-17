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
mod source;
mod specs;

pub(crate) use crate::compiler_frontend::compiler_messages::TemplateHeadConflictReason;
pub use compatibility::{TemplateHeadCompatibility, TemplateHeadTag};
pub use registry::StyleDirectiveRegistry;
pub(crate) use source::{
    SourceDirectiveAnnotation, SourceOperandFoldResources, capture_source_directive_operand,
    constraint_directive_spans, fold_source_directive_operand,
    parse_statement_source_directive_run, source_constraints_from_annotations,
};
pub(crate) use specs::DirectiveOccurrence;
pub(crate) use specs::{
    AFTER_DIRECTIVE_NAME, BEFORE_DIRECTIVE_NAME, ConstraintDirectiveSpans, INSERT_DIRECTIVE_NAME,
    SLOT_DIRECTIVE_NAME, SourceConstraintSet, SourceDirectiveKind,
    StatementSourceConstraintAttachment,
};
pub use specs::{
    CoreStyleDirectiveKind, SourceModifierOperand, StyleDirectiveArgumentType,
    StyleDirectiveArgumentValue, StyleDirectiveEffects, StyleDirectiveHandlerSpec,
    StyleDirectiveKind, StyleDirectiveSpec,
};

#[cfg(test)]
#[path = "tests/style_directives_tests.rs"]
mod style_directives_tests;
