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
//! `HirStatementKind::CastOp`. User-defined cast evidence lowers to a direct user-function call
//! during HIR lowering, and `ResolvedCastEvidence::GenericBound` is validation-only and must not
//! reach HIR.

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapOp, ValueKind,
};
use crate::compiler_frontend::hir::ids::{HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode, RangeStepFailureCause,
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

    /// Call a function and optionally capture the result.
    ///
    /// WHAT: invokes `target` with evaluated `args` and binds the return value to `result`
    ///       when present.
    /// WHY: nested calls are flattened into statement preludes during expression lowering;
    ///      a top-level call in statement position is represented directly as a `Call`.
    Call {
        target: CallTarget,
        args: HirValueRange,
        result: Option<HirLocalDestination>,
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

    /// Explicit deterministic drop.
    #[allow(dead_code)] // Planned: explicit drop statements after ownership lowering matures.
    Drop(LocalId),

    // -------------------------
    //  Cast Builtins
    // -------------------------
    /// Apply a compiler-owned builtin cast to a source value and capture the result.
    ///
    /// WHAT: evaluates `source`, applies `policy`, and stores the produced value (or fallible
    ///      carrier) in `result` when present.
    /// WHY: AST already resolved the target, evidence, fallibility, and optional wrap flag, so
    ///      HIR only materializes the resulting builtin runtime cast. Fallible casts need a
    ///      statement-shaped operation so HIR can branch on the resulting carrier without hiding
    ///      control flow inside an expression. Infallible casts may also use this form when the
    ///      result is needed as a statement-local temporary.
    CastOp {
        policy: BuiltinCastPolicyId,
        source: HirValueId,
        result: Option<HirLocalDestination>,
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
    /// WHY: arithmetic failures (overflow, divide by zero, invalid exponent, non-finite `Float`)
    ///      are semantic effects that must be visible to HIR validation and backend lowering.
    ///
    /// Result-local contract:
    /// - In `NumericFailureMode::Trap` the result local receives the scalar success value.
    ///   Failure is a runtime trap/throw and does not produce a user-visible carrier.
    /// - In `NumericFailureMode::ReturnError` the result local receives the internal fallible
    ///   carrier (success value or builtin `Error`). A later lowering helper is expected to branch
    ///   with `HirTerminator::FallibleBranch` and unwrap success/error before borrow validation.
    NumericOp {
        /// The checked numeric operation (operator plus canonical numeric domain).
        op: HirNumericOp,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// The operand(s) to the operation.
        operands: HirNumericOperands,
        /// Local that receives the operation result or fallible carrier.
        result: HirLocalDestination,
    },

    /// Produce the failure selected by a language-defined range-step guard.
    ///
    /// The enclosing CFG has already established zero step or lack of progress. Keeping this
    /// producer statement-shaped lets the private failure lane retarget it exactly like NumericOp.
    /// Its result follows NumericOp's scalar/carrier contract with Bool as the unused success type.
    RangeStepFailure {
        cause: RangeStepFailureCause,
        failure_mode: NumericFailureMode,
        result: HirLocalDestination,
    },

    /// Compute the next candidate for a compiler-generated binary-float range loop.
    ///
    /// WHAT: performs exactly one profile-precision add/subtract into a backend-local scratch,
    ///       then reports whether that finite candidate satisfies the authored range endpoint.
    /// WHY: the rounded step-vs-distance precheck can reject an inclusive endpoint reached by
    ///      rounding. The candidate destination is written only when the result is finite and in
    ///      range; the Bool result is always written.
    ///
    /// `domain` is limited to profile `Float` and fixed `F32`/`F64` ranges. The expressions are
    /// already converted to that domain by HIR lowering.
    FloatRangeCandidate {
        current: HirValueId,
        step: HirValueId,
        end: HirValueId,
        ascending: HirValueId,
        inclusive: bool,
        domain: NumericScalar,
        candidate_result: HirLocalDestination,
        in_range_result: HirLocalDestination,
    },

    // -------------------------
    //  Float Formatting & Validation
    // -------------------------
    /// Format a finite `Float` into a `String` using Moth's formatting contract.
    ///
    /// WHAT: evaluates `source` (which must be a valid Moth `Float`) and stores the formatted
    ///      string in `result`.
    /// WHY: `Float -> String` casts and runtime Float template interpolation must share one
    ///      Moth-owned formatter instead of relying on target-native stringification.
    ///
    /// Result-local contract:
    /// - In `NumericFailureMode::Trap` the result local receives the scalar `String` success value.
    ///   Failure (an unexpected non-finite input) is a runtime trap/throw.
    /// - In `NumericFailureMode::ReturnError` the result local receives the internal fallible
    ///   carrier (`String` success value or builtin `Error`). A later lowering helper is expected to
    ///   branch with `HirTerminator::FallibleBranch` and unwrap success/error before borrow
    ///   validation.
    FormatFloat {
        /// The `Float` expression to format.
        source: HirValueId,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// Local that receives the formatted string or fallible carrier.
        result: HirLocalDestination,
    },

    /// Validate that a `Float` value is finite before exposing it as an ordinary Moth `Float`.
    ///
    /// WHAT: evaluates `source` (a `Float` value coming from an external/backend boundary) and
    ///      stores the validated finite `Float` in `result`.
    /// WHY: Moth `Float` is finite `f64`; values entering from external functions or backend
    ///      boundaries must be checked explicitly rather than trusted implicitly.
    ///
    /// Result-local contract:
    /// - In `NumericFailureMode::Trap` the result local receives the scalar `Float` success value.
    ///   Failure (a non-finite input) is a runtime trap/throw.
    /// - In `NumericFailureMode::ReturnError` the result local receives the internal fallible
    ///   carrier (`Float` success value or builtin `Error`). A later lowering helper is expected to
    ///   branch with `HirTerminator::FallibleBranch` and unwrap success/error before borrow
    ///   validation.
    ValidateFloat {
        /// The `Float` expression to validate.
        source: HirValueId,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// Local that receives the validated float or fallible carrier.
        result: HirLocalDestination,
    },
}
