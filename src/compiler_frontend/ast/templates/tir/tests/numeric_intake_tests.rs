//! Retained numeric template intake lifecycle tests.
//! WHAT: drives store-side retention APIs using real body domains, preparing retained conditions
//!       and range positions through the shared render-unit owner before header completion.
//!       Inputs use real retained literal, range-counter and comparison recipes.
//! WHY: body-root completion pins domain-scoped intake and counter-header lookup, one-time
//!      installation at each position, recordless bound-range copy ownership, and the rule that
//!      range operand records name their exact slot and never install independently. Authored
//!      parses and end-to-end behavior are covered by template source tests and statement lanes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::compiler_frontend::ast::ast_nodes::{Declaration, RangeEndKind, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredExpressionSuffix, DeferredNumericExpression, DeferredRangeHeader, DeferredRangeSpec,
    NumericBindingId, NumericConstructionDomainId, NumericConstructionState, NumericFamily,
    NumericRecipe, RetainedRangeCounter, StatementExpression,
};
use crate::compiler_frontend::ast::templates::template::Style;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;
use crate::compiler_frontend::ast::templates::template_render_units::{
    ControlFlowRenderUnitRequest, prepare_control_flow_render_units,
};
use crate::compiler_frontend::ast::templates::tir::expression_constness::const_conditional_loop_condition_value;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, TemplateIrNode, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateIrStore,
};
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One test body domain and its module-shared TIR store. The string table and path fork stay alive
/// with the context so interned names, paths and captured literal tokens remain valid for the test.
struct IntakeBody {
    store: Rc<RefCell<TemplateIrStore>>,
    context: ScopeContext,
    string_table: StringTable,
    path_fork: PathInternerFork,
}

impl IntakeBody {
    fn new() -> Self {
        Self::with_store(Rc::new(RefCell::new(TemplateIrStore::new())))
    }

    fn with_store(store: Rc<RefCell<TemplateIrStore>>) -> Self {
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();

        let scope = path_fork
            .try_intern_portable_path("numeric_intake_tests.moth", &mut string_table)
            .expect("test path fits");

        // A runtime body owns its numeric domain; a constant context never does.
        let context = ScopeContext::new_for_tests(
            ContextKind::Function,
            scope,
            Rc::new(TopLevelDeclarationTable::new(vec![], &path_fork)),
            Arc::new(ExternalPackageRegistry::default()),
            0,
        )
        .with_template_ir_store(Rc::clone(&store));

        Self {
            store,
            context,
            string_table,
            path_fork,
        }
    }

    fn construction(&self) -> TemplateConstructionContext {
        TemplateConstructionContext::new(Rc::clone(&self.store), None)
    }

    fn domain(&self) -> NumericConstructionDomainId {
        self.context.numeric_domain_id()
    }

    /// Declares one pending numeric binding in this body's live domain.
    ///
    /// WHAT: the binding's recipe is the real retained literal spelling of `3`, captured as a
    ///       pending numeric literal item exactly as the numeric capture path produces it.
    fn declare_pending_binding(&mut self, name: &str, literal: &str) -> NumericBindingId {
        let name = self.string_table.intern(name);
        let binding_path = self
            .path_fork
            .try_intern_components(&[name])
            .expect("test path fits")
            .to_owned();
        let literal_item = self.pending_literal_item(literal);

        self.context
            .declare_pending_numeric_binding(
                AuthoredBindingFacts {
                    name,
                    path: binding_path,
                    span: None,
                    binding_span: None,
                    value_mode: ValueMode::ImmutableOwned,
                    mutable: false,
                },
                NumericRecipe::Raw {
                    items: vec![literal_item],
                    family: NumericFamily::Whole,
                },
            )
            .expect("test pending binding allocation fits")
    }

    /// Declares one real pending range-counter binding in this body's live domain.
    fn declare_pending_range_counter(
        &mut self,
        name: &str,
        end_binding: NumericBindingId,
    ) -> NumericBindingId {
        let name = self.string_table.intern(name);
        let binding_path = self
            .path_fork
            .try_intern_components(&[name])
            .expect("test path fits")
            .to_owned();
        let range = DeferredRangeSpec {
            start: None,
            end: StatementExpression::Retained(self.pending_numeric_fragment(end_binding)),
            end_kind: RangeEndKind::Exclusive,
            step: None,
            span: None,
        };

        self.context
            .declare_pending_numeric_binding(
                AuthoredBindingFacts {
                    name,
                    path: binding_path,
                    span: None,
                    binding_span: None,
                    value_mode: ValueMode::ImmutableOwned,
                    mutable: false,
                },
                NumericRecipe::RangeCounter {
                    range: Box::new(range),
                },
            )
            .expect("test range-counter allocation fits")
    }

    /// The retained literal spelling of one authored numeral.
    fn pending_literal_item(&mut self, literal: &str) -> ExpressionRpnItem {
        ExpressionRpnItem::PendingNumericLiteral {
            token: NumericLiteralToken::test_new(literal, &mut self.string_table),
            span: None,
            value_mode: ValueMode::ImmutableOwned,
        }
    }

    /// The boolean comparison fragment `pending_value < 3`.
    fn pending_comparison_fragment(
        &mut self,
        binding: NumericBindingId,
    ) -> DeferredNumericExpression {
        let literal = self.pending_literal_item("3");

        DeferredNumericExpression {
            items: vec![
                ExpressionRpnItem::PendingNumericBinding {
                    binding,
                    span: None,
                    value_mode: ValueMode::ImmutableOwned,
                    is_copy: false,
                },
                literal,
                ExpressionRpnItem::Operator {
                    operator: Operator::LessThan,
                    span: None,
                },
            ],
            expected: ExpectedType::Infer,
            value_mode: ValueMode::ImmutableOwned,
            span: None,
            suffix: DeferredExpressionSuffix::None,
        }
    }

    /// The numeric fragment `pending_value`: a lone retained reference, which is the authored
    /// shape of a range operand that still awaits its selected type.
    fn pending_numeric_fragment(&self, binding: NumericBindingId) -> DeferredNumericExpression {
        DeferredNumericExpression {
            items: vec![ExpressionRpnItem::PendingNumericBinding {
                binding,
                span: None,
                value_mode: ValueMode::ImmutableOwned,
                is_copy: false,
            }],
            expected: ExpectedType::Infer,
            value_mode: ValueMode::ImmutableOwned,
            span: None,
            suffix: DeferredExpressionSuffix::None,
        }
    }

    fn empty_body_node(&self) -> TemplateIrNodeId {
        self.store.borrow_mut().push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: Vec::new(),
            },
            None,
        ))
    }
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, None, ValueMode::ImmutableOwned)
}

fn i32_expression(value: i64) -> Expression {
    Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, value).expect("test value fits I32"),
        None,
        ValueMode::ImmutableOwned,
    )
}

#[test]
fn retained_condition_intake_is_domain_scoped_and_installs_once_at_its_position() {
    let mut body = IntakeBody::new();
    let binding = body.declare_pending_binding("pending_value", "3");
    let domain = body.domain();
    let other_domain = NumericConstructionState::new().domain_id();
    let loop_body = body.empty_body_node();
    let fragment = body.pending_comparison_fragment(binding);

    let mut construction = body.construction();
    let position = construction
        .record_retained_loop_condition(domain, fragment, loop_body, None)
        .expect("retained condition registers");

    let style = Style::default();
    prepare_control_flow_render_units(
        &construction,
        ControlFlowRenderUnitRequest {
            style: &style,
            context: &body.context,
            string_table: &mut body.string_table,
        },
    )
    .expect("retained control-flow render unit is prepared before header completion");

    assert!(
        body.store
            .borrow_mut()
            .take_pending_numeric_intakes(other_domain)
            .is_empty(),
        "an independent body domain must not take this body's retained intake"
    );

    let taken = body.store.borrow_mut().take_pending_numeric_intakes(domain);
    let intake = taken
        .first()
        .expect("the retained position owns one intake record");
    assert_eq!(
        intake.position.node_id(),
        position,
        "the record must name the position node it completes into"
    );

    assert!(
        body.store
            .borrow_mut()
            .take_pending_numeric_intakes(domain)
            .is_empty(),
        "taking a retained intake is once per position"
    );

    body.store
        .borrow_mut()
        .install_completed_numeric_intake(intake.site, bool_expression(true), None)
        .expect("the completed condition installs at its position");

    assert!(
        matches!(
            body.store
                .borrow()
                .get_node(position)
                .expect("position node exists")
                .kind,
            TemplateIrNodeKind::Loop { .. }
        ),
        "the position itself must become the concrete loop node"
    );

    assert!(
        body.store
            .borrow_mut()
            .install_completed_numeric_intake(intake.site, bool_expression(false), None)
            .is_err(),
        "installing one retained intake twice is a broken invariant"
    );
}

/// The const condition a completed conditional `Loop` carries at `node_id`, when one is installed.
fn completed_loop_condition(
    store: &RefCell<TemplateIrStore>,
    node_id: TemplateIrNodeId,
) -> Option<bool> {
    let store = store.borrow();
    let node = store
        .get_node(node_id)
        .expect("retained loop position exists");
    match &node.kind {
        TemplateIrNodeKind::Loop {
            header: TemplateLoopHeader::Conditional { condition },
            ..
        } => const_conditional_loop_condition_value(condition),
        _ => None,
    }
}

#[test]
fn copied_retained_condition_completes_independently_of_its_source() {
    let mut body = IntakeBody::new();
    let binding = body.declare_pending_binding("pending_value", "3");
    let domain = body.domain();
    let loop_body = body.empty_body_node();
    let fragment = body.pending_comparison_fragment(binding);

    let mut construction = body.construction();
    let source_position = construction
        .record_retained_loop_condition(domain, fragment, loop_body, None)
        .expect("retained condition registers");

    let style = Style::default();
    prepare_control_flow_render_units(
        &construction,
        ControlFlowRenderUnitRequest {
            style: &style,
            context: &body.context,
            string_table: &mut body.string_table,
        },
    )
    .expect("retained control-flow render unit is prepared before the copy");

    let copied_position = {
        let copied_node = body
            .store
            .borrow()
            .get_node(source_position)
            .cloned()
            .expect("retained condition position exists");
        body.store.borrow_mut().push_node(copied_node)
    };
    body.store
        .borrow_mut()
        .replicate_pending_numeric_intakes_for_copied_node(source_position, copied_position)
        .expect("a copied retained condition registers its own intake");

    let taken = body.store.borrow_mut().take_pending_numeric_intakes(domain);
    let source_intake = taken
        .iter()
        .find(|intake| intake.position.node_id() == source_position)
        .expect("the source condition owns one intake");
    let copied_intake = taken
        .iter()
        .find(|intake| intake.position.node_id() == copied_position)
        .expect("the copied condition owns its own intake");

    body.store
        .borrow_mut()
        .install_completed_numeric_intake(source_intake.site, bool_expression(true), None)
        .expect("the source condition installs its own completed value");
    body.store
        .borrow_mut()
        .install_completed_numeric_intake(copied_intake.site, bool_expression(false), None)
        .expect("the copy installs its own completed value");

    assert_eq!(
        completed_loop_condition(&body.store, source_position),
        Some(true),
        "completing the copy must not change the source's resolved condition"
    );
    assert_eq!(
        completed_loop_condition(&body.store, copied_position),
        Some(false),
        "the copy must resolve to its own completed condition"
    );
    assert!(
        body.store
            .borrow_mut()
            .take_pending_numeric_intakes(domain)
            .is_empty(),
        "each source and copy intake is taken and installed once"
    );
}

#[test]
fn bound_range_headers_are_isolated_by_body_domain_and_copies_keep_their_row() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut outer = IntakeBody::with_store(Rc::clone(&store));
    let mut local = IntakeBody::with_store(Rc::clone(&store));

    let outer_limit = outer.declare_pending_binding("outer_limit", "2");
    let outer_counter = outer.declare_pending_range_counter("outer_item", outer_limit);
    let local_limit = local.declare_pending_binding("local_limit", "3");
    let local_counter = local.declare_pending_range_counter("local_item", local_limit);
    let outer_domain = outer.domain();
    let local_domain = local.domain();

    assert_ne!(
        outer_domain, local_domain,
        "each body owns its numeric domain"
    );
    assert_eq!(
        outer_counter.as_usize(),
        local_counter.as_usize(),
        "independent allocators produce colliding dense counter IDs"
    );

    let outer_body = outer.empty_body_node();
    let local_body = local.empty_body_node();
    let mut outer_construction = outer.construction();
    let outer_position = outer_construction
        .record_retained_loop_range(
            outer_domain,
            DeferredRangeHeader::Bound(Box::new(RetainedRangeCounter {
                counter: outer_counter,
                index: None,
            })),
            outer_body,
            None,
        )
        .expect("outer range registers");
    let mut local_construction = local.construction();
    let local_position = local_construction
        .record_retained_loop_range(
            local_domain,
            DeferredRangeHeader::Bound(Box::new(RetainedRangeCounter {
                counter: local_counter,
                index: None,
            })),
            local_body,
            None,
        )
        .expect("local range registers");

    let style = Style::default();
    prepare_control_flow_render_units(
        &outer_construction,
        ControlFlowRenderUnitRequest {
            style: &style,
            context: &outer.context,
            string_table: &mut outer.string_table,
        },
    )
    .expect("outer retained range render unit is prepared");
    prepare_control_flow_render_units(
        &local_construction,
        ControlFlowRenderUnitRequest {
            style: &style,
            context: &local.context,
            string_table: &mut local.string_table,
        },
    )
    .expect("body-local retained range render unit is prepared");

    let copied_outer_position = {
        let copied_node = store
            .borrow()
            .get_node(outer_position)
            .cloned()
            .expect("outer retained position exists");
        store.borrow_mut().push_node(copied_node)
    };
    store
        .borrow_mut()
        .replicate_pending_numeric_intakes_for_copied_node(outer_position, copied_outer_position)
        .expect("a recordless retained-counter range copy keeps its registered header");

    let outer_counter_name = outer.string_table.intern("outer_item");
    let outer_counter_path = outer
        .path_fork
        .try_intern_components(&[outer_counter_name])
        .expect("test counter path fits")
        .to_owned();

    let selected_outer = store
        .borrow()
        .pending_range_header_node_for_counter(outer_domain, outer_counter)
        .expect("outer body selects its own pending counter header");
    assert_eq!(selected_outer, outer_position);
    store
        .borrow_mut()
        .install_completed_numeric_range_header_with_counter(
            selected_outer,
            RangeLoopSpec {
                start: i32_expression(0),
                end: i32_expression(2),
                end_kind: RangeEndKind::Exclusive,
                step: None,
            },
            Declaration {
                id: outer_counter_path,
                value: i32_expression(0),
                binding_span: None,
                config_qualifier: None,
            },
        )
        .expect("the selected outer header installs");

    assert_eq!(
        store
            .borrow()
            .pending_range_header_node_for_counter(outer_domain, outer_counter),
        Some(copied_outer_position),
        "installing one outer header leaves its copied sibling pending in that domain"
    );
    assert_eq!(
        store
            .borrow()
            .pending_range_header_node_for_counter(local_domain, local_counter),
        Some(local_position),
        "the colliding local counter still selects only its own pending header"
    );

    // Keep the retained role intact while deliberately omitting its header registration.
    let unregistered_source = {
        let retained_node = store
            .borrow()
            .get_node(copied_outer_position)
            .cloned()
            .expect("copied retained position exists");
        store.borrow_mut().push_node(retained_node)
    };
    let unregistered_copy = {
        let copied_node = store
            .borrow()
            .get_node(unregistered_source)
            .cloned()
            .expect("unregistered retained source exists");
        store.borrow_mut().push_node(copied_node)
    };
    assert!(
        store
            .borrow_mut()
            .replicate_pending_numeric_intakes_for_copied_node(
                unregistered_source,
                unregistered_copy,
            )
            .is_err(),
        "a recordless source without a retained-counter header is unregistered"
    );
}

#[test]
fn retained_range_operand_takes_one_slot_record_and_never_installs_singly() {
    let mut body = IntakeBody::new();
    let binding = body.declare_pending_binding("pending_value", "1");
    let domain = body.domain();
    let loop_body = body.empty_body_node();
    let end_fragment = body.pending_numeric_fragment(binding);

    let header = DeferredRangeHeader::Unbound(Box::new(DeferredRangeSpec {
        start: None,
        end: StatementExpression::Retained(end_fragment),
        end_kind: RangeEndKind::Exclusive,
        step: None,
        span: None,
    }));

    let mut construction = body.construction();
    let position = construction
        .record_retained_loop_range(domain, header, loop_body, None)
        .expect("retained range registers");

    let taken = body.store.borrow_mut().take_pending_numeric_intakes(domain);
    let intake = taken
        .first()
        .expect("the retained end operand owns one intake record");
    assert_eq!(
        taken.len(),
        1,
        "an omitted start and step must not register operand records"
    );
    assert_eq!(intake.position.node_id(), position);
    assert!(
        intake.position.range_operand_slot().is_some(),
        "the record must name its operand slot so completion can group by position"
    );

    assert!(
        body.store
            .borrow_mut()
            .install_completed_numeric_intake(intake.site, bool_expression(true), None)
            .is_err(),
        "a range operand never installs on its own; the header installs once as a set"
    );
}

#[test]
#[cfg(feature = "benchmark_counters")]
fn bound_range_completion_inspects_each_pending_header_once() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };
    let _guard = lock_counter_test();
    for count in [32, 128] {
        reset_ast_counters();
        let mut body = IntakeBody::new();
        let mut sibling = IntakeBody::with_store(Rc::clone(&body.store));
        let domain = body.domain();
        let limit = body.declare_pending_binding("limit", "2");
        let sibling_limit = sibling.declare_pending_binding("limit", "2");
        let loop_body = body.empty_body_node();
        let mut sibling_construction = sibling.construction();
        let mut counters = Vec::new();
        let style = Style::default();
        for index in 0..count {
            let mut construction = body.construction();
            let counter = body.declare_pending_range_counter(&format!("counter_{index}"), limit);
            let sibling_counter =
                sibling.declare_pending_range_counter(&format!("counter_{index}"), sibling_limit);
            let source = construction
                .record_retained_loop_range(
                    domain,
                    DeferredRangeHeader::Bound(Box::new(RetainedRangeCounter {
                        counter,
                        index: None,
                    })),
                    loop_body,
                    None,
                )
                .expect("range registers");
            prepare_control_flow_render_units(
                &construction,
                ControlFlowRenderUnitRequest {
                    style: &style,
                    context: &body.context,
                    string_table: &mut body.string_table,
                },
            )
            .expect("range render unit is prepared");
            sibling_construction
                .record_retained_loop_range(
                    sibling.domain(),
                    DeferredRangeHeader::Bound(Box::new(RetainedRangeCounter {
                        counter: sibling_counter,
                        index: None,
                    })),
                    loop_body,
                    None,
                )
                .expect("sibling range registers");
            counters.push((counter, source));
        }
        let name = body.string_table.intern("completed_counter");
        let path = body
            .path_fork
            .try_intern_components(&[name])
            .expect("test path fits")
            .to_owned();
        for (counter, source) in counters {
            let copy = {
                let mut store = body.store.borrow_mut();
                let node = store.get_node(source).expect("source exists").clone();
                let copy = store.push_node(node);
                store
                    .replicate_pending_numeric_intakes_for_copied_node(source, copy)
                    .expect("bound copy registers its pending header");
                copy
            };
            for expected in [source, copy] {
                let mut store = body.store.borrow_mut();
                assert_eq!(
                    store.pending_range_header_node_for_counter(domain, counter),
                    Some(expected)
                );
                store
                    .install_completed_numeric_range_header_with_counter(
                        expected,
                        RangeLoopSpec {
                            start: i32_expression(0),
                            end: i32_expression(2),
                            end_kind: RangeEndKind::Exclusive,
                            step: None,
                        },
                        Declaration {
                            id: path,
                            value: i32_expression(0),
                            binding_span: None,
                            config_qualifier: None,
                        },
                    )
                    .expect("pending header installs");
            }
            assert_eq!(
                body.store
                    .borrow()
                    .pending_range_header_node_for_counter(domain, counter),
                None
            );
        }
        assert_eq!(
            test_read_ast_counter(AstCounter::TirNumericCounterHeadersInspected),
            2 * count,
            "completed rows and sibling domains must never be inspected",
        );
    }
}

#[test]
#[cfg(feature = "benchmark_counters")]
fn copied_positions_inspect_only_their_registered_numeric_rows() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };
    let _guard = lock_counter_test();
    for count in [32, 128] {
        reset_ast_counters();
        let mut body = IntakeBody::new();
        let binding = body.declare_pending_binding("limit", "2");
        let domain = body.domain();
        let loop_body = body.empty_body_node();
        let mut construction = body.construction();
        let mut sources = Vec::new();
        for _ in 0..count {
            let range = DeferredRangeHeader::Unbound(Box::new(DeferredRangeSpec {
                start: Some(StatementExpression::Retained(
                    body.pending_numeric_fragment(binding),
                )),
                end: StatementExpression::Retained(body.pending_numeric_fragment(binding)),
                end_kind: RangeEndKind::Exclusive,
                step: Some(StatementExpression::Retained(
                    body.pending_numeric_fragment(binding),
                )),
                span: None,
            }));
            sources.push(
                construction
                    .record_retained_loop_range(domain, range, loop_body, None)
                    .expect("retained range registers"),
            );
        }
        let mut copies = Vec::new();
        for source in sources {
            let copy = {
                let mut store = body.store.borrow_mut();
                let node = store.get_node(source).expect("source exists").clone();
                let copy = store.push_node(node);
                store
                    .replicate_pending_numeric_intakes_for_copied_node(source, copy)
                    .expect("copy registers three operands and one header");
                copy
            };
            copies.push(copy);
        }
        let mut store = body.store.borrow_mut();
        assert_eq!(
            test_read_ast_counter(AstCounter::TirNumericCopyRowsInspected),
            4 * count
        );
        let taken = store.take_pending_numeric_intakes(domain);
        assert_eq!(
            taken.len(),
            6 * count,
            "source and copied operands complete independently"
        );
        for copy in copies {
            assert_eq!(
                taken
                    .iter()
                    .filter(|row| row.position.node_id() == copy)
                    .count(),
                3
            );
        }
    }
}

#[test]
fn duplicate_numeric_intake_registration_preserves_the_original_record() {
    use super::super::numeric_intake::NumericTemplateIntakePosition;

    let mut body = IntakeBody::new();
    let binding = body.declare_pending_binding("limit", "2");
    let domain = body.domain();
    let position = NumericTemplateIntakePosition::HeadExpression(body.empty_body_node());
    let fragment = body.pending_numeric_fragment(binding);
    let mut store = body.store.borrow_mut();
    let first = store
        .register_pending_numeric_template_intake(
            domain,
            position,
            fragment.items.clone(),
            fragment.expected,
            fragment.value_mode.clone(),
            fragment.suffix,
        )
        .expect("first registration succeeds");
    let error = store
        .register_pending_numeric_template_intake(
            domain,
            position,
            fragment.items,
            fragment.expected,
            fragment.value_mode,
            fragment.suffix,
        )
        .expect_err("the same position cannot own a second intake");
    assert!(error.msg.contains("registered twice"));
    assert_eq!(store.numeric_template_intakes.len(), 1);
    let pending = store.take_pending_numeric_intakes(domain);
    assert_eq!(
        pending.len(),
        1,
        "failed registration must not append to the domain queue"
    );
    assert_eq!(pending[0].site, first);
    assert_eq!(pending[0].position, position);
}
