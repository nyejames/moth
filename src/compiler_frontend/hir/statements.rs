//! HIR statements.
//!
//! WHAT: effectful operations inside HIR blocks.
//! WHY: statements are where writes, calls, side-effect expressions, and runtime fragment pushes
//! become explicit before borrow validation and backend lowering.
//!
//! ## Cast contract
//!
//! AST resolves all cast targets, evidence, fallibility, and optional wrapping flags before HIR.
//! HIR only carries compiler-owned builtin runtime casts as `HirExpressionKind::Cast` or
//! `HirFallibleOperation::CastOp`. User-defined cast evidence lowers to a direct user-function call
//! during HIR lowering, and `ResolvedCastEvidence::GenericBound` is validation-only and must not
//! reach HIR.

use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapOp, ValueKind,
};
use crate::compiler_frontend::hir::ids::{HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, LoopStepFailureCause, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::source::SourceSpan;
#[derive(Debug, Clone)]
pub struct HirStatement {
    pub id: HirNodeId,
    pub kind: HirStatementKind,
    /// Exact authored syntax span; compiler-generated HIR statements are span-free.
    pub span: Option<SourceSpan>,
}

/// The binding event produced by an operation that writes a local result.
///
/// A definition creates a new dynamic binding occurrence for this local when the operation
/// executes while preserving the produced value's allocation provenance. An update writes an
/// already-existing local and preserves ordinary place and alias semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirLocalDestination {
    Define(LocalId),
    Update(LocalId),
}

impl HirLocalDestination {
    pub const fn local(self) -> LocalId {
        match self {
            Self::Define(local) | Self::Update(local) => local,
        }
    }
}

/// Target of one ordinary value write.
///
/// Definitions target a local binding directly. Updates target an existing place, including a
/// local or a projected field/index, so a mutable alias keeps its write-through semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirWriteTarget {
    DefineLocal(LocalId),
    AssignPlace(HirPlace),
}

impl HirWriteTarget {
    /// Whether this update reads the same unprojected binding it writes.
    ///
    /// A direct self-update preserves the binding's existing relationship. Definitions,
    /// projected writes and reads from another binding keep their ordinary write semantics.
    pub(crate) fn is_direct_self_update_of(self, value: &HirExpression) -> bool {
        let Self::AssignPlace(destination) = self else {
            return false;
        };
        if !destination.projections.is_empty() || value.value_kind != ValueKind::Place {
            return false;
        }

        matches!(
            &value.kind,
            HirExpressionKind::Load(source)
                if source.root == destination.root && source.projections.is_empty()
        )
    }
}

#[derive(Debug, Clone)]
pub enum HirStatementKind {
    Write {
        target: HirWriteTarget,
        value: HirValueId,
    },

    /// Call an infallible function and bind its ordered success slots.
    Call {
        target: CallTarget,
        args: HirValueRange,
        results: Vec<HirLocalDestination>,
    },

    /// Expression evaluated only for side effects.
    Expr(HirValueId),

    /// Accumulate one runtime string value into the entry start() fragment vec.
    ///
    /// WHAT: explicit HIR primitive that lowers from `NodeKind::PushStartRuntimeFragment`.
    /// WHY: backends handle fragment accumulation without needing to inspect the entry start
    /// function body for heuristic push patterns.
    PushRuntimeFragment {
        /// The local holding the Vec<String> accumulator inside entry start().
        vec_local: LocalId,
        /// Expression that produces the string value to push.
        value: HirValueId,
    },

    // -------------------------
    //  Map Builtins
    // -------------------------
    /// Perform a compiler-owned map builtin operation.
    ///
    /// WHAT: lowers `get`, `contains`, `set`, `remove`, `clear`, and `length` into an explicit
    ///       HIR statement so backends do not need to rediscover map builtin semantics.
    /// WHY: map operations are language builtins, not external package calls. Keeping them
    ///      as dedicated statements preserves receiver mutability, argument order, and
    ///      result local shape for borrow validation and backend lowering.
    MapOp {
        /// The specific builtin operation (get, contains, set, remove, clear, length).
        op: HirMapOp,
        /// The map value being operated on.
        receiver: HirValueId,
        /// Operation-specific arguments such as lookup keys or inserted values.
        args: HirValueRange,
        /// Local that receives the operation result, if any.
        result: Option<HirLocalDestination>,
    },

    // -------------------------
    //  Checked Numeric Operations
    // -------------------------
    /// Perform a checked numeric operation and capture the result.
    ///
    /// WHAT: evaluates `operands` according to `op` and stores the produced value in `result`.
    /// WHY: arithmetic failures (overflow, divide by zero, invalid exponent, non-finite binary
    ///      float) are semantic effects that must be visible to HIR validation and backend
    ///      lowering.
    ///
    /// Trap and Infallible modes produce a scalar value; recoverable operations use `Invoke`.
    NumericOp {
        /// The checked numeric operation (operator plus canonical numeric domain).
        op: HirNumericOp,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// The operand(s) to the operation.
        operands: HirNumericOperands,
        /// Local that receives the scalar operation result.
        result: HirLocalDestination,
    },

    /// Produce the failure selected by a language-defined loop-step guard.
    ///
    /// The enclosing CFG has established zero step or lack of progress.
    /// Recoverable loop-step failures use `Invoke` instead.
    LoopStepFailure {
        cause: LoopStepFailureCause,
        failure_mode: NumericFailureMode,
        result: HirLocalDestination,
    },

    /// Compute the next candidate for a compiler-generated range loop.
    ///
    /// WHAT: performs exactly one add of the signed step to `current`, rounds binary floats at the
    ///       counter's exact precision, then reports whether that value lies within the authored
    ///       endpoint in the step's direction. `in_range` is the exclusive `<`/`>` or inclusive
    ///       `<=`/`>=` comparison selected by the step's sign. The candidate destination is
    ///       written only when the result is in range; the Bool result is always written.
    /// WHY: this is the one narrow backend-neutral operation that can produce a counter-typed
    ///      successor from an independent signed step. Checked numeric operations cannot express
    ///      the mixed pair, and an integer lowering must never materialise an out-of-domain
    ///      successor. The precondition is that `current` already satisfied the bound in the
    ///      step's direction.
    ///
    /// `domain` is the counter domain: a fixed integer or exact binary float. `current` and
    /// `end` use that counter type. `step` keeps its own completed type: a signed fixed integer
    /// for integer counters, or exactly the counter type for binary floats. HIR lowering reads
    /// that type from the step expression and never converts it to the counter.
    RangeCandidate {
        current: HirValueId,
        step: HirValueId,
        end: HirValueId,
        inclusive: bool,
        domain: NumericScalar,
        candidate_result: HirLocalDestination,
        in_range_result: HirLocalDestination,
    },

    // -------------------------
    //  Binary-float Formatting & Validation
    // -------------------------
    /// Format a finite binary float into a `String` using Moth's formatting contract.
    ///
    /// WHAT: evaluates `source` (which must be a valid Moth binary float) and stores the formatted
    ///      string in `result`.
    /// WHY: binary-float template interpolation must share one Moth-owned formatter instead of
    ///      relying on target-native stringification.
    ///
    /// Recoverable formatting uses `Invoke`; statement results are ordinary strings.
    FormatFloat {
        /// The binary-float expression to format.
        source: HirValueId,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// Local that receives the formatted string.
        result: HirLocalDestination,
    },

    /// Validate that a binary-float value is finite before ordinary Moth code observes it.
    ///
    /// WHAT: evaluates `source` (an exact binary-float value coming from an external/backend
    ///      boundary) and stores the validated finite value in `result`.
    /// WHY: supported binary floats are finite by language contract, so values entering from
    ///      external functions or backend boundaries must be rounded at their resolved precision
    ///      and checked explicitly rather than trusted implicitly.
    ///
    /// Recoverable validation uses `Invoke`; statement results are ordinary finite values.
    ValidateFloat {
        /// The exact binary-float expression to validate.
        source: HirValueId,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// Local that receives the validated binary-float value.
        result: HirLocalDestination,
    },
}
