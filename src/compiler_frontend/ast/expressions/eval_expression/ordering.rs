//! Expression ordering helpers for AST evaluation.
//!
//! WHAT: converts nested infix fragments into one RPN stream using a shunting-yard pass.
//! WHY: operator typing and folding need deterministic precedence/associativity ordering before
//! they can validate or reduce the expression.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means an internal compiler invariant or setup failure only.
//! Source-authored syntax failures are rejected earlier with `CompilerDiagnostic`.

use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::{eval_log, return_compiler_error};

/// Order a parsed expression fragment into RPN and return its authored span anchor.
pub(super) fn order_expression_nodes(
    nodes: Vec<ExpressionRpnItem>,
) -> Result<(Vec<ExpressionRpnItem>, Option<SourceSpan>), CompilerError> {
    if nodes.is_empty() {
        return_compiler_error!("No nodes found in expression. This should never happen.");
    }

    let span = extract_expression_span(&nodes)?;
    // Groups can contain more items than the outer fragment. Grow the shared buffers rather
    // than repeatedly walking or copying each child subtree to compute exact capacities.
    let mut output_queue = Vec::with_capacity(nodes.len());
    let mut operator_stack = Vec::with_capacity(nodes.len() / 2);
    // Each frame saves the parent iterator and the parent's operator floor, so group-free
    // fragments never allocate frame storage.
    let mut frames = Vec::new();
    let mut operator_floor = 0;
    let mut current_nodes = nodes.into_iter();
    let mut processed_items = 0;

    loop {
        let Some(node) = current_nodes.next() else {
            // A finished group releases its own operators, innermost first, before its parent
            // resumes at the group's operand position.
            output_queue.extend(operator_stack.drain(operator_floor..).rev());
            let Some((parent_nodes, parent_floor)) = frames.pop() else {
                break;
            };
            current_nodes = parent_nodes;
            operator_floor = parent_floor;
            continue;
        };

        eval_log!("Evaluating node in expression: ", Pretty node);
        match node {
            ExpressionRpnItem::PendingGroup { nodes, .. } => {
                frames.push((current_nodes, operator_floor));
                operator_floor = operator_stack.len();
                current_nodes = nodes.into_iter();
            }
            operand @ (ExpressionRpnItem::Operand(..)
            | ExpressionRpnItem::PendingNumericLiteral { .. }) => {
                processed_items += 1;
                output_queue.push(operand);
            }
            operator_item @ ExpressionRpnItem::Operator { .. } => {
                processed_items += 1;
                let operator = operator_from_item(&operator_item)?;
                pop_higher_precedence(
                    &mut operator_stack,
                    &mut output_queue,
                    operator.precedence(),
                    operator.is_left_associative(),
                    operator_floor,
                )?;
                operator_stack.push(operator_item);
            }
        }
    }
    add_ast_counter(AstCounter::ExpressionOrderingInputItems, processed_items);

    Ok((output_queue, span))
}

// Standard shunting-yard pop rule: earlier operators leave the stack when they bind at least
// as tightly as the new operator, adjusted for right-associative cases like exponentiation.
fn pop_higher_precedence(
    operator_stack: &mut Vec<ExpressionRpnItem>,
    output_queue: &mut Vec<ExpressionRpnItem>,
    current_precedence: u32,
    left_associative: bool,
    operator_floor: usize,
) -> Result<(), CompilerError> {
    while operator_stack.len() > operator_floor {
        // The floor guard proves that the active group owns the top operator.
        let top_operator = operator_stack
            .last()
            .expect("operator remains above group floor");
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

/// Returns the authored span of the first non-operator node in the fragment.
///
/// Falls back to the first node's span if every node is an operator.
pub(crate) fn extract_expression_span(
    nodes: &[ExpressionRpnItem],
) -> Result<Option<SourceSpan>, CompilerError> {
    if nodes.is_empty() {
        return_compiler_error!("No nodes found in expression. This should never happen.");
    }

    // Skip operator nodes and return the span of the first expression node.
    for node in nodes {
        if node.is_operand_shape() {
            return Ok(node.source_span());
        }
    }

    // Fallback to first node if all nodes are operators (should not happen).
    Ok(nodes[0].source_span())
}
