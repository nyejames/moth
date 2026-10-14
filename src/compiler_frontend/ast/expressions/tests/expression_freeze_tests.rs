//! Hidden graph ownership invariants at the single consuming freeze boundary.
use super::*;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallPassingMode};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ConstructorField, ExprId, ExpressionOp, HandlingId, HandlingRecord,
    MapEntry, OwnedExpr, PlaceId, PlaceProjection, PlaceRecord, ResolvedArgument, ResolvedCall,
    ResolvedCallableTarget, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleExpressionHandling;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity,
};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, runtime_template_handoff_expr,
};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

#[derive(Default)]
struct Roots {
    expressions: Vec<ExprId>,
    places: Vec<PlaceId>,
}
impl ExpressionFreezeRoots for Roots {
    fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId)) {
        for &id in &self.expressions {
            visitor(id);
        }
    }
    fn visit_expression_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut ExprId)) {
        for id in &mut self.expressions {
            visitor(id);
        }
    }
    fn visit_place_ids(&self, visitor: &mut dyn FnMut(PlaceId)) {
        for &id in &self.places {
            visitor(id);
        }
    }
    fn visit_place_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut PlaceId)) {
        for id in &mut self.places {
            visitor(id);
        }
    }
}

fn row(builder: &mut AstExpressionBuilder, operation: ExpressionOp<OwnedExpr>) -> OwnedExpr {
    builder
        .append_with_success_shape(
            operation,
            None,
            SuccessShape::One(builtin_type_ids::BOOL),
            ValueMode::ImmutableOwned,
        )
        .expect("checked row")
}

fn template(builder: &mut AstExpressionBuilder) -> OwnedExpr {
    // Freeze rejects the operation before consulting TIR: its authority has already been released.
    let fact = builder
        .append_template(
            Template {
                tir_reference: TemplateTirReference {
                    root: TemplateIrId::new(0),
                    phase: TemplateTirPhase::Parsed,
                    context: TemplateViewContext::default(),
                },
                span: None,
            },
            None,
        )
        .expect("build-only template payload");
    row(builder, ExpressionOp::Template(fact))
}

fn assert_exact_capacity(store: &ExpressionStore) {
    let statistics = store.storage_statistics();
    assert_eq!(statistics.rows, statistics.row_capacity);
    assert_eq!(statistics.row_bytes, statistics.row_capacity_bytes);
    assert_eq!(statistics.side_entries, statistics.side_capacity);
    assert_eq!(statistics.side_bytes, statistics.side_capacity_bytes);
}
#[test]
fn freeze_removes_unreachable_rows_and_template_dependent_sides() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let discarded = template(&mut builder);
    let discarded_children = builder
        .append_children(vec![discarded], None)
        .expect("discarded range");
    row(&mut builder, ExpressionOp::Collection(discarded_children));
    let live = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("literal");
    let ExpressionOp::Constant(value) = builder.view_owned(live).unwrap().operation_id() else {
        panic!("retained constant")
    };
    // A completed validation graph retains real handler and handoff payloads until its last
    // reader omits that root. Freeze must release their allocations, not merely their IDs.
    builder
        .construct(|candidate| {
            Ok(CandidateEdge::New(
                super::super::scratch::tests::append_payload_graph(candidate, live, value)?,
            ))
        })
        .expect("discarded validation graph");
    let before = builder.store().storage_statistics();
    assert!(before.row_capacity_bytes > before.row_bytes);
    assert!(before.side_capacity_bytes > 0);
    assert_eq!(builder.storage.value_blocks.len(), 1);
    assert_eq!(builder.storage.runtime_templates.len(), 1);
    assert_eq!(builder.storage.runtime_slots.len(), 1);
    assert_eq!(builder.storage.handling.len(), 1);
    assert_eq!(builder.storage.failures[0].implicit.len(), 1);
    assert_eq!(
        builder.storage.occurrences[0]
            .provenance
            .source_config_member_names()
            .collect::<Vec<_>>(),
        ["discarded"]
    );
    let mut roots = Roots {
        expressions: vec![live.id()],
        places: Vec::new(),
    };
    let store = builder
        .freeze(&mut roots)
        .expect("unreachable template is removed");
    assert_eq!(store.len(), 1);
    assert_eq!(roots.expressions, vec![ExprId::at(0)]);
    assert!(store.templates.is_empty());
    assert!(store.children.is_empty());
    assert_exact_capacity(&store);
    assert_eq!(store.storage_statistics().side_capacity_bytes, 0);
    assert!(store.value_blocks.is_empty());
    assert!(store.runtime_templates.is_empty());
    assert!(store.runtime_slots.is_empty());
    assert!(store.handling.is_empty());
    assert!(store.failures.is_empty());
    assert!(store.occurrences.is_empty());
    assert!(matches!(store.rows[0].operation, ExpressionOp::Constant(id) if id == value));
    assert!(matches!(
        constants.payload(value),
        Some(ConstValuePayload::Bool(true))
    ));
}

#[test]
fn mostly_folded_producers_release_payloads_and_preserve_same_typed_slot_values() {
    // The unused generic's validation graph leaves freeze roots, while same-typed folded
    // result slots deliberately have different values to expose a remapping swap.
    let (ast, paths, strings) = parse_single_file_ast(
        "identity type T |value T| -> T:\n\
             return value\n\
         ;\n\
         pair || -> I32, I32:\n\
             return ((1 + 2) * (3 + 4)) + (5 * 6), ((9 - 3) * 2) + (7 - 4)\n\
         ;\n",
    );
    let body = function_body_by_name(&ast, &paths, &strings, "pair");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("folded pair return")
    };
    assert_eq!(values.len(), 2);
    for (&root, expected) in values.iter().zip([51, 15]) {
        let expression = ast.expressions.view(root).expect("retained result slot");
        assert_eq!(
            expression.success_types(),
            &[builtin_type_ids::fixed_scalar(FixedScalar::I32)]
        );
        let ExpressionOp::Constant(value) = expression.operation_id() else {
            panic!("fully folded producer")
        };
        assert!(matches!(
            ast.const_values.payload(value),
            Some(ConstValuePayload::FixedScalar(value))
                if *value == FixedScalarValue::signed(FixedScalar::I32, expected).unwrap()
        ));
    }
    assert_eq!(ast.expressions.len(), 2);
    assert!(ast.expressions.numeric.is_empty());
    assert!(ast.expressions.children.is_empty());
    assert_exact_capacity(&ast.expressions);
}

#[test]
fn repeatedly_replaced_roots_release_old_facts_and_keep_root_order() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let first = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .unwrap();
    let second = builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .unwrap();
    let mut latest = first;
    for count in 0..32 {
        let mut replacement = builder
            .view_owned(latest)
            .unwrap()
            .to_completed_input(&builder)
            .unwrap();
        replacement.failure = Some(ExpressionFailureFacts {
            safety_call_count: count % 3,
            authored_cast: count == 31,
            ..ExpressionFailureFacts::default()
        });
        replacement.occurrence = Some(OccurrenceFacts {
            provenance: SyntheticInterfaceProvenance::single(
                SyntheticInterfaceMemberIdentity::new(
                    SyntheticInterfaceClass::ProjectContext,
                    "source-config",
                    format!("replacement_{count}"),
                ),
            ),
            diagnostic_spelling: None,
        });
        latest = builder.commit_replacement(replacement).unwrap();
    }
    let before = builder.store().storage_statistics();
    assert_eq!(before.rows, 34);
    assert_eq!(builder.storage.failures.len(), 32);
    assert_eq!(builder.storage.occurrences.len(), 32);
    let mut roots = Roots {
        expressions: vec![latest.id(), second.id(), latest.id()],
        places: Vec::new(),
    };
    let store = builder.freeze(&mut roots).expect("replacement freeze");
    assert_eq!(
        roots.expressions,
        [ExprId::at(1), ExprId::at(0), ExprId::at(1)]
    );
    assert_eq!(store.len(), 2);
    assert_eq!(store.failures.len(), 1);
    assert!(store.failures[0].authored_cast);
    assert_eq!(store.failures[0].safety_call_count, 1);
    assert_eq!(store.occurrences.len(), 1);
    assert_eq!(
        store.occurrences[0]
            .provenance
            .source_config_member_names()
            .collect::<Vec<_>>(),
        ["replacement_31"]
    );
    for (&root, expected) in roots.expressions.iter().zip([true, false, true]) {
        let ExpressionOp::Constant(value) = store.view(root).unwrap().operation_id() else {
            panic!("constant replacement")
        };
        assert!(
            matches!(constants.payload(value), Some(ConstValuePayload::Bool(value)) if *value == expected)
        );
    }
    assert_exact_capacity(&store);
    assert!(store.storage_statistics().row_capacity_bytes < before.row_capacity_bytes);
    assert!(store.storage_statistics().side_capacity_bytes < before.side_capacity_bytes);
}

#[test]
fn reachable_template_fails_without_rewriting_caller_roots() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let root = template(&mut builder);
    let values = builder
        .append_children(vec![root], None)
        .expect("template edge");
    let parent = row(&mut builder, ExpressionOp::Collection(values));
    let mut roots = Roots {
        expressions: vec![parent.id()],
        places: Vec::new(),
    };
    assert!(builder.freeze(&mut roots).is_err());
    assert_eq!(roots.expressions, vec![parent.id()]);
}

#[test]
fn shared_ranges_and_shared_dag_edges_are_remapped_once() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("dead prefix");
    let leaf = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("shared leaf");
    builder
        .append_children(vec![leaf], None)
        .expect("unused side prefix");
    let children = builder
        .append_children(vec![leaf, leaf], None)
        .expect("shared range");
    let left = row(&mut builder, ExpressionOp::Collection(children));
    let right = row(&mut builder, ExpressionOp::Collection(children));
    let map = builder
        .append_map_entries(
            vec![MapEntry {
                key: left,
                value: right,
            }],
            None,
        )
        .expect("map edges");
    let root = row(&mut builder, ExpressionOp::Map(map));
    let mut roots = Roots {
        expressions: vec![root.id(), right.id(), root.id()],
        places: Vec::new(),
    };
    let store = builder.freeze(&mut roots).expect("shared graph freeze");
    assert_eq!(store.len(), 4);
    assert_eq!(store.children.as_slice(), &[ExprId::at(0), ExprId::at(0)]);
    assert_eq!(store.map_entries[0].key, ExprId::at(1));
    assert_eq!(store.map_entries[0].value, ExprId::at(2));
    assert_eq!(
        roots.expressions,
        vec![ExprId::at(3), ExprId::at(2), ExprId::at(3)]
    );
    for expected in [store.view(ExprId::at(1)), store.view(ExprId::at(2))] {
        let ExpressionOp::Collection(range) = expected.expect("shared range").operation_id() else {
            panic!("collection range")
        };
        assert_eq!((range.start, range.len), (0, 2));
    }
}

#[test]
fn external_assignment_place_retains_and_remaps_its_index_without_copy_row() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("dead row");
    let index = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("index leaf");
    let unused = builder
        .append_place_projections(Vec::new(), None)
        .expect("empty range");
    builder
        .append_place(
            PlaceRecord {
                root: PathId::ROOT,
                root_type_id: builtin_type_ids::BOOL,
                projections: unused,
                type_id: builtin_type_ids::BOOL,
                value_mode: ValueMode::ImmutableOwned,
                span: None,
            },
            None,
        )
        .expect("unused place");
    let projections = builder
        .append_place_projections(
            vec![PlaceProjection::Index {
                index,
                result_type: builtin_type_ids::BOOL,
            }],
            None,
        )
        .expect("projection");
    let place = builder
        .append_place(
            PlaceRecord {
                root: PathId::ROOT,
                root_type_id: builtin_type_ids::BOOL,
                projections,
                type_id: builtin_type_ids::BOOL,
                value_mode: ValueMode::ImmutableOwned,
                span: None,
            },
            None,
        )
        .expect("assignment target");
    let mut roots = Roots {
        expressions: Vec::new(),
        places: vec![place, place],
    };
    let store = builder.freeze(&mut roots).expect("place-only roots");
    assert_eq!(store.len(), 1);
    assert_eq!(store.places.len(), 1);
    assert_eq!(roots.places, vec![PlaceId::at(0), PlaceId::at(0)]);
    assert!(
        matches!(store.projections[0], PlaceProjection::Index { index, .. } if index == ExprId::at(0))
    );
}

#[test]
fn runtime_handoff_leaves_and_multiple_result_ranges_survive_remap() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("dead prefix");
    builder
        .append_types(vec![builtin_type_ids::STRING], None)
        .expect("unused type prefix");
    let leaf = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("dynamic leaf");
    let shape = builder
        .success_from_slots(vec![builtin_type_ids::BOOL, builtin_type_ids::BOOL], None)
        .expect("multiple shape");
    let multiple = builder
        .append_with_success_shape(
            ExpressionOp::LogicalNot { operand: leaf },
            None,
            shape,
            ValueMode::ImmutableOwned,
        )
        .expect("multiple producer");
    let payload = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: vec![
                OwnedRuntimeTemplateNode::DynamicExpression {
                    expression: multiple.id(),
                    span: None,
                },
                OwnedRuntimeTemplateNode::DynamicExpression {
                    expression: leaf.id(),
                    span: None,
                },
            ],
            span: None,
        }),
    };
    let root =
        runtime_template_handoff_expr(&mut builder, payload, None, ValueMode::ImmutableOwned);
    let mut roots = Roots {
        expressions: vec![root],
        places: Vec::new(),
    };
    let store = builder.freeze(&mut roots).expect("handoff freeze");
    assert_eq!(
        store.result_types.as_slice(),
        &[builtin_type_ids::BOOL, builtin_type_ids::BOOL]
    );
    assert_eq!(
        store
            .view(ExprId::at(1))
            .expect("multiple producer")
            .success_types(),
        &[builtin_type_ids::BOOL, builtin_type_ids::BOOL]
    );
    let mut leaves = Vec::new();
    store.runtime_templates[0].visit_expression_ids(&mut |id| leaves.push(id));
    assert_eq!(leaves, vec![ExprId::at(1), ExprId::at(0)]);
    assert_eq!(roots.expressions, [ExprId::at(2)]);
    assert_exact_capacity(&store);
}

#[test]
fn empty_roots_release_every_typed_owner_and_allow_empty_ranges() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let children = builder
        .append_children(Vec::new(), None)
        .expect("empty children");
    let root = row(&mut builder, ExpressionOp::Collection(children));
    let mut roots = Roots {
        expressions: vec![root.id()],
        places: Vec::new(),
    };
    let store = builder
        .freeze(&mut roots)
        .expect("empty collection retained");
    assert!(
        matches!(store.rows[0].operation, ExpressionOp::Collection(range) if range.start == 0 && range.len == 0)
    );
    let mut builder = AstExpressionBuilder::new().unwrap();
    template(&mut builder);
    let store = builder
        .freeze(&mut Roots::default())
        .expect("no surviving graph");
    assert_eq!(store.len(), 0);
    assert_eq!(store.storage_statistics().side_entries, 0);
    assert_exact_capacity(&store);
    assert_eq!(store.storage_statistics().row_capacity_bytes, 0);
    assert_eq!(store.storage_statistics().side_capacity_bytes, 0);
}

#[test]
fn call_arguments_constructor_fields_and_handling_side_ids_move_once() {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("dead prefix");
    let leaf = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("argument");
    let make_argument = |value| ResolvedArgument {
        value,
        target_param: None,
        parameter_slot: None,
        access_mode: CallAccessMode::Shared,
        passing_mode: CallPassingMode::Shared,
        span: None,
        target_span: None,
        marker_span: None,
        following_separator_span: None,
    };
    builder
        .append_arguments(vec![make_argument(leaf)], None)
        .expect("unused argument prefix");
    let arguments = builder
        .append_arguments(vec![make_argument(leaf)], None)
        .expect("arguments");
    let make_handling = || HandlingRecord {
        handling: FallibleExpressionHandling::Propagate,
        propagation_span: None,
        error_type: Some(builtin_type_ids::STRING),
    };
    builder
        .append_handling(make_handling(), None)
        .expect("unused handling prefix");
    let handling = builder
        .append_handling(make_handling(), None)
        .expect("call handling");
    let make_call = |arguments| ResolvedCall {
        target: ResolvedCallableTarget::Function(PathId::ROOT),
        arguments,
        handling: Some(handling),
        requires_external_float_validation: true,
        error_type: Some(builtin_type_ids::STRING),
        span: None,
    };
    builder
        .append_call(make_call(arguments), None)
        .expect("unused call prefix");
    let call = builder
        .append_call(make_call(arguments), None)
        .expect("call");
    let call_root = row(
        &mut builder,
        ExpressionOp::MethodCall {
            receiver: leaf,
            call,
        },
    );
    builder
        .append_constructor_fields(
            vec![ConstructorField {
                name: PathId::ROOT,
                slot: 0,
                value: leaf,
            }],
            None,
        )
        .expect("unused field prefix");
    let fields = builder
        .append_constructor_fields(
            vec![ConstructorField {
                name: PathId::ROOT,
                slot: 2,
                value: call_root,
            }],
            None,
        )
        .expect("choice fields");
    let choice = row(&mut builder, ExpressionOp::Choice { tag: 7, fields });
    let mut roots = Roots {
        expressions: vec![choice.id()],
        places: Vec::new(),
    };
    let store = builder.freeze(&mut roots).expect("typed sides frozen");
    assert_eq!(store.arguments.len(), 1);
    assert_eq!(store.arguments[0].value, ExprId::at(0));
    assert_eq!(store.arguments[0].parameter_slot, None);
    assert_eq!(store.calls.len(), 1);
    assert_eq!(store.calls[0].arguments.start, 0);
    assert_eq!(store.calls[0].handling, Some(HandlingId::at(0)));
    assert!(store.calls[0].requires_external_float_validation);
    assert_eq!(store.handling.len(), 1);
    assert_eq!(store.handling[0].error_type, Some(builtin_type_ids::STRING));
    assert_eq!(store.constructor_fields[0].slot, 2);
    assert_eq!(store.constructor_fields[0].value, ExprId::at(1));
    assert!(
        matches!(store.rows[2].operation, ExpressionOp::Choice { tag: 7, fields } if fields.start == 0 && fields.len == 1)
    );
    assert_exact_capacity(&store);
}

#[test]
fn malformed_root_returns_compiler_error_before_rewriting_roots() {
    let builder = AstExpressionBuilder::new().unwrap();
    let mut roots = Roots {
        expressions: vec![ExprId::at(99)],
        places: Vec::new(),
    };
    assert!(builder.freeze(&mut roots).is_err());
    assert_eq!(roots.expressions, vec![ExprId::at(99)]);
}

#[cfg(debug_assertions)]
#[test]
fn parity_check_rejects_missing_mutable_root_visit() {
    struct BrokenRoots(ExprId);
    impl ExpressionFreezeRoots for BrokenRoots {
        fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId)) {
            visitor(self.0);
        }
        fn visit_expression_ids_mut(&mut self, _: &mut dyn FnMut(&mut ExprId)) {}
        fn visit_place_ids(&self, _: &mut dyn FnMut(PlaceId)) {}
        fn visit_place_ids_mut(&mut self, _: &mut dyn FnMut(&mut PlaceId)) {}
    }
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("dead prefix");
    let leaf = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("retained root");
    assert!(builder.freeze(&mut BrokenRoots(leaf.id())).is_err());
}
