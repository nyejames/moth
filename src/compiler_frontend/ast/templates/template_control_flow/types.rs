//! Data shapes for structured template control flow.
//!
//! These shared types are used by the parser, TIR, folding, validation and
//! runtime handoff paths. Control-flow structure itself is owned by TIR
//! `Conditional` and `Loop` nodes; these types carry only the selector, header,
//! parser inputs and validation modes that multiple stages genuinely share.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::source::SourceSpan;
/// Supported template branch selectors.
#[derive(Clone, Debug)]
pub(crate) enum TemplateBranchSelector {
    Bool(Expression),
    OptionPresentCapture {
        scrutinee: Expression,
        pattern: Box<MatchPattern>,
    },
}

impl TemplateBranchSelector {
    /// Returns the expression used to determine whether this selector matches.
    pub(crate) fn condition_expression(&self) -> &Expression {
        match self {
            Self::Bool(expression) => expression,
            Self::OptionPresentCapture { scrutinee, .. } => scrutinee,
        }
    }
}

/// Supported template loop headers.
#[derive(Clone, Debug)]
pub(crate) enum TemplateLoopHeader {
    Conditional {
        condition: Box<Expression>,
    },
    Range {
        bindings: Box<LoopBindings>,
        range: Box<RangeLoopSpec>,
    },
    Collection {
        bindings: Box<LoopBindings>,
        iterable: Box<Expression>,
    },
}

/// Structural output result from appending an owned runtime-template body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateBodyEmission {
    NoOutput,
    Output,
}

/// Body parser mode selected by the template head.
///
/// Template heads build this handoff, then body parsing consumes the selected
/// body and constructs the matching TIR control-flow node.
#[derive(Clone)]
pub(crate) enum TemplateBodyParseMode {
    Normal,
    If(Box<TemplateIfBodyParseInput>),
    Loop(Box<TemplateLoopBodyParseInput>),
}

/// Selects the post-parse validation rules for structured template control flow.
///
/// Runtime-capable templates may carry AST-prepared runtime slot application
/// plans, but unresolved helper artifacts that escape routing/composition are
/// invalid before HIR. Const-required templates use stricter foldability
/// validation instead, which lets compile-time helper templates keep slot
/// structure until a parent template composes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateControlFlowValidationMode {
    RuntimeCapable,
    ConstRequired,
}

/// Parsed `if` suffix state needed by the body parser.
#[derive(Clone)]
pub(crate) struct TemplateIfBodyParseInput {
    pub(crate) selector: TemplateBranchSelector,
    pub(crate) then_context: ScopeContext,
    pub(crate) span: Option<SourceSpan>,
}

/// Parsed `loop` suffix state needed by the body parser.
#[derive(Clone)]
pub(crate) struct TemplateLoopBodyParseInput {
    pub(crate) header: TemplateLoopHeader,
    pub(crate) body_context: ScopeContext,
    pub(crate) span: Option<SourceSpan>,
}
