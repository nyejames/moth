//! Expression result-type resolution for AST evaluation.
//!
//! Indexes authored RPN structure before materialising literals. Iterative resolution lets Dec
//! and exact fixed-scalar receivers and local peers reach literal-only arithmetic without
//! natural-type intermediates. The resolving owner annotates transient operator entries with
//! canonical facts for fixed-edge node construction. Summaries follow authored RPN order even
//! when peer typing runs first.

use super::evaluator::{
    PendingLiteralDestination, materialize_pending_literal, pending_expression_item_bug,
    pending_literal_destination_for_type_id,
};
use super::operator_policy::{resolve_binary_operator_type, resolve_unary_operator_type};
use super::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, ResolvedComparisonOperation, ResolvedNumericOperation, ResolvedOperation,
    ResolvedOperationKind,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::failure_facts::ExpressionFailureFacts;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ImplicitFailureContributor, ImplicitFailureSource,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidExpressionReason, OperatorOperandPosition, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericComparisonDomain, NumericOperator, comparison_domain, numeric_failure_codes,
    numeric_operation_cannot_fail, range_iteration_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(crate) struct ResolvedExpressionType {
    pub(crate) type_id: TypeId,
    pub(crate) failure_facts: ExpressionFailureFacts,
}

#[allow(
    clippy::too_many_arguments,
    reason = "result-type resolution keeps the output queue, span, literal destination, expression builder, const store, string table, type environment, and path fork as separate borrows"
)]
pub(crate) fn resolve_expression_result_type(
    output_queue: &mut [ExpressionRpnItem],
    expression_span: Option<SourceSpan>,
    literal_destination: Option<PendingLiteralDestination>,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
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
            expressions,
            const_values,
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

    // Peer selection can resolve the right subtree first. Summaries must still follow authored
    // RPN order; each contributor remains on the operation that introduces it.
    let failure_facts = collect_operation_summaries(output_queue);
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
            // Group markers are consumed by ordering, so result typing never sees one.
            ExpressionRpnItem::GroupOpen { .. }
            | ExpressionRpnItem::GroupClose { .. }
            | ExpressionRpnItem::PendingNumericBinding { .. }
            | ExpressionRpnItem::PendingNumericCall { .. }
            | ExpressionRpnItem::PendingNumericCast { .. }
            | ExpressionRpnItem::PendingNumericExpression { .. } => {
                return Err(pending_expression_item_bug("expression result typing").into());
            }
            ExpressionRpnItem::Operator { operator, span }
            | ExpressionRpnItem::ResolvedOperator(ResolvedOperation { operator, span, .. }) => {
                match operator.required_values() {
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
                }
            }
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
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
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
                    nodes[index].type_id = Some(
                        expressions
                            .view(*expression)?
                            .require_single(ValueReceiverContext::OperatorOperand)?,
                    );
                }
                ExpressionRpnItem::PendingNumericLiteral { .. } => {
                    nodes[index].type_id = Some(materialize_pending_literal(
                        items,
                        index,
                        destination,
                        expressions,
                        const_values,
                        string_table,
                    )?);
                }
                // Group markers are consumed by ordering, so typed evaluation never sees one.
                ExpressionRpnItem::GroupOpen { .. }
                | ExpressionRpnItem::GroupClose { .. }
                | ExpressionRpnItem::PendingNumericBinding { .. }
                | ExpressionRpnItem::PendingNumericCall { .. }
                | ExpressionRpnItem::PendingNumericCast { .. }
                | ExpressionRpnItem::PendingNumericExpression { .. } => {
                    // Construction-only items are replaced by body completion before typed
                    // evaluation; any later stage reports their presence as a broken invariant.
                    return Err(pending_expression_item_bug("expression result typing").into());
                }
                ExpressionRpnItem::Operator { operator, .. }
                | ExpressionRpnItem::ResolvedOperator(ResolvedOperation { operator, .. }) => {
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
                            // Any eligible exact numeric destination types raw literal leaves
                            // through unresolved arithmetic and grouping. A named operand
                            // keeps its identity; a completed result is never retagged.
                            if destination.is_some() && operator.numeric_operator().is_some() {
                                // A Dec exponent stays in I32: the exact exponent contract is
                                // `DecN ^ I32 -> DecN`. A typed fixed-integer base routes its
                                // exponent through that same integer context so the power stays a
                                // fixed-integer input to the exact Dec mixed rule. Resolve the
                                // base first, then route a lone or literal-only exponent through
                                // the peer path; a Dec base keeps the I32 exponent edge.
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
                // a Dec peer on the power exponent edge which keeps the I32 exponent contract.
                // Every other numeric peer types its exponent the same as any other operand.
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
                // Compound literal-only subtrees take context from any exact numeric peer,
                // except on the Dec power exponent edge, which keeps the I32 exponent contract.
                // Other peers resolve naturally.
                let peer_supplies_arithmetic_context = peer_destination.is_some();
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
                // exponent edge follows the base type: a fixed-integer base routes the
                // exponent through the fixed-integer peer path, while any other base
                // (Dec included) keeps the I32 exponent edge. The
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
                if let ExpressionRpnItem::ResolvedOperator(operation) = &items[index] {
                    nodes[index].type_id = Some(operation.result_type);
                    continue;
                }
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
                let (left_type, right_type) = match nodes[index].children {
                    ExpressionChildren::Unary(child) => (resolved_type(nodes, child)?, None),
                    ExpressionChildren::Binary { left, right } => (
                        resolved_type(nodes, left)?,
                        Some(resolved_type(nodes, right)?),
                    ),
                    ExpressionChildren::Leaf => unreachable!("operator children validated above"),
                };
                let operation = resolved_operation(
                    *operator,
                    *span,
                    type_id,
                    left_type,
                    right_type,
                    type_environment,
                )?;
                items[index] = ExpressionRpnItem::ResolvedOperator(operation);
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

fn collect_operation_summaries(items: &[ExpressionRpnItem]) -> ExpressionFailureFacts {
    let mut facts = ExpressionFailureFacts::default();
    for item in items {
        if let ExpressionRpnItem::ResolvedOperator(operation) = item {
            facts.checked_numeric_operation |=
                matches!(operation.kind, ResolvedOperationKind::Numeric(_));
            if let Some(contributor) = operation.failure {
                facts.summary.record_implicit(contributor);
            }
        }
    }
    facts
}

/// Captures selected numeric semantics while canonical operand and result types are available.
pub(crate) fn resolved_operation(
    operator: Operator,
    span: Option<SourceSpan>,
    result_type: TypeId,
    left_type: TypeId,
    right_type: Option<TypeId>,
    environment: &TypeEnvironment,
) -> Result<ResolvedOperation, ExpressionTypingError> {
    let left = NumericScalar::from_type_id(left_type, environment);
    let right = right_type.and_then(|type_id| NumericScalar::from_type_id(type_id, environment));
    let mut failure = None;
    let kind = if let Some(numeric_operator) = operator.numeric_operator() {
        let left_domain = left.ok_or_else(|| {
            CompilerError::compiler_error("Resolved arithmetic lost its canonical operand domain.")
        })?;
        let domain = NumericScalar::from_type_id(result_type, environment).ok_or_else(|| {
            CompilerError::compiler_error("Resolved arithmetic lost its canonical result domain.")
        })?;
        let discharged =
            numeric_operation_cannot_fail(numeric_operator, left_domain, right, domain);
        let codes = numeric_failure_codes(numeric_operator, domain);
        if !discharged && !codes.is_empty() {
            failure = Some(ImplicitFailureContributor {
                span,
                codes,
                source: ImplicitFailureSource::NumericOperation,
            });
        }
        let exponent_edge = numeric_operator == NumericOperator::Power
            && matches!(domain, NumericScalar::Number(_));
        ResolvedOperationKind::Numeric(ResolvedNumericOperation {
            operator: numeric_operator,
            left_domain,
            right_domain: right,
            domain,
            left_conversion: (left_domain != domain).then_some(domain),
            right_conversion: right
                .filter(|right| *right != domain && !exponent_edge)
                .map(|_| domain),
            discharged,
        })
    } else if matches!(operator, Operator::Range) {
        let domain = left
            .zip(right)
            .and_then(|(left, right)| range_iteration_domain(left, right, None))
            .ok_or_else(|| {
                CompilerError::compiler_error("Resolved range lost its iteration domain.")
            })?;
        ResolvedOperationKind::Range { domain }
    } else if let Some((left_domain, right_domain, domain)) =
        left.zip(right).and_then(|(left, right)| {
            comparison_domain(left, right).map(|domain| (left, right, domain))
        })
    {
        let conversion_domain = match domain {
            NumericComparisonDomain::Number(scale) => Some(NumericScalar::Number(scale)),
            NumericComparisonDomain::ExactInteger | NumericComparisonDomain::BinaryFloat => None,
        };
        ResolvedOperationKind::Comparison(ResolvedComparisonOperation {
            left_domain,
            right_domain,
            domain,
            left_conversion: conversion_domain.filter(|domain| *domain != left_domain),
            right_conversion: conversion_domain.filter(|domain| *domain != right_domain),
        })
    } else {
        ResolvedOperationKind::NonNumeric
    };
    Ok(ResolvedOperation {
        operator,
        span,
        result_type,
        left_type,
        right_type,
        kind,
        failure,
    })
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
/// infers its natural type (for example F64) before the lone literal materialises. A
/// compound literal-only subtree against a typed operand resolves operand-first, letting the
/// compound frame apply Dec or exact fixed-scalar context before folding. Every other shape
/// resolves naturally.
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
