//! Expression evaluation and AST-side constant folding.
//!
//! WHAT: resolves parsed infix expression fragments into typed AST expressions.
//! WHY: AST is the semantic boundary that owns operator typing, fallible handling checks, and the
//! final decision about whether an expression can collapse at compile time or must survive to HIR.

mod evaluator;
mod operator_policy;
mod ordering;
pub(crate) mod result_type;
mod typing_error;

#[cfg(test)]
pub(crate) use crate::compiler_frontend::value_mode::ValueMode;
pub use evaluator::evaluate_expression;
#[cfg(test)]
pub(crate) use evaluator::expression_from_resolved_items;

pub(super) use ordering::extract_expression_span;
pub(crate) use typing_error::ExpressionTypingError;

#[cfg(test)]
#[path = "../tests/eval_expression_tests.rs"]
mod eval_expression_tests;

#[cfg(test)]
#[path = "../tests/evaluator_escape_tests.rs"]
mod evaluator_escape_tests;
