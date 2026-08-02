//! HIR operators.
//!
//! WHAT: normalized binary and unary operator enums used by HIR expressions.
//! WHY: backends should consume semantic operators rather than frontend token kinds.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirBinOp {
    /// Compiler-owned append used only by runtime template lowering.
    StringAppend,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    IntDiv,
    Exponent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirUnaryOp {
    Neg,
    Not,
}
