//! Expression ordering helpers for AST evaluation.
//!
//! WHAT: converts one flat infix fragment, with transient group markers, into an RPN stream
//!       using a shunting-yard pass. Group markers are consumed here and never reach the output.
//! WHY: operator typing and folding need deterministic precedence/associativity ordering before
//! they can validate or reduce the expression.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means an internal compiler invariant or setup failure only.
//! Source-authored syntax failures are rejected earlier as diagnostic drafts.

use super::evaluator::pending_expression_item_bug;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::{eval_log, return_compiler_error};

/// Order a parsed expression fragment into RPN and return its authored span anchor.
pub(super) fn order_expression_nodes(
    nodes: Vec<ExpressionRpnItem>,
    expressions: &AstExpressionBuilder,
) -> Result<(Vec<ExpressionRpnItem>, Option<SourceSpan>), CompilerError> {
    if nodes.is_empty() {
        return_compiler_error!("No nodes found in expression. This should never happen.");
    }

    let span = extract_expression_span(expressions, &nodes)?;
    let mut output_queue = Vec::with_capacity(nodes.len());
    let mut operator_stack = Vec::with_capacity(nodes.len() / 2);
    // Each open group saves its parent's operator floor, so group-free fragments never allocate
    // floor storage.
    let mut parent_floors = Vec::new();
    let mut operator_floor = 0;
    let mut processed_items = 0;

    for node in nodes {
        eval_log!("Evaluating node in expression: ", Pretty node);
        match node {
            // An open group saves its parent's floor, so its operators cannot pop the parent's.
            ExpressionRpnItem::GroupOpen { .. } => {
                parent_floors.push(operator_floor);
                operator_floor = operator_stack.len();
            }
            // A closed group releases its own operators, innermost first, before its parent
            // resumes at the group's operand position.
            ExpressionRpnItem::GroupClose { .. } => {
                output_queue.extend(operator_stack.drain(operator_floor..).rev());
                let Some(parent_floor) = parent_floors.pop() else {
                    return_compiler_error!("Expression ordering found an unmatched group close.");
                };
                operator_floor = parent_floor;
            }
            operand @ (ExpressionRpnItem::Operand(..)
            | ExpressionRpnItem::PendingNumericLiteral { .. }) => {
                processed_items += 1;
                output_queue.push(operand);
            }
            // Construction-only items are replaced by body completion before evaluation, and
            // the parser retains every fragment that still contains one. Ordering therefore
            // reports their presence as the broken compiler invariant it is.
            ExpressionRpnItem::PendingNumericBinding { .. }
            | ExpressionRpnItem::PendingNumericCall { .. }
            | ExpressionRpnItem::PendingNumericCast { .. }
            | ExpressionRpnItem::PendingNumericExpression { .. } => {
                return Err(pending_expression_item_bug("expression ordering"));
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
            ExpressionRpnItem::ResolvedOperator(_) => {
                return Err(CompilerError::compiler_error(
                    "Resolved operation reached transient infix expression ordering.",
                ));
            }
        }
    }
    if !parent_floors.is_empty() {
        return_compiler_error!("Expression ordering ended inside an unclosed group.");
    }
    output_queue.extend(operator_stack.drain(operator_floor..).rev());
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
    expressions: &AstExpressionBuilder,
    nodes: &[ExpressionRpnItem],
) -> Result<Option<SourceSpan>, CompilerError> {
    if nodes.is_empty() {
        return_compiler_error!("No nodes found in expression. This should never happen.");
    }

    // Skip operator nodes and return the span of the first expression node.
    for node in nodes {
        if node.is_operand_shape() {
            return node.source_span(expressions);
        }
    }

    // Fallback to first node if all nodes are operators (should not happen).
    nodes[0].source_span(expressions)
}
