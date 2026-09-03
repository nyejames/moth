//! Final-view TIR folding tests.
//!
//! WHAT: exercises prepared exact-view folding for final effective views rooted at
//!       control-flow bodies, aggregate wrappers, formatted text, and runtime
//!       slot application rejection. These tests prove the store-backed fold
//!       path handles the supported final-view shapes.
//!
//! WHY: production finalization folds through stable store-backed `TirView`s,
//!      so the final-view entry point needs focused coverage for those surfaces.
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};

use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::fold::{
    fold_prepared_const_template_pattern, fold_prepared_template,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind,
};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::summary::TemplateIrSummary;
use crate::compiler_frontend::ast::templates::tir::view::{TemplateTirPhase, TirView};
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, RuntimeTemplateReason, TemplatePreparationMode,
    TemplatePreparationOutcome, prepare_tir_view,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::path::Path;

use std::cell::RefCell;
use std::rc::Rc;

fn build_test_fold_context<'a>(string_table: &'a mut StringTable) -> TirFoldContext<'a> {
    TirFoldContext {
        string_table,
        template_const_loop_iteration_limit:
            crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    }
}

fn int_expression(value: i32) -> Expression {
    Expression::int(value, SourceLocation::default(), ValueMode::ImmutableOwned)
}

fn bool_expression(value: bool) -> Expression {
    Expression::bool(value, SourceLocation::default(), ValueMode::ImmutableOwned)
}

fn emission_to_string(emission: TemplateEmission, string_table: &StringTable) -> String {
    match emission {
        TemplateEmission::NoOutput => String::new(),
        TemplateEmission::Output(ConstStringValue::Text(output))
        | TemplateEmission::Break(Some(ConstStringValue::Text(output)))
        | TemplateEmission::Continue(Some(ConstStringValue::Text(output))) => {
            string_table.resolve(output).to_owned()
        }
        TemplateEmission::Output(ConstStringValue::Pieces(_))
        | TemplateEmission::Break(Some(ConstStringValue::Pieces(_)))
        | TemplateEmission::Continue(Some(ConstStringValue::Pieces(_))) => {
            panic!("structural emission reached a text-only assertion")
        }
        TemplateEmission::Break(None) | TemplateEmission::Continue(None) => String::new(),
    }
}

/// Builds a shared store and a view over a freshly constructed template,
/// then folds it through the prepared exact-view entry point.
struct FinalViewFoldFixture {
    store: Rc<RefCell<TemplateIrStore>>,
    template_id: crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId,
    context: crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext,
}

fn build_final_view_fixture<F>(
    string_table: &mut StringTable,
    build_template: F,
) -> FinalViewFoldFixture
where
    F: FnOnce(
        &mut StringTable,
        &mut TemplateIrStore,
    ) -> crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId,
{
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template_id = {
        let mut store_borrow = store.borrow_mut();
        build_template(string_table, &mut store_borrow)
    };

    FinalViewFoldFixture {
        store,
        template_id,
        context,
    }
}

fn fold_final_view_fixture(
    fixture: &FinalViewFoldFixture,
    string_table: &mut StringTable,
    phase: TemplateTirPhase,
) -> Result<TemplateEmission, TemplateError> {
    let store = fixture.store.borrow();
    let view = TirView::new(&store, fixture.template_id, phase, fixture.context)
        .expect("test view should construct");

    let mut fold_context = build_test_fold_context(string_table);

    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)?;
    if !matches!(prepared.outcome, TemplatePreparationOutcome::Foldable) {
        return Err(TemplateError::Infrastructure(Box::new(
            CompilerError::compiler_error("test view was not foldable"),
        )));
    }
    // Existing final-view text assertions do not own semantic provenance.
    let TemplateFoldResult { emission, .. } =
        fold_prepared_template(&prepared, view, &mut fold_context)?;
    Ok(emission)
}

fn project_pattern_for_template(
    store: &TemplateIrStore,
    template_id: crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId,
    string_table: &mut StringTable,
) -> Result<Vec<FoldedConstTemplatePiece>, TemplateError> {
    let view = TirView::with_minimum_phase(
        store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )?;
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)?;
    let mut fold_context = build_test_fold_context(string_table);
    Ok(fold_prepared_const_template_pattern(prepared, view, &mut fold_context)?.pieces)
}

#[test]
fn const_template_fold_keeps_resource_as_text_run_boundary() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let before = string_table.intern("before");
    let after = string_table.intern("after");
    let resource_origin = StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("fold-test"),
            "pages".to_owned(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("test resource path should be portable"),
    );
    let mut resource_table = ModuleResourceTable::new();
    let resource = resource_table.intern_origin(resource_origin, SourceLocation::default());
    let structural_expression = Expression::new(
        crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind::StructuralString {
            pieces: vec![ConstStringPiece::Resource(resource), ConstStringPiece::SiteRoot],
        },
        SourceLocation::default(),
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let before_node = builder.push_text_node(
            before,
            "before".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let resource_node = builder.push_dynamic_expression_node(
            structural_expression,
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let after_node = builder.push_text_node(
            after,
            "after".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let root = builder.push_sequence_node(
            vec![before_node, resource_node, after_node],
            SourceLocation::default(),
        );
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )?;
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)?;
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
    let mut fold_context = build_test_fold_context(&mut string_table);
    let pattern = fold_prepared_const_template_pattern(prepared, view, &mut fold_context)?;

    assert_eq!(
        pattern.pieces,
        vec![
            FoldedConstTemplatePiece::Text("before".to_owned()),
            FoldedConstTemplatePiece::Resource(resource),
            FoldedConstTemplatePiece::SiteRoot,
            FoldedConstTemplatePiece::Text("after".to_owned()),
        ],
        "resource anchors must preserve authored order and prevent text-run coalescing",
    );
    assert_eq!(
        pattern.emission,
        TemplateEmission::Output(ConstStringValue::Pieces(vec![
            ConstStringPiece::Text(before),
            ConstStringPiece::Resource(resource),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(after),
        ])),
        "emission must flush text after the final structural anchor",
    );
    Ok(())
}
#[test]
fn const_template_projection_preserves_structured_slot_order() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let before = string_table.intern("before");
    let after = string_table.intern("after");
    let location = SourceLocation::default();

    let (template_id, occurrence_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let before_node = builder.push_text_node(
            before,
            "before".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location.clone());
        let after_node = builder.push_text_node(
            after,
            "after".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let root =
            builder.push_sequence_node(vec![before_node, slot_node, after_node], location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::default(),
            location,
        );
        let occurrence_id = match store.get_node(slot_node).expect("slot node should exist") {
            TemplateIrNode {
                kind: TemplateIrNodeKind::Slot { placeholder },
                ..
            } => placeholder.occurrence_id,
            _ => panic!("expected a slot node"),
        };
        (template_id, occurrence_id)
    };

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )
    .expect("slot-insert view should construct");
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)?;
    let mut fold_context = build_test_fold_context(&mut string_table);

    let pattern = fold_prepared_const_template_pattern(prepared, view, &mut fold_context)?;
    assert_eq!(
        pattern.pieces,
        vec![
            FoldedConstTemplatePiece::Text("before".to_owned()),
            FoldedConstTemplatePiece::Slot(occurrence_id),
            FoldedConstTemplatePiece::Text("after".to_owned()),
        ]
    );

    Ok(())
}

#[test]
fn const_template_projection_preserves_nested_child_slot_order() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let location = SourceLocation::default();
    let before = string_table.intern("before");
    let after = string_table.intern("after");

    let (child_template_id, child_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let before_node = builder.push_text_node(
            before,
            "before".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location.clone());
        let after_node = builder.push_text_node(
            after,
            "after".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let root =
            builder.push_sequence_node(vec![before_node, slot_node, after_node], location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location.clone(),
        );
        let occurrence = match &store.get_node(slot_node).expect("child slot exists").kind {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected child slot, got {other:?}"),
        };
        (template_id, occurrence)
    };

    let parent_template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                child_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            location.clone(),
        );
        let root = builder.push_sequence_node(vec![child_node], location.clone());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::default(),
            location,
        )
    };

    assert_eq!(
        project_pattern_for_template(&store, parent_template_id, &mut string_table)?,
        vec![
            FoldedConstTemplatePiece::Text("before".to_owned()),
            FoldedConstTemplatePiece::Slot(child_occurrence),
            FoldedConstTemplatePiece::Text("after".to_owned()),
        ]
    );

    Ok(())
}

#[test]
fn const_template_projection_preserves_selected_branch_and_fallback_slots()
-> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let location = SourceLocation::default();

    let build_branch_template = |store: &mut TemplateIrStore, selected: bool| {
        let mut builder = TemplateIrBuilder::new(store);
        let selected_slot = builder.push_slot_node(SlotKey::Default, location.clone());
        let fallback_slot = builder.push_slot_node(SlotKey::Default, location.clone());
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(selected)),
            selected_slot,
            location.clone(),
            builder.store.next_expression_site_id(),
        );
        let root =
            builder.push_branch_chain_node(vec![branch], Some(fallback_slot), location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location.clone(),
        );
        let selected_occurrence = match &store
            .get_node(selected_slot)
            .expect("selected slot exists")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected selected slot, got {other:?}"),
        };
        let fallback_occurrence = match &store
            .get_node(fallback_slot)
            .expect("fallback slot exists")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected fallback slot, got {other:?}"),
        };
        (template_id, selected_occurrence, fallback_occurrence)
    };

    let (selected_template, selected_occurrence, selected_fallback) =
        build_branch_template(&mut store, true);
    assert_eq!(
        project_pattern_for_template(&store, selected_template, &mut string_table)?,
        vec![FoldedConstTemplatePiece::Slot(selected_occurrence)]
    );
    assert_ne!(selected_occurrence, selected_fallback);

    let (fallback_template, _selected_occurrence, fallback_occurrence) =
        build_branch_template(&mut store, false);
    assert_eq!(
        project_pattern_for_template(&store, fallback_template, &mut string_table)?,
        vec![FoldedConstTemplatePiece::Slot(fallback_occurrence)]
    );

    Ok(())
}

#[test]
fn const_template_projection_repeats_slots_in_const_loops() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let location = SourceLocation::default();
    let (template_id, occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_slot_node(SlotKey::Default, location.clone());
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: int_expression(0),
                end: int_expression(2),
                step: None,
                end_kind: RangeEndKind::Exclusive,
            }),
        };
        let root = builder.push_loop_node(header, body, None, location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        );
        let occurrence = match &store.get_node(body).expect("loop slot exists").kind {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected loop slot, got {other:?}"),
        };
        (template_id, occurrence)
    };

    assert_eq!(
        project_pattern_for_template(&store, template_id, &mut string_table)?,
        vec![
            FoldedConstTemplatePiece::Slot(occurrence),
            FoldedConstTemplatePiece::Slot(occurrence),
        ]
    );

    Ok(())
}

#[test]
fn const_template_projection_preserves_slot_in_child_wrapper() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let location = SourceLocation::default();
    let named_key = SlotKey::Named(string_table.intern("named"));

    let (parent_template_id, wrapper_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let wrapper_slot = builder.push_slot_node(named_key, location.clone());
        let wrapper_root = builder.push_sequence_node(vec![wrapper_slot], location.clone());
        let wrapper_template_id = builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            location.clone(),
        );
        let wrapper_occurrence = match &builder
            .store
            .get_node(wrapper_slot)
            .expect("wrapper slot exists")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected wrapper slot, got {other:?}"),
        };
        let child_text = string_table.intern("child");
        let child_node = builder.push_text_node(
            child_text,
            "child".len(),
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let child_root = builder.push_sequence_node(vec![child_node], location.clone());
        let parent_template_id = builder.finish_template(
            child_root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        );
        let wrapper_set = builder.store.push_or_reuse_wrapper_set(vec![
            crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference::new(
                wrapper_template_id,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
        ]);
        builder
            .store
            .set_conditional_child_wrapper_set(parent_template_id, wrapper_set)
            .expect("wrapper set should attach to parent");
        (parent_template_id, wrapper_occurrence)
    };

    assert_eq!(
        project_pattern_for_template(&store, parent_template_id, &mut string_table)?,
        vec![
            FoldedConstTemplatePiece::Slot(wrapper_occurrence),
            FoldedConstTemplatePiece::Text("child".to_owned()),
        ]
    );

    Ok(())
}

#[test]
fn const_template_projection_preserves_loop_aggregate_content() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let location = SourceLocation::default();
    let (template_id, body_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body_slot = builder.push_slot_node(SlotKey::Default, location.clone());
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: int_expression(0),
                end: int_expression(2),
                step: None,
                end_kind: RangeEndKind::Exclusive,
            }),
        };
        let aggregate_output = builder.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::AggregateOutput,
            location.clone(),
        ));
        let open = builder.push_text_node(
            string_table.intern("<"),
            1,
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let close = builder.push_text_node(
            string_table.intern(">"),
            1,
            TemplateSegmentOrigin::Body,
            location.clone(),
        );
        let aggregate_wrapper =
            builder.push_sequence_node(vec![open, aggregate_output, close], location.clone());
        let root =
            builder.push_loop_node(header, body_slot, Some(aggregate_wrapper), location.clone());
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        );
        let body_occurrence = match &builder
            .store
            .get_node(body_slot)
            .expect("body slot exists")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected body slot, got {other:?}"),
        };
        (template_id, body_occurrence)
    };

    assert_eq!(
        project_pattern_for_template(&store, template_id, &mut string_table)?,
        vec![
            FoldedConstTemplatePiece::Text("<".to_owned()),
            FoldedConstTemplatePiece::Slot(body_occurrence),
            FoldedConstTemplatePiece::Slot(body_occurrence),
            FoldedConstTemplatePiece::Text(">".to_owned()),
        ]
    );

    Ok(())
}

#[test]
fn const_template_projection_keeps_structural_no_output_empty() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let location = SourceLocation::default();

    let false_branch_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let hidden_slot = builder.push_slot_node(SlotKey::Default, location.clone());
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(false)),
            hidden_slot,
            location.clone(),
            builder.store.next_expression_site_id(),
        );
        let root = builder.push_branch_chain_node(vec![branch], None, location.clone());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location.clone(),
        )
    };
    assert!(
        project_pattern_for_template(&store, false_branch_template, &mut string_table,)?.is_empty()
    );

    let zero_iteration_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_slot_node(SlotKey::Default, location.clone());
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: int_expression(0),
                end: int_expression(0),
                step: None,
                end_kind: RangeEndKind::Exclusive,
            }),
        };
        let root = builder.push_loop_node(header, body, None, location.clone());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        )
    };
    assert!(
        project_pattern_for_template(&store, zero_iteration_template, &mut string_table,)?
            .is_empty()
    );

    Ok(())
}

// -------------------------
//  Branch/fallback bodies
// -------------------------

#[test]
fn final_view_fold_branch_selects_body() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let yes_text = string_table.intern("yes");
        let yes_node = builder.push_text_node(
            yes_text,
            3,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(true)),
            yes_node,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let root = builder.push_branch_chain_node(vec![branch], None, SourceLocation::default());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    });

    let emission = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect("final view fold should succeed");

    assert_eq!(
        emission_to_string(emission, &string_table),
        "yes",
        "true branch body should be selected through the final view"
    );
}

#[test]
fn final_view_fold_false_branch_no_else_is_no_output() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let yes_text = string_table.intern("yes");
        let yes_node = builder.push_text_node(
            yes_text,
            3,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(false)),
            yes_node,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let root = builder.push_branch_chain_node(vec![branch], None, SourceLocation::default());

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    });

    let emission = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect("final view fold should succeed");

    assert_eq!(
        emission,
        TemplateEmission::NoOutput,
        "false branch with no else should produce structural no-output"
    );
}

#[test]
fn final_view_fold_false_branch_selects_fallback() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let yes_text = string_table.intern("yes");
        let no_text = string_table.intern("no");
        let yes_node = builder.push_text_node(
            yes_text,
            3,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let fallback_node = builder.push_text_node(
            no_text,
            2,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(bool_expression(false)),
            yes_node,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let root = builder.push_branch_chain_node(
            vec![branch],
            Some(fallback_node),
            SourceLocation::default(),
        );

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    });

    let emission = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect("final view fold should succeed");

    assert_eq!(
        emission_to_string(emission, &string_table),
        "no",
        "fallback body should be selected when all branches are false"
    );
}

// -------------------------
//  Loop bodies
// -------------------------

fn build_range_loop_template(
    _string_table: &mut StringTable,
    store: &mut TemplateIrStore,
    start: i32,
    end: i32,
    body_root: crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId,
    aggregate_wrapper: Option<crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId>,
) -> crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let header = TemplateLoopHeader::Range {
        bindings: Box::new(LoopBindings {
            item: None,
            index: None,
        }),
        range: Box::new(RangeLoopSpec {
            start: int_expression(start),
            end: int_expression(end),
            step: None,
            end_kind: RangeEndKind::Exclusive,
        }),
    };
    let root = builder.push_loop_node(
        header,
        body_root,
        aggregate_wrapper,
        SourceLocation::default(),
    );

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        SourceLocation::default(),
    )
}

#[test]
fn final_view_fold_loop_body_concatenates_iterations() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let dot_text = string_table.intern(".");
        let dot_node = builder.push_text_node(
            dot_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        build_range_loop_template(string_table, store, 0, 3, dot_node, None)
    });

    let emission = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect("final view fold should succeed");

    assert_eq!(
        emission_to_string(emission, &string_table),
        "...",
        "range loop body should be repeated through the final view"
    );
}

#[test]
fn final_view_fold_loop_binding_provenance_reaches_exact_result() {
    let mut string_table = StringTable::new();
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "range",
    );
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let item_path = InternedPath::from_single_str("item", string_table);
        let mut builder = TemplateIrBuilder::new(store);
        let body = builder.push_dynamic_expression_node(
            Expression::reference_with_type_id(
                item_path.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                SourceLocation::default(),
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let range_provenance = SyntheticInterfaceProvenance::single(member.clone());
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: Some(Declaration {
                    id: item_path,
                    value: Expression::int(0, SourceLocation::default(), ValueMode::ImmutableOwned),
                    config_qualifier: None,
                }),
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: Expression::int(0, SourceLocation::default(), ValueMode::ImmutableOwned)
                    .with_synthetic_interface_provenance(range_provenance.clone()),
                end: Expression::int(2, SourceLocation::default(), ValueMode::ImmutableOwned)
                    .with_synthetic_interface_provenance(range_provenance),
                step: None,
                end_kind: RangeEndKind::Exclusive,
            }),
        };
        let root = builder.push_loop_node(header, body, None, SourceLocation::default());
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    });

    let store = fixture.store.borrow();
    let view = TirView::new(
        &store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("range loop view should construct");
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("range loop view should prepare");
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
    let mut fold_context = build_test_fold_context(&mut string_table);
    let result = fold_prepared_template(&prepared, view, &mut fold_context)
        .expect("range loop exact fold should succeed");

    assert_eq!(
        emission_to_string(result.emission, &string_table),
        "01",
        "the selected range loop should render its bound values"
    );
    assert_eq!(
        result.provenance.members(),
        &[member],
        "range provenance must reach the exact folded result through the resolved binding"
    );
}

#[test]
fn final_view_fold_zero_iteration_loop_is_no_output() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let dot_text = string_table.intern(".");
        let dot_node = builder.push_text_node(
            dot_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        build_range_loop_template(string_table, store, 0, 0, dot_node, None)
    });

    let emission = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect("final view fold should succeed");

    assert_eq!(
        emission,
        TemplateEmission::NoOutput,
        "zero-iteration loop should produce structural no-output"
    );
}

#[test]
fn final_view_fold_zero_iteration_loop_rejects_missing_body_authority() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        build_range_loop_template(
            string_table,
            store,
            0,
            0,
            crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId::new(999),
            None,
        )
    });

    let error = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect_err("zero-iteration loops must not hide malformed body authority");

    let TemplateError::Infrastructure(error) = error else {
        panic!("missing loop-body authority should remain on the infrastructure lane");
    };
    assert!(
        error.msg.contains("TIR preparation: node"),
        "expected a stable preparation node error, got: {}",
        error.msg
    );
}

#[test]
fn final_view_fold_loop_preserves_output_before_break_and_continue() {
    let mut string_table = StringTable::new();

    // [break] stops the loop after the first iteration, preserving only the
    // output produced before the break signal.
    let break_fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let dot_text = string_table.intern(".");
        let after_text = string_table.intern("after");
        let dot_node = builder.push_text_node(
            dot_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let break_node = builder
            .push_loop_control_node(TemplateLoopControlKind::Break, SourceLocation::default());
        let after_node = builder.push_text_node(
            after_text,
            5,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let body_root = builder.push_sequence_node(
            vec![dot_node, break_node, after_node],
            SourceLocation::default(),
        );
        build_range_loop_template(string_table, store, 0, 3, body_root, None)
    });
    let break_emission = fold_final_view_fixture(
        &break_fixture,
        &mut string_table,
        TemplateTirPhase::Composed,
    )
    .expect("break fold should succeed");
    assert_eq!(
        emission_to_string(break_emission, &string_table),
        ".",
        "output before [break] should be preserved once and iteration should stop"
    );

    // [continue] skips the rest of the body but continues iterating, so the
    // output before the continue signal accumulates across all iterations.
    let continue_fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let dot_text = string_table.intern(".");
        let after_text = string_table.intern("after");
        let dot_node = builder.push_text_node(
            dot_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let continue_node = builder
            .push_loop_control_node(TemplateLoopControlKind::Continue, SourceLocation::default());
        let after_node = builder.push_text_node(
            after_text,
            5,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let body_root = builder.push_sequence_node(
            vec![dot_node, continue_node, after_node],
            SourceLocation::default(),
        );
        build_range_loop_template(string_table, store, 0, 3, body_root, None)
    });
    let continue_emission = fold_final_view_fixture(
        &continue_fixture,
        &mut string_table,
        TemplateTirPhase::Composed,
    )
    .expect("continue fold should succeed");
    assert_eq!(
        emission_to_string(continue_emission, &string_table),
        "...",
        "output before [continue] should be preserved each iteration"
    );
}

// -------------------------
//  Aggregate wrapper root
// -------------------------

#[test]
fn final_view_fold_aggregate_wrapper_preserves_aggregate_output_position() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let aggregate_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::AggregateOutput,
            SourceLocation::default(),
        ));

        let mut builder = TemplateIrBuilder::new(store);
        let open_text = string_table.intern("[");
        let close_text = string_table.intern("]");
        let x_text = string_table.intern("x");

        let open_node = builder.push_text_node(
            open_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let close_node = builder.push_text_node(
            close_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let wrapper_root = builder.push_sequence_node(
            vec![open_node, aggregate_node, close_node],
            SourceLocation::default(),
        );

        let body_node = builder.push_text_node(
            x_text,
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        build_range_loop_template(string_table, store, 0, 3, body_node, Some(wrapper_root))
    });

    let emission = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect("final view fold should succeed");

    assert_eq!(
        emission_to_string(emission, &string_table),
        "[xxx]",
        "aggregate wrapper should replace AggregateOutput with the folded aggregate"
    );
}

#[test]
fn final_view_fold_validates_present_aggregate_wrapper_without_body_output() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let empty_body = builder.push_sequence_node(vec![], SourceLocation::default());
        build_range_loop_template(
            string_table,
            store,
            0,
            1,
            empty_body,
            Some(crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId::new(999)),
        )
    });

    let error = fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Composed)
        .expect_err("a present aggregate wrapper must be validated even when the body is empty");

    let TemplateError::Infrastructure(error) = error else {
        panic!("missing aggregate-wrapper authority should remain on the infrastructure lane");
    };
    assert!(
        error.msg.contains("TIR preparation: node"),
        "expected a stable aggregate-wrapper authority error, got: {}",
        error.msg
    );
}

#[test]
fn final_view_aggregate_output_outside_wrapper_classifies_as_runtime() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |_string_table, store| {
        let aggregate_node = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::AggregateOutput,
            SourceLocation::default(),
        ));

        let mut builder = TemplateIrBuilder::new(store);
        builder.finish_template(
            aggregate_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    });

    // AggregateOutput outside an aggregate wrapper is not foldable: preparation
    // classifies it as runtime so the fold path is never reached.
    let store = fixture.store.borrow();
    let view = TirView::new(
        &store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("view should construct");
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("preparation should classify AggregateOutput outside a wrapper");
    assert!(
        matches!(
            prepared.outcome,
            TemplatePreparationOutcome::Runtime(RuntimeTemplateReason::AggregateOutput)
        ),
        "AggregateOutput outside a wrapper should classify as runtime, got: {prepared:?}"
    );
}

// -------------------------
//  Formatted text
// -------------------------

fn build_formatted_markdown_fixture(string_table: &mut StringTable) -> FinalViewFoldFixture {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let style = Style {
        formatter: Some(markdown_formatter()),
        ..Style::default()
    };

    let template_id = {
        let mut store_borrow = store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store_borrow);
        let text = string_table.intern("Hello `code`");
        let root = builder.push_text_node(
            text,
            "Hello `code`".len(),
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        builder.finish_template(
            root,
            style.clone(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    };

    let formatted_root = {
        let mut store_borrow = store.borrow_mut();
        crate::compiler_frontend::ast::templates::tir::formatter_view::format_tir_template(
            &mut store_borrow,
            template_id,
            TemplateTirPhase::Parsed,
            context,
            &style,
            string_table,
        )
        .expect("TIR formatter should succeed")
        .root
    };

    let formatted_template_id = {
        let mut store_borrow = store.borrow_mut();
        store_borrow
            .push_structurally_derived_template(
                template_id,
                formatted_root,
                crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(),
            )
            .expect("formatted root should publish with a matching summary")
    };

    FinalViewFoldFixture {
        store,
        template_id: formatted_template_id,
        context,
    }
}

#[test]
fn final_view_fold_formatted_markdown_text() {
    let mut string_table = StringTable::new();
    let fixture = build_formatted_markdown_fixture(&mut string_table);
    let emission =
        fold_final_view_fixture(&fixture, &mut string_table, TemplateTirPhase::Formatted)
            .expect("formatted final view should fold");

    let output = emission_to_string(emission, &string_table);
    assert!(
        output.contains("<code>code</code>"),
        "formatted markdown should fold to rendered HTML, got: {}",
        output
    );
}

// -------------------------
//  Runtime slot applications
// -------------------------

#[test]
fn final_view_runtime_slot_application_requires_handoff() {
    let mut string_table = StringTable::new();
    let fixture = build_final_view_fixture(&mut string_table, |_string_table, store| {
        let mut builder = TemplateIrBuilder::new(store);
        let handoff = OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Text {
                text: OwnedFoldedString::Text("<shell>".to_owned()),
                reactive_subscription: None,
                location: SourceLocation::default(),
            },
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            location: SourceLocation::default(),
        };
        let expression =
            Expression::runtime_slot_application_handoff(handoff, ValueMode::ImmutableOwned);
        let dynamic_node = builder.push_dynamic_expression_node(
            expression,
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );

        builder.finish_template(
            dynamic_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            SourceLocation::default(),
        )
    });

    let store = fixture.store.borrow();
    let view = TirView::new(
        &store,
        fixture.template_id,
        TemplateTirPhase::Finalized,
        fixture.context,
    )
    .expect("final view should construct");
    let first = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("runtime slot application should prepare as runtime");
    assert!(matches!(
        first.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));

    let second = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("runtime slot application should remain a runtime result");
    assert!(matches!(
        second.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));
}
