//! HIR checked numeric operations.
//!
//! WHAT: defines checked arithmetic and division/modulo-by-zero handling as a backend-neutral
//!       operator plus a canonical numeric domain. Statement operations trap or are proven
//!       infallible, while `Invoke` owns recoverable success and error delivery.
//! WHY: numeric failures are semantic effects that belong in HIR, not in source expression trees,
//!      so backends consume explicit operations and failure edges instead of rediscovering source
//!      operator fallibility. Keeping the operator and domain separate avoids multiplying variants.

use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::ids::HirValueId;

/// Failure behaviour for statement operations. Recoverable delivery belongs to `Invoke`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericFailureMode {
    /// Stop execution on failure, producing only a scalar success value.
    Trap,

    /// Semantically proven unable to fail; no failure edge.
    ///
    /// WHAT: the shared discharge predicate proved the operation cannot fail from its operand
    ///       types, so the operation lowers through the same operation as `Trap` whose check is
    ///       dead. The result local receives the scalar success value and no failure edge exists.
    /// WHY: backend-independent proof removes the semantic failure without changing evaluation
    ///      or demanding optional check elision; only `NumericOp` statements may carry it.
    Infallible,
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
/// WHAT: identifies scalar arithmetic and its result domain (one exact fixed scalar or one exact
///       `Dec` scale).
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
///      exact `I32` exponent of a Dec power operation.
#[derive(Debug, Clone)]
pub enum HirNumericOperands {
    Unary { operand: HirValueId },
    Binary { left: HirValueId, right: HirValueId },
}
