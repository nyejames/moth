//! Scratch-to-store promotion invariants.
//!
//! WHAT: checks that unchanged roots append nothing and that reachable shared DAGs move once.
//! Dead side payloads are discarded, metadata replacement reuses committed ranges, and invalid
//! promotions fail before the store changes.
//! WHY: promotion is the only path from producer-local scratch into the phase store, so a failed
//! promotion must leave the prefix store untouched.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_types::{
    FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ImplicitFailureContributor, ImplicitFailureSource,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ProducedValues, ValueCatchBlock,
};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity,
};
pub(crate) fn constant() -> ConstValueId {
    ConstValueStore::default()
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .unwrap()
}
fn input<'p>(
    operation: CompletedOperation<CandidateEdge<'p>>,
) -> ScratchCompletedExpressionInput<'p> {
    ScratchCompletedExpressionInput {
        operation,
        success: SuccessShape::One(builtin_type_ids::BOOL),
        error_type: None,
        span: None,
        value_mode: ValueMode::ImmutableOwned,
        classification: ExpressionClassification {
            const_record_state: ConstRecordState::RuntimeValue,
            contains_regular_division: false,
        },
        failure: None,
        occurrence: None,
    }
}
pub(crate) fn seed(builder: &mut AstExpressionBuilder, value: ConstValueId) -> OwnedExpr {
    builder
        .append_completed(CompletedExpressionInput {
            operation: ExpressionOp::Constant(value),
            success: SuccessShape::One(builtin_type_ids::BOOL),
            error_type: None,
            span: None,
            value_mode: ValueMode::ImmutableOwned,
            classification: ExpressionClassification {
                const_record_state: ConstRecordState::RuntimeValue,
                contains_regular_division: false,
            },
            failure: None,
            occurrence: None,
        })
        .unwrap()
}

/// Non-empty cold and statement payloads must die with a losing candidate, not its prefix.
pub(crate) fn append_payload_graph<'p>(
    candidate: &mut ExpressionCandidate<'p>,
    original: OwnedExpr,
    value: ConstValueId,
) -> Result<ScratchExprId<'p>, ExpressionConstructionFailure> {
    let retained = CandidateEdge::Existing(candidate.retain(candidate.view(original)?)?);
    let mut cold = input(CompletedOperation::Constant(value));
    cold.failure = Some(ExpressionFailureFacts {
        implicit: vec![ImplicitFailureContributor {
            span: None,
            codes: &[],
            source: ImplicitFailureSource::PrivateCall(PathId::ROOT),
        }],
        checked_numeric_operation: true,
        ..ExpressionFailureFacts::default()
    });
    cold.occurrence = Some(OccurrenceFacts {
        provenance: SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "source-config",
            "discarded",
        )),
        diagnostic_spelling: None,
    });
    let cold = candidate.append_completed(cold)?;
    let block = candidate.append_completed(input(CompletedOperation::ValueBlock(
        ValueBlock::Catch(ValueCatchBlock {
            handled_value: original.id(),
            handler: FallibleHandling::Handler {
                error: None,
                body: vec![AstNode {
                    kind: NodeKind::ThenValue(ProducedValues {
                        expressions: vec![original.id()],
                        span: None,
                    }),
                    span: None,
                    scope: PathId::ROOT,
                }],
            },
            result_type_ids: vec![builtin_type_ids::BOOL],
        }),
    )))?;
    let dynamic = || OwnedRuntimeTemplateNode::Sequence {
        children: vec![OwnedRuntimeTemplateNode::DynamicExpression {
            expression: original.id(),
            span: None,
        }],
        span: None,
    };
    let template = candidate.append_completed(input(CompletedOperation::RuntimeTemplate(
        OwnedRuntimeTemplateHandoff {
            body: OwnedRuntimeTemplateBody::Render(dynamic()),
        },
    )))?;
    let slots = candidate.append_completed(input(CompletedOperation::RuntimeSlotApplication(
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: dynamic(),
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        },
    )))?;
    let handling = candidate.append_completed(input(CompletedOperation::HandleFallible {
        value: retained,
        handling: HandlingRecord {
            handling: FallibleExpressionHandling::Recover,
            propagation_span: None,
            error_type: Some(builtin_type_ids::STRING),
        },
    }))?;
    candidate.append_completed(input(CompletedOperation::Collection(vec![
        CandidateEdge::New(cold),
        CandidateEdge::New(block),
        CandidateEdge::New(template),
        CandidateEdge::New(slots),
        CandidateEdge::New(handling),
    ])))
}

fn storage_state(builder: &AstExpressionBuilder) -> [usize; 8] {
    let statistics = builder.store().storage_statistics();
    [
        statistics.rows,
        statistics.row_capacity,
        statistics.row_bytes,
        statistics.row_capacity_bytes,
        statistics.side_entries,
        statistics.side_capacity,
        statistics.side_bytes,
        statistics.side_capacity_bytes,
    ]
}
#[test]
fn unchanged_existing_root_appends_nothing_even_with_losing_scratch() {
    let value = constant();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let original = seed(&mut builder, value);
    let before = storage_state(&builder);
    let root = builder
        .construct(|candidate| {
            append_payload_graph(candidate, original, value)?;
            Ok(CandidateEdge::Existing(
                candidate.retain(candidate.view(original)?)?,
            ))
        })
        .unwrap();
    assert_eq!(root, original);
    assert_eq!(builder.len(), 1);
    assert_eq!(storage_state(&builder), before);
    assert!(builder.storage.value_blocks.is_empty());
    assert!(builder.storage.runtime_templates.is_empty());
    assert!(builder.storage.runtime_slots.is_empty());
    assert!(builder.storage.handling.is_empty());
    assert!(builder.storage.failures.is_empty());
    assert!(builder.storage.occurrences.is_empty());
    let failed = builder.construct(|candidate| {
        append_payload_graph(candidate, original, value)?;
        Err(invariant("candidate rejected after validation").into())
    });
    assert!(matches!(
        failed,
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(storage_state(&builder), before);
    assert!(matches!(
        builder.view_owned(original).unwrap().operation_id(),
        ExpressionOp::Constant(id) if id == value
    ));
}
#[test]
fn reachable_shared_dag_moves_once_and_discards_dead_side_payloads() {
    let value = constant();
    let mut builder = AstExpressionBuilder::new().unwrap();
    // Result ranges are committed before the candidate borrows the prefix.
    let pair = builder
        .append_types(vec![builtin_type_ids::BOOL; 2], None)
        .unwrap();
    let root = builder
        .construct(|candidate| {
            let leaf = candidate.append_completed(input(CompletedOperation::Constant(value)))?;
            let mut dead = input(CompletedOperation::Collection(vec![CandidateEdge::New(
                leaf,
            )]));
            dead.success = SuccessShape::Multiple(pair);
            dead.failure = Some(ExpressionFailureFacts::default());
            candidate.append_completed(dead)?;
            let root = candidate.append_completed(input(CompletedOperation::LogicalBinary {
                left: CandidateEdge::New(leaf),
                right: CandidateEdge::New(leaf),
                operation: LogicalOperator::And,
            }))?;
            Ok(CandidateEdge::New(root))
        })
        .unwrap();
    assert_eq!(builder.len(), 2);
    assert!(builder.storage.children.is_empty());
    assert_eq!(builder.storage.result_types.len(), 2);
    assert!(builder.storage.failures.is_empty());
    let ExpressionOperationView::LogicalBinary { left, right, .. } =
        builder.view_owned(root).unwrap().operation()
    else {
        panic!("logical root")
    };
    assert_eq!(left.id(), right.id());
    assert_eq!(left.id().index(), 0);
}
#[test]
fn owner_serial_and_index_fail_before_candidate_mutation() {
    let value = constant();
    let mut prefix = AstExpressionBuilder::new().unwrap();
    let local = seed(&mut prefix, value);
    let mut foreign = AstExpressionBuilder::new().unwrap();
    let other = seed(&mut foreign, value);
    let mut first = ExpressionCandidate {
        prefix: &prefix,
        serial: CandidateSerial(1),
        rows: Vec::new(),
    };
    let mut sibling = ExpressionCandidate {
        prefix: &prefix,
        serial: CandidateSerial(2),
        rows: Vec::new(),
    };
    assert!(first.retain(foreign.view_owned(other).unwrap()).is_err());
    let child = first
        .append_completed(input(CompletedOperation::Constant(value)))
        .unwrap();
    assert!(matches!(
        sibling.append_completed(input(CompletedOperation::LogicalNot {
            operand: CandidateEdge::New(child)
        })),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert!(sibling.rows.is_empty());
    let retained = first.retain(prefix.view_owned(local).unwrap()).unwrap();
    assert!(matches!(
        sibling.append_completed(input(CompletedOperation::LogicalNot {
            operand: CandidateEdge::Existing(retained)
        })),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert!(sibling.rows.is_empty());
    let missing = ScratchExprId {
        owner: prefix.storage.owner,
        candidate: CandidateSerial(2),
        index: 0,
        prefix: std::marker::PhantomData,
    };
    assert!(matches!(
        sibling.append_completed(input(CompletedOperation::LogicalNot {
            operand: CandidateEdge::New(missing)
        })),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert!(sibling.rows.is_empty());
}
#[test]
fn cycles_are_invariant_errors_without_prefix_mutation() {
    let value = constant();
    let prefix = AstExpressionBuilder::new().unwrap();
    let mut candidate = ExpressionCandidate {
        prefix: &prefix,
        serial: CandidateSerial(1),
        rows: Vec::new(),
    };
    let root = candidate
        .append_completed(input(CompletedOperation::Constant(value)))
        .unwrap();
    candidate.rows[0].as_mut().unwrap().operation = CompletedOperation::LogicalNot {
        operand: CandidateEdge::New(root),
    };
    assert!(matches!(
        candidate.prepare(CandidateEdge::New(root)),
        Err(ExpressionConstructionFailure::Invariant(_))
    ));
    assert_eq!(prefix.len(), 0);
}
#[test]
fn promotion_capacity_preflight_prevents_partial_prefix_append() {
    let value = constant();
    let mut builder = AstExpressionBuilder {
        limit: 6,
        ..AstExpressionBuilder::new().unwrap()
    };
    let original = seed(&mut builder, value);
    let before = storage_state(&builder);
    let result = builder.construct(|candidate| {
        let root = append_payload_graph(candidate, original, value)?;
        Ok(CandidateEdge::New(root))
    });
    assert!(matches!(
        result,
        Err(ExpressionConstructionFailure::Diagnostic(_))
    ));
    assert_eq!(builder.len(), 1);
    assert!(builder.storage.failures.is_empty());
    assert_eq!(storage_state(&builder), before);
    assert!(builder.storage.occurrences.is_empty());
    assert!(builder.storage.children.is_empty());
    assert!(builder.storage.value_blocks.is_empty());
    assert!(builder.storage.runtime_templates.is_empty());
    assert!(builder.storage.runtime_slots.is_empty());
    assert!(builder.storage.handling.is_empty());
}
#[test]
fn metadata_replacement_reuses_committed_side_and_result_ranges() {
    let value = constant();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let root = seed(&mut builder, value);
    let shape = SuccessShape::Multiple(
        builder
            .append_types(vec![builtin_type_ids::BOOL; 2], None)
            .unwrap(),
    );
    let mut initial = builder
        .view_owned(root)
        .unwrap()
        .to_completed_input(&builder)
        .unwrap();
    initial.success = shape;
    let root = builder.append_completed(initial).unwrap();
    let before = builder.storage.result_types.len();
    let mut changed = builder
        .view_owned(root)
        .unwrap()
        .to_completed_input(&builder)
        .unwrap();
    changed.failure = Some(ExpressionFailureFacts::default());
    let replacement = builder
        .construct(|candidate| Ok(CandidateEdge::New(candidate.append_replacement(changed)?)))
        .unwrap();
    assert_eq!(builder.storage.result_types.len(), before);
    assert_eq!(builder.view_owned(replacement).unwrap().success(), shape);
    assert!(builder.view_owned(root).unwrap().failure_facts().is_none());
    assert!(
        builder
            .view_owned(replacement)
            .unwrap()
            .failure_facts()
            .is_some()
    );
}
