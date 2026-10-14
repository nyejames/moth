//! Transient expression syntax and resolved operation facts.
//!
//! RPN exists only while ordering, resolving and folding a producer. Completed operands name
//! phase-owned immutable rows; no RPN container survives into the completed AST.

use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::expressions::failure_facts::ImplicitFailureContributor;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericComparisonDomain, NumericOperator,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::value_mode::ValueMode;

#[derive(Clone, Debug)]
pub(crate) enum ExpressionRpnItem {
    /// A completed expression retained as a raw row ID in reused construction buffers.
    ///
    /// Evaluation issues an owned handle from the local builder before installing an edge.
    Operand(ExprId),
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
    /// Opens a deferred group in the flat fragment stream.
    ///
    /// WHAT: marks where a parenthesised group that could not complete at its close begins.
    ///       Its content follows in source order, and a matching `GroupClose` ends it. The
    ///       parser writes `anchor_span` when the group closes, so a fragment that starts with
    ///       a group reports the same authored anchor as its close.
    /// WHY: deep nesting must not cost one Rust frame or one owned `Vec` per level. Ordering
    ///      pushes an operator floor here, and every other reader treats the pair as transparent.
    ///      Markers live only in retained construction payloads and are gone once
    ///      `evaluate_expression` completes the fragment.
    GroupOpen { anchor_span: Option<SourceSpan> },
    /// Ends the group whose `GroupOpen` sits `open_distance` items earlier.
    ///
    /// WHAT: occupies the group's operand position. `anchor_span` is the group's first operand
    ///       span, which is the authored anchor for diagnostics about the deferred group.
    /// WHY: a relative distance survives moving the group range with `split_off`, so postfix
    ///      retention can take the whole group in O(group) without a side table.
    GroupClose {
        open_distance: u32,
        anchor_span: Option<SourceSpan>,
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
    /// Transient operator spelling awaiting result-type resolution.
    Operator {
        operator: Operator,
        span: Option<SourceSpan>,
    },
    /// Completed operator facts selected once by AST result-type resolution.
    ResolvedOperator(ResolvedOperation),
}

/// Canonical result and operation facts shared by folding and executable lowering.
///
/// Operand types describe authored edges. Conversion targets describe only inserted operation
/// boundaries, so a decimal power never mistakes its I32 exponent for a decimal operand.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedOperation {
    pub operator: Operator,
    pub span: Option<SourceSpan>,
    pub result_type: TypeId,
    pub left_type: TypeId,
    pub right_type: Option<TypeId>,
    pub kind: ResolvedOperationKind,
    pub(crate) failure: Option<ImplicitFailureContributor>,
}

/// Arithmetic and comparison facts are separate because exact integer comparisons need no
/// common arithmetic domain.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ResolvedOperationKind {
    Numeric(ResolvedNumericOperation),
    Comparison(ResolvedComparisonOperation),
    Range { domain: NumericScalar },
    NonNumeric,
}

/// Computation boundaries and semantic discharge selected by the canonical numeric policy.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedNumericOperation {
    pub operator: NumericOperator,
    pub left_domain: NumericScalar,
    pub right_domain: Option<NumericScalar>,
    pub domain: NumericScalar,
    pub left_conversion: Option<NumericScalar>,
    pub right_conversion: Option<NumericScalar>,
    pub discharged: bool,
}

/// Exact source domains survive comparisons whose Bool result carries no numeric information.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedComparisonOperation {
    pub left_domain: NumericScalar,
    pub right_domain: NumericScalar,
    pub domain: NumericComparisonDomain,
    pub left_conversion: Option<NumericScalar>,
    pub right_conversion: Option<NumericScalar>,
}
/// Strips enclosing group pairs and returns the one operand item they wrap.
///
/// WHY: `(x)` must classify like `x` wherever a consumer asks for a single operand. Every
///      single-item match over a fragment goes through here so groups stay transparent.
pub(crate) fn single_operand(items: &[ExpressionRpnItem]) -> Option<&ExpressionRpnItem> {
    match &items[enclosed_range(items)] {
        [item] => Some(item),
        _ => None,
    }
}

/// Mutable form of [`single_operand`] for completion sites that update the wrapped item.
pub(crate) fn single_operand_mut(
    items: &mut [ExpressionRpnItem],
) -> Option<&mut ExpressionRpnItem> {
    let range = enclosed_range(items);
    match &mut items[range] {
        [item] => Some(item),
        _ => None,
    }
}

/// Owned form of [`single_operand`] for sites that consume the fragment.
pub(crate) fn into_single_operand(mut items: Vec<ExpressionRpnItem>) -> Option<ExpressionRpnItem> {
    let range = enclosed_range(&items);
    (range.len() == 1).then(|| items.swap_remove(range.start))
}

/// Index range left after peeling every group pair that encloses the whole slice.
fn enclosed_range(items: &[ExpressionRpnItem]) -> std::ops::Range<usize> {
    let (mut start, mut end) = (0, items.len());
    while end - start >= 2
        && matches!(items[start], ExpressionRpnItem::GroupOpen { .. })
        && let Some(ExpressionRpnItem::GroupClose { open_distance, .. }) = items[start..end].last()
        && *open_distance as usize == end - start - 1
    {
        start += 1;
        end -= 1;
    }
    start..end
}

/// Index where the last operand of a fragment begins.
///
/// A trailing `GroupClose` spans back to its `GroupOpen`. Any other trailing item is its own
/// operand. Returns `None` for an empty fragment.
pub(crate) fn trailing_operand_start(
    items: &[ExpressionRpnItem],
) -> Result<Option<usize>, CompilerError> {
    let Some(last) = items.len().checked_sub(1) else {
        return Ok(None);
    };
    let start = match &items[last] {
        ExpressionRpnItem::GroupClose { open_distance, .. } => usize::try_from(*open_distance)
            .ok()
            .and_then(|distance| last.checked_sub(distance))
            .filter(|open| matches!(items[*open], ExpressionRpnItem::GroupOpen { .. }))
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "grouped expression close does not match its open marker",
                )
            })?,
        _ => last,
    };
    Ok(Some(start))
}

impl ExpressionRpnItem {
    /// Exact authored span, binding completed operands to their issuing owner.
    pub(crate) fn source_span(
        &self,
        expressions: &AstExpressionBuilder,
    ) -> Result<Option<SourceSpan>, CompilerError> {
        Ok(match self {
            ExpressionRpnItem::Operand(expression) => expressions.view(*expression)?.span(),
            ExpressionRpnItem::PendingNumericLiteral { span, .. }
            | ExpressionRpnItem::GroupOpen {
                anchor_span: span, ..
            }
            | ExpressionRpnItem::GroupClose {
                anchor_span: span, ..
            }
            | ExpressionRpnItem::PendingNumericBinding { span, .. }
            | ExpressionRpnItem::PendingNumericCall { span, .. }
            | ExpressionRpnItem::PendingNumericCast { span, .. }
            | ExpressionRpnItem::PendingNumericExpression { span, .. } => *span,
            ExpressionRpnItem::Operator { span, .. } => *span,
            ExpressionRpnItem::ResolvedOperator(operation) => operation.span,
        })
    }

    /// Authored span that needs no completed-expression owner.
    ///
    /// WHAT: returns the span a pending or operator item carries itself; a completed
    ///       operand has no authored span without its issuing owner, so it yields `None`.
    /// WHY: deferral fallbacks in statement positions have no builder in scope; only the
    ///      still-open leaves need a span for diagnostics.
    pub(crate) fn authored_span(&self) -> Option<SourceSpan> {
        match self {
            ExpressionRpnItem::Operand(_) => None,
            ExpressionRpnItem::PendingNumericLiteral { span, .. }
            | ExpressionRpnItem::GroupOpen {
                anchor_span: span, ..
            }
            | ExpressionRpnItem::GroupClose {
                anchor_span: span, ..
            }
            | ExpressionRpnItem::PendingNumericBinding { span, .. }
            | ExpressionRpnItem::PendingNumericCall { span, .. }
            | ExpressionRpnItem::PendingNumericCast { span, .. }
            | ExpressionRpnItem::PendingNumericExpression { span, .. } => *span,
            ExpressionRpnItem::Operator { span, .. } => *span,
            ExpressionRpnItem::ResolvedOperator(operation) => operation.span,
        }
    }

    /// True for resolved operands and deferred syntax occupying one operand position.
    ///
    /// A deferred group occupies its operand position through its `GroupClose`. `GroupOpen` is
    /// neither operand nor operator. Parse-time adjacency and named-entry checks need shape,
    /// not a completed value.
    pub fn is_operand_shape(&self) -> bool {
        matches!(
            self,
            ExpressionRpnItem::Operand(_)
                | ExpressionRpnItem::PendingNumericLiteral { .. }
                | ExpressionRpnItem::GroupClose { .. }
                | ExpressionRpnItem::PendingNumericBinding { .. }
                | ExpressionRpnItem::PendingNumericCall { .. }
                | ExpressionRpnItem::PendingNumericCast { .. }
                | ExpressionRpnItem::PendingNumericExpression { .. }
        )
    }
}
