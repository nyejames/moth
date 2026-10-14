//! Data shapes for structured template control flow.
//!
//! These shared types are used by the parser, TIR, folding, validation and
//! runtime handoff paths. Control-flow structure itself is owned by TIR
//! `Conditional` and `Loop` nodes; these types carry only the selector, header,
//! parser inputs and validation modes that multiple stages genuinely share.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression_store::ExprId;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, DeferredNumericExpression, DeferredRangeHeader, NumericBindingId,
    NumericConstructionDomainId,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::source::SourceSpan;
/// Supported template branch selectors.
#[derive(Clone, Debug)]
pub(crate) enum TemplateBranchSelector {
    Bool(ExprId),
    OptionPresentCapture {
        scrutinee: ExprId,
        pattern: Box<MatchPattern>,
    },
}

impl TemplateBranchSelector {
    /// Returns the expression used to determine whether this selector matches.
    pub(crate) fn condition_expression(&self) -> ExprId {
        match self {
            Self::Bool(expression) => *expression,
            Self::OptionPresentCapture { scrutinee, .. } => *scrutinee,
        }
    }
}

/// Supported template loop headers.
#[derive(Clone, Debug)]
pub(crate) enum TemplateLoopHeader {
    Conditional {
        condition: ExprId,
    },
    Range {
        bindings: Box<LoopBindings>,
        range: Box<RangeLoopSpec>,
    },
    Collection {
        bindings: Box<LoopBindings>,
        iterable: ExprId,
    },
}

/// Loop header handoff from the suffix parser to body construction.
///
/// WHAT: a concrete header, or a boolean condition whose numeric fragment is still retained.
/// WHY: a template `loop` suffix parses its body before the loop node exists, and a condition
///      that still references a pending numeric origin has no selected type yet, so the loop node
///      cannot be built at parse time. The retained case carries the fragment and the owning
///      numeric body domain, and body construction records a payload-free position that
///      body-root completion installs the completed `Loop` node into.
#[derive(Clone, Debug)]
pub(crate) enum TemplateLoopHeaderDraft {
    /// Condition built, range, or collection header.
    Concrete(TemplateLoopHeader),

    /// Boolean condition retained until body-root numeric completion selects its origin's type.
    RetainedCondition {
        /// Numeric body domain the fragment was raised in; its body root owns completion.
        domain: NumericConstructionDomainId,
        /// Retained fragment and its receiving metadata.
        deferred: Box<DeferredNumericExpression>,
    },

    /// Range header whose operands complete at the owning body root.
    ///
    /// WHY: the header's exact domain is a forward result of its operands' selected types, so a
    ///      header with a retained operand keeps its authored form until the owning body root owns
    ///      every permitted use. The producer's own deferred shapes travel verbatim:
    ///      `Unbound` completes its own operand slots through template intake records, while
    ///      `Bound` belongs to a retained counter row that resolves before the loop construct, so
    ///      the counter's finished spec and declaration install the header instead.
    RetainedRange {
        /// Numeric body domain whose root owns completion.
        domain: NumericConstructionDomainId,
        /// Producer-owned deferred range header (retained counter, or the authored operand slots).
        header: Box<DeferredRangeHeader>,
    },

    /// Collection header whose iterable still reaches a pending numeric origin.
    ///
    /// WHY: the callee's returned collection type is known, so the loop bindings are final at parse
    ///      time, but the iterable expression itself cannot be built until the body root selects its
    ///      origin's single static type. The bindings travel with the position instead of being
    ///      re-derived from the fragment later.
    ///      A pending numeric collection item is declared by the body root from the source's
    ///      completed element type, before this header completes.
    RetainedCollection {
        /// Numeric body domain whose root owns completion.
        domain: NumericConstructionDomainId,
        /// Concrete loop bindings from the callee's known return metadata.
        bindings: Box<LoopBindings>,
        /// Pending numeric item; the body root declares it before this header completes.
        pending_item: Option<NumericBindingId>,
        /// Retained iterable fragment and its receiving metadata.
        deferred: Box<DeferredNumericExpression>,
    },
}

/// Branch selector handoff from the suffic parser to body construction.
///
/// WHAT: a concrete selector, or a selector whose retaining expression is a retained numeric
///       fragment.
/// WHY: a branch's selector type is selected at the body root, so a template `if` suffix whose
///      condition or option subject still references a pending numeric origin cannot build the
///      conditional node at parse time even though the body must still parse under its binding
///      scope.
// `Concrete` is the common conditional path, moved once into `TemplateIfBodyParseInput`, which is
// already boxed in `TemplateBodyParseMode::If`. Boxing `Concrete` would add a second heap
// allocation to every built conditional head; the retained variants already box their payloads.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum TemplateBranchSelectorDraft {
    /// Condition or subject built, pattern (if any) final.
    Concrete(TemplateBranchSelector),

    /// Boolean condition retained until body-root numeric completion selects its origin's type.
    RetainedBool {
        /// Numeric body domain the fragment was raised in; its body root owns completion.
        domain: NumericConstructionDomainId,
        /// Retained fragment and its receiving metadata.
        deferred: Box<DeferredNumericExpression>,
    },

    /// Option-present subject retained until body-root numeric completion; the deferred capture
    /// pattern travels with the position and finishes against the completed subject there.
    RetainedOptionPresent {
        /// Numeric body domain the fragment was raised in; its body root owns completion.
        domain: NumericConstructionDomainId,
        /// Retained fragment and its receiving metadata.
        deferred: Box<DeferredNumericExpression>,
        /// Deferred option-present capture pattern; the body root finishes it against the
        /// completed subject.
        pattern: Box<DeferredArmPattern>,
    },
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
    pub(crate) selector: TemplateBranchSelectorDraft,
    pub(crate) then_context: ScopeContext,
    pub(crate) span: Option<SourceSpan>,
}

/// Parsed `loop` suffix state needed by the body parser.
#[derive(Clone)]
pub(crate) struct TemplateLoopBodyParseInput {
    pub(crate) header: TemplateLoopHeaderDraft,
    pub(crate) body_context: ScopeContext,
    pub(crate) span: Option<SourceSpan>,
}
