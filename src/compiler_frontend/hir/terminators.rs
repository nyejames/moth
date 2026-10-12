//! HIR block terminators.
//!
//! WHAT: explicit control-flow exits for each block.
//! WHY: control flow must be structured enough for borrow validation and backend lowering.

use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirVariantCarrier, ValueKind};
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::patterns::HirMatchArm;

#[derive(Debug, Clone)]
pub enum HirTerminator {
    Jump {
        target: BlockId,
        args: Vec<HirJumpArgument>,
    },

    If {
        condition: HirValueId,
        then_block: BlockId,
        else_block: BlockId, // Required, must jump or return somewhere (Could just be continuation)
    },

    /// Branch on an internal fallible carrier's success/error state.
    ///
    /// WHAT: routes the Ok/success path to `success_block` and the Err/error path to
    /// `error_block`.
    /// WHY: fallible control flow is part of the HIR CFG contract. Keeping the branch as a
    /// terminator avoids hiding an error edge inside an ordinary boolean expression.
    FallibleBranch {
        result: HirValueId,
        success_block: BlockId,
        error_block: BlockId,
    },

    Match {
        scrutinee: HirValueId,
        arms: Vec<HirMatchArm>, // Each arm's body block must end with Jump or Return
    },

    Break {
        target: BlockId,
    },

    Continue {
        target: BlockId,
    },

    Return(HirValueId),

    /// Return through the function's fallible success slot.
    ///
    /// WHAT: represents `return value` from a fallible function without constructing a runtime
    /// fallible carrier in HIR.
    /// WHY: explicit success/error terminators keep the HIR control-flow contract aligned with
    /// Moth's fallible signature model.
    ReturnSuccess(HirValueId),

    /// Return through the function's fallible error slot.
    ///
    /// WHAT: represents `return! value` without constructing a runtime fallible carrier in HIR.
    /// WHY: Phase 8 moves fallible control flow toward explicit success/error edges so borrow
    /// validation and backend lowering do not need to infer error paths from variant values.
    ReturnError(HirValueId),

    /// Internal placeholder for blocks that have not yet received a real terminator.
    ///
    /// WHAT: marks a block as incomplete during HIR construction.
    /// WHY: the old panic terminator was previously overloaded as both a placeholder and a real
    ///      runtime stop. This dedicated variant removes that ambiguity.
    /// MUST NOT survive to validated HIR or backend lowering.
    Uninitialized,

    /// Compiler-generated unrecoverable runtime failure.
    ///
    /// WHAT: keeps internal runtime safety stops distinct from source-authored assertions.
    /// WHY: range-loop runtime guards and exhaustive-match fallbacks are compiler lowering
    ///      machinery, not the public `assert` statement surface.
    RuntimeFailure {
        message: String,
        cause: Option<RuntimeFailureCause>,
    },

    /// Assertion failure — unrecoverable runtime stop.
    ///
    /// WHAT: represents a failed `assert` statement.
    /// WHY: this is the only source-level unrecoverable stop in Alpha Moth.
    /// The message is an optional String value; its evaluation fact tells target validation
    /// whether construction is the default, fully folded, or runtime work.
    AssertFailure {
        message: HirValueId,
        message_evaluation: HirAssertionMessageEvaluation,
    },
}

/// One local value transferred across a CFG edge into a newly defined destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirJumpArgument {
    pub source: LocalId,
    pub destination: LocalId,
}

/// Semantic origin retained until private failure lanes have been installed.
///
/// WHY: implicit casts and compound write-backs preserve their failure carrier independently of
/// the backend-facing runtime failure message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFailureCause {
    StoreConversion { carrier: LocalId },
    AuthoredCastConversion { carrier: LocalId },
}

/// Evaluation fact retained with an assertion failure message.
///
/// WHAT: distinguishes the default option, a fully folded present option, and runtime message
///       construction after HIR has lowered the expression.
/// WHY: target validation must consume one authoritative fact rather than reclassifying source or
///      backend expression shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirAssertionMessageEvaluation {
    Default,
    Folded,
    Runtime,
}

/// Classifies the lowered assertion message shape used by validation and target gating.
///
/// WHAT: derives the evaluation fact from the canonical lowered `Option<String>` shape.
/// WHY: lowering, HIR validation, reachability and backend feature checks must share one
///      classifier rather than independently interpreting variant or constant details.
pub(crate) fn classify_assertion_message_evaluation(
    expressions: &HirExpressionStore,
    message: HirValueId,
) -> HirAssertionMessageEvaluation {
    match &expressions.expression(message).kind {
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 0,
            fields,
        } if expressions.variant_fields(*fields).is_empty() => {
            HirAssertionMessageEvaluation::Default
        }
        HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 1,
            fields,
        } if expressions
            .variant_fields(*fields)
            .iter()
            .all(|field| expressions.expression(field.value).value_kind == ValueKind::Const) =>
        {
            HirAssertionMessageEvaluation::Folded
        }
        _ => HirAssertionMessageEvaluation::Runtime,
    }
}
