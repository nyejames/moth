//! Runtime expression lowering internals.
//!
//! WHAT: organizes runtime RPN lowering into dedicated units for tree construction, generic
//! tree lowering, short-circuit CFG construction, and merge-temp assignment behavior.
//! WHY: short-circuit lowering is one of the highest-risk semantic paths and benefits from
//! isolated, auditable helpers.

use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[derive(Debug, Clone)]
pub(super) enum RuntimeRpnTree {
    Leaf(Box<Expression>),
    Unary {
        op: Operator,
        operand: Box<RuntimeRpnTree>,
        location: SourceLocation,
        span: Option<SourceSpan>,
    },
    Binary {
        left: Box<RuntimeRpnTree>,
        op: Operator,
        right: Box<RuntimeRpnTree>,
        location: SourceLocation,
        span: Option<SourceSpan>,
    },
}

mod rpn_tree;
mod short_circuit_cfg;
mod temp_assignment;
mod tree_lowering;
