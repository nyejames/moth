//! Match-pattern types.
//!
//! WHAT: defines the data shapes produced by pattern parsing.
//! WHY: separating types from parsing logic keeps the public contract readable
//! and prevents circular imports between parser submodules.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::symbols::string_interning::StringId;

/// One arm of a match expression, pairing a pattern with an optional guard and body.
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub guard: Option<Expression>,
    pub body: Vec<AstNode>,
}

/// One payload field capture inside a choice-variant match pattern.
///
/// WHY: match arms can destructure payload variants by binding each field to a
/// local name. Captured names must exactly match the declared field
/// names in declaration order.
#[derive(Debug, Clone)]
pub struct ParsedChoicePayloadCapture {
    pub binding_name: StringId,
    pub field_index: usize,
    pub type_id: TypeId,
    /// Exact span of the declared payload field name in the pattern.
    pub span: Option<SourceSpan>,
    /// Exact span of the actual local binding (`as` alias or field name).
    pub binding_span: Option<SourceSpan>,
}

/// Resolved payload capture for a choice-variant match pattern.
///
/// Produced after path resolution; carries the fully-qualified `binding_path`
/// that later lowering stages use to register the arm-local binding.
#[derive(Debug, Clone)]
pub struct ChoicePayloadCapture {
    pub field_index: usize,
    pub type_id: TypeId,
    pub binding_path: PathId,
    /// Exact span of the declared payload field name in the pattern.
    pub span: Option<SourceSpan>,
    /// Exact span of the actual local binding (`as` alias or field name).
    pub binding_span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    Literal(Expression),

    /// Presence check for compiler-owned option values.
    ///
    /// WHAT: `none =>` matches only the absent branch of a `T?` scrutinee.
    /// WHY: option matching intentionally supports presence checks and capture
    /// patterns without introducing public `Option` constructors.
    OptionNone {
        span: Option<SourceSpan>,
    },

    /// Value comparison against the inner payload of a compiler-owned option.
    ///
    /// WHAT: `<literal> =>` on a `T?` scrutinee checks the option is present
    /// and then compares the contained `T` value.
    /// WHY: option matches use explicit option-aware pattern forms rather than
    /// exposing public `Option` constructors in source code.
    OptionValue {
        value: Expression,
        span: Option<SourceSpan>,
    },

    /// Present-value capture for compiler-owned option values.
    ///
    /// WHAT: `|name| =>` on a `T?` scrutinee matches any present value and binds
    /// the inner `T` payload to `name` for the guard and arm body.
    /// WHY: option unwrapping uses the same capture-local registration and
    /// guard-substitution model as choice payload captures.
    OptionPresentCapture {
        name: StringId,
        binding_path: PathId,
        inner_type_id: TypeId,
        span: Option<SourceSpan>,
        binding_span: Option<SourceSpan>,
    },

    Relational {
        op: RelationalPatternOp,
        value: Expression,
        span: Option<SourceSpan>,
    },

    ChoiceVariant {
        nominal_path: PathId,
        tag: usize,
        captures: Vec<ChoicePayloadCapture>,
        span: Option<SourceSpan>,
    },
}

impl MatchPattern {
    /// Return the exact authored span that identifies this pattern.
    pub fn span(&self) -> Option<SourceSpan> {
        match self {
            MatchPattern::Literal(expression) => expression.span,
            MatchPattern::OptionNone { span }
            | MatchPattern::OptionValue { span, .. }
            | MatchPattern::OptionPresentCapture { span, .. }
            | MatchPattern::Relational { span, .. }
            | MatchPattern::ChoiceVariant { span, .. } => *span,
        }
    }
}

pub struct ParsedChoicePattern {
    pub nominal_path: PathId,
    pub variant: StringId,
    pub tag: usize,
    pub captures: Vec<ParsedChoicePayloadCapture>,
    pub span: Option<SourceSpan>,
}

/// Relational operators allowed in match patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationalPatternOp {
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}
