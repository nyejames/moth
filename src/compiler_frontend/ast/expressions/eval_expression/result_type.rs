//! Expression result-type resolution for AST evaluation.
//!
//! Indexes authored RPN structure before materialising literals. Iterative resolution lets Dec
//! and Uint receivers and local peers reach literal-only arithmetic without natural-type
//! intermediates. Operator policy remains canonical, and failure facts are recorded afterwards
//! in RPN order.

use super::evaluator::{
    PendingLiteralDestination, materialize_pending_literal, pending_expression_item_bug,
    pending_literal_destination_for_type_id,
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
    literal_destination: Option<PendingLiteralDestination>,
    context: &ScopeContext,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> Result<ResolvedExpressionType, ExpressionTypingError> {
    add_ast_counter(AstCounter::ExpressionTypedStackItems, output_queue.len());
    let (mut nodes, roots) = index_expression_nodes(output_queue, string_table)?;

    // Resolve every root before reporting a malformed final shape, preserving pending-literal
    // diagnostics. A lone pending root keeps the direct receiving destination.
    for &root in &roots {
        let destination = if roots.len() == 1 {
            literal_destination
        } else {
            None
        };
        resolve_indexed_expression(
            output_queue,
            &mut nodes,
            root,
            destination,
            context,
            string_table,
            type_environment,
            path_fork,
        )?;
    }
    let [root] = roots.as_slice() else {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::UnresolvedStackShape,
            expression_span,
        )
        .into());
    };

    // Peer selection can resolve the right subtree first. Failure witnesses must still follow
    // authored RPN order, independently of the order in which their types became known.
    let failure_facts = collect_numeric_failure_facts(output_queue, &nodes, type_environment)?;
    Ok(ResolvedExpressionType {
        type_id: resolved_type(&nodes, *root)?,
        failure_facts,
    })
}

#[derive(Clone, Copy)]
struct IndexedExpressionNode {
    children: ExpressionChildren,
    literal_only: bool,
    type_id: Option<TypeId>,
}

#[derive(Clone, Copy)]
enum ExpressionChildren {
    Leaf,
    Unary(usize),
    Binary { left: usize, right: usize },
}

// The temporary index references the single RPN buffer. Each child belongs to exactly one
// parent, so indexing and resolution are linear and never copy or rescan a subtree.
fn index_expression_nodes(
    items: &[ExpressionRpnItem],
    string_table: &mut StringTable,
) -> Result<(Vec<IndexedExpressionNode>, Vec<usize>), ExpressionTypingError> {
    let mut nodes: Vec<IndexedExpressionNode> = Vec::with_capacity(items.len());
    let mut stack = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let node = match item {
            ExpressionRpnItem::Operand(_) => IndexedExpressionNode {
                children: ExpressionChildren::Leaf,
                literal_only: false,
                type_id: None,
            },
            ExpressionRpnItem::PendingNumericLiteral { .. } => IndexedExpressionNode {
                children: ExpressionChildren::Leaf,
                literal_only: true,
                type_id: None,
            },
            ExpressionRpnItem::PendingGroup { .. } => {
                return Err(pending_expression_item_bug("expression result typing").into());
            }
            ExpressionRpnItem::Operator { operator, span } => match operator.required_values() {
                1 => {
                    let child = stack.pop().ok_or_else(|| {
                        missing_operand_error(
                            operator,
                            OperatorOperandPosition::Unary,
                            *span,
                            string_table,
                        )
                    })?;
                    IndexedExpressionNode {
                        children: ExpressionChildren::Unary(child),
                        literal_only: matches!(operator, Operator::Negate)
                            && nodes[child].literal_only,
                        type_id: None,
                    }
                }
                2 => {
                    let right = stack.pop().ok_or_else(|| {
                        missing_operand_error(
                            operator,
                            OperatorOperandPosition::BinaryRight,
                            *span,
                            string_table,
                        )
                    })?;
                    let left = stack.pop().ok_or_else(|| {
                        missing_operand_error(
                            operator,
                            OperatorOperandPosition::BinaryLeft,
                            *span,
                            string_table,
                        )
                    })?;
                    IndexedExpressionNode {
                        children: ExpressionChildren::Binary { left, right },
                        literal_only: operator.numeric_operator().is_some()
                            && nodes[left].literal_only
                            && nodes[right].literal_only,
                        type_id: None,
                    }
                }
                _ => {
                    return Err(CompilerError::compiler_error(format!(
                        "Unsupported operator arity during expression typing: {operator:?}",
                    ))
                    .into());
                }
            },
        };
        nodes.push(node);
        stack.push(index);
    }
    Ok((nodes, stack))
}

enum ResolutionFrame {
    Visit {
        index: usize,
        destination: Option<PendingLiteralDestination>,
    },
    VisitPeerLiteral {
        index: usize,
        peer: usize,
        exponent_edge: bool,
    },
    VisitCompoundPeer {
        index: usize,
        peer: usize,
        exponent_edge: bool,
    },
    VisitReceiverExponent {
        base: usize,
        exponent: usize,
    },
    CompleteOperator(usize),
}

/// Shape of a binary peer interaction worth resolving out of order.
enum LiteralPeerShape {
    LoneLiteral { literal: usize, peer: usize },
    CompoundContextual { compound: usize, peer: usize },
}

#[allow(
    clippy::too_many_arguments,
    reason = "resolution keeps the indexed RPN, indexed nodes and shared semantic resources as separate borrows"
)]
fn resolve_indexed_expression(
    items: &mut [ExpressionRpnItem],
    nodes: &mut [IndexedExpressionNode],
    root: usize,
    destination: Option<PendingLiteralDestination>,
    context: &ScopeContext,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> Result<(), ExpressionTypingError> {
    let mut frames = vec![ResolutionFrame::Visit {
        index: root,
        destination,
    }];
    while let Some(frame) = frames.pop() {
        match frame {
            ResolutionFrame::Visit { index, destination } => match &items[index] {
                ExpressionRpnItem::Operand(expression) => {
                    nodes[index].type_id = Some(expression.type_id);
                }
                ExpressionRpnItem::PendingNumericLiteral { .. } => {
                    nodes[index].type_id = Some(materialize_pending_literal(
                        items,
                        index,
                        destination,
                        context,
                        string_table,
                    )?);
                }
                ExpressionRpnItem::PendingGroup { .. } => {
                    return Err(pending_expression_item_bug("expression result typing").into());
                }
                ExpressionRpnItem::Operator { operator, .. } => {
                    frames.push(ResolutionFrame::CompleteOperator(index));
                    match nodes[index].children {
                        ExpressionChildren::Unary(child) => frames.push(ResolutionFrame::Visit {
                            index: child,
                            destination: if matches!(operator, Operator::Negate) {
                                destination
                            } else {
                                None
                            },
                        }),
                        ExpressionChildren::Binary { left, right } => {
                            let exponent = matches!(operator, Operator::Exponent);
                            if matches!(
                                destination,
                                Some(
                                    PendingLiteralDestination::Number { .. }
                                        | PendingLiteralDestination::Uint
                                )
                            ) && operator.numeric_operator().is_some()
                            {
                                // Uint exponents share the Uint domain, while a Dec exponent
                                // stays in profile Int. A Dec receiver cannot decide the
                                // exponent edge from its destination alone: a typed Uint
                                // base must route its exponent through Uint context so the
                                // power stays a Uint input to the exact Dec mixed rule.
                                // Resolve the base first, then route a lone or literal-only
                                // exponent through the peer path; a Dec base keeps Int.
                                let on_dec_exponent_edge = exponent
                                    && matches!(
                                        destination,
                                        Some(PendingLiteralDestination::Number { .. })
                                    );
                                if on_dec_exponent_edge
                                    && literal_peer_shape(nodes, items, left, right).is_some()
                                {
                                    frames.push(ResolutionFrame::VisitReceiverExponent {
                                        base: left,
                                        exponent: right,
                                    });
                                    frames.push(ResolutionFrame::Visit {
                                        index: left,
                                        destination,
                                    });
                                } else {
                                    frames.push(ResolutionFrame::Visit {
                                        index: right,
                                        destination: if on_dec_exponent_edge {
                                            None
                                        } else {
                                            destination
                                        },
                                    });
                                    frames.push(ResolutionFrame::Visit {
                                        index: left,
                                        destination,
                                    });
                                }
                            } else if operator_accepts_peer_literal(operator)
                                && let Some(peer_shape) =
                                    literal_peer_shape(nodes, items, left, right)
                            {
                                // Lone-literal peers resolve the peer side first and read its
                                // type; compound literal-only peers do the same so the compound
                                // frame can apply contextual arithmetic before folding.
                                match peer_shape {
                                    LiteralPeerShape::LoneLiteral { literal, peer } => {
                                        frames.push(ResolutionFrame::VisitPeerLiteral {
                                            index: literal,
                                            peer,
                                            exponent_edge: exponent && literal == right,
                                        });
                                        frames.push(ResolutionFrame::Visit {
                                            index: peer,
                                            destination: None,
                                        });
                                    }
                                    LiteralPeerShape::CompoundContextual { compound, peer } => {
                                        frames.push(ResolutionFrame::VisitCompoundPeer {
                                            index: compound,
                                            peer,
                                            exponent_edge: exponent && compound == right,
                                        });
                                        frames.push(ResolutionFrame::Visit {
                                            index: peer,
                                            destination: None,
                                        });
                                    }
                                }
                            } else {
                                frames.push(ResolutionFrame::Visit {
                                    index: right,
                                    destination: None,
                                });
                                frames.push(ResolutionFrame::Visit {
                                    index: left,
                                    destination: None,
                                });
                            }
                        }
                        ExpressionChildren::Leaf => {
                            return Err(CompilerError::compiler_error(
                                "Expression typing indexed an operator without children.",
                            )
                            .into());
                        }
                    }
                }
            },
            ResolutionFrame::VisitPeerLiteral {
                index,
                peer,
                exponent_edge,
            } => {
                let peer_type = resolved_type(nodes, peer)?;
                let peer_destination =
                    pending_literal_destination_for_type_id(peer_type, type_environment);
                let peer_is_dec = matches!(
                    peer_destination,
                    Some(PendingLiteralDestination::Number { .. })
                );
                // Baseline immediate-peer rule: any numeric peer selects a lone literal, except
                // a Dec peer on the power exponent edge which keeps profile Int. A Uint peer
                // types its exponent the same as any other operand.
                let destination = if exponent_edge && peer_is_dec {
                    None
                } else {
                    peer_destination
                };
                frames.push(ResolutionFrame::Visit { index, destination });
            }
            ResolutionFrame::VisitCompoundPeer {
                index,
                peer,
                exponent_edge,
            } => {
                let peer_type = resolved_type(nodes, peer)?;
                let peer_destination =
                    pending_literal_destination_for_type_id(peer_type, type_environment);
                // Compound literal-only subtrees take context only from a Dec or Uint peer,
                // except on the Dec power exponent edge. Uint exponents share the Uint domain,
                // so only a Dec peer drops context there. Other peers resolve naturally.
                let peer_supplies_arithmetic_context = matches!(
                    peer_destination,
                    Some(
                        PendingLiteralDestination::Number { .. } | PendingLiteralDestination::Uint
                    )
                );
                let on_dec_exponent_edge = exponent_edge
                    && matches!(
                        peer_destination,
                        Some(PendingLiteralDestination::Number { .. })
                    );
                let destination = if peer_supplies_arithmetic_context && !on_dec_exponent_edge {
                    peer_destination
                } else {
                    None
                };
                frames.push(ResolutionFrame::Visit { index, destination });
            }
            ResolutionFrame::VisitReceiverExponent { base, exponent } => {
                // The base resolved first under the receiver destination, so the
                // exponent edge follows the base type: a Uint base routes the
                // exponent through the Uint peer path, while any other base
                // (Dec included) keeps the profile-Int exponent edge. The
                // operator's own CompleteOperator frame is already queued
                // beneath this one, so only the children are scheduled here.
                // A resolved peer is never re-queued: the base visit queued by
                // the receiver arm already typed it, and re-entering it would
                // rescan the whole base subtree for every nested power.
                match literal_peer_shape(nodes, items, base, exponent) {
                    Some(LiteralPeerShape::LoneLiteral { literal, peer }) => {
                        frames.push(ResolutionFrame::VisitPeerLiteral {
                            index: literal,
                            peer,
                            exponent_edge: literal == exponent,
                        });
                        if nodes[peer].type_id.is_none() {
                            frames.push(ResolutionFrame::Visit {
                                index: peer,
                                destination: None,
                            });
                        }
                    }
                    Some(LiteralPeerShape::CompoundContextual { compound, peer }) => {
                        frames.push(ResolutionFrame::VisitCompoundPeer {
                            index: compound,
                            peer,
                            exponent_edge: compound == exponent,
                        });
                        if nodes[peer].type_id.is_none() {
                            frames.push(ResolutionFrame::Visit {
                                index: peer,
                                destination: None,
                            });
                        }
                    }
                    None => {
                        if nodes[exponent].type_id.is_none() {
                            frames.push(ResolutionFrame::Visit {
                                index: exponent,
                                destination: None,
                            });
                        }
                    }
                }
            }
            ResolutionFrame::CompleteOperator(index) => {
                let ExpressionRpnItem::Operator { operator, span } = &items[index] else {
                    return Err(CompilerError::compiler_error(
                        "Expression typing lost an indexed operator.",
                    )
                    .into());
                };
                let type_id = match nodes[index].children {
                    ExpressionChildren::Unary(child) => resolve_unary_operator_type(
                        operator,
                        resolved_type(nodes, child)?,
                        *span,
                        type_environment,
                    )?,
                    ExpressionChildren::Binary { left, right } => resolve_binary_operator_type(
                        resolved_type(nodes, left)?,
                        resolved_type(nodes, right)?,
                        operator,
                        *span,
                        type_environment,
                        path_fork,
                    )?,
                    ExpressionChildren::Leaf => {
                        return Err(CompilerError::compiler_error(
                            "Expression typing completed an operator without children.",
                        )
                        .into());
                    }
                };
                nodes[index].type_id = Some(type_id);
            }
        }
    }
    Ok(())
}

fn resolved_type(
    nodes: &[IndexedExpressionNode],
    index: usize,
) -> Result<TypeId, ExpressionTypingError> {
    nodes[index].type_id.ok_or_else(|| {
        CompilerError::compiler_error(
            "Expression typing consumed a child before resolving its type.",
        )
        .into()
    })
}

fn collect_numeric_failure_facts(
    items: &[ExpressionRpnItem],
    nodes: &[IndexedExpressionNode],
    type_environment: &TypeEnvironment,
) -> Result<ExpressionFailureFacts, ExpressionTypingError> {
    let mut facts = ExpressionFailureFacts::default();
    for (index, item) in items.iter().enumerate() {
        let ExpressionRpnItem::Operator { operator, span } = item else {
            continue;
        };
        let Some(numeric_operator) = operator.numeric_operator() else {
            continue;
        };
        let scalar = |index| {
            resolved_type(nodes, index)
                .map(|type_id| NumericScalar::from_type_id(type_id, type_environment))
        };
        // A binary operation records a witness only when both operands are numeric scalars.
        let operands = match nodes[index].children {
            ExpressionChildren::Unary(child) => scalar(child)?.map(|left| (left, None)),
            ExpressionChildren::Binary { left, right } => scalar(left)?
                .zip(scalar(right)?)
                .map(|(left, right)| (left, Some(right))),
            ExpressionChildren::Leaf => {
                return Err(CompilerError::compiler_error(
                    "Numeric failure recording found an operator without children.",
                )
                .into());
            }
        };
        if let Some((left, right)) = operands
            && let Some(domain) = scalar(index)?
        {
            facts.record_numeric_operation(numeric_operator, left, right, domain, *span);
        }
    }
    Ok(facts)
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

/// Classify a binary peer interaction for out-of-order resolution.
///
/// A lone `PendingNumericLiteral` always reads its peer through the baseline immediate-peer
/// rule: the peer side resolves first with no destination, so a compound literal-only peer
/// infers its natural type (for example Float) before the lone literal materialises. A
/// compound literal-only subtree against a typed operand resolves operand-first, letting the
/// compound frame apply Dec or Uint context before folding. Every other shape resolves
/// naturally.
fn literal_peer_shape(
    nodes: &[IndexedExpressionNode],
    items: &[ExpressionRpnItem],
    left: usize,
    right: usize,
) -> Option<LiteralPeerShape> {
    let left_lone = matches!(
        &items[left],
        ExpressionRpnItem::PendingNumericLiteral { .. }
    );
    let right_lone = matches!(
        &items[right],
        ExpressionRpnItem::PendingNumericLiteral { .. }
    );
    if left_lone != right_lone {
        let (literal, peer) = if left_lone {
            (left, right)
        } else {
            (right, left)
        };
        return Some(LiteralPeerShape::LoneLiteral { literal, peer });
    }
    if left_lone {
        return None;
    }
    if nodes[left].literal_only != nodes[right].literal_only {
        let (compound, peer) = if nodes[left].literal_only {
            (left, right)
        } else {
            (right, left)
        };
        return Some(LiteralPeerShape::CompoundContextual { compound, peer });
    }
    None
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
