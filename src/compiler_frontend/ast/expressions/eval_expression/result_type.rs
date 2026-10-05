//! Expression result-type resolution for AST evaluation.
//!
//! WHAT: mirrors RPN evaluation with a stack of typed IDs and unresolved literal indices.
//! WHY: AST enforces operator typing before folding/lowering; one pass resolves each pending
//!      literal at its direct or immediate-peer boundary before the canonical policy runs.

use super::evaluator::{
    PendingLiteralDestination, materialize_pending_literal, pending_literal_destination_for_type_id,
};
use super::operator_policy::{resolve_binary_operator_type, resolve_unary_operator_type};
use super::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::failure_facts::ExpressionFailureFacts;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidExpressionReason, OperatorOperandPosition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(super) struct ResolvedExpressionType {
    pub(super) type_id: TypeId,
    pub(super) failure_facts: ExpressionFailureFacts,
}

pub(super) fn resolve_expression_result_type(
    output_queue: &mut [ExpressionRpnItem],
    expression_span: Option<SourceSpan>,
    direct_destination: Option<PendingLiteralDestination>,
    context: &ScopeContext,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> Result<ResolvedExpressionType, ExpressionTypingError> {
    // Resolve pending literals immediately before applying the operator that consumes them, so
    // the type stack carries the same results as the canonical operator policy.
    add_ast_counter(AstCounter::ExpressionTypedStackItems, output_queue.len());

    let mut stack = Vec::with_capacity(output_queue.len());
    let mut failure_facts = ExpressionFailureFacts::default();

    // ------------------------
    //  Walk RPN output queue
    // ------------------------

    for index in 0..output_queue.len() {
        let (prior_items, current_and_rest) = output_queue.split_at_mut(index);
        match &current_and_rest[0] {
            ExpressionRpnItem::Operand(expression) => {
                stack.push(ExpressionTypeStackSlot::Typed(expression.type_id));
            }
            ExpressionRpnItem::PendingNumericLiteral { .. } => {
                stack.push(ExpressionTypeStackSlot::Pending(index));
            }
            ExpressionRpnItem::Operator { operator, span } => match operator.required_values() {
                1 => {
                    let Some(operand) = stack.pop() else {
                        return Err(missing_operand_error(
                            operator,
                            OperatorOperandPosition::Unary,
                            *span,
                            string_table,
                        ));
                    };
                    let operand_type = materialize_type_stack_slot(
                        operand,
                        None,
                        prior_items,
                        context,
                        string_table,
                    )?;
                    let result_type = resolve_unary_operator_type(
                        operator,
                        operand_type,
                        *span,
                        type_environment,
                    )?;
                    if let Some(numeric_operator) = operator.numeric_operator()
                        && let Some(operand) =
                            NumericScalar::from_type_id(operand_type, type_environment)
                        && let Some(domain) =
                            NumericScalar::from_type_id(result_type, type_environment)
                    {
                        failure_facts.record_numeric_operation(
                            numeric_operator,
                            operand,
                            None,
                            domain,
                            *span,
                        );
                    }
                    stack.push(ExpressionTypeStackSlot::Typed(result_type));
                }
                2 => {
                    let Some(rhs) = stack.pop() else {
                        return Err(missing_operand_error(
                            operator,
                            OperatorOperandPosition::BinaryRight,
                            *span,
                            string_table,
                        ));
                    };
                    let Some(lhs) = stack.pop() else {
                        return Err(missing_operand_error(
                            operator,
                            OperatorOperandPosition::BinaryLeft,
                            *span,
                            string_table,
                        ));
                    };

                    let accepts_peer_literal = operator_accepts_peer_literal(operator);
                    // Dec power is the one operator whose typed Dec peer must not
                    // materialize its pending exponent; `^` always consumes profile Int.
                    let exponent_keeps_int = matches!(operator, Operator::Exponent)
                        && matches!(
                            lhs,
                            ExpressionTypeStackSlot::Typed(type_id)
                                if type_environment.number_scale(type_id).is_some()
                        );
                    let lhs_peer = accepts_peer_literal
                        .then(|| numeric_literal_peer(rhs, type_environment))
                        .flatten();
                    let rhs_peer = (accepts_peer_literal && !exponent_keeps_int)
                        .then(|| numeric_literal_peer(lhs, type_environment))
                        .flatten();
                    let lhs_type = materialize_type_stack_slot(
                        lhs,
                        lhs_peer,
                        prior_items,
                        context,
                        string_table,
                    )?;
                    let rhs_type = materialize_type_stack_slot(
                        rhs,
                        rhs_peer,
                        prior_items,
                        context,
                        string_table,
                    )?;

                    let result_type = resolve_binary_operator_type(
                        lhs_type,
                        rhs_type,
                        operator,
                        *span,
                        type_environment,
                        path_fork,
                    )?;
                    if let Some(numeric_operator) = operator.numeric_operator()
                        && let Some(left) = NumericScalar::from_type_id(lhs_type, type_environment)
                        && let Some(right) = NumericScalar::from_type_id(rhs_type, type_environment)
                        && let Some(domain) =
                            NumericScalar::from_type_id(result_type, type_environment)
                    {
                        failure_facts.record_numeric_operation(
                            numeric_operator,
                            left,
                            Some(right),
                            domain,
                            *span,
                        );
                    }
                    stack.push(ExpressionTypeStackSlot::Typed(result_type));
                }
                _ => {
                    return Err(CompilerError::compiler_error(format!(
                        "Unsupported operator arity during expression typing: {:?}",
                        operator
                    ))
                    .into());
                }
            },
        }
    }

    // A pending literal left alone by malformed RPN has no peer; preserve its one-item
    // receiving path, then let the stack-shape diagnostic report the malformed expression.
    let lone_pending_destination = match stack.as_slice() {
        [ExpressionTypeStackSlot::Pending(_)] => direct_destination,
        _ => None,
    };
    for slot in &mut stack {
        if let ExpressionTypeStackSlot::Pending(index) = *slot {
            *slot = ExpressionTypeStackSlot::Typed(materialize_pending_literal(
                output_queue,
                index,
                lone_pending_destination,
                context,
                string_table,
            )?);
        }
    }

    // ------------------------
    //  Validate final stack shape
    // ------------------------

    if stack.len() != 1 {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::UnresolvedStackShape,
            expression_span,
        )
        .into());
    }

    // stack.len() == 1 guarantees pop() returns Some; the None arm guards a compiler bug.
    match stack.pop() {
        Some(ExpressionTypeStackSlot::Typed(type_id)) => Ok(ResolvedExpressionType {
            type_id,
            failure_facts,
        }),
        Some(ExpressionTypeStackSlot::Pending(_)) => {
            Err(super::evaluator::pending_numeric_literal_bug("expression result typing").into())
        }
        None => Err(CompilerError::compiler_error(
            "Expression typing stack unexpectedly empty after shape validation.",
        )
        .into()),
    }
}

#[derive(Clone, Copy)]
enum ExpressionTypeStackSlot {
    Typed(TypeId),
    Pending(usize),
}

fn materialize_type_stack_slot(
    slot: ExpressionTypeStackSlot,
    peer_destination: Option<PendingLiteralDestination>,
    ordered_items: &mut [ExpressionRpnItem],
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<TypeId, ExpressionTypingError> {
    match slot {
        ExpressionTypeStackSlot::Typed(type_id) => Ok(type_id),
        ExpressionTypeStackSlot::Pending(index) => materialize_pending_literal(
            ordered_items,
            index,
            peer_destination,
            context,
            string_table,
        ),
    }
}

fn operator_accepts_peer_literal(operator: &Operator) -> bool {
    operator
        .numeric_operator()
        .is_some_and(|numeric_operator| !numeric_operator.is_unary())
        || matches!(
            operator,
            Operator::Equality
                | Operator::NotEqual
                | Operator::GreaterThan
                | Operator::GreaterThanOrEqual
                | Operator::LessThan
                | Operator::LessThanOrEqual
        )
}

fn numeric_literal_peer(
    slot: ExpressionTypeStackSlot,
    type_environment: &TypeEnvironment,
) -> Option<PendingLiteralDestination> {
    match slot {
        ExpressionTypeStackSlot::Typed(type_id) => {
            pending_literal_destination_for_type_id(type_id, type_environment)
        }
        ExpressionTypeStackSlot::Pending(_) => None,
    }
}

/// Build a missing-operand diagnostic for the given operator and stack position.
fn missing_operand_error(
    operator: &Operator,
    position: OperatorOperandPosition,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> ExpressionTypingError {
    CompilerDiagnostic::missing_operator_operand(
        string_table.get_or_intern(operator.to_str().to_owned()),
        position,
        span,
    )
    .into()
}
