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
    pub(crate) child_wrappers: Vec<TemplateWrapperReference>,
}

impl TemplateBuildState {
    /// Creates a fresh build state with default kind, style, and no wrappers.
    pub(crate) fn new() -> Self {
        Self {
            kind: TemplateType::StringFunction,
            style: Style::default(),
            child_wrappers: vec![],
        }
    }

    /// Applies generic String/StringFunction classification from complete
    /// preparation facts for the effective TIR view.
    pub(crate) fn refresh_kind_from_preparation(&mut self, facts: &TemplatePreparationFacts) {
        refresh_kind_from_preparation(&mut self.kind, facts);
    }
}
