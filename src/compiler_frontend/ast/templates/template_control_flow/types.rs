//! Data shapes for structured template control flow.
//!
//! These shared types are used by the parser, TIR, folding, validation and
//! runtime handoff paths. Control-flow structure itself is owned by TIR
//! `BranchChain` and `Loop` nodes; these types carry only the selector, header,
//! loop-control kind, parser inputs and validation modes that multiple stages
//! genuinely share.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
/// Supported template branch selectors.
#[derive(Clone, Debug)]
pub(crate) enum TemplateBranchSelector {
    Bool(Expression),
    OptionPresentCapture {
        scrutinee: Expression,
        pattern: Box<MatchPattern>,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateLoopControlKind {
    Break,
    Continue,
}

/// Output/control result from a selected template body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateBodyEmission {
    NoOutput,
    Output,
    Break,
    Continue,
}

/// Authored `[else]` marker provenance for a branch-chain fallback.
///
/// WHAT: records the exact source location and span of the `[else]` sentinel
///       that introduced a branch chain's fallback body.
/// WHY: the marker is authored metadata independent of the fallback body and of
///      the enclosing `if` opening. Substituting either would misattribute
///      diagnostics, so the marker travels beside them through TIR, copies,
///      and the runtime handoff. `None` on chains without a fallback body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateElseMarker {
    /// Legacy source location of the `[else]` token.
    pub(crate) location: SourceLocation,

    /// Exact authored byte range of the `[else]` token in its source.
    pub(crate) span: Option<SourceSpan>,
}

/// Body parser mode selected by the template head.
///
/// Template heads build this handoff, then body parsing consumes the non-normal
/// modes to split branch/body content and construct the TIR `BranchChain` or
/// `Loop` node.
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
    pub(crate) else_context: ScopeContext,
    pub(crate) location: SourceLocation,
    pub(crate) span: Option<SourceSpan>,
}

/// Parsed `loop` suffix state needed by the body parser.
#[derive(Clone)]
pub(crate) struct TemplateLoopBodyParseInput {
    pub(crate) header: TemplateLoopHeader,
    pub(crate) body_context: ScopeContext,
    pub(crate) location: SourceLocation,
    pub(crate) span: Option<SourceSpan>,
}
