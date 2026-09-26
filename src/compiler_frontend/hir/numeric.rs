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

/// One backend-neutral checked numeric operator.
///
/// WHAT: the arithmetic shape without its scalar domain. `Divide` is real `/` and
///       `IntegerDivide` is truncating `//`.
/// WHY: the operator vocabulary stays fixed while numeric domains grow; HIR validation and
///      backends combine this with `NumericScalar` to recover the full checked operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirNumericOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    IntegerDivide,
    Remainder,
    Power,
    Negate,
}

impl HirNumericOperator {
    /// The canonical operator name used in HIR display (`Int.add`, `Float.divide`, ...).
    fn name(self) -> &'static str {
        match self {
            HirNumericOperator::Add => "add",
            HirNumericOperator::Subtract => "subtract",
            HirNumericOperator::Multiply => "multiply",
            HirNumericOperator::Divide => "divide",
            HirNumericOperator::IntegerDivide => "integer_divide",
            HirNumericOperator::Remainder => "remainder",
            HirNumericOperator::Power => "power",
            HirNumericOperator::Negate => "negate",
        }
    }
}

/// A checked numeric operation: a backend-neutral operator plus its canonical numeric domain.
///
/// WHAT: identifies the scalar arithmetic operation and the domain it runs in (`Int`, `Float`,
///       or a later fixed-width scalar).
/// WHY: backends must know both the operation (add, divide, power, ...) and the domain so they
///      can apply the correct checked runtime helper, without one enum variant per combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirNumericOp {
    pub operator: HirNumericOperator,
    pub domain: NumericScalar,
}

impl HirNumericOp {
    /// Whether the operation takes one operand.
    pub(crate) fn is_unary(self) -> bool {
        matches!(self.operator, HirNumericOperator::Negate)
    }
}

impl Display for HirNumericOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}.{}", self.domain.name(), self.operator.name())
    }
}

/// Operand carrier for a checked numeric operation.
///
/// WHAT: represents either a unary or binary numeric operation in one HIR-local shape.
/// WHY: keeps `HirStatementKind::NumericOp` a single variant while still distinguishing unary
///      negation from binary arithmetic for validation and backend lowering.
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
