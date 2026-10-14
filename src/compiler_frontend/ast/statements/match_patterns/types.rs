//! Match-pattern types.
//!
//! WHAT: defines the data shapes produced by pattern parsing.
//! WHY: separating types from parsing logic keeps the public contract readable
//! and prevents circular imports between parser submodules.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::DeclaredChoiceSubject;
use crate::compiler_frontend::ast::numeric_known_shape::KnownShapeId;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::symbols::string_interning::StringId;

/// One arm of a match expression, pairing a pattern with an optional guard and body.
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub guard: Option<ExprId>,
    pub body: Vec<AstNode>,
}

/// The match subject a pattern is resolved against.
///
/// WHAT: the subject's exact type when it is already selected, a pending numeric origin whose
///       type only the body root selects, a pending generic choice whose declared owner names
///       its variants while the instance stays unselected, or a pending optional whose inner
///       shape is known while the optional instance stays unselected.
/// WHY: pattern values materialise at the selected subject type, so a pending subject keeps the
///      authored fragment instead of fabricating a typed pattern early; a pending choice still
///      fixes variant identity at parse time from its declared owner, and a pending option fixes
///      its payload shape from the retained inner shape.
#[derive(Debug, Clone, Copy)]
pub(crate) enum MatchSubject {
    /// The subject's exact type is already selected.
    Known(TypeId),
    /// The subject is a pending numeric origin whose exact type is not selected yet.
    Pending,
    /// The subject is a retained generic choice: its declared owner fixes the variant names.
    PendingChoice(DeclaredChoiceSubject),
    /// The subject is a retained optional: its inner shape is known while the optional instance
    /// stays unselected, so a payload capture or value pattern can name its payload shape.
    PendingOption { inner: KnownShapeId },
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

/// A pattern value keeps constant content separate from occurrence metadata.
/// Runtime values remain executable expressions when completion does not fold them.
#[derive(Debug, Clone, Copy)]
pub enum PatternValueKind {
    Constant(ConstValueId),
    Runtime(ExprId),
}

#[derive(Debug, Clone, Copy)]
pub struct PatternValue {
    pub kind: PatternValueKind,
    pub type_id: TypeId,
    pub span: Option<SourceSpan>,
}

impl PatternValue {
    pub(crate) fn from_expression(value: ExpressionView<'_>) -> Result<Self, CompilerError> {
        Ok(Self {
            kind: match value.operation() {
                ExpressionOperationView::Constant(id) => PatternValueKind::Constant(id),
                _ => PatternValueKind::Runtime(value.id()),
            },
            type_id: value.value_type_id()?,
            span: value.span(),
        })
    }
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    Literal(PatternValue),

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
        value: PatternValue,
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
        value: PatternValue,
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

    pub(crate) fn visit_expression_ids(&self, visit: &mut dyn FnMut(ExprId)) {
        let value = match self {
            Self::Literal(value)
            | Self::OptionValue { value, .. }
            | Self::Relational { value, .. } => value,
            _ => return,
        };
        if let PatternValueKind::Runtime(id) = value.kind {
            visit(id);
        }
    }

    pub(crate) fn visit_expression_ids_mut(&mut self, visit: &mut dyn FnMut(&mut ExprId)) {
        let value = match self {
            Self::Literal(value)
            | Self::OptionValue { value, .. }
            | Self::Relational { value, .. } => value,
            _ => return,
        };
        if let PatternValueKind::Runtime(id) = &mut value.kind {
            visit(id);
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
