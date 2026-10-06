//! Expression-owned runtime RPN and place representations.
//!
//! WHAT: defines the narrow contracts that replace broad `AstNode` fragments inside
//! expression payloads. `ExpressionRpn` carries only `Expression` operands and
//! located operators; `PlaceExpression` carries only local or field places.
//! WHY: keeping runtime expression representation frontend-owned prevents statement
//! nodes from leaking into value contexts and gives constant folding, HIR lowering,
//! and template substitution one shared narrow language.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::{ExpressionKind, Operator};
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::value_mode::ValueMode;

/// Reverse-Polish-Notation payload for a runtime expression.
///
/// WHAT: ordered list of operands and operators that survive AST constant folding.
/// WHY: the operand type is `Expression`, not `AstNode`, so runtime RPN cannot
/// smuggle statement bodies or broad parser fragments into expression values.
#[derive(Clone, Debug)]
pub struct ExpressionRpn {
    pub items: Vec<ExpressionRpnItem>,
}

impl ExpressionRpn {
    /// Returns an empty expression-owned RPN stack for coercion fixtures.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self { items: Vec::new() }
    }

    /// Returns true if the RPN contains at least one regular division operator.
    pub fn contains_regular_division(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item,
                ExpressionRpnItem::Operator {
                    operator: Operator::Divide,
                    ..
                }
            )
        })
    }

    /// Validate that RPN contains only ordinary operands or completed receiving-site recovery.
    ///
    /// A compound update evaluates its already-handled RHS before the separate arithmetic and
    /// write-back checks. That completed catch is a value operand, not a new receiving boundary.
    pub fn validate_no_statement_bodies(&self) -> bool {
        self.items.iter().all(|item| match item {
            ExpressionRpnItem::Operand(expression) => match &expression.kind {
                ExpressionKind::ValueBlock { block } => matches!(
                    block.as_ref(),
                    ValueBlock::Catch(catch)
                        if matches!(
                            catch.handled_value.failure_facts.disposition,
                            FailureDisposition::HandledByCatch { .. }
                        )
                ),
                _ => true,
            },
            // Pending syntax never survives evaluation, so it fails this post-evaluation
            // validation like any other non-lowered shape; the fallible HIR lowering arm
            // reports the broken invariant with its span.
            ExpressionRpnItem::PendingNumericLiteral { .. }
            | ExpressionRpnItem::PendingGroup { .. } => false,
            ExpressionRpnItem::Operator { .. } => true,
        })
    }
}

/// One item in a runtime RPN expression.
// The `Operand` variant intentionally owns a full `Expression` (not a `Box<Expression>`)
// so that RPN items stay self-contained for constant folding and HIR lowering. Cloning
// an RPN item therefore copies the operand expression; this is acceptable for frontend-sized
// expression payloads and keeps the API surface predictable.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum ExpressionRpnItem {
    /// An expression operand whose value is only known at runtime.
    Operand(Expression),
    /// A numeric literal awaiting destination-aware materialisation.
    ///
    /// WHAT: retains the literal token (with parser-owned `-` already folded into its sign),
    ///       its span and its value mode between parsing and `evaluate_expression`.
    /// WHY: numeric receivers and local peers must select the domain before any bounded
    ///      integer or binary-float intermediate is produced. `evaluate_expression`
    ///      resolves every pending item; later stages never see one.
    PendingNumericLiteral {
        token: NumericLiteralToken,
        span: Option<SourceSpan>,
        value_mode: ValueMode,
    },
    /// A group's infix fragment awaiting ordering with the surrounding literal context.
    ///
    /// The stored span is its first non-operator anchor, not the opening parenthesis.
    /// Ordering moves its items into one RPN stream without copying child fragments.
    PendingGroup {
        nodes: Vec<ExpressionRpnItem>,
        span: Option<SourceSpan>,
    },
    /// A symbolic or keyword operator with its exact source span preserved for diagnostics.
    Operator {
        operator: Operator,
        span: Option<SourceSpan>,
    },
}

impl ExpressionRpnItem {
    /// Exact authored span of this RPN item, if the owning file has an identity.
    pub fn source_span(&self) -> Option<SourceSpan> {
        match self {
            ExpressionRpnItem::Operand(expression) => expression.span,
            ExpressionRpnItem::PendingNumericLiteral { span, .. }
            | ExpressionRpnItem::PendingGroup { span, .. } => *span,
            ExpressionRpnItem::Operator { span, .. } => *span,
        }
    }

    /// True for resolved operands and deferred syntax occupying one operand position.
    ///
    /// Parse-time adjacency and named-entry checks need shape, not a completed value.
    pub fn is_operand_shape(&self) -> bool {
        matches!(
            self,
            ExpressionRpnItem::Operand(_)
                | ExpressionRpnItem::PendingNumericLiteral { .. }
                | ExpressionRpnItem::PendingGroup { .. }
        )
    }
}

/// Frontend place expression.
///
/// WHAT: identifies a readable or writable storage location: a local variable or a field
/// projection from another place.
/// WHY: copy expressions and assignment targets need a narrow place representation that
/// cannot carry statement bodies or arbitrary expression-shaped AST nodes.
#[derive(Clone, Debug)]
pub struct PlaceExpression {
    pub kind: PlaceExpressionKind,
    pub type_id: TypeId,
    pub diagnostic_type: DataType,
    pub value_mode: ValueMode,
    pub span: Option<SourceSpan>,
}

#[derive(Clone, Debug)]
pub enum PlaceExpressionKind {
    /// A local variable by its interned path.
    Local(PathId),
    /// A field projection from another place.
    Field {
        base: Box<PlaceExpression>,
        field: StringId,
    },
}
