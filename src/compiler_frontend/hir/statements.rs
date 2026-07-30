//! HIR statements.
//!
//! WHAT: effectful operations inside HIR blocks.
//! WHY: statements are where assignment, calls, side-effect expressions, and runtime fragment pushes
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
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirMapOp};
use crate::compiler_frontend::hir::ids::{HirNodeId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[derive(Debug, Clone)]
pub struct HirStatement {
    pub id: HirNodeId,
    pub kind: HirStatementKind,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub enum HirStatementKind {
    Assign {
        target: HirPlace,
        value: HirExpression,
    },

    /// Call a function and optionally capture the result.
    ///
    /// WHAT: invokes `target` with evaluated `args` and binds the return value to `result`
    ///       when present.
    /// WHY: nested calls are flattened into statement preludes during expression lowering;
    ///      a top-level call in statement position is represented directly as a `Call`.
    Call {
        target: CallTarget,
        args: Vec<HirExpression>,
        result: Option<LocalId>,
    },

    /// Expression evaluated only for side effects.
    Expr(HirExpression),

    /// Accumulate one runtime string value into the entry start() fragment vec.
    ///
    /// WHAT: explicit HIR primitive that lowers from `NodeKind::PushStartRuntimeFragment`.
    /// WHY: backends handle fragment accumulation without needing to inspect the entry start
    /// function body for heuristic push patterns.
    PushRuntimeFragment {
        /// The local holding the Vec<String> accumulator inside entry start().
        vec_local: LocalId,
        /// Expression that produces the string value to push.
        value: HirExpression,
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
        source: HirExpression,
        result: Option<LocalId>,
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
        receiver: HirExpression,
        /// Operation-specific arguments such as lookup keys or inserted values.
        args: Vec<HirExpression>,
        /// Local that receives the operation result, if any.
        result: Option<LocalId>,
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
        /// The specific checked numeric operation (e.g. `IntAdd`, `FloatDiv`).
        op: HirNumericOp,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// The operand(s) to the operation.
        operands: HirNumericOperands,
        /// Local that receives the operation result or fallible carrier.
        result: LocalId,
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
        source: HirExpression,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// Local that receives the formatted string or fallible carrier.
        result: LocalId,
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
        source: HirExpression,
        /// How the operation should behave on failure.
        failure_mode: NumericFailureMode,
        /// Local that receives the validated float or fallible carrier.
        result: LocalId,
    },
}

impl HirStatement {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.location.remap_string_ids(remap);

        match &mut self.kind {
            HirStatementKind::Assign { target, value } => {
                target.remap_string_ids(remap);
                value.remap_string_ids(remap);
            }
            HirStatementKind::Call { args, .. } => {
                for argument in args {
                    argument.remap_string_ids(remap);
                }
            }
            HirStatementKind::Expr(expression) => expression.remap_string_ids(remap),
            HirStatementKind::PushRuntimeFragment { value, .. }
            | HirStatementKind::CastOp { source: value, .. }
            | HirStatementKind::FormatFloat { source: value, .. }
            | HirStatementKind::ValidateFloat { source: value, .. } => {
                value.remap_string_ids(remap);
            }
            HirStatementKind::MapOp { receiver, args, .. } => {
                receiver.remap_string_ids(remap);
                for argument in args {
                    argument.remap_string_ids(remap);
                }
            }
            HirStatementKind::NumericOp { operands, .. } => match operands {
                HirNumericOperands::Unary { operand } => operand.remap_string_ids(remap),
                HirNumericOperands::Binary { left, right } => {
                    left.remap_string_ids(remap);
                    right.remap_string_ids(remap);
                }
            },
            HirStatementKind::Drop(_) => {}
        }
    }
}
