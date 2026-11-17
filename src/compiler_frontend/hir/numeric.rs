//! HIR checked numeric operations.
//!
//! WHAT: defines checked and fast arithmetic and division/modulo-by-zero handling as a
//!       backend-neutral operator plus a canonical numeric domain. Statement operations trap,
//!       fail fatally or are proven infallible, while `Invoke` owns recoverable success and error
//!       delivery.
//! WHY: numeric failures are semantic effects that belong in HIR, not in source expression trees,
//!      so backends consume explicit operations and failure edges instead of rediscovering source
//!      operator fallibility. Keeping the operator and domain separate avoids multiplying variants.

use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, NumericSemantics, fast_numeric_trap_codes,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::ids::HirValueId;

/// Failure behaviour for statement operations. Recoverable delivery belongs to `Invoke`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericFailureMode {
    /// Fatal trap; no Moth handler or inferred lane may recover it.
    ///
    /// WHAT: the delivery for every trapping fast numeric operation, every fast range guard and
    ///       a fast range loop's generated ordinal overflow.
    /// WHY: fast failures must never become a recoverable `Invoke`, even inside a lane function.
    ///      Recording that on the delivery mode keeps the private-lane rule one fact: only `Trap`
    ///      is lane-eligible, so no lane decision reads arithmetic semantics.
    Fatal,

    /// Trap; a private no-slot function's inferred lane may retarget this failure.
    ///
    /// WHAT: the delivery for ordinary recoverable-by-installation numeric and loop-step
    ///       failures.
    /// WHY: the private-lane installer converts only this mode. Fast arithmetic and fast range
    ///      guards use `Fatal`, so they stay statement traps without re-deriving the mode from
    ///      operation semantics.
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

impl NumericFailureMode {
    /// The statement delivery of a generated range guard under `semantics`.
    ///
    /// WHAT: `Fatal` under `$fast_math`, else `Trap`, which private-lane installation may
    ///       retarget onto a recoverable edge.
    /// WHY: lowering writes this mode and validation re-derives it. One owner keeps the two from
    ///      disagreeing and rejecting valid HIR.
    pub(crate) fn range_guard(semantics: NumericSemantics) -> Self {
        if semantics.is_fast() {
            Self::Fatal
        } else {
            Self::Trap
        }
    }
}

/// Language-defined loop-step failures, distinct from resource and invariant traps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStepFailureCause {
    ZeroStep,
    NoProgress,
}

impl LoopStepFailureCause {
    pub(crate) fn builtin_error_code(self) -> BuiltinErrorCode {
        match self {
            Self::ZeroStep => BuiltinErrorCode::InvalidLoopStep,
            Self::NoProgress => BuiltinErrorCode::RangeStepNoProgress,
        }
    }
}

/// A checked numeric operation: a backend-neutral operator, its canonical numeric domain and the
/// arithmetic semantics selected at resolution.
///
/// WHAT: identifies scalar arithmetic, its result domain (one exact fixed scalar or one exact
///       `Dec` scale) and whether the operation evaluates under checked or `$fast_math` rules.
/// WHY: backends consume one operator/domain/semantics fact rather than a target-specific
///      operation family. The semantics is not the delivery mode: a fast operation is still a
///      statement operation, and `NumericFailureMode` records trap, fatal or discharge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirNumericOp {
    pub operator: NumericOperator,
    pub domain: NumericScalar,
    /// Checked arithmetic, or fast arithmetic whose fixed integers wrap and whose remaining
    /// faults trap.
    pub semantics: NumericSemantics,
}

impl HirNumericOp {
    /// Builds one checked (non-`$fast_math`) operation for a test fixture.
    ///
    /// WHAT: the explicit ordinary-semantics constructor, used where an operation is known to be
    ///       checked and the call site has no mode to state.
    /// WHY: test fixtures would otherwise repeat the same three-field literal; production sites
    ///      state `semantics` inline so every real selection is visible at its construction.
    #[cfg(test)]
    pub(crate) fn ordinary(operator: NumericOperator, domain: NumericScalar) -> Self {
        Self {
            operator,
            domain,
            semantics: NumericSemantics::Ordinary,
        }
    }

    /// Whether the operation takes one operand.
    pub(crate) fn is_unary(self) -> bool {
        self.operator.is_unary()
    }

    /// The statement failure mode a fast operation must use, derived from its trap causes.
    ///
    /// WHAT: `Infallible` when `fast_numeric_trap_codes` is empty and `Fatal` otherwise.
    /// WHY: fast operations never use `Invoke`, and their remaining faults stay fatal even in a
    ///      private no-slot function with an inferred lane, so the semantic trap-cause table — not
    ///      the ordinary recoverable code set — chooses the delivery mode. HIR lowering and
    ///      validation read this one selector, so the two cannot disagree.
    pub(crate) fn fast_statement_failure_mode(self) -> NumericFailureMode {
        if fast_numeric_trap_codes(self.operator, self.domain).is_empty() {
            NumericFailureMode::Infallible
        } else {
            NumericFailureMode::Fatal
        }
    }
}

impl Display for HirNumericOp {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let semantics = if self.semantics.is_fast() {
            "fast_"
        } else {
            ""
        };
        write!(f, "{}.{}{}", self.domain, semantics, self.operator.name())
    }
}

/// How compiler-generated arithmetic delivers its failure edge.
///
/// WHAT: `Inherit` follows the enclosing builtin `Error` handler and the current function's
///       return contract; `Fatal` always emits a fatal trapping statement and never `Invoke`.
/// WHY: a fast range loop's generated ordinal overflow must fail fatally even inside a catch
/// handler, an `Error!` function or a private no-slot function with an inferred lane, so the
/// generated caller states that delivery instead of letting the generic contract selection decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedNumericDelivery {
    /// Follow the active builtin `Error` handler and the current function's return contract.
    Inherit,
    /// Always fail fatally; never recoverable and never lane-retargeted.
    Fatal,
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

/// One compiler-inserted `$fast_math` compound-assignment write-back conversion.
///
/// WHAT: the exact source arithmetic domain and the destination target domain. The conversion is
///       not an explicit `cast` and not an ordinary checked store conversion.
/// WHY: a fast write-back has its own semantics — integer destinations wrap, float destinations
///      round and trap when non-finite, and float sources truncate and trap when the integer
///      target cannot hold them. Keeping these two domains together lets HIR validation,
///      reachability and both backends agree on one conversion fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirFastStoreConversion {
    pub source: NumericScalar,
    pub target: NumericScalar,
}

/// The four conversion shapes a fast compound write-back permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastStoreConversionClass {
    /// Integer to integer: keep the destination low bits and signedness. Total.
    IntegerWrap,
    /// Float to integer: truncate toward zero and trap when out of the destination range.
    FloatToInteger,
    /// Integer to binary float: round at the destination precision and trap when non-finite.
    IntegerToFloat,
    /// Binary float to binary float: round at the destination precision and trap when non-finite.
    FloatToFloat,
}

impl HirFastStoreConversion {
    /// Classifies one conversion, rejecting `Dec` and same-domain pairs.
    ///
    /// WHY: `Dec` arithmetic is never fast, so any conversion involving it stays ordinary; a
    ///      same-domain conversion is not a write-back at all.
    pub(crate) fn classify(self) -> Option<FastStoreConversionClass> {
        if self.source == self.target {
            return None;
        }

        match (self.source, self.target) {
            (source, target) if source.is_integer() && target.is_integer() => {
                Some(FastStoreConversionClass::IntegerWrap)
            }
            (source, target) if source.is_binary_float() && target.is_integer() => {
                Some(FastStoreConversionClass::FloatToInteger)
            }
            (source, target) if source.is_integer() && target.is_binary_float() => {
                Some(FastStoreConversionClass::IntegerToFloat)
            }
            (source, target) if source.is_binary_float() && target.is_binary_float() => {
                Some(FastStoreConversionClass::FloatToFloat)
            }
            _ => None,
        }
    }

    /// The delivery a fast write-back must use: `Infallible` only for an integer wrap.
    pub(crate) fn required_failure_mode(self) -> Option<NumericFailureMode> {
        match self.classify()? {
            FastStoreConversionClass::IntegerWrap => Some(NumericFailureMode::Infallible),
            FastStoreConversionClass::FloatToInteger
            | FastStoreConversionClass::IntegerToFloat
            | FastStoreConversionClass::FloatToFloat => Some(NumericFailureMode::Fatal),
        }
    }
}

impl Display for HirFastStoreConversion {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{} -> {}", self.source, self.target)
    }
}
