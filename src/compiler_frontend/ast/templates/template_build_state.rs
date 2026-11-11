//! Parser-local mutable build state for template construction.
//!
//! WHAT: `TemplateBuildState` is the mutable parser accumulator for template
//! head/body metadata — `kind`, `style`, and direct-child wrapper refs — while
//! a template is being parsed.
//!
//! WHY: `Template` is the durable AST value. The mutable parser accumulator is
//! shorter-lived: it exists only while syntax is being parsed, render units are
//! shaped, and parser-emitted TIR is finalized. Keeping mutable parse-time
//! fields on a dedicated build state means the durable `Template` is constructed
//! once, after authoritative TIR identity exists, instead of being mutated
//! throughout parsing.

use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationFacts, TemplateWrapperReference, refresh_kind_from_preparation,
};
use crate::compiler_frontend::source::SourceSpan;

/// Parser-local mutable state accumulated during template head and body parsing.
///
/// WHAT: carries `kind`, mutable `style`, and direct-child wrapper refs that
///       head and body parsing need to share without threading `&mut Template`.
/// WHY: the durable `Template` is constructed once after authoritative TIR
///      identity exists; this build state is the single mutable owner during
///      parsing and render-unit preparation.
pub(crate) struct TemplateBuildState {
    pub(crate) kind: TemplateType,
    pub(crate) style: Style,
    /// Authored `$before` wrapper value, awaiting composition with `$after`.
    pub(crate) child_wrapper_before: Option<TemplateWrapperReference>,
    /// Authored `$after` wrapper value, awaiting composition with `$before`.
    pub(crate) child_wrapper_after: Option<TemplateWrapperReference>,
    /// The single direct-child wrapper composed from `$before` and `$after`.
    ///
    /// At most one entry: both edges are one composite wrapper so downstream
    /// wrapper sets, folding and runtime handoff see the same nesting.
    pub(crate) child_wrappers: Vec<TemplateWrapperReference>,
    /// Authored `$note` token span when this template's kind is `Comment`.
    ///
    /// WHAT: retained only for diagnostics that must reject the semantic kind after parsing.
    /// WHY: the durable template span starts at the opener; the directive token is the authored
    ///      source location for wrong-context diagnostics.
    pub(crate) comment_directive_span: Option<SourceSpan>,
}

impl TemplateBuildState {
    /// Creates a fresh build state with default kind, style, and no wrappers.
    pub(crate) fn new() -> Self {
        Self {
            kind: TemplateType::StringFunction,
            style: Style::default(),
            child_wrapper_before: None,
            child_wrapper_after: None,
            child_wrappers: vec![],
            comment_directive_span: None,
        }
    }

    /// Applies generic String/StringFunction classification from complete
    /// preparation facts for the effective TIR view.
    pub(crate) fn refresh_kind_from_preparation(&mut self, facts: &TemplatePreparationFacts) {
        refresh_kind_from_preparation(&mut self.kind, facts);
    }
}
