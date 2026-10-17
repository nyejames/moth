//! HIR block terminators.
//!
//! WHAT: explicit control-flow exits for each block.
//! WHY: control flow must be structured enough for borrow validation and backend lowering.

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirValueRange};
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapOp, HirVariantCarrier, ValueKind,
};
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, RangeStepFailureCause,
};
use crate::compiler_frontend::hir::patterns::HirMatchArm;
use crate::compiler_frontend::source::SourceSpan;

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

    /// Evaluate a fallible operation and define only the selected edge's result locals.
    /// Both destination sets are killed before selecting an edge, preventing stale loop values.
    Invoke {
        id: HirNodeId,
        span: Option<SourceSpan>,
        operation: HirFallibleOperation,
        success: BlockId,
        success_results: Vec<LocalId>,
        failure: BlockId,
        error_result: LocalId,
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

    /// Return the function's ordered success slots, including an empty success result.
    Return(HirValueRange),

    /// Return one value through the function's separate error channel.
    ReturnError(HirValueId),

    /// Internal placeholder for blocks that have not yet received a real terminator.
    ///
    /// WHAT: marks a block as incomplete during HIR construction.
    /// WHY: a dedicated variant keeps construction placeholders distinct from real runtime stops.
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

/// Fallible producers retain their exact operation facts without a runtime carrier value.
#[derive(Debug, Clone)]
pub enum HirFallibleOperation {
    Call {
        target: CallTarget,
        args: HirValueRange,
    },
    CastOp {
        policy: BuiltinCastPolicyId,
        source: HirValueId,
    },
    MapOp {
        op: HirMapOp,
        receiver: HirValueId,
        args: HirValueRange,
    },
    NumericOp {
        op: HirNumericOp,
        operands: HirNumericOperands,
    },
    RangeStepFailure {
        cause: RangeStepFailureCause,
    },
    FormatFloat {
        source: HirValueId,
    },
    ValidateFloat {
        source: HirValueId,
    },
}

impl HirFallibleOperation {
    /// Enumerate evaluated inputs for structural and analysis consumers without allocating.
    pub fn operands<'a>(
        &'a self,
        expressions: &'a HirExpressionStore,
    ) -> impl Iterator<Item = HirValueId> + 'a {
        let mut inline = [None, None];
        let mut arguments = &[][..];
        match self {
            Self::Call { args, .. } => arguments = expressions.values(*args),
            Self::MapOp { receiver, args, .. } => {
                inline[0] = Some(*receiver);
                arguments = expressions.values(*args);
            }
            Self::CastOp { source, .. }
            | Self::FormatFloat { source }
            | Self::ValidateFloat { source } => inline[0] = Some(*source),
            Self::NumericOp { operands, .. } => match operands {
                HirNumericOperands::Unary { operand } => inline[0] = Some(*operand),
                HirNumericOperands::Binary { left, right } => inline = [Some(*left), Some(*right)],
            },
            Self::RangeStepFailure { .. } => {}
        }
        inline
            .into_iter()
            .flatten()
            .chain(arguments.iter().copied())
    }
}

/// One local value transferred across a CFG edge into a newly defined destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirJumpArgument {
    pub source: LocalId,
    pub destination: LocalId,
}

/// Semantic origin retained until private failure lanes have been installed.
///
/// WHY: implicit casts and compound write-backs retain the edge-local error independently of
/// the backend-facing runtime failure message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFailureCause {
    StoreConversion { error: LocalId },
    AuthoredCastConversion { error: LocalId },
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
        HirExpressionKind::RouteConstruct { .. } => HirAssertionMessageEvaluation::Runtime,
        _ => HirAssertionMessageEvaluation::Runtime,
    }
}
