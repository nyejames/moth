//! HIR checked numeric operations.
//!
//! WHAT: defines the statement-level numeric operation surface used by HIR to expose checked
//!       arithmetic, division/modulo-by-zero handling, and recoverable vs trapping failure modes
//!       as a backend-neutral operator plus a canonical numeric domain.
//! WHY: numeric failures are semantic effects that belong in HIR, not in source expression trees,
//!      so backends receive an explicit operation with a known failure mode instead of rediscovering
//!      source operator fallibility. Recording the operator and the domain separately keeps one
//!      small operator vocabulary while later slices add fixed-width domains without multiplying
//!      variants.

use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::HirExpression;

/// How a checked numeric operation should behave on failure.
///
/// WHAT: selects between returning a recoverable builtin `Error!` carrier and trapping.
/// WHY: the choice depends on the enclosing function's error return slot. A builtin `Error!`
///      function can recover numeric failures through the normal fallible-carrier path; any other
///      fallible channel or non-fallible context must trap because the failure cannot be represented
///      as a user-visible value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericFailureMode {
    /// Produce an internal fallible carrier (success value or builtin `Error`).
    ///
    /// WHAT: the enclosing function has builtin `Error!` exactly as its error return slot, so
    ///       numeric failures can be returned through the normal fallible-carrier path.
    /// WHY: this keeps recoverable numeric failures in the same control-flow shape as explicit
    ///      `cast!` propagation and lets later lowering emit `HirTerminator::FallibleBranch`.
    ReturnError,

    /// Stop execution on failure.
    ///
    /// WHAT: the operation has no recoverable channel. The result local receives only the scalar
    ///       success value; failure is a runtime trap/throw.
    /// WHY: custom fallible channels, top-level `start()`, and non-fallible functions cannot
    ///      represent numeric failures as user values, so the backend must halt.
    Trap,
}

/// Language-defined range guards, distinct from resource and invariant traps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeStepFailureCause {
    ZeroStep,
    NoProgress,
}

impl RangeStepFailureCause {
    pub(crate) fn builtin_error_code(self) -> BuiltinErrorCode {
        match self {
            Self::ZeroStep => BuiltinErrorCode::InvalidRangeStep,
            Self::NoProgress => BuiltinErrorCode::RangeStepNoProgress,
        }
    }
}

/// A checked numeric operation: a backend-neutral operator plus its canonical numeric domain.
///
/// WHAT: identifies scalar arithmetic and its result domain (`Int`, `Float`, a fixed scalar, or
///       one exact `Dec` scale).
/// WHY: backends consume one operator/domain fact rather than a target-specific operation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirNumericOp {
    pub operator: NumericOperator,
    pub domain: NumericScalar,
}

impl HirNumericOp {
    /// Whether the operation takes one operand.
    pub(crate) fn is_unary(self) -> bool {
        self.operator.is_unary()
    }
}

impl Display for HirNumericOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}.{}", self.domain, self.operator.name())
    }
}

/// Operand carrier for a checked numeric operation.
///
/// WHAT: represents either a unary or binary numeric operation in one HIR-local shape.
/// WHY: keeps `HirStatementKind::NumericOp` a single variant while still distinguishing unary
///      negation from binary arithmetic. Lowering converts operands to the domain except for the
///      profile-`Int` exponent of a Dec power operation.
#[derive(Debug, Clone)]
pub enum HirNumericOperands {
    Unary {
        operand: HirExpression,
    },
    Binary {
        left: HirExpression,
        right: HirExpression,
    },
}
