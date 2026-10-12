//! Known-shape identity matching and shared-DAG traversal complexity invariants.

#![cfg(feature = "benchmark_counters")]

use crate::compiler_frontend::ast::numeric_construction::{
    NumericConstructionState, NumericFamily, NumericPin,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeBinding, KnownShapeBindingRange, KnownShapeId, pin_numeric_shape,
    shapes_join_refused, union_shape_classes,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};

fn declared_range(state: &NumericConstructionState, shape: KnownShapeId) -> KnownShapeBindingRange {
    let KnownShape::DeclaredSubstitution { bindings, .. } = state.shape(shape) else {
        panic!("test shape must be a declared substitution");
    };
    bindings
}

#[test]
#[cfg(feature = "benchmark_counters")]
fn declared_binding_matching_scales_with_parameter_count() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };

    let _guard = lock_counter_test();
    let environment = TypeEnvironment::new();
    let mut measurements = Vec::new();
    for count in [64usize, 128] {
        let mut state = NumericConstructionState::new();
        let mut left_edges = Vec::new();
        let mut right_edges = Vec::new();
        for index in 0..count {
            let parameter = GenericParameterId(index as u32);
            for edges in [&mut left_edges, &mut right_edges] {
                let class = state.create_class().expect("numeric parameter class");
                state.note_class_family(class, NumericFamily::Whole);
                let shape = state
                    .intern_shape(KnownShape::Numeric(class))
                    .expect("parameter shape");
                edges.push(KnownShapeBinding { parameter, shape });
            }
        }
        // The source edge order is deliberately different on both sides. Identity, not
        // declaration position, must own matching after canonicalisation.
        left_edges.reverse();
        right_edges.rotate_left(count / 3);
        let left = state
            .intern_declared_substitution(TypeId(0), &left_edges)
            .expect("left envelope");
        let right = state
            .intern_declared_substitution(TypeId(0), &right_edges)
            .expect("right envelope");
        let right_range = declared_range(&state, right);

        reset_ast_counters();
        for edge in &right_edges {
            assert_eq!(
                state.declared_binding_shape(right_range, edge.parameter),
                Some(edge.shape)
            );
        }
        assert!(
            state
                .declared_binding_shape(right_range, GenericParameterId(count as u32))
                .is_none()
        );
        let lookup = test_read_ast_counter(AstCounter::NumericDeclaredBindingComparisons);
        let lookup_bound = (count + 1) * (count.ilog2() as usize + 2);

        reset_ast_counters();
        assert!(!shapes_join_refused(&mut state, &environment, left, right));
        let refusal = test_read_ast_counter(AstCounter::NumericDeclaredBindingComparisons);

        reset_ast_counters();
        assert!(union_shape_classes(&mut state, &environment, left, right));
        let union = test_read_ast_counter(AstCounter::NumericDeclaredBindingComparisons);
        eprintln!(
            "declared bindings: parameters={count}, lookup={lookup}, lookup_bound={lookup_bound}, refusal={refusal}, union={union}, merge_bound={count}"
        );
        measurements.push((count, lookup, lookup_bound, refusal, union));
    }
    // Collect both sizes before asserting so a failing baseline still reports growth.
    for (count, lookup, lookup_bound, refusal, union) in measurements {
        assert!(
            lookup <= lookup_bound,
            "{count} individual lookups compared {lookup} bindings, bound {lookup_bound}"
        );
        assert!(
            refusal <= count,
            "{count} refusal edges compared {refusal} bindings"
        );
        assert!(
            union <= count,
            "{count} union edges compared {union} bindings"
        );
    }
}

#[test]
#[cfg(feature = "benchmark_counters")]
fn numeric_shape_pinning_visits_shared_nodes_once_per_traversal() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };

    let _guard = lock_counter_test();
    let mut measurements = Vec::new();
    for depth in [8usize, 12, 16] {
        let mut state = NumericConstructionState::new();
        let class = state.create_class().expect("numeric leaf class");
        state.note_class_family(class, NumericFamily::Whole);
        let mut shape = state
            .intern_shape(KnownShape::Numeric(class))
            .expect("numeric leaf shape");
        for _ in 0..depth {
            let edges = [
                KnownShapeBinding {
                    parameter: GenericParameterId(0),
                    shape,
                },
                KnownShapeBinding {
                    parameter: GenericParameterId(1),
                    shape,
                },
            ];
            shape = state
                .intern_declared_substitution(TypeId(0), &edges)
                .expect("shared predecessor envelope");
        }
        for traversal in 0..2 {
            reset_ast_counters();
            pin_numeric_shape(&mut state, shape, NumericPin::Bound);
            let visits = test_read_ast_counter(AstCounter::NumericShapePinNodesVisited);
            eprintln!(
                "shared shape pinning: depth={depth}, traversal={traversal}, visits={visits}, bound={}",
                depth + 1
            );
            measurements.push((depth, traversal, visits));
        }
    }
    for (depth, traversal, visits) in measurements {
        assert_eq!(
            visits,
            depth + 1,
            "depth {depth}, traversal {traversal} must visit each shared node once"
        );
    }
}

fn shared_unary_declared_map(
    environment: &mut TypeEnvironment,
) -> (TypeId, [GenericParameterId; 2]) {
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    let mut strings = StringTable::new();
    let key = environment.register_synthetic_generic_parameter(strings.intern("Key"));
    let value = environment.register_synthetic_generic_parameter(strings.intern("Value"));
    let parameters = [
        environment
            .bare_generic_parameter(key)
            .expect("declared key parameter"),
        environment
            .bare_generic_parameter(value)
            .expect("declared value parameter"),
    ];
    (environment.intern_map(key, value), parameters)
}

fn shared_unary_dag(
    state: &mut NumericConstructionState,
    declared_type: TypeId,
    parameters: [GenericParameterId; 2],
    leaf: KnownShapeId,
    depth: usize,
) -> KnownShapeId {
    let mut shape = leaf;
    for level in 0..depth {
        shape = if level % 2 == 0 {
            state
                .intern_shape(KnownShape::Map {
                    key: shape,
                    value: shape,
                })
                .expect("shared map envelope")
        } else {
            let edges = parameters.map(|parameter| KnownShapeBinding { parameter, shape });
            state
                .intern_declared_substitution(declared_type, &edges)
                .expect("shared declared envelope")
        };
    }
    shape
}

#[test]
fn shared_unary_materialisation_uses_current_receivers_and_canonical_types() {
    use crate::compiler_frontend::ast::numeric_known_shape::materialise_known_shape;
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let mut environment = TypeEnvironment::new();
    let (declared_type, parameters) = shared_unary_declared_map(&mut environment);
    let mut state = NumericConstructionState::new();
    let class = state.create_class().expect("numeric leaf class");
    state.note_class_family(class, NumericFamily::Whole);
    let leaf = state
        .intern_shape(KnownShape::Numeric(class))
        .expect("numeric leaf");
    let depth = 10;
    let root = shared_unary_dag(&mut state, declared_type, parameters, leaf, depth);
    let mut expected = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    for _ in 0..depth {
        expected = environment.intern_map(expected, expected);
    }
    assert_eq!(
        materialise_known_shape(&mut state, &mut environment, root)
            .expect("default materialisation"),
        expected
    );

    let receiver = builtin_type_ids::fixed_scalar(FixedScalar::U64);
    state.add_receiver_witness(class, receiver, None, &environment);
    expected = receiver;
    for _ in 0..depth {
        expected = environment.intern_map(expected, expected);
    }
    assert_eq!(
        materialise_known_shape(&mut state, &mut environment, root)
            .expect("received materialisation"),
        expected,
        "a new invocation must not reuse the earlier family default"
    );
}

#[test]
fn shared_unary_projection_preserves_child_identity_and_linear_output_size() {
    use crate::compiler_frontend::ast::numeric_known_shape::{
        exact_shape_type, project_exact_shape,
    };
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;
    use std::collections::HashSet;

    for depth in [8usize, 12, 16] {
        let mut environment = TypeEnvironment::new();
        let (declared_type, parameters) = shared_unary_declared_map(&mut environment);
        let mut state = NumericConstructionState::new();
        let class = state.create_class().expect("numeric leaf class");
        let leaf = state
            .intern_shape(KnownShape::Numeric(class))
            .expect("numeric leaf");
        let root = shared_unary_dag(&mut state, declared_type, parameters, leaf, depth);
        assert_eq!(
            project_exact_shape(&mut state, root).expect("unresolved projection"),
            root,
            "an unchanged shared DAG must reuse its original root"
        );
        assert_eq!(
            exact_shape_type(&mut state, &mut environment, root).expect("open shape query"),
            None
        );

        let receiver = builtin_type_ids::fixed_scalar(FixedScalar::U64);
        state.add_receiver_witness(class, receiver, None, &environment);
        let projected = project_exact_shape(&mut state, root)
            .expect("exact projection after receiver mutation");
        assert_ne!(projected, root);
        let mut distinct = HashSet::new();
        let mut pending = vec![projected];
        while let Some(shape) = pending.pop() {
            if !distinct.insert(shape) {
                continue;
            }
            match state.shape(shape) {
                KnownShape::Map { key, value } => {
                    assert_eq!(key, value, "projected map children must retain sharing");
                    pending.extend([key, value]);
                }
                KnownShape::DeclaredSubstitution { bindings, .. } => {
                    let children: Vec<_> = bindings
                        .indices()
                        .map(|index| state.declared_binding(index).shape)
                        .collect();
                    assert_eq!(children.len(), 2);
                    assert_eq!(
                        children[0], children[1],
                        "declared edges must retain sharing"
                    );
                    pending.extend(children);
                }
                KnownShape::Concrete { type_id, .. } => assert_eq!(type_id, receiver),
                _ => panic!(
                    "exact projected DAG must contain only map envelopes and concrete leaves"
                ),
            }
        }
        assert_eq!(
            distinct.len(),
            depth + 1,
            "projection output must grow with distinct input nodes, not expanded paths"
        );
        let mut expected = receiver;
        for _ in 0..depth {
            expected = environment.intern_map(expected, expected);
        }
        assert_eq!(
            exact_shape_type(&mut state, &mut environment, root).expect("closed shape query"),
            Some(expected)
        );
    }
}

#[test]
fn shared_unary_forward_predicate_finds_nested_forward_numeric_leaf() {
    use crate::compiler_frontend::ast::numeric_known_shape::shape_has_forward_leaf;
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let mut environment = TypeEnvironment::new();
    let (declared_type, parameters) = shared_unary_declared_map(&mut environment);
    let mut state = NumericConstructionState::new();
    let concrete = state
        .intern_shape(KnownShape::Concrete {
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            span: None,
        })
        .expect("concrete leaf");
    let concrete_root = shared_unary_dag(&mut state, declared_type, parameters, concrete, 16);
    assert!(!shape_has_forward_leaf(&state, concrete_root));

    let class = state.create_class().expect("forward numeric class");
    let numeric = state
        .intern_shape(KnownShape::Numeric(class))
        .expect("numeric leaf");
    let numeric_root = shared_unary_dag(&mut state, declared_type, parameters, numeric, 16);
    assert!(!shape_has_forward_leaf(&state, numeric_root));
    state.mark_forward(class);
    assert!(
        shape_has_forward_leaf(&state, numeric_root),
        "the next invocation must observe a nested leaf's new forward flag"
    );
}

#[test]
fn shared_unary_leaf_pinning_processes_shared_numeric_node_once() {
    use crate::compiler_frontend::ast::numeric_construction::merged_receiver_type;
    use crate::compiler_frontend::ast::numeric_known_shape::pin_numeric_leaves;
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let _guard = lock_counter_test();
    let mut environment = TypeEnvironment::new();
    let (declared_type, parameters) = shared_unary_declared_map(&mut environment);
    let mut state = NumericConstructionState::new();
    let class = state.create_class().expect("numeric leaf class");
    state.note_class_family(class, NumericFamily::Whole);
    let leaf = state
        .intern_shape(KnownShape::Numeric(class))
        .expect("numeric leaf");
    let root = shared_unary_dag(&mut state, declared_type, parameters, leaf, 12);
    let expected = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    state.add_receiver_witness(class, expected, None, &environment);
    for invocation in 1..=2 {
        reset_ast_counters();
        pin_numeric_leaves(&mut state, &environment, root, None, invocation as u64)
            .expect("pin shared numeric leaf");
        // One summary read chooses the pinned type, then one insertion compares its witness.
        // Repeated incoming edges must not repeat either operation.
        assert_eq!(
            test_read_ast_counter(AstCounter::NumericReceiverWitnessesInspected),
            2,
            "one pin invocation must read and register the shared leaf exactly once"
        );
        assert_eq!(
            merged_receiver_type(&mut state, class).expect("pinned receiver"),
            Some(expected)
        );
    }
}

#[test]
fn shared_unary_shape_pinning_applies_later_stronger_pin() {
    use crate::compiler_frontend::ast::numeric_construction::merged_receiver_type;
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let mut environment = TypeEnvironment::new();
    let (declared_type, parameters) = shared_unary_declared_map(&mut environment);
    let mut state = NumericConstructionState::new();
    let class = state.create_class().expect("numeric leaf class");
    state.note_class_family(class, NumericFamily::Whole);
    let leaf = state
        .intern_shape(KnownShape::Numeric(class))
        .expect("numeric leaf");
    let root = shared_unary_dag(&mut state, declared_type, parameters, leaf, 12);
    let receiver = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    state.add_receiver_witness(class, receiver, None, &environment);
    pin_numeric_shape(&mut state, root, NumericPin::Bound);
    assert_eq!(state.class_pinned_family(class), Some(NumericFamily::Whole));
    assert_eq!(
        merged_receiver_type(&mut state, class).expect("bound receiver"),
        Some(receiver)
    );
    pin_numeric_shape(&mut state, root, NumericPin::LoopItem);
    assert!(
        merged_receiver_type(&mut state, class).is_err(),
        "a stronger loop-item pin must revisit the shared leaf and reject its float receiver"
    );
}

#[test]
fn shared_unary_pending_producer_query_finishes_without_expanding_paths() {
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let mut environment = TypeEnvironment::new();
    let (declared_type, parameters) = shared_unary_declared_map(&mut environment);
    let mut state = NumericConstructionState::new();
    let leaf = state
        .intern_shape(KnownShape::Concrete {
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            span: None,
        })
        .expect("concrete leaf");
    // Forty shared levels encode over a trillion paths but only forty-one nodes.
    let root = shared_unary_dag(&mut state, declared_type, parameters, leaf, 40);
    for _ in 0..2 {
        assert_eq!(
            state
                .next_pending_forward_producer(root)
                .expect("pending producer query"),
            None
        );
    }
}

#[test]
fn shared_unary_map_leaf_preserves_distinct_receiver_witnesses() {
    use crate::compiler_frontend::ast::numeric_construction::merged_receiver_type;
    use crate::compiler_frontend::ast::numeric_known_shape::register_shape_witness;
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let mut environment = TypeEnvironment::new();
    let mut state = NumericConstructionState::new();
    let class = state.create_class().expect("numeric leaf class");
    let leaf = state
        .intern_shape(KnownShape::Numeric(class))
        .expect("numeric leaf");
    let root = state
        .intern_shape(KnownShape::Map {
            key: leaf,
            value: leaf,
        })
        .expect("shared numeric map");
    let key_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let value_type = builtin_type_ids::fixed_scalar(FixedScalar::I64);
    let receiver = environment.intern_map(key_type, value_type);
    register_shape_witness(&mut state, &environment, root, receiver, None);
    assert!(
        merged_receiver_type(&mut state, class).is_err(),
        "sharing a shape does not collapse different receiver types into one witness"
    );
}

#[test]
fn paired_union_revisits_concrete_peers_after_merging_a_bound_origin() {
    use crate::compiler_frontend::ast::numeric_construction::merged_receiver_type;
    use crate::compiler_frontend::datatypes::builtin_type_ids;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    let environment = TypeEnvironment::new();
    let mut state = NumericConstructionState::new();
    let origin = state.create_class().expect("unbound whole class");
    state.note_class_family(origin, NumericFamily::Whole);
    let origin_shape = state
        .intern_shape(KnownShape::Numeric(origin))
        .expect("origin shape");
    let bound = state.create_class().expect("bound whole class");
    state.note_class_family(bound, NumericFamily::Whole);
    state.pin_numeric_origin(bound, NumericPin::Bound);
    let bound_shape = state
        .intern_shape(KnownShape::Numeric(bound))
        .expect("bound shape");
    let accepted_type = builtin_type_ids::fixed_scalar(FixedScalar::F32);
    let accepted = state
        .intern_shape(KnownShape::Concrete {
            type_id: accepted_type,
            span: None,
        })
        .expect("early concrete peer");
    let refused = state
        .intern_shape(KnownShape::Concrete {
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::F64),
            span: None,
        })
        .expect("later concrete peer");
    // Parameter order makes the same concrete peer appear before and after a class merge.
    // The final peer has its own provenance and must follow the newly inherited pin policy.
    let left_edges = (0..4)
        .map(|parameter| KnownShapeBinding {
            parameter: GenericParameterId(parameter),
            shape: origin_shape,
        })
        .collect::<Vec<_>>();
    let right_edges = [accepted, bound_shape, accepted, refused]
        .into_iter()
        .enumerate()
        .map(|(parameter, shape)| KnownShapeBinding {
            parameter: GenericParameterId(parameter as u32),
            shape,
        })
        .collect::<Vec<_>>();
    let left = state
        .intern_declared_substitution(TypeId(0), &left_edges)
        .expect("left repeated origin envelope");
    let right = state
        .intern_declared_substitution(TypeId(0), &right_edges)
        .expect("right ordered peer envelope");

    assert!(union_shape_classes(&mut state, &environment, left, right));
    assert!(state.classes_share_root(origin, bound));
    assert_eq!(
        state.class_pinned_family(origin),
        Some(NumericFamily::Whole)
    );
    assert_eq!(
        merged_receiver_type(&mut state, origin).expect("receiver admitted before pin merge"),
        Some(accepted_type),
        "the earlier admitted witness survives while the later distinct float peer is refused"
    );
    // A subsequent traversal must observe the stronger pin rather than reuse earlier results.
    pin_numeric_shape(&mut state, origin_shape, NumericPin::LoopItem);
    assert!(union_shape_classes(&mut state, &environment, left, right));
    assert!(
        merged_receiver_type(&mut state, origin).is_err(),
        "a loop-item pin must retain the ineligible float evidence across repeated unions"
    );
}

#[test]
fn paired_refusal_observes_numeric_family_changes_between_operations() {
    let environment = TypeEnvironment::new();
    let mut state = NumericConstructionState::new();
    let pinned = state.create_class().expect("pinned whole class");
    state.note_class_family(pinned, NumericFamily::Whole);
    state.pin_numeric_origin(pinned, NumericPin::Bound);
    let pinned_shape = state
        .intern_shape(KnownShape::Numeric(pinned))
        .expect("pinned shape");
    let open = state.create_class().expect("open whole class");
    state.note_class_family(open, NumericFamily::Whole);
    let open_shape = state
        .intern_shape(KnownShape::Numeric(open))
        .expect("open shape");
    let decimal = state.create_class().expect("decimal class");
    state.note_class_family(decimal, NumericFamily::Decimal);
    let decimal_shape = state
        .intern_shape(KnownShape::Numeric(decimal))
        .expect("decimal shape");
    let pinned_map = state
        .intern_shape(KnownShape::Map {
            key: pinned_shape,
            value: pinned_shape,
        })
        .expect("shared pinned map");
    let open_map = state
        .intern_shape(KnownShape::Map {
            key: open_shape,
            value: open_shape,
        })
        .expect("shared open map");
    let decimal_map = state
        .intern_shape(KnownShape::Map {
            key: decimal_shape,
            value: decimal_shape,
        })
        .expect("shared decimal map");

    assert!(!shapes_join_refused(
        &mut state,
        &environment,
        pinned_map,
        open_map
    ));
    assert!(union_shape_classes(
        &mut state,
        &environment,
        open_map,
        decimal_map
    ));
    assert_eq!(state.class_family(open), NumericFamily::Decimal);
    for _ in 0..2 {
        assert!(
            shapes_join_refused(&mut state, &environment, pinned_map, open_map),
            "a fresh refusal traversal must see the formerly whole peer's merged decimal family"
        );
        assert!(!union_shape_classes(
            &mut state,
            &environment,
            pinned_map,
            open_map
        ));
        assert!(!state.classes_share_root(pinned, open));
    }
}
