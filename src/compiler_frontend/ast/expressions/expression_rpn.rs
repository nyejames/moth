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
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
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
            | ExpressionRpnItem::PendingGroup { .. }
            | ExpressionRpnItem::PendingNumericBinding { .. }
            | ExpressionRpnItem::PendingNumericCall { .. }
            | ExpressionRpnItem::PendingNumericCast { .. }
            | ExpressionRpnItem::PendingNumericExpression { .. } => false,
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
    /// A reference to a body-owned pending numeric binding.
    ///
    /// WHAT: retains the binding ID, authored span and value mode between parsing and body
    ///       completion. The binding's single static type is selected from every permitted
    ///       use at the function root, so no `TypeId`, `Declaration` or default value exists yet.
    /// WHY: a numeric-dependent expression position must keep its authored RPN structure
    ///      without fabricating a placeholder type or a second untyped AST. Body completion
    ///      replaces this item with the canonical reference operand before publication; any
    ///      later stage treats its presence as a broken compiler invariant.
    PendingNumericBinding {
        binding: crate::compiler_frontend::ast::numeric_construction::NumericBindingId,
        span: Option<SourceSpan>,
        value_mode: ValueMode,
        /// Whether the author wrote `copy` in front of the reference.
        ///
        /// WHY: completion must build the exact authored shape (a value copy of a place versus a
        ///      plain reference) once the binding's type is selected.
        is_copy: bool,
    },
    /// A call-shaped expression whose numeric arguments must resolve first.
    ///
    /// WHAT: retains the parsed callee identity, argument routing and spans of one call whose
    ///       argument list contains construction-only numeric items.
    /// WHY: argument compatibility cannot run before numeric selection, and the parser must not
    ///      parse the argument list again. Body completion resumes the retained call in place and
    ///      replaces this item with one operand.
    PendingNumericCall {
        call: Box<crate::compiler_frontend::ast::numeric_construction::DeferredCall>,
        span: Option<SourceSpan>,
    },
    /// A cast whose source must resolve before its pair can be classified.
    ///
    /// WHAT: carries the retained source fragment in one operand position plus the already-known
    ///       concrete cast target and authored recovery ownership. The source is resolved
    ///       independently; the cast target never constrains or defaults it.
    /// WHY: `cast` is a prefix keyword whose pair classification needs a concrete source type.
    ///      Body completion materialises the source, resumes the existing cast constructor and
    ///      replaces this item with one operand.
    PendingNumericCast {
        target_type_id: TypeId,
        target: crate::compiler_frontend::builtins::casts::targets::BuiltinCastTarget,
        requires_optional_wrap_after_cast: bool,
        handling: CastHandling,
        source: Vec<ExpressionRpnItem>,
        span: Option<SourceSpan>,
    },
    /// A retained expression constructor whose statement bodies hold numeric continuations.
    ///
    /// WHAT: one operand position holding a value-producing block or handled fallible expression
    ///       that must rebuild through its owning module's constructor once the body root has
    ///       resolved the numeric origins.
    /// WHY: an expression-level constructor owns statement bodies, so it cannot build at its
    ///      receiving site while those bodies still hold continuations. Body completion replaces
    ///      this item with one completed operand, and any later stage treats its presence as a
    ///      broken compiler invariant.
    PendingNumericExpression {
        expression: Box<crate::compiler_frontend::ast::numeric_construction::DeferredExpression>,
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
            | ExpressionRpnItem::PendingGroup { span, .. }
            | ExpressionRpnItem::PendingNumericBinding { span, .. }
            | ExpressionRpnItem::PendingNumericCall { span, .. }
            | ExpressionRpnItem::PendingNumericCast { span, .. }
            | ExpressionRpnItem::PendingNumericExpression { span, .. } => *span,
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
                | ExpressionRpnItem::PendingNumericBinding { .. }
                | ExpressionRpnItem::PendingNumericCall { .. }
                | ExpressionRpnItem::PendingNumericCast { .. }
                | ExpressionRpnItem::PendingNumericExpression { .. }
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
