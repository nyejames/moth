//! TIR view-fold tests.
//
// WHAT: protects module-local view folding, fold determinism and overlay-aware
// folding at the owning TIR boundary.
// WHY: these tests cover the semantic fold invariants of the exact-view reducer.

use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateFoldBinding,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::fold::fold_prepared_template;
use crate::compiler_frontend::ast::templates::tir::ids::{
    SlotOccurrenceId, TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind,
};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolution,
    TirSlotResolutionOverlay,
};
use crate::compiler_frontend::ast::templates::tir::preparation::TemplatePreparationFacts;
use crate::compiler_frontend::ast::templates::tir::refs::{
    TemplateTirChildReference, TemplateTirReference,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparation, TemplatePreparationMode, TemplatePreparationOutcome, prepare_tir_view,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;

struct TextFixture {
    store: TemplateIrStore,
    template_id: TemplateIrId,
    context: TemplateViewContext,
}

fn build_text_fixture(string_table: &mut StringTable, text: &str) -> TextFixture {
    let mut store = TemplateIrStore::new();
    let text_id = string_table.intern(text);
    let mut builder = TemplateIrBuilder::new(&mut store);
    let text_node = builder.push_text_node(
        text_id,
        text.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![text_node], SourceLocation::default());
    let template_id = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    let context = TemplateViewContext::default();

    TextFixture {
        store,
        template_id,
        context,
    }
}

fn fold_context<'a>(string_table: &'a mut StringTable) -> TirFoldContext<'a> {
    TirFoldContext {
        string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    }
}

fn fold_prepared_view(
    view: &TirView<'_>,
    context: &mut TirFoldContext<'_>,
) -> Result<TemplateEmission, TemplateError> {
    let prepared = prepare_tir_view(view, TemplatePreparationMode::Value)?;
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
    // This convenience helper exposes only text; the cache provenance invariant has its own test.
    let TemplateFoldResult { emission, .. } =
        fold_prepared_template(&prepared, view.clone(), context)?;
    Ok(emission)
}

#[test]
fn fold_view_matches_direct_template_fold_for_simple_text() {
    let mut string_table = StringTable::new();
    let fixture = build_text_fixture(&mut string_table, "hello");
    let view = TirView::new(
        &fixture.store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("view should construct");

    let mut context = fold_context(&mut string_table);
    let emission = fold_prepared_view(&view, &mut context).expect("view fold should succeed");

    assert_eq!(
        emission,
        TemplateEmission::Output(ConstStringValue::Text(string_table.intern("hello")))
    );
}

#[test]
fn fold_view_is_deterministic_with_and_without_active_bindings() {
    let mut string_table = StringTable::new();
    let fixture = build_text_fixture(&mut string_table, "cached");
    let view = TirView::new(
        &fixture.store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("view should construct");

    // Folding the same view twice in one context must produce the same result.
    {
        let mut empty_context = fold_context(&mut string_table);
        let first = fold_prepared_view(&view, &mut empty_context)
            .expect("empty-binding fold should succeed");
        let second =
            fold_prepared_view(&view, &mut empty_context).expect("repeat fold should succeed");
        assert_eq!(first, second, "repeated folds must agree");
    };

    // An active binding stack does not change a view that reads no bindings.
    let path = InternedPath::from_single_str("value", &mut string_table);
    let mut active_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![TemplateFoldBinding {
            path,
            value: Expression::int(1, SourceLocation::default(), ValueMode::ImmutableOwned),
        }],
    };
    fold_prepared_view(&view, &mut active_context)
        .expect("active-binding fold should still succeed");
}

#[test]
fn prepared_view_rejects_identity_mismatch() {
    let mut string_table = StringTable::new();
    let mut fixture = build_text_fixture(&mut string_table, "identity");
    let alternate_id = {
        let text_id = string_table.intern("alternate");
        let mut builder = TemplateIrBuilder::new(&mut fixture.store);
        let node = builder.push_text_node(
            text_id,
            9,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![node], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };
    let original_view = TirView::new(
        &fixture.store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("original view should construct");
    let alternate_view = TirView::new(
        &fixture.store,
        alternate_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("alternate view should construct");
    let preparation = prepare_tir_view(&original_view, TemplatePreparationMode::Value)
        .expect("preparation should succeed");
    assert!(matches!(
        preparation.outcome,
        TemplatePreparationOutcome::Foldable
    ));

    let mut context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };
    let error = fold_prepared_template(&preparation, alternate_view, &mut context)
        .expect_err("prepared identity mismatch should fail");
    assert!(format!("{error:?}").contains("root"));
}

#[test]
fn foldable_preparation_accepts_simple_text() {
    let mut string_table = StringTable::new();
    let fixture = build_text_fixture(&mut string_table, "safe");
    let view = TirView::new(
        &fixture.store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("view should construct");
    let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("simple text should have a valid preparation");
    assert!(
        matches!(preparation.outcome, TemplatePreparationOutcome::Foldable),
        "text-only view should produce a foldable result"
    );
}

#[test]
fn fold_view_slot_overlay_resolves_filled_and_missing_to_empty() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let fill_text = string_table.intern("filled");
    let mut builder = TemplateIrBuilder::new(&mut store);
    let fill_node = builder.push_text_node(
        fill_text,
        6,
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let fill_root = builder.push_sequence_node(vec![fill_node], SourceLocation::default());
    let fill_template_id = builder.finish_template(
        fill_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );
    let slot_node = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
    let wrapper_root = builder.push_sequence_node(vec![slot_node], SourceLocation::default());
    let wrapper_template_id = builder.finish_template(
        wrapper_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    );

    // A resolved slot overlay folds the fill template into the wrapper output.
    let resolved_overlay_id = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(
                SlotOccurrenceId::new(0),
                TirSlotResolution::resolved(SlotKey::Default, vec![fill_template_id]),
            )],
        })
        .expect("test overlay allocation");
    let resolved_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(resolved_overlay_id),
        wrapper_context: None,
    };
    let resolved_view = TirView::new(
        &store,
        wrapper_template_id,
        TemplateTirPhase::Finalized,
        resolved_context,
    )
    .expect("resolved view should construct");
    let mut context = fold_context(&mut string_table);
    let resolved_emission = fold_prepared_view(&resolved_view, &mut context)
        .expect("resolved slot overlay fold should succeed");
    assert_eq!(
        resolved_emission,
        TemplateEmission::Output(ConstStringValue::Text(fill_text)),
        "resolved slot overlay must fold the fill template into output"
    );

    // An unresolved slot overlay folds to structural no-output.
    let missing_overlay_id = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay::default())
        .expect("test overlay allocation");
    let missing_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(missing_overlay_id),
        wrapper_context: None,
    };
    let missing_view = TirView::new(
        &store,
        wrapper_template_id,
        TemplateTirPhase::Finalized,
        missing_context,
    )
    .expect("missing view should construct");
    let missing_emission = fold_prepared_view(&missing_view, &mut context)
        .expect("unresolved slot should fold to no output");
    assert_eq!(
        missing_emission,
        TemplateEmission::NoOutput,
        "unresolved slot overlay must fold to structural no-output"
    );
}

// -------------------------
//  Additional surviving fold-view invariants
// -------------------------

/// Builds a template whose root is a single child-template reference.
fn finish_single_child_template(
    store: &mut TemplateIrStore,
    child_reference: TemplateTirChildReference,
) -> TemplateIrId {
    let occurrence_id = store.next_child_template_occurrence_id();
    let child_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_reference,
            occurrence_id,
        },
        SourceLocation::default(),
    ));
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![child_node],
        },
        SourceLocation::default(),
    ));
    store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    ))
}

/// Builds a finalized text template and returns its id plus the text intern id.
fn text_template(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    text: &str,
) -> TemplateIrId {
    let text_id = string_table.intern(text);
    let mut builder = TemplateIrBuilder::new(store);
    let node = builder.push_text_node(
        text_id,
        text.len(),
        TemplateSegmentOrigin::Body,
        SourceLocation::default(),
    );
    let root = builder.push_sequence_node(vec![node], SourceLocation::default());
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    )
}

#[test]
fn fold_prepared_template_rejects_parsed_phase() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let template_id = text_template(&mut store, &mut string_table, "parsed");
    let context = TemplateViewContext::default();
    let view = TirView::new(&store, template_id, TemplateTirPhase::Parsed, context)
        .expect("view should construct");
    let mut context = fold_context(&mut string_table);

    let prepared = TemplatePreparation {
        identity: view.identity(),
        facts: TemplatePreparationFacts {
            is_const_evaluable_shape: true,
            has_unresolved_slot_occurrences: false,
            has_resolved_slot_sources: false,
            has_escaped_insert_helpers: false,
            wrapper_foldable: true,
            has_runtime_slot_plan: false,
            has_runtime_slot_sites: false,
            has_reactive_dependence: false,
            final_value_kind:
                crate::compiler_frontend::ast::templates::template::TemplateConstValueKind::RenderableString,
        },
        outcome: TemplatePreparationOutcome::Foldable,
    };
    let error = fold_prepared_template(&prepared, view, &mut context)
        .expect_err("a Parsed view must not fold");

    assert!(
        format!("{error:?}").contains("Composed"),
        "error must name the required Composed phase: {error:?}"
    );
}

#[test]
fn prepared_fold_rejects_missing_node_in_untaken_branch() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let body = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("taken"),
            byte_len: 5,
            origin: TemplateSegmentOrigin::Body,
        },
        SourceLocation::default(),
    ));
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(Expression::bool(
            false,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        body,
        SourceLocation::default(),
        store.next_expression_site_id(),
    );
    let missing_body = TemplateIrNodeId::new(999);
    let untaken_branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(Expression::bool(
            true,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        missing_body,
        SourceLocation::default(),
        store.next_expression_site_id(),
    );
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch, untaken_branch],
            fallback: None,
        },
        SourceLocation::default(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    ));
    let context = TemplateViewContext::default();
    let view = TirView::new(&store, template_id, TemplateTirPhase::Composed, context)
        .expect("view should construct");
    let mut context = fold_context(&mut string_table);

    let error = fold_prepared_view(&view, &mut context)
        .expect_err("a missing node in an untaken branch must still be rejected");

    assert!(
        format!("{error:?}").contains("node"),
        "error must report the missing node: {error:?}"
    );
}

#[test]
fn prepared_fold_emits_each_occurrence_of_a_repeated_composed_child_view() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let child_template_id = text_template(&mut store, &mut string_table, "child");
    let child_reference = TemplateTirChildReference::new(
        child_template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    );

    let parent_template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let first_child = builder
            .push_child_template_node_with_reference(child_reference, SourceLocation::default());
        let second_child = builder
            .push_child_template_node_with_reference(child_reference, SourceLocation::default());
        let root =
            builder.push_sequence_node(vec![first_child, second_child], SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let view = TirView::new(
        &store,
        parent_template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("parent view should construct");
    let expected_output = string_table.intern("childchild");
    let mut context = fold_context(&mut string_table);

    let emission = fold_prepared_view(&view, &mut context)
        .expect("repeated composed child views should fold successfully");

    assert_eq!(
        emission,
        TemplateEmission::Output(ConstStringValue::Text(expected_output))
    );
}

#[test]
fn prepared_fold_preserves_root_expression_overlay_through_nested_children() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let empty_context = TemplateViewContext::default();

    let structural_text = string_table.intern("structural-leaf");
    let leaf_template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let leaf_expression = builder.push_dynamic_expression_node(
            Expression::string_slice(
                structural_text,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let leaf_root =
            builder.push_sequence_node(vec![leaf_expression], SourceLocation::default());
        builder.finish_template(
            leaf_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };
    let middle_template_id = finish_single_child_template(
        &mut store,
        TemplateTirChildReference::new(leaf_template_id, TemplateTirPhase::Composed, empty_context),
    );
    let root_template_id = finish_single_child_template(
        &mut store,
        TemplateTirChildReference::new(
            middle_template_id,
            TemplateTirPhase::Composed,
            empty_context,
        ),
    );

    // Recover the leaf dynamic-expression site id from the leaf template root.
    let leaf_site_id = {
        let leaf_root = store
            .get_template(leaf_template_id)
            .expect("leaf template should exist")
            .root;
        let leaf_node = store.get_node(leaf_root).expect("leaf root should exist");
        let TemplateIrNodeKind::Sequence { children } = &leaf_node.kind else {
            panic!("leaf root should be a sequence");
        };
        let expression_node = store
            .get_node(children[0])
            .expect("leaf expression node should exist");
        let TemplateIrNodeKind::DynamicExpression { site_id, .. } = expression_node.kind else {
            panic!("leaf child should be a dynamic expression");
        };
        site_id
    };

    let first_text = string_table.intern("first-root-overlay");
    let second_text = string_table.intern("second-root-overlay");
    let first_context = {
        let overlay_id = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(
                    leaf_site_id,
                    Box::new(Expression::string_slice(
                        first_text,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )),
                )],
            })
            .expect("test overlay allocation");
        TemplateViewContext {
            expression_overlay: Some(overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        }
    };
    let second_context = {
        let overlay_id = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(
                    leaf_site_id,
                    Box::new(Expression::string_slice(
                        second_text,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )),
                )],
            })
            .expect("test overlay allocation");
        TemplateViewContext {
            expression_overlay: Some(overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        }
    };

    let first_view = TirView::new(
        &store,
        root_template_id,
        TemplateTirPhase::Composed,
        first_context,
    )
    .expect("first view should construct");
    let second_view = TirView::new(
        &store,
        root_template_id,
        TemplateTirPhase::Composed,
        second_context,
    )
    .expect("second view should construct");

    let first = {
        let mut context = fold_context(&mut string_table);
        fold_prepared_view(&first_view, &mut context)
    }
    .expect("first overlay fold should succeed");
    let second = {
        let mut context = fold_context(&mut string_table);
        fold_prepared_view(&second_view, &mut context)
    }
    .expect("second overlay fold should succeed");

    assert_eq!(
        first,
        TemplateEmission::Output(ConstStringValue::Text(first_text))
    );
    assert_eq!(
        second,
        TemplateEmission::Output(ConstStringValue::Text(second_text))
    );
}

#[test]
fn repeated_prepared_fold_reuses_effective_expression_provenance() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let text = string_table.intern("effective");
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "effective",
    );
    let (template_id, site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic = builder.push_dynamic_expression_node(
            Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(vec![dynamic], SourceLocation::default());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let TemplateIrNodeKind::DynamicExpression { site_id, .. } = &store
            .get_node(dynamic)
            .expect("dynamic node should exist")
            .kind
        else {
            panic!("expected dynamic expression node")
        };
        (template_id, *site_id)
    };
    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                site_id,
                Box::new(
                    Expression::string_slice(
                        text,
                        SourceLocation::default(),
                        ValueMode::ImmutableOwned,
                    )
                    .with_synthetic_interface_provenance(
                        SyntheticInterfaceProvenance::single(member.clone()),
                    ),
                ),
            )],
        })
        .expect("test overlay allocation");
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateViewContext {
            expression_overlay: Some(overlay_id),
            ..TemplateViewContext::default()
        },
    )
    .expect("effective view should construct");
    let prepared =
        prepare_tir_view(&view, TemplatePreparationMode::Value).expect("view should prepare");
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
    let mut context = fold_context(&mut string_table);
    let first = fold_prepared_template(&prepared, view.clone(), &mut context)
        .expect("first exact fold should succeed");
    let second = fold_prepared_template(&prepared, view, &mut context)
        .expect("repeated exact fold should succeed");

    assert_eq!(
        first, second,
        "repeated folds must retain the exact provenance"
    );
    assert_eq!(
        first.provenance.members(),
        &[member],
        "the fold result must retain effective dynamic-expression provenance"
    );
}

#[test]
fn prepared_fold_below_composed_child_ignores_unconsumed_overlay_identity() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let parent_context = TemplateViewContext::default();
    let missing_context = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };
    let child_text = string_table.intern("parsed child");

    let parent_template_id = {
        let child_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Text {
                text: child_text,
                byte_len: "parsed child".len(),
                origin: TemplateSegmentOrigin::Body,
            },
            SourceLocation::default(),
        ));
        let child_template_id = store.push_template(TemplateIr::new(
            child_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        ));
        let occurrence_id = store.next_child_template_occurrence_id();
        let parent_child_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::ChildTemplate {
                reference: TemplateTirChildReference::new(
                    child_template_id,
                    TemplateTirPhase::Parsed,
                    missing_context,
                ),
                occurrence_id,
            },
            SourceLocation::default(),
        ));
        let parent_root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![parent_child_node],
            },
            SourceLocation::default(),
        ));
        store.push_template(TemplateIr::new(
            parent_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        ))
    };

    let view = TirView::new(
        &store,
        parent_template_id,
        TemplateTirPhase::Composed,
        parent_context,
    )
    .expect("parent view should construct");
    let mut context = fold_context(&mut string_table);

    let emission = fold_prepared_view(&view, &mut context)
        .expect("a Parsed child's unconsumed overlay identity must not block folding");

    assert_eq!(
        emission,
        TemplateEmission::Output(ConstStringValue::Text(child_text))
    );
}

// -------------------------
//  Deferred fold-authority and attribution invariants
// -------------------------
//
#[test]
fn prepared_runtime_plan_validates_plan_authority_before_handoff() {
    let mut string_table = StringTable::new();
    let mut fixture = build_text_fixture(&mut string_table, "runtime plan");
    let missing_slot_plan_id = TemplateSlotPlanId::new(999);
    super::super::store::MalformedTirStore::new(&mut fixture.store)
        .set_runtime_slot_plan(fixture.template_id, Some(missing_slot_plan_id));

    let view = TirView::new(
        &fixture.store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("view should construct");
    let error = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect_err("preparation must validate its required runtime slot plan");
    assert!(
        format!("{error:?}").contains("TIR preparation: slot plan"),
        "expected a stable runtime-plan authority error, got: {error:?}"
    );
}

/// Folds an outer template whose dynamic-expression payload is a nested AST
/// template whose root node is missing, so fold traversal catches the
/// malformed nested-template authority on the infrastructure lane.
fn fold_dynamic_ast_template_with_missing_root_authority() -> TemplateError {
    let mut string_table = StringTable::new();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let outer_template_id = {
        let mut tir = store.borrow_mut();
        let nested_template_id = tir.push_template(TemplateIr::new(
            TemplateIrNodeId::new(999),
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        ));

        let nested_template = Template {
            tir_reference: TemplateTirReference {
                root: nested_template_id,
                phase: TemplateTirPhase::Composed,
                context,
            },
            location: SourceLocation::default(),
        };

        let mut builder = TemplateIrBuilder::new(&mut tir);
        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::template(nested_template, ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let outer_root = builder.push_sequence_node(vec![dynamic_node], SourceLocation::default());
        builder.finish_template(
            outer_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let store_ref = store.borrow();
    let view = TirView::new(
        &store_ref,
        outer_template_id,
        TemplateTirPhase::Composed,
        context,
    )
    .expect("outer view should construct");
    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };

    fold_prepared_view(&view, &mut fold_context)
        .expect_err("dynamic AST templates must enter their own fold authority boundary")
}

#[test]
fn prepared_fold_dynamic_ast_template_validates_malformed_root_authority() {
    let error = fold_dynamic_ast_template_with_missing_root_authority();
    let TemplateError::Infrastructure(error) = error else {
        panic!("malformed dynamic template root should remain on the infrastructure lane");
    };
    assert!(
        error.msg.contains("does not exist in the module store"),
        "expected dynamic-template root authority failure, got: {}",
        error.msg
    );
}

#[test]
fn prepared_fold_rejects_direct_sequence_node_cycle_as_infrastructure() {
    let mut store = TemplateIrStore::new();
    let context = TemplateViewContext::default();
    // The first pushed node gets index 0, so a Sequence root whose only child
    // is `TemplateIrNodeId::new(0)` is a malformed self-cycle.
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![TemplateIrNodeId::new(0)],
        },
        SourceLocation::default(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    ));

    let view = TirView::new(&store, template_id, TemplateTirPhase::Composed, context)
        .expect("cyclic view should construct");
    let mut string_table = StringTable::new();
    let mut fold_context = fold_context(&mut string_table);
    let TemplateError::Infrastructure(error) =
        fold_prepared_view(&view, &mut fold_context).expect_err("direct cycle must fail")
    else {
        panic!("direct node cycle must remain on the infrastructure lane");
    };
    assert!(
        error.msg.contains("recursively referenced directly"),
        "expected a direct-cycle authority error, got: {}",
        error.msg
    );
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn prepared_fold_increments_phase1_attribution_counters() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };

    let _guard = lock_counter_test();

    let mut string_table = StringTable::new();
    let fixture = build_text_fixture(&mut string_table, "counter probe");
    let view = TirView::new(
        &fixture.store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("view should construct");
    let mut fold_context = fold_context(&mut string_table);

    reset_ast_counters();
    let first = fold_prepared_view(&view, &mut fold_context).expect("first fold should succeed");
    let second = fold_prepared_view(&view, &mut fold_context).expect("second fold should succeed");
    assert_eq!(first, second, "repeated folds must agree");

    assert_eq!(
        test_read_ast_counter(AstCounter::TirViewFoldsAttempted),
        2,
        "prepared fold should be attempted twice"
    );
    assert_eq!(
        test_read_ast_counter(AstCounter::TirViewFoldOverlayEmpty),
        2,
        "every fold attributes its overlay shape"
    );
}
