//! HIR block terminators.
//!
//! WHAT: explicit control-flow exits for each block.
//! WHY: control flow must be structured enough for borrow validation and backend lowering.

use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::patterns::HirMatchArm;

#[derive(Debug, Clone)]
pub enum HirTerminator {
    Jump {
        target: BlockId,
        args: Vec<LocalId>, // Not SSA - just passing current local values
    },

    If {
        condition: HirExpression,
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
        result: HirExpression,
        success_block: BlockId,
        error_block: BlockId,
    },

    Match {
        scrutinee: HirExpression,
        arms: Vec<HirMatchArm>, // Each arm's body block must end with Jump or Return
    },

    Break {
        target: BlockId,
    },

    Continue {
        target: BlockId,
    },

    Return(HirExpression),

    /// Return through the function's fallible success slot.
    ///
    /// WHAT: represents `return value` from a fallible function without constructing a runtime
    /// fallible carrier in HIR.
    /// WHY: explicit success/error terminators keep the HIR control-flow contract aligned with
    /// Moth's fallible signature model.
    ReturnSuccess(HirExpression),

    /// Return through the function's fallible error slot.
    ///
    /// WHAT: represents `return! value` without constructing a runtime fallible carrier in HIR.
    /// WHY: Phase 8 moves fallible control flow toward explicit success/error edges so borrow
    /// validation and backend lowering do not need to infer error paths from variant values.
    ReturnError(HirExpression),

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
    },

    /// Assertion failure — unrecoverable runtime stop.
    ///
    /// WHAT: represents a failed `assert` statement.
    /// WHY: this is the only source-level unrecoverable stop in Alpha Moth.
    /// `message: None` means the default "assertion failed" message.
    AssertFailure {
        message: Option<String>,
    },
}
