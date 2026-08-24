//! Expression ordering helpers for AST evaluation.
//!
//! WHAT: converts a flat infix fragment into RPN using a shunting-yard pass.
//! WHY: operator typing and folding need deterministic precedence/associativity ordering before
//! they can validate or reduce the expression.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means an internal compiler invariant or setup failure only.
//! Source-authored syntax failures are rejected earlier with `CompilerDiagnostic`.

use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::{eval_log, return_compiler_error};

/// Order a parsed expression fragment into RPN and return the expression source location anchor.
pub(super) fn order_expression_nodes(
    nodes: Vec<ExpressionRpnItem>,
) -> Result<(Vec<ExpressionRpnItem>, SourceLocation), CompilerError> {
    if nodes.is_empty() {
        return_compiler_error!("No nodes found in expression. This should never happen.");
    }

    add_ast_counter(AstCounter::ExpressionOrderingInputItems, nodes.len());

    // Every input node ends up in the output queue exactly once, so the input length is the
    // exact capacity, not an estimate. A well-formed infix fragment of `n` items carries at most
    // `(n - 1) / 2` operators, so the operator stack is bounded by half the input.
    let mut output_queue: Vec<ExpressionRpnItem> = Vec::with_capacity(nodes.len());
    let mut operator_stack: Vec<ExpressionRpnItem> = Vec::with_capacity(nodes.len() / 2);
    let location = extract_expression_location(&nodes)?;

    // The parser already handled parentheses recursively, so this pass only needs to order the
    // flat infix fragment by precedence and associativity before typing/folding it.

    for node in nodes {
        eval_log!("Evaluating node in expression: ", Pretty node);
        match &node {
            ExpressionRpnItem::Operand(..) => output_queue.push(node),

            ExpressionRpnItem::Operator { operator, .. } => {
                let current_precedence = operator.precedence();
                let left_associative = operator.is_left_associative();

                pop_higher_precedence(
                    &mut operator_stack,
                    &mut output_queue,
                    current_precedence,
                    left_associative,
                )?;

                operator_stack.push(node);
            }
        }
    }

    // Drain any remaining operators onto the output queue.
    while let Some(operator) = operator_stack.pop() {
        output_queue.push(operator);
    }

    Ok((output_queue, location))
}

// Standard shunting-yard pop rule: earlier operators leave the stack when they bind at least
// as tightly as the new operator, adjusted for right-associative cases like exponentiation.
fn pop_higher_precedence(
    operator_stack: &mut Vec<ExpressionRpnItem>,
    output_queue: &mut Vec<ExpressionRpnItem>,
    current_precedence: u32,
    left_associative: bool,
) -> Result<(), CompilerError> {
    while let Some(top_operator) = operator_stack.last() {
        let existing_precedence = operator_from_item(top_operator)?.precedence();

        let should_pop = if left_associative {
            existing_precedence >= current_precedence
        } else {
            existing_precedence > current_precedence
        };

        if should_pop {
            let Some(operator) = operator_stack.pop() else {
                return_compiler_error!(
                    "Expression ordering lost operator stack state during shunting-yard pop."
                );
            };
            output_queue.push(operator);
        } else {
            break;
        }
    }

    Ok(())
}

fn operator_from_item(item: &ExpressionRpnItem) -> Result<&Operator, CompilerError> {
    let ExpressionRpnItem::Operator { operator, .. } = item else {
        return_compiler_error!("Expression ordering stored an operand on the operator stack.");
    };

    Ok(operator)
}

/// Returns the source location of the first non-operator node in the fragment.
///
/// Falls back to the first node's location if every node is an operator.
pub(super) fn extract_expression_location(
    nodes: &[ExpressionRpnItem],
) -> Result<SourceLocation, CompilerError> {
    if nodes.is_empty() {
        return_compiler_error!("No nodes found in expression. This should never happen.");
    }

    // Skip operator nodes and return the location of the first expression node.
    for node in nodes {
        if matches!(node, ExpressionRpnItem::Operand(_)) {
            return Ok(node.source_location());
        }
    }

    // Fallback to first node if all nodes are operators (should not happen).
    Ok(nodes[0].source_location())
}
