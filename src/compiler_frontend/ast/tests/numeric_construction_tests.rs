//! Body-local numeric lifecycle and traversal-order invariants.
//!
//! Real lexer/body-parser drafts expose retained work before completion, while the existing
//! construction APIs and production counters expose consumption, release and bounded query work.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView,
};
use crate::compiler_frontend::ast::module_ast::scope_context::{
    AuthoredBindingFacts, EnclosingCallableContract, EnclosingCallableOwner,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredExpressionSuffix, DirectPendingReference, NumericBindingId, NumericConstructionState,
    NumericContinuation, NumericFamily, NumericRecipe, ReturnContinuation,
    accept_forward_binding_result, binding_annotation_type, binding_known_shape,
    binding_reference_shape, deferred_numeric_expression, merged_receiver_type,
    register_binding_annotation, register_direct_reference_witness,
    register_pending_reference_view, resolve_raw_binding, union_alias_classes,
};
use crate::compiler_frontend::ast::numeric_known_shape::KnownShape;
use crate::compiler_frontend::ast::statements::body_dispatch::ParsedBody;
use crate::compiler_frontend::ast::statements::numeric_body_completion::complete_numeric_body;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{
    ContextKind, ScopeContext, TopLevelDeclarationTable, function_body_to_ast,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, compact_label_message_text,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, TypeMismatchContext};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::display::{TypeDisplayShape, TypeDisplayView};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::test_support::TestSourceContext;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, LocalSpan, SourceId, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TokenRange, TokenizerEntryMode};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::rc::Rc;
use std::sync::Arc;

fn body_context(scope: PathId, paths: &PathInternerFork) -> ScopeContext {
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![], paths)),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    context.enclosing_callable = Some(Rc::new(EnclosingCallableContract {
        owner: EnclosingCallableOwner::Function,
        success_types: Vec::new(),
        authored_success_slot_count: 0,
        error_type: None,
        parameter_contracts: Vec::new(),
    }));
    context
}
#[derive(Debug, PartialEq, Eq)]
struct NumericMismatchSnapshot {
    code: &'static str,
    expected_identity: TypeDisplayShape<TypeDisplayId>,
    expected_name: String,
    found_identity: TypeDisplayShape<TypeDisplayId>,
    found_name: String,
    context: TypeMismatchContext,
    primary_span: Option<SourceSpan>,
    labels: Vec<NumericLabelSnapshot>,
}

#[derive(Debug, PartialEq, Eq)]
struct NumericLabelSnapshot {
    span: Option<SourceSpan>,
    message: Option<String>,
    owner: NumericLabelOwner,
}

#[derive(Debug, PartialEq, Eq)]
enum NumericLabelOwner {
    Absent,
    Local,
    Foreign(Option<*const FrozenIdentityContext>),
}

fn numeric_mismatch_snapshot(
    diagnostic: DiagnosticDraft,
    environment: &TypeEnvironment,
) -> NumericMismatchSnapshot {
    let strings = StringTable::new();
    let paths = PathInternerFork::empty();
    let path_table = paths.snapshot_table();
    let render_context = DiagnosticRenderContext::new(&strings).with_path_table(&path_table);
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), Some(environment));
    let view = compact
        .first()
        .expect("numeric conflict emits one diagnostic");
    let facts = view
        .family_view::<TypeMismatchView>()
        .expect("numeric conflict retains type mismatch facts");
    NumericMismatchSnapshot {
        code: view.descriptor().code,
        expected_identity: view.types().shape(facts.expected),
        expected_name: view.types().display(facts.expected, &strings, &path_table),
        found_identity: view.types().shape(facts.found),
        found_name: view.types().display(facts.found, &strings, &path_table),
        context: facts.context,
        primary_span: site_span(view.primary_site()),
        labels: view
            .durable_labels()
            .map(|label| NumericLabelSnapshot {
                span: site_span(label.site),
                message: compact_label_message_text(label.message, view.types(), render_context),
                owner: site_owner(label.site),
            })
            .collect(),
    }
}

fn site_span(site: SiteRef<'_>) -> Option<SourceSpan> {
    match site {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}

fn site_owner(site: SiteRef<'_>) -> NumericLabelOwner {
    match site {
        SiteRef::Absent => NumericLabelOwner::Absent,
        SiteRef::Local(_) => NumericLabelOwner::Local,
        SiteRef::Foreign { context, .. } => NumericLabelOwner::Foreign(
            context.map(|context| context as *const FrozenIdentityContext),
        ),
    }
}

fn empty_return(scope: PathId) -> NumericContinuation {
    NumericContinuation::Return(ReturnContinuation {
        slots: vec![],
        is_error_return: false,
        span: None,
        scope,
    })
}

fn assert_released(state: &mut NumericConstructionState) {
    assert!(state.bindings().is_empty());
    assert_eq!(state.continuation_count(), 0);
    assert_eq!(state.forward_producer_checkpoint(), 0);
    assert!(
        state.create_class().is_err(),
        "released domains cannot accept new origins"
    );
    assert!(
        state
            .intern_shape(KnownShape::Concrete {
                type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                span: None,
            })
            .is_err(),
        "released domains cannot accept new shapes"
    );
}

#[test]
fn recipes_continuations_and_forward_results_are_consumed_once() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("lifecycle.moth", &mut strings)
        .expect("test scope should intern");
    let context = body_context(scope, &paths);
    let name = strings.intern("value");
    let path = paths
        .try_intern_child(scope, name)
        .expect("binding path should intern");
    let binding = context
        .declare_pending_numeric_binding(
            AuthoredBindingFacts {
                name,
                path,
                span: None,
                binding_span: None,
                value_mode: ValueMode::ImmutableOwned,
                mutable: false,
            },
            NumericRecipe::Raw {
                items: vec![],
                family: NumericFamily::Whole,
            },
        )
        .expect("pending binding should register");
    let mut state = context.numeric_state_mut();
    assert!(matches!(
        state.take_binding_recipe(binding),
        Ok(NumericRecipe::Raw { .. })
    ));
    assert!(state.binding(binding).recipe.is_none());
    assert!(state.take_binding_recipe(binding).is_err());

    let continuation = state
        .register_continuation(empty_return(scope))
        .expect("continuation should register");
    assert!(state.continuation(continuation).is_some());
    assert!(state.take_continuation(continuation).is_some());
    assert!(state.continuation(continuation).is_none());
    assert!(state.take_continuation(continuation).is_none());

    let class = state
        .create_class()
        .expect("producer origin should register");
    let shape = state
        .intern_shape(KnownShape::Numeric(class))
        .expect("producer shape should register");
    let producer = state
        .register_forward_producer(
            class,
            shape,
            deferred_numeric_expression(
                vec![],
                ExpectedType::Infer,
                ValueMode::ImmutableOwned,
                None,
                DeferredExpressionSuffix::None,
            ),
        )
        .expect("forward producer should register");
    assert_eq!(
        state
            .next_pending_forward_producer(shape)
            .expect("producer lookup"),
        Some(producer)
    );
    assert!(
        state
            .take_completed_forward_producer(producer)
            .expect("pending result lookup")
            .is_none()
    );
    assert!(
        state
            .claim_pending_forward_producer(producer)
            .expect("first claim")
            .is_some()
    );
    assert!(
        state
            .claim_pending_forward_producer(producer)
            .expect("second claim")
            .is_none()
    );
    assert_eq!(
        state
            .next_pending_forward_producer(shape)
            .expect("resolving lookup"),
        None
    );
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let scalar = FixedScalarValue::signed(FixedScalar::I32, 7).expect("test value fits I32");
    let value = expressions
        .fixed_scalar(&mut constants, scalar, None, ValueMode::ImmutableOwned)
        .expect("test value should construct");
    state
        .complete_forward_producer(producer, value.id())
        .expect("claimed result should complete");
    let completed = state
        .take_completed_forward_producer(producer)
        .expect("completed result lookup")
        .expect("completed result should be available once");
    let ExpressionOperationView::Constant(payload) = expressions
        .view(completed)
        .expect("completed view")
        .operation()
    else {
        panic!("completed fixed scalar must be a constant");
    };
    assert!(matches!(
        constants.payload(payload),
        Some(ConstValuePayload::FixedScalar(stored))
            if stored.scalar() == FixedScalar::I32 && stored.as_i64() == Some(7)
    ));
    assert_eq!(
        expressions
            .view(completed)
            .expect("completed view")
            .value_type_id()
            .expect("completed type"),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert!(
        state
            .take_completed_forward_producer(producer)
            .expect("taken result lookup")
            .is_none()
    );
    assert!(
        state
            .complete_forward_producer(producer, completed)
            .is_err()
    );
    state.release();
    assert_released(&mut state);
    assert!(state.binding_value_mode(binding).is_none());
    assert!(state.register_continuation(empty_return(scope)).is_err());
}

fn declare_origin_query_binding(
    context: &ScopeContext,
    strings: &mut StringTable,
    paths: &mut PathInternerFork,
    scope: PathId,
    index: usize,
    recipe: NumericRecipe,
) -> NumericBindingId {
    let name = strings.intern(&format!("origin_{index}"));
    let path = paths.try_intern_child(scope, name).expect("binding path");
    context
        .declare_pending_numeric_binding(
            AuthoredBindingFacts {
                name,
                path,
                span: None,
                binding_span: None,
                value_mode: ValueMode::ImmutableOwned,
                mutable: false,
            },
            recipe,
        )
        .expect("origin-query binding")
}

fn raw_origin_query_recipe() -> NumericRecipe {
    NumericRecipe::Raw {
        items: vec![],
        family: NumericFamily::Whole,
    }
}

fn assert_unshaped_origin_queries(state: &NumericConstructionState, binding: NumericBindingId) {
    assert_eq!(binding_known_shape(state, binding), None);
    assert_eq!(binding_annotation_type(state, binding), None);
    assert_eq!(binding_reference_shape(state, binding), None);
}

#[test]
#[cfg(feature = "benchmark_counters")]
fn alias_origin_queries_and_receivers_visit_linear_total_rows() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };

    let _guard = lock_counter_test();
    let mut measurements = Vec::new();
    for count in [32, 128, 512] {
        reset_ast_counters();
        let mut strings = StringTable::new();
        let mut paths = PathInternerFork::empty();
        let scope = paths
            .try_intern_portable_path("alias_queries.moth", &mut strings)
            .expect("test scope");
        let context = body_context(scope, &paths);
        let environment = TypeEnvironment::new();
        let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
        let origin = declare_origin_query_binding(
            &context,
            &mut strings,
            &mut paths,
            scope,
            0,
            raw_origin_query_recipe(),
        );
        let mut bindings = Vec::with_capacity(count + 1);
        bindings.push(origin);
        for index in 1..=count {
            let source = bindings[index - 1];
            let binding = declare_origin_query_binding(
                &context,
                &mut strings,
                &mut paths,
                scope,
                index,
                NumericRecipe::Alias {
                    source,
                    is_copy: index % 2 == 0,
                },
            );
            let mut state = context.numeric_state_mut();
            union_alias_classes(&mut state, binding, source);
            register_direct_reference_witness(
                &mut state,
                &environment,
                &DirectPendingReference {
                    binding,
                    span: None,
                    is_copy: index % 2 == 0,
                },
                u64_type,
            );
            bindings.push(binding);
        }
        let mut query_count = 0;
        {
            let mut state = context.numeric_state_mut();
            assert_unshaped_origin_queries(&state, bindings[count]);
            query_count += 3;
            for _ in 0..8 {
                for depth in [1, count / 4, count / 2, count] {
                    assert_unshaped_origin_queries(&state, bindings[depth]);
                    query_count += 3;
                }
            }
            for binding in [origin, bindings[count / 2], bindings[count]] {
                let class = state.binding(binding).class;
                assert_eq!(
                    merged_receiver_type(&mut state, class).expect("receiver"),
                    Some(u64_type)
                );
            }
        }
        let steps = test_read_ast_counter(AstCounter::NumericAliasOriginSteps);
        let bound = 24 * (count + 1) + 4 * query_count;
        eprintln!(
            "alias origin queries: aliases={count}, queries={query_count}, steps={steps}, bound={bound}"
        );
        measurements.push((count, steps, bound));
    }
    for (count, steps, bound) in measurements {
        assert!(steps > 0, "production origin queries must be instrumented");
        assert!(
            steps <= bound,
            "{count} aliases visited {steps} rows, exceeding linear bound {bound}"
        );
    }
}

#[test]
#[cfg(feature = "benchmark_counters")]
fn sibling_loop_element_receivers_inspect_linear_total_witnesses() {
    use crate::compiler_frontend::ast::numeric_construction::NumericPin;
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };

    let _guard = lock_counter_test();
    let environment = TypeEnvironment::new();
    let receiver = builtin_type_ids::fixed_scalar(FixedScalar::U64);
    let mut measurements = Vec::new();
    for count in [32, 128, 512] {
        reset_ast_counters();
        let mut state = NumericConstructionState::new();
        let element = state.create_class().expect("shared collection element");
        state.note_class_family(element, NumericFamily::Whole);
        state.pin_numeric_origin(element, NumericPin::LoopItem);
        // Sibling loop items share the same element class but each contributes a receiving use.
        for _ in 0..count {
            let item = state.create_class().expect("sibling loop item");
            state.union_classes(element, item);
            state.add_receiver_witness(item, receiver, None, &environment);
        }
        for _ in 0..count {
            assert_eq!(
                merged_receiver_type(&mut state, element).expect("integer receiver"),
                Some(receiver),
            );
        }
        let inspected = test_read_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
        let bound = 12 * count;
        eprintln!("sibling loop receivers: items={count}, inspected={inspected}, bound={bound}");
        measurements.push((count, inspected, bound));
    }
    for (count, inspected, bound) in measurements {
        assert!(
            inspected > 0,
            "production receiver selection must be instrumented"
        );
        assert!(
            inspected <= bound,
            "{count} sibling loop items inspected {inspected} witnesses, exceeding {bound}",
        );
    }
}

#[test]
fn flat_alias_origin_queries_fit_a_small_host_stack() {
    std::thread::Builder::new()
        .name("flat-alias-origin-queries".into())
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut strings = StringTable::new();
            let mut paths = PathInternerFork::empty();
            let scope = paths
                .try_intern_portable_path("deep_alias_queries.moth", &mut strings)
                .expect("test scope");
            let context = body_context(scope, &paths);
            let mut binding = declare_origin_query_binding(
                &context,
                &mut strings,
                &mut paths,
                scope,
                0,
                raw_origin_query_recipe(),
            );
            // Flat declarations and ID edges avoid recursive fixture construction or destruction.
            for index in 1..=50_000 {
                binding = declare_origin_query_binding(
                    &context,
                    &mut strings,
                    &mut paths,
                    scope,
                    index,
                    NumericRecipe::Alias {
                        source: binding,
                        is_copy: true,
                    },
                );
            }
            let mut state = context.numeric_state_mut();
            assert_unshaped_origin_queries(&state, binding);
            register_direct_reference_witness(
                &mut state,
                &TypeEnvironment::new(),
                &DirectPendingReference {
                    binding,
                    span: None,
                    is_copy: true,
                },
                builtin_type_ids::fixed_scalar(FixedScalar::U64),
            );
            assert_unshaped_origin_queries(&state, binding);
        })
        .expect("small-stack query worker")
        .join()
        .expect("flat alias queries must not overflow the host stack");
}

#[test]
fn alias_origin_queries_preserve_collection_and_map_reference_boundaries() {
    for map in [false, true] {
        let mut strings = StringTable::new();
        let mut paths = PathInternerFork::empty();
        let scope = paths
            .try_intern_portable_path("shape_alias_queries.moth", &mut strings)
            .expect("test scope");
        let context = body_context(scope, &paths);
        let mut environment = TypeEnvironment::new();
        let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
        let i64_type = builtin_type_ids::fixed_scalar(FixedScalar::I64);
        let (class, original, first_view, second_view) = {
            let mut state = context.numeric_state_mut();
            let class = state.create_class().expect("leaf class");
            state.note_class_family(class, NumericFamily::Whole);
            let leaf = state
                .intern_shape(KnownShape::Numeric(class))
                .expect("numeric leaf");
            let first = state
                .intern_shape(KnownShape::Concrete {
                    type_id: u64_type,
                    span: None,
                })
                .expect("first leaf view");
            let second = state
                .intern_shape(KnownShape::Concrete {
                    type_id: i64_type,
                    span: None,
                })
                .expect("second leaf view");
            let mut wrap = |element| {
                let shape = if map {
                    KnownShape::Map {
                        key: first,
                        value: element,
                    }
                } else {
                    KnownShape::Collection {
                        element,
                        fixed_capacity: Some(4),
                    }
                };
                state.intern_shape(shape).expect("composite shape")
            };
            (class, wrap(leaf), wrap(first), wrap(second))
        };
        let origin = declare_origin_query_binding(
            &context,
            &mut strings,
            &mut paths,
            scope,
            0,
            NumericRecipe::Directed {
                items: vec![],
                shape: Some(original),
            },
        );
        register_pending_reference_view(&mut context.numeric_state_mut(), origin, first_view);
        let alias = declare_origin_query_binding(
            &context,
            &mut strings,
            &mut paths,
            scope,
            1,
            NumericRecipe::Alias {
                source: origin,
                is_copy: true,
            },
        );
        let mut state = context.numeric_state_mut();
        assert_eq!(state.class_pinned_family(class), Some(NumericFamily::Whole));
        assert_eq!(binding_known_shape(&state, alias), Some(original));
        assert_eq!(binding_reference_shape(&state, alias), Some(first_view));
        assert_eq!(binding_annotation_type(&state, alias), None);
        register_pending_reference_view(&mut state, alias, second_view);
        assert_eq!(binding_reference_shape(&state, alias), Some(second_view));
        assert_eq!(binding_known_shape(&state, alias), Some(original));
        assert_eq!(
            binding_annotation_type(&state, alias),
            None,
            "a partial view is not an annotation"
        );
        let annotated_type = if map {
            environment.intern_map(u64_type, u64_type)
        } else {
            environment.intern_collection(u64_type, Some(4))
        };
        state
            .take_binding_recipe(origin)
            .expect("view origin recipe consumed");
        state
            .take_binding_recipe(alias)
            .expect("view alias recipe consumed");
        assert_eq!(binding_known_shape(&state, alias), Some(original));
        assert_eq!(binding_reference_shape(&state, alias), Some(second_view));
        assert_eq!(binding_annotation_type(&state, alias), None);
        drop(state);

        let annotated_origin = declare_origin_query_binding(
            &context,
            &mut strings,
            &mut paths,
            scope,
            2,
            NumericRecipe::Directed {
                items: vec![],
                shape: Some(original),
            },
        );
        // Declaration facts are installed at birth, before later aliases can name the source.
        register_binding_annotation(
            &mut context.numeric_state_mut(),
            &environment,
            annotated_origin,
            annotated_type,
            None,
        )
        .expect("origin annotation");
        let annotated_alias = declare_origin_query_binding(
            &context,
            &mut strings,
            &mut paths,
            scope,
            3,
            NumericRecipe::Alias {
                source: annotated_origin,
                is_copy: true,
            },
        );
        let mut state = context.numeric_state_mut();
        register_pending_reference_view(&mut state, annotated_alias, second_view);
        // An inherited annotation outranks the alias's own partial reference view.
        assert_eq!(
            binding_annotation_type(&state, annotated_alias),
            Some(annotated_type)
        );
        let annotation =
            binding_reference_shape(&state, annotated_alias).expect("annotation reference");
        assert!(
            matches!(state.shape(annotation), KnownShape::Concrete { type_id, .. } if type_id == annotated_type)
        );
        assert_eq!(binding_known_shape(&state, annotated_alias), Some(original));
        let alias_type = if map {
            environment.intern_map(u64_type, i64_type)
        } else {
            environment.intern_collection(i64_type, Some(4))
        };
        register_binding_annotation(&mut state, &environment, annotated_alias, alias_type, None)
            .expect("alias annotation");
        assert_eq!(
            binding_annotation_type(&state, annotated_alias),
            Some(alias_type)
        );
        assert_eq!(
            binding_annotation_type(&state, annotated_origin),
            Some(annotated_type)
        );
        state
            .take_binding_recipe(annotated_origin)
            .expect("origin recipe consumed");
        assert_eq!(
            binding_known_shape(&state, annotated_alias),
            Some(original),
            "consumption preserves the origin shape"
        );
        let exact_reference = binding_reference_shape(&state, annotated_alias);
        state
            .take_binding_recipe(annotated_alias)
            .expect("alias recipe consumed");
        assert_eq!(binding_known_shape(&state, annotated_alias), Some(original));
        assert_eq!(
            binding_annotation_type(&state, annotated_alias),
            Some(alias_type)
        );
        assert_eq!(
            binding_reference_shape(&state, annotated_alias),
            exact_reference
        );
    }
}

#[test]
fn alias_receivers_distinguish_raw_and_forward_origins_in_one_equality_class() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("forward_alias_queries.moth", &mut strings)
        .expect("test scope");
    let context = body_context(scope, &paths);
    let raw = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        0,
        raw_origin_query_recipe(),
    );
    let forward = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        1,
        NumericRecipe::Directed {
            items: vec![],
            shape: None,
        },
    );
    let raw_alias = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        2,
        NumericRecipe::Alias {
            source: raw,
            is_copy: true,
        },
    );
    let forward_alias = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        3,
        NumericRecipe::Alias {
            source: forward,
            is_copy: false,
        },
    );
    let environment = TypeEnvironment::new();
    let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
    let mut state = context.numeric_state_mut();
    union_alias_classes(&mut state, raw_alias, raw);
    union_alias_classes(&mut state, forward_alias, forward);
    union_alias_classes(&mut state, raw, forward);
    let class = state.binding(raw).class;
    for binding in [forward, forward_alias] {
        register_direct_reference_witness(
            &mut state,
            &environment,
            &DirectPendingReference {
                binding,
                span: None,
                is_copy: false,
            },
            builtin_type_ids::fixed_scalar(FixedScalar::I64),
        );
        assert_eq!(
            merged_receiver_type(&mut state, class).expect("forward query"),
            None
        );
    }
    register_direct_reference_witness(
        &mut state,
        &environment,
        &DirectPendingReference {
            binding: raw_alias,
            span: None,
            is_copy: true,
        },
        u64_type,
    );
    assert_eq!(
        merged_receiver_type(&mut state, class).expect("raw receiver"),
        Some(u64_type)
    );
    assert!(
        matches!(state.binding(raw_alias).recipe, Some(NumericRecipe::Alias { source, is_copy: true }) if source == raw)
    );
    assert!(
        matches!(state.binding(forward_alias).recipe, Some(NumericRecipe::Alias { source, is_copy: false }) if source == forward)
    );
}

#[test]
fn conflicting_receivers_on_aliases_keep_authored_spans_and_copy_equality() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("alias_conflicts.moth", &mut strings)
        .expect("test scope");
    let context = body_context(scope, &paths);
    let origin = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        0,
        raw_origin_query_recipe(),
    );
    let first = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        1,
        NumericRecipe::Alias {
            source: origin,
            is_copy: true,
        },
    );
    let last = declare_origin_query_binding(
        &context,
        &mut strings,
        &mut paths,
        scope,
        2,
        NumericRecipe::Alias {
            source: first,
            is_copy: false,
        },
    );
    let mut spans = ExtendedSpanBuilder::new();
    let first_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(10, 5, &mut spans).expect("first span"),
    );
    let last_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(30, 5, &mut spans).expect("last span"),
    );
    let environment = TypeEnvironment::new();
    let mut state = context.numeric_state_mut();
    union_alias_classes(&mut state, first, origin);
    union_alias_classes(&mut state, last, first);
    assert_unshaped_origin_queries(&state, last);
    for (binding, span, scalar) in [
        (first, first_span, FixedScalar::U64),
        (last, last_span, FixedScalar::I64),
    ] {
        register_direct_reference_witness(
            &mut state,
            &environment,
            &DirectPendingReference {
                binding,
                span: Some(span),
                is_copy: binding == first,
            },
            builtin_type_ids::fixed_scalar(scalar),
        );
    }
    for binding in [origin, first, last] {
        let class = state.binding(binding).class;
        let diagnostic = match merged_receiver_type(&mut state, class) {
            Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
            other => panic!("aliases share the exact receiver conflict: {other:?}"),
        };
        let snapshot = numeric_mismatch_snapshot(diagnostic, &environment);
        assert_eq!(snapshot.primary_span, Some(last_span));
        assert!(
            snapshot
                .labels
                .iter()
                .any(|label| label.span == Some(first_span))
        );
    }
}

#[test]
fn multi_bind_slot_origin_queries_preserve_carriers_and_producer_eligibility() {
    use crate::compiler_frontend::ast::numeric_construction::{
        PendingMultiBindProducer, PendingMultiBindProducerKind,
    };

    for shaped in [false, true] {
        for producer_case in 0..4 {
            let mut strings = StringTable::new();
            let mut paths = PathInternerFork::empty();
            let scope = paths
                .try_intern_portable_path("slot_origin_queries.moth", &mut strings)
                .expect("test scope");
            let context = body_context(scope, &paths);
            let environment = TypeEnvironment::new();
            let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
            let raw = declare_origin_query_binding(
                &context,
                &mut strings,
                &mut paths,
                scope,
                0,
                raw_origin_query_recipe(),
            );
            {
                let mut state = context.numeric_state_mut();
                let view = state
                    .intern_shape(KnownShape::Concrete {
                        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I64),
                        span: None,
                    })
                    .expect("producer view");
                register_pending_reference_view(&mut state, raw, view);
                register_binding_annotation(&mut state, &environment, raw, u64_type, None)
                    .expect("producer annotation");
            }
            let forward = declare_origin_query_binding(
                &context,
                &mut strings,
                &mut paths,
                scope,
                1,
                NumericRecipe::Directed {
                    items: vec![],
                    shape: None,
                },
            );
            let (continuation, carrier, carrier_class) = {
                let mut state = context.numeric_state_mut();
                let continuation = state
                    .register_continuation(empty_return(scope))
                    .expect("producer continuation");
                let class = state.create_class().expect("carrier class");
                let shape = state
                    .intern_shape(KnownShape::Numeric(class))
                    .expect("carrier shape");
                (continuation, shaped.then_some(shape), class)
            };
            let kind = match producer_case {
                0 => PendingMultiBindProducerKind::Raw(NumericFamily::Whole),
                1 => PendingMultiBindProducerKind::Binding(raw),
                2 => PendingMultiBindProducerKind::Binding(forward),
                3 => PendingMultiBindProducerKind::Forward,
                _ => unreachable!("four producer cases"),
            };
            let mut producers = vec![PendingMultiBindProducer {
                continuation,
                value_index: 0,
                kind,
                span: None,
                shape: None,
            }];
            if producer_case == 3 {
                // One forward arm blocks receiving inference even beside a raw producer.
                producers.push(PendingMultiBindProducer {
                    continuation,
                    value_index: 1,
                    kind: PendingMultiBindProducerKind::Raw(NumericFamily::Whole),
                    span: None,
                    shape: None,
                });
            }
            let slot = declare_origin_query_binding(
                &context,
                &mut strings,
                &mut paths,
                scope,
                2,
                NumericRecipe::MultiBindSlot {
                    producers,
                    shape: carrier,
                },
            );
            let alias = declare_origin_query_binding(
                &context,
                &mut strings,
                &mut paths,
                scope,
                3,
                NumericRecipe::Alias {
                    source: slot,
                    is_copy: true,
                },
            );
            let mut state = context.numeric_state_mut();
            union_alias_classes(&mut state, alias, slot);
            // Slot queries read its merged carrier, never the producer's view or annotation.
            for binding in [slot, alias] {
                assert_eq!(binding_known_shape(&state, binding), carrier);
                assert_eq!(binding_reference_shape(&state, binding), carrier);
                assert_eq!(binding_annotation_type(&state, binding), None);
                register_direct_reference_witness(
                    &mut state,
                    &environment,
                    &DirectPendingReference {
                        binding,
                        span: None,
                        is_copy: binding == alias,
                    },
                    u64_type,
                );
            }
            let class = if shaped {
                carrier_class
            } else {
                state.binding(slot).class
            };
            assert_eq!(
                merged_receiver_type(&mut state, class).expect("slot receiver"),
                (producer_case < 2).then_some(u64_type),
                "shaped={shaped}, producer_case={producer_case}",
            );
            state
                .take_binding_recipe(slot)
                .expect("slot recipe consumed");
            state
                .take_binding_recipe(alias)
                .expect("alias recipe consumed");
            for binding in [slot, alias] {
                assert_eq!(binding_known_shape(&state, binding), carrier);
                assert_eq!(binding_reference_shape(&state, binding), carrier);
                assert_eq!(binding_annotation_type(&state, binding), None);
            }
        }
    }
}

fn complete_folded_body(binding_count: usize, conflicting: bool) {
    let mut source = String::new();
    for index in 0..binding_count {
        source.push_str(&format!("origin_{index} = 1 + 2\n"));
    }
    for index in 0..binding_count {
        source.push_str(&format!("received_{index} I64 = origin_{index}\n"));
    }
    if conflicting {
        source.push_str("incompatible U64 = origin_0\n");
    }
    source.push_str(";\n");

    let mut source_context = TestSourceContext::new("folded_body.moth");
    let mut paths = PathInternerFork::empty();
    let lexed = source_context
        .tokenize(
            &source,
            &mut paths,
            &StyleDirectiveRegistry::built_ins(),
            TokenizerEntryMode::SourceFile,
        )
        .expect("body source should tokenize");
    let root = body_context(lexed.logical_path, &paths);
    let handle = root.numeric_construction_handle();
    let range = TokenRange::from_raw(lexed.tokens.source(), 0, lexed.tokens.len() as u32)
        .expect("body range should fit");
    let mut cursor =
        AstCursor::from_source_tokens(&lexed.tokens, range).expect("body cursor should build");
    let (strings, _) = source_context.preparation_parts();
    let mut environment = TypeEnvironment::new();
    let mut compatibility = TypeCompatibilityCache::new();
    let mut interner = AstTypeInterner::new(&mut environment, &mut compatibility);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut warnings = DiagnosticBag::new();
    let parsed = function_body_to_ast(
        &mut cursor,
        root.clone(),
        &mut interner,
        &mut expressions,
        &mut constants,
        &mut warnings,
        strings,
        &mut paths,
    )
    .expect("nested body should retain its numeric work");
    let ParsedBody::Draft(draft) = parsed else {
        panic!("a child scope must not complete its parent's numeric domain");
    };
    {
        let state = handle.borrow();
        assert!(state.bindings().len() >= binding_count);
        assert!(state.bindings().len() <= binding_count * 2 + usize::from(conflicting));
        assert!(state.continuation_count() <= binding_count * 2 + usize::from(conflicting));
        assert!(state.continuation_count() >= binding_count);
    }
    let result = complete_numeric_body(
        draft,
        &root,
        &mut interner,
        &mut expressions,
        &mut constants,
        strings,
        &mut paths,
    );
    assert_released(&mut handle.borrow_mut());
    if conflicting {
        assert!(
            matches!(result, Err(ExpressionParseError::Diagnostic(_))),
            "conflicting exact receivers must stay in the source-diagnostic lane"
        );
        return;
    }
    let nodes = result.expect("folded body should complete");
    assert_eq!(
        nodes.len(),
        binding_count * 2,
        "each authored declaration resumes once"
    );
    for node in &nodes {
        let NodeKind::VariableDeclaration(declaration) = &node.kind else {
            panic!("the body contains only declarations");
        };
        assert_eq!(
            declaration.type_id,
            builtin_type_ids::fixed_scalar(FixedScalar::I64)
        );
        let initializer = declaration
            .value
            .initializer()
            .expect("fixture initializer");
        assert_eq!(
            expressions
                .view(initializer)
                .expect("fixture view")
                .value_type_id()
                .expect("fixture type"),
            builtin_type_ids::fixed_scalar(FixedScalar::I64)
        );
    }
    for node in &nodes[..binding_count] {
        let NodeKind::VariableDeclaration(declaration) = &node.kind else {
            unreachable!()
        };
        let initializer = declaration
            .value
            .initializer()
            .expect("fixture initializer");
        let ExpressionOperationView::Constant(value) = expressions
            .view(initializer)
            .expect("fixture view")
            .operation()
        else {
            panic!("folded origin must be a constant");
        };
        assert!(matches!(
            constants.payload(value),
            Some(ConstValuePayload::FixedScalar(scalar))
                if scalar.scalar() == FixedScalar::I64 && scalar.as_i64() == Some(3)
        ));
    }
}

#[test]
fn large_folded_body_retains_linear_work_and_releases_after_completion() {
    complete_folded_body(256, false);
}

#[test]
fn large_folded_body_releases_pending_storage_after_receiver_conflict() {
    complete_folded_body(256, true);
}

#[test]
fn receiver_conflict_witnesses_ignore_registration_and_union_traversal_order() {
    // Parsing assigns keys in authored order. Deferred witnesses and equality traversal may
    // subsequently register and union those sites in any order without changing diagnostics.
    let mut spans = ExtendedSpanBuilder::new();
    let witnesses = [
        (
            builtin_type_ids::fixed_scalar(FixedScalar::I64),
            SourceSpan::new(
                SourceId::COMPILATION_ROOT,
                LocalSpan::exact(10, 5, &mut spans).expect("first receiver span"),
            ),
        ),
        (
            builtin_type_ids::fixed_scalar(FixedScalar::U64),
            SourceSpan::new(
                SourceId::COMPILATION_ROOT,
                LocalSpan::exact(30, 5, &mut spans).expect("second receiver span"),
            ),
        ),
        (
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            SourceSpan::new(
                SourceId::COMPILATION_ROOT,
                LocalSpan::exact(50, 5, &mut spans).expect("third receiver span"),
            ),
        ),
    ];
    let environment = TypeEnvironment::new();
    let mut baseline: Option<NumericMismatchSnapshot> = None;
    for order in [
        [0, 1, 2],
        [2, 1, 0],
        [1, 0, 2],
        [1, 2, 0],
        [0, 2, 1],
        [2, 0, 1],
    ] {
        let mut state = NumericConstructionState::new();
        let classes = [
            state.create_class().expect("first class"),
            state.create_class().expect("second class"),
            state.create_class().expect("third class"),
        ];
        let authored = witnesses.map(|(type_id, span)| {
            let shape = state
                .intern_shape(KnownShape::Concrete {
                    type_id,
                    span: Some(span),
                })
                .expect("authored concrete shape");
            state.shape_authored_order(shape)
        });
        for index in order {
            let (type_id, span) = witnesses[index];
            state.add_ordered_receiver_witness(
                classes[index],
                type_id,
                Some(span),
                authored[index],
                &environment,
            );
        }
        let joined = state.union_classes(classes[order[0]], classes[order[1]]);
        let joined = state.union_classes(classes[order[2]], joined);
        let diagnostic = match merged_receiver_type(&mut state, joined) {
            Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
            other => panic!("different exact receivers must diagnose: {other:?}"),
        };
        let snapshot = numeric_mismatch_snapshot(diagnostic, &environment);
        assert_eq!(snapshot.primary_span, Some(witnesses[1].1));
        assert!(
            snapshot
                .labels
                .iter()
                .any(|label| label.span == Some(witnesses[0].1))
        );
        if let Some(expected) = &baseline {
            assert_eq!(&snapshot, expected);
        } else {
            baseline = Some(snapshot);
        }
    }
}

#[test]
fn completion_receiver_seed_reuses_the_declarations_authored_order() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("seed_order.moth", &mut strings)
        .expect("test scope should intern");
    let context = body_context(scope, &paths);
    let name = strings.intern("origin");
    let path = paths.try_intern_child(scope, name).expect("binding path");
    let mut spans = ExtendedSpanBuilder::new();
    let declaration = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(10, 5, &mut spans).expect("declaration span"),
    );
    let receiver = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(30, 5, &mut spans).expect("receiver span"),
    );
    let binding = context
        .declare_pending_numeric_binding(
            AuthoredBindingFacts {
                name,
                path,
                span: Some(declaration),
                binding_span: Some(declaration),
                value_mode: ValueMode::ImmutableOwned,
                mutable: false,
            },
            NumericRecipe::Raw {
                items: vec![],
                family: NumericFamily::Whole,
            },
        )
        .expect("pending binding");
    let environment = TypeEnvironment::new();
    let mut state = context.numeric_state_mut();
    let class = state.binding(binding).class;
    let order = state.binding_authored_order(binding);
    state.add_receiver_witness(
        class,
        builtin_type_ids::fixed_scalar(FixedScalar::U64),
        Some(receiver),
        &environment,
    );
    // Storage seeding happens after parsing, but its diagnostic witness belongs to the
    // declaration that was parsed before the receiving use.
    state.add_ordered_receiver_witness(
        class,
        builtin_type_ids::fixed_scalar(FixedScalar::I64),
        Some(declaration),
        order,
        &environment,
    );
    let diagnostic = match merged_receiver_type(&mut state, class) {
        Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
        other => panic!("different exact receivers must diagnose: {other:?}"),
    };
    let snapshot = numeric_mismatch_snapshot(diagnostic, &environment);
    assert_eq!(snapshot.primary_span, Some(receiver));
    assert!(
        snapshot
            .labels
            .iter()
            .any(|label| label.span == Some(declaration))
    );
}

#[test]
fn family_and_forward_conflicts_use_authored_receivers_after_ranked_unions() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("receiver_order.moth", &mut strings)
        .expect("test scope");
    let name = strings.intern("origin");
    let path = paths.try_intern_child(scope, name).expect("binding path");
    let mut spans = ExtendedSpanBuilder::new();
    let [recipe_span, earlier_span, later_span] = [10, 30, 50].map(|start| {
        SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            LocalSpan::exact(start, 5, &mut spans).expect("authored span"),
        )
    });
    let receiver_type = builtin_type_ids::fixed_scalar(FixedScalar::F32);
    let computed_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let environment = TypeEnvironment::new();
    let mut baseline: Option<[NumericMismatchSnapshot; 2]> = None;
    for higher_rank in [None, Some(0), Some(1)] {
        for reverse_merge in [false, true] {
            let context = body_context(scope, &paths);
            let binding = context
                .declare_pending_numeric_binding(
                    AuthoredBindingFacts {
                        name,
                        path,
                        span: Some(recipe_span),
                        binding_span: Some(recipe_span),
                        value_mode: ValueMode::ImmutableOwned,
                        mutable: false,
                    },
                    NumericRecipe::Raw {
                        items: vec![],
                        family: NumericFamily::Whole,
                    },
                )
                .expect("pending origin");
            let mut state = context.numeric_state_mut();
            let classes = [
                state.binding(binding).class,
                state.create_class().expect("second equality class"),
            ];
            let earlier_order = state.take_authored_order();
            let later_order = state.take_authored_order();
            // Register later first, then vary rank and merge direction without changing evidence.
            state.add_ordered_receiver_witness(
                classes[1],
                receiver_type,
                Some(later_span),
                later_order,
                &environment,
            );
            state.add_ordered_receiver_witness(
                classes[0],
                receiver_type,
                Some(earlier_span),
                earlier_order,
                &environment,
            );
            if let Some(index) = higher_rank {
                let extra = state.create_class().expect("rank-raising class");
                assert_eq!(state.union_classes(classes[index], extra), classes[index]);
            }
            let joined = if reverse_merge {
                state.union_classes(classes[1], classes[0])
            } else {
                state.union_classes(classes[0], classes[1])
            };
            let expected_root = higher_rank.unwrap_or(usize::from(reverse_merge));
            assert_eq!(joined, classes[expected_root]);
            assert_eq!(
                merged_receiver_type(&mut state, joined).expect("same receivers"),
                Some(receiver_type)
            );
            let family_diagnostic = match resolve_raw_binding(
                &mut state,
                binding,
                NumericFamily::Whole,
                Some(recipe_span),
                &environment,
                false,
            ) {
                Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
                other => panic!("whole origin cannot receive F32: {other:?}"),
            };
            let forward_diagnostic = match accept_forward_binding_result(
                &mut state,
                binding,
                computed_type,
                Some(recipe_span),
            ) {
                Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
                other => panic!("computed I32 cannot receive F32: {other:?}"),
            };
            let diagnostics = [family_diagnostic, forward_diagnostic];
            let snapshots =
                diagnostics.map(|diagnostic| numeric_mismatch_snapshot(diagnostic, &environment));
            for snapshot in &snapshots {
                assert!(
                    snapshot
                        .labels
                        .iter()
                        .any(|label| label.span == Some(recipe_span))
                );
                assert!(
                    !snapshot
                        .labels
                        .iter()
                        .any(|label| label.span == Some(later_span))
                );
            }
            if let Some(expected) = &baseline {
                assert_eq!(&snapshots, expected);
            } else {
                baseline = Some(snapshots);
            }
        }
    }
}

#[test]
fn receiver_summary_preserves_type_ties_and_replaced_first_across_ranked_unions() {
    let environment = TypeEnvironment::new();
    let mut types =
        [FixedScalar::I32, FixedScalar::I64, FixedScalar::U64].map(builtin_type_ids::fixed_scalar);
    types.sort_by_key(|type_id| type_id.0);
    let mut spans = ExtendedSpanBuilder::new();
    let sites = [10, 20, 30, 40, 50, 60].map(|start| {
        SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            LocalSpan::exact(start, 5, &mut spans).expect("receiver span"),
        )
    });
    // The first class has a repeated first type. The second class changes the merged first
    // type and contributes three different types at one authored key.
    let witnesses = [
        (types[2], 30, sites[0]),
        (types[2], 10, sites[1]),
        (types[1], 20, sites[2]),
        (types[2], 5, sites[3]),
        (types[1], 5, sites[4]),
        (types[0], 5, sites[5]),
    ];
    for order in [
        [0, 1, 2],
        [2, 1, 0],
        [1, 0, 2],
        [1, 2, 0],
        [0, 2, 1],
        [2, 0, 1],
    ] {
        for higher_rank in [None, Some(0), Some(1)] {
            for reverse in [false, true] {
                let mut state = NumericConstructionState::new();
                let classes = [
                    state.create_class().expect("first class"),
                    state.create_class().expect("second class"),
                ];
                for class_index in 0..2 {
                    for index in order {
                        let (type_id, authored_order, span) = witnesses[3 * class_index + index];
                        state.add_ordered_receiver_witness(
                            classes[class_index],
                            type_id,
                            Some(span),
                            authored_order,
                            &environment,
                        );
                    }
                }
                if let Some(index) = higher_rank {
                    let extra = state.create_class().expect("rank-raising class");
                    state.union_classes(classes[index], extra);
                }
                let joined = if reverse {
                    state.union_classes(classes[1], classes[0])
                } else {
                    state.union_classes(classes[0], classes[1])
                };
                assert_eq!(state.first_receiver_type(classes[0]), Some(types[0]));
                assert_eq!(
                    state
                        .first_receiver_witness(classes[1])
                        .expect("first")
                        .span,
                    Some(sites[5])
                );
                let diagnostic = match merged_receiver_type(&mut state, joined) {
                    Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
                    other => panic!("three exact types must conflict: {other:?}"),
                };
                let snapshot = numeric_mismatch_snapshot(diagnostic, &environment);
                assert_eq!(snapshot.primary_span, Some(sites[4]));
                assert!(
                    snapshot
                        .labels
                        .iter()
                        .any(|label| label.span == Some(sites[5]))
                );
            }
        }
    }
}

#[test]
fn late_loop_pin_merges_earliest_ineligible_receiver_and_every_refused_peer() {
    use crate::compiler_frontend::ast::numeric_construction::NumericPin;

    let environment = TypeEnvironment::new();
    let integer = builtin_type_ids::fixed_scalar(FixedScalar::I64);
    let float = builtin_type_ids::fixed_scalar(FixedScalar::F32);
    let mut spans = ExtendedSpanBuilder::new();
    let sites = [10, 20, 30].map(|start| {
        SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            LocalSpan::exact(start, 5, &mut spans).expect("offender span"),
        )
    });
    for higher_rank in [None, Some(0), Some(1)] {
        for reverse in [false, true] {
            for refused_first in [false, true] {
                let mut state = NumericConstructionState::new();
                let classes = [
                    state.create_class().expect("receiver class"),
                    state.create_class().expect("refused class"),
                ];
                state.note_class_family(classes[0], NumericFamily::Whole);
                state.add_ordered_receiver_witness(classes[0], integer, None, 0, &environment);
                // This non-integer witness predates the pin, and a later same-type witness
                // must not overwrite it. Refused integer peers are still loop offenders.
                state.add_ordered_receiver_witness(
                    classes[0],
                    float,
                    Some(sites[1]),
                    2,
                    &environment,
                );
                state.add_ordered_receiver_witness(
                    classes[0],
                    float,
                    Some(sites[2]),
                    3,
                    &environment,
                );
                let refused_order = if refused_first { 1 } else { 4 };
                state.refuse_ordered_pinned_peer(
                    classes[1],
                    integer,
                    Some(sites[0]),
                    refused_order,
                );
                state.refuse_ordered_pinned_peer(classes[1], float, Some(sites[2]), 5);
                if let Some(index) = higher_rank {
                    let extra = state.create_class().expect("rank-raising class");
                    state.union_classes(classes[index], extra);
                }
                let joined = if reverse {
                    state.union_classes(classes[1], classes[0])
                } else {
                    state.union_classes(classes[0], classes[1])
                };
                state.pin_numeric_origin(joined, NumericPin::LoopItem);
                let diagnostic = match merged_receiver_type(&mut state, joined) {
                    Err(ExpressionParseError::Diagnostic(diagnostic)) => diagnostic,
                    other => panic!("late loop pin must reject retained offender: {other:?}"),
                };
                let snapshot = numeric_mismatch_snapshot(diagnostic, &environment);
                assert_eq!(
                    snapshot.primary_span,
                    Some(sites[usize::from(!refused_first)])
                );
                assert!(
                    snapshot.labels.is_empty(),
                    "loop policy precedes exact-type conflict"
                );
            }
        }
    }
    let mut state = NumericConstructionState::new();
    let class = state.create_class().expect("refused-only class");
    state.note_class_family(class, NumericFamily::Whole);
    state.refuse_ordered_pinned_peer(class, integer, Some(sites[0]), 0);
    let receiving_class = state.create_class().expect("integer receiving class");
    state.add_ordered_receiver_witness(receiving_class, integer, Some(sites[1]), 1, &environment);
    let class = state.union_classes(receiving_class, class);
    assert_eq!(
        merged_receiver_type(&mut state, class).expect("unpinned refused peer"),
        Some(integer)
    );
    state.pin_numeric_origin(class, NumericPin::Bound);
    assert_eq!(
        merged_receiver_type(&mut state, class).expect("bound refused peer"),
        Some(integer)
    );
    state.pin_numeric_origin(class, NumericPin::LoopItem);
    let diagnostic = merged_receiver_type(&mut state, class)
        .expect_err("the loop pin must reject the refused receiver");
    let ExpressionParseError::Diagnostic(diagnostic) = diagnostic else {
        panic!("the loop pin should produce a diagnostic: {diagnostic:?}");
    };
    let snapshot = numeric_mismatch_snapshot(diagnostic, &environment);
    assert_eq!(snapshot.primary_span, Some(sites[0]));
}
