//! Final-view TIR folding tests.
//!
//! WHAT: exercises prepared exact-view folding for final effective views rooted at
//!       control-flow bodies, aggregate wrappers, formatted text, and runtime
//!       slot application rejection. These tests prove the store-backed fold
//!       path handles the supported final-view shapes.
//!
//! WHY: production finalization folds through stable store-backed `TirView`s,
//!      so the final-view entry point needs focused coverage for those surfaces.
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore,
};

use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeEndKind, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeCursor, TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrBuilder;
use crate::compiler_frontend::ast::templates::tir::fold::{
    fold_prepared_const_template_pattern, fold_prepared_template,
};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIrNode, TemplateIrNodeKind};
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
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::ast_fixture_support::{
    make_test_variable, runtime_slot_application_handoff_expr,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::path::Path;

use std::cell::RefCell;
use std::rc::Rc;

fn build_test_fold_context<'a>(
    string_table: &'a mut StringTable,
    expressions: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
) -> TirFoldContext<'a> {
    TirFoldContext {
        string_table,
        expression_builder: expressions,
        const_values,
        template_const_loop_iteration_limit:
            crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
        source_scope: None,
    }
}

fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32")
}

fn i32_expression(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
) -> ExprId {
    expressions
        .fixed_scalar(constants, i32_value(value), None, ValueMode::ImmutableOwned)
        .expect("test i32 literal constructs")
        .id()
}

fn u64_expression(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: u64,
) -> ExprId {
    expressions
        .fixed_scalar(
            constants,
            FixedScalarValue::unsigned(FixedScalar::U64, value).expect("test literal fits U64"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test u64 literal constructs")
        .id()
}

fn float_expression(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: f64,
) -> ExprId {
    float_expression_at(expressions, constants, FixedScalar::F64, value)
}

fn float32_expression(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: f32,
) -> ExprId {
    float_expression_at(expressions, constants, FixedScalar::F32, f64::from(value))
}

/// One exact binary-float literal; the caller supplies a value already representable at the scalar.
fn float_expression_at(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    scalar: FixedScalar,
    value: f64,
) -> ExprId {
    expressions
        .fixed_scalar(
            constants,
            FixedScalarValue::binary_float(scalar, value)
                .expect("test float is exactly representable at its scalar"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test float literal constructs")
        .id()
}

fn bool_expression(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: bool,
) -> ExprId {
    expressions
        .bool(constants, value, None, ValueMode::ImmutableOwned)
        .expect("test bool literal constructs")
        .id()
}

/// Test-only structural string over the given pieces; mirrors the production
/// `structural_string` constructor (resolved text content plus a constant site).
fn structural_string_expression(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    pieces: Vec<ConstStringPiece>,
    value_mode: ValueMode,
) -> ExprId {
    let value = constants
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
            None,
        )
        .expect("test structural string content inserts");
    expressions
        .constant(value, builtin_type_ids::STRING, None, value_mode)
        .expect("test structural string constructs")
        .id()
}

fn emission_to_string(emission: TemplateEmission, string_table: &StringTable) -> String {
    match emission {
        TemplateEmission::NoOutput => String::new(),
        TemplateEmission::Output(ConstStringValue::Text(output)) => {
            string_table.resolve(output).to_owned()
        }
        TemplateEmission::Output(ConstStringValue::Pieces(_)) => {
            panic!("structural emission reached a text-only assertion")
        }
    }
}

/// Builds a shared store and a view over a freshly constructed template,
/// then folds it through the prepared exact-view entry point.
struct FinalViewFoldFixture {
    store: Rc<RefCell<TemplateIrStore>>,
    template_id: crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId,
    context: crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext,
    expressions: AstExpressionBuilder,
    const_values: ConstValueStore,
}

fn build_final_view_fixture<F>(
    string_table: &mut StringTable,
    build_template: F,
) -> FinalViewFoldFixture
where
    F: FnOnce(
        &mut StringTable,
        &mut TemplateIrStore,
        &mut AstExpressionBuilder,
        &mut ConstValueStore,
    ) -> crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId,
{
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let template_id = {
        let mut store_borrow = store.borrow_mut();
        build_template(
            string_table,
            &mut store_borrow,
            &mut expressions,
            &mut const_values,
        )
    };

    FinalViewFoldFixture {
        store,
        template_id,
        context,
        expressions,
        const_values,
    }
}

fn fold_final_view_fixture(
    fixture: &mut FinalViewFoldFixture,
    string_table: &mut StringTable,
    phase: TemplateTirPhase,
) -> Result<TemplateEmission, TemplateError> {
    let FinalViewFoldFixture {
        store,
        template_id,
        context,
        expressions,
        const_values,
    } = fixture;
    let store_ref = store.borrow();
    let view = TirView::new(&store_ref, *template_id, phase, *context)
        .expect("test view should construct");

    let prepared = prepare_tir_view(
        &view,
        expressions,
        const_values,
        TemplatePreparationMode::Value,
    )?;
    if !matches!(prepared.outcome, TemplatePreparationOutcome::Foldable) {
        return Err(TemplateError::Infrastructure(
            CompilerError::compiler_error("test view was not foldable").into(),
        ));
    }
    let mut fold_context = build_test_fold_context(string_table, expressions, const_values);
    // Existing final-view text assertions do not own semantic provenance.
    let TemplateFoldResult { emission, .. } =
        fold_prepared_template(&prepared, view, &mut fold_context)?;
    Ok(emission)
}

fn project_pattern_for_template(
    store: &TemplateIrStore,
    template_id: crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId,
    string_table: &mut StringTable,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<Vec<FoldedConstTemplatePiece>, TemplateError> {
    let view = TirView::with_minimum_phase(
        store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )?;
    let prepared = prepare_tir_view(
        &view,
        expressions,
        const_values,
        TemplatePreparationMode::ConstRequired,
    )?;
    let mut fold_context = build_test_fold_context(string_table, expressions, const_values);
    Ok(fold_prepared_const_template_pattern(prepared, view, &mut fold_context)?.pieces)
}

#[test]
fn const_template_fold_keeps_resource_as_text_run_boundary() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let resource = resource_table.intern_origin(resource_origin, None);
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let structural_expression = structural_string_expression(
        &mut expressions,
        &mut const_values,
        vec![
            ConstStringPiece::Resource(resource),
            ConstStringPiece::SiteRoot,
        ],
        ValueMode::ImmutableOwned,
    );

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let before_node =
            builder.push_text_node(before, "before".len(), TemplateSegmentOrigin::Body, None);
        let resource_node = builder.push_dynamic_expression_node(
            structural_expression,
            TemplateSegmentOrigin::Body,
            None,
        );
        let after_node =
            builder.push_text_node(after, "after".len(), TemplateSegmentOrigin::Body, None);
        let root = builder.push_sequence_node(vec![before_node, resource_node, after_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    };

    let view = TirView::with_minimum_phase(
        &store,
        template_id,
        TemplateTirPhase::Composed,
        TemplateTirPhase::Composed,
        TemplateViewContext::default(),
    )?;
    let prepared = prepare_tir_view(
        &view,
        &expressions,
        &const_values,
        TemplatePreparationMode::ConstRequired,
    )?;
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
    let mut fold_context =
        build_test_fold_context(&mut string_table, &mut expressions, &mut const_values);
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
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let before = string_table.intern("before");
    let after = string_table.intern("after");
    let location = None;

    let (template_id, occurrence_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let before_node = builder.push_text_node(
            before,
            "before".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location);
        let after_node =
            builder.push_text_node(after, "after".len(), TemplateSegmentOrigin::Body, location);
        let root = builder.push_sequence_node(vec![before_node, slot_node, after_node], location);
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
    let prepared = prepare_tir_view(
        &view,
        &expressions,
        &const_values,
        TemplatePreparationMode::ConstRequired,
    )?;
    let mut fold_context =
        build_test_fold_context(&mut string_table, &mut expressions, &mut const_values);

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
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let location = None;
    let before = string_table.intern("before");
    let after = string_table.intern("after");

    let (child_template_id, child_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let before_node = builder.push_text_node(
            before,
            "before".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location);
        let after_node =
            builder.push_text_node(after, "after".len(), TemplateSegmentOrigin::Body, location);
        let root = builder.push_sequence_node(vec![before_node, slot_node, after_node], location);
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
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
            location,
        );
        let root = builder.push_sequence_node(vec![child_node], location);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::default(),
            location,
        )
    };

    assert_eq!(
        project_pattern_for_template(
            &store,
            parent_template_id,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?,
        vec![
            FoldedConstTemplatePiece::Text("before".to_owned()),
            FoldedConstTemplatePiece::Slot(child_occurrence),
            FoldedConstTemplatePiece::Text("after".to_owned()),
        ]
    );

    Ok(())
}

#[test]
fn const_template_projection_preserves_selected_conditional_slot() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let location = None;

    let condition = bool_expression(&mut expressions, &mut const_values, true);

    let build_conditional_template = |store: &mut TemplateIrStore| {
        let mut builder = TemplateIrBuilder::new(store);
        let selected_slot = builder.push_slot_node(SlotKey::Default, location);
        let root = builder.push_conditional_node(
            TemplateBranchSelector::Bool(condition),
            selected_slot,
            location,
        );
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        );
        let selected_occurrence = match &store
            .get_node(selected_slot)
            .expect("selected slot exists")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            other => panic!("expected selected slot, got {other:?}"),
        };
        (template_id, selected_occurrence)
    };

    let (selected_template, selected_occurrence) = build_conditional_template(&mut store);
    assert_eq!(
        project_pattern_for_template(
            &store,
            selected_template,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?,
        vec![FoldedConstTemplatePiece::Slot(selected_occurrence)]
    );
    Ok(())
}

#[test]
fn const_template_projection_repeats_slots_in_const_loops() -> Result<(), TemplateError> {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let location = None;
    let (template_id, occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_slot_node(SlotKey::Default, location);
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: i32_expression(&mut expressions, &mut const_values, 0),
                end: i32_expression(&mut expressions, &mut const_values, 2),
                step: None,
                end_kind: RangeEndKind::Exclusive,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        };
        let root = builder.push_loop_node(header, body, None, location);
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
        project_pattern_for_template(
            &store,
            template_id,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?,
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
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let location = None;
    let named_key = SlotKey::Named(string_table.intern("named"));

    let (parent_template_id, wrapper_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let wrapper_slot = builder.push_slot_node(named_key, location);
        let wrapper_root = builder.push_sequence_node(vec![wrapper_slot], location);
        let wrapper_template_id = builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            location,
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
            location,
        );
        let child_root = builder.push_sequence_node(vec![child_node], location);
        let parent_template_id = builder.finish_template(
            child_root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        );
        let wrapper_set = builder.store.push_or_reuse_wrapper_set(&[
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
        project_pattern_for_template(
            &store,
            parent_template_id,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?,
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
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let location = None;
    let (template_id, body_occurrence) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body_slot = builder.push_slot_node(SlotKey::Default, location);
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: i32_expression(&mut expressions, &mut const_values, 0),
                end: i32_expression(&mut expressions, &mut const_values, 2),
                step: None,
                end_kind: RangeEndKind::Exclusive,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        };
        let aggregate_output = builder.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::AggregateOutput,
            None,
        ));
        let open = builder.push_text_node(
            string_table.intern("<"),
            1,
            TemplateSegmentOrigin::Body,
            location,
        );
        let close = builder.push_text_node(
            string_table.intern(">"),
            1,
            TemplateSegmentOrigin::Body,
            location,
        );
        let aggregate_wrapper =
            builder.push_sequence_node(vec![open, aggregate_output, close], location);
        let root = builder.push_loop_node(header, body_slot, Some(aggregate_wrapper), location);
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
        project_pattern_for_template(
            &store,
            template_id,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?,
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
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let location = None;

    let false_conditional_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let hidden_slot = builder.push_slot_node(SlotKey::Default, location);
        let root = builder.push_conditional_node(
            TemplateBranchSelector::Bool(bool_expression(
                &mut expressions,
                &mut const_values,
                false,
            )),
            hidden_slot,
            location,
        );
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        )
    };
    assert!(
        project_pattern_for_template(
            &store,
            false_conditional_template,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?
        .is_empty()
    );

    let zero_iteration_template = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_slot_node(SlotKey::Default, location);
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: i32_expression(&mut expressions, &mut const_values, 0),
                end: i32_expression(&mut expressions, &mut const_values, 0),
                step: None,
                end_kind: RangeEndKind::Exclusive,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        };
        let root = builder.push_loop_node(header, body, None, location);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::empty(),
            location,
        )
    };
    assert!(
        project_pattern_for_template(
            &store,
            zero_iteration_template,
            &mut string_table,
            &mut expressions,
            &mut const_values
        )?
        .is_empty()
    );

    Ok(())
}

// -------------------------
//  Conditional bodies
// -------------------------

#[test]
fn final_view_fold_true_conditional_selects_body() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, const_values| {
            let mut builder = TemplateIrBuilder::new(store);
            let yes_text = string_table.intern("yes");
            let yes_node = builder.push_text_node(yes_text, 3, TemplateSegmentOrigin::Body, None);
            let root = builder.push_conditional_node(
                TemplateBranchSelector::Bool(bool_expression(expressions, const_values, true)),
                yes_node,
                None,
            );

            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );

    let emission =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
            .expect("final view fold should succeed");

    assert_eq!(
        emission_to_string(emission, &string_table),
        "yes",
        "true conditional body should be selected through the final view"
    );
}

#[test]
fn final_view_fold_false_conditional_is_no_output() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, const_values| {
            let mut builder = TemplateIrBuilder::new(store);
            let yes_text = string_table.intern("yes");
            let yes_node = builder.push_text_node(yes_text, 3, TemplateSegmentOrigin::Body, None);
            let root = builder.push_conditional_node(
                TemplateBranchSelector::Bool(bool_expression(expressions, const_values, false)),
                yes_node,
                None,
            );

            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );

    let emission =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
            .expect("final view fold should succeed");

    assert_eq!(
        emission,
        TemplateEmission::NoOutput,
        "false conditional should produce structural no-output"
    );
}

// -------------------------
//  Loop bodies
// -------------------------

#[allow(
    clippy::too_many_arguments,
    reason = "test helper keeps range bounds, body root, and aggregate wrapper as separate inputs"
)]
fn build_range_loop_template(
    _string_table: &mut StringTable,
    store: &mut TemplateIrStore,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    start: i64,
    end: i64,
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
            start: i32_expression(expressions, constants, start),
            end: i32_expression(expressions, constants, end),
            step: None,
            end_kind: RangeEndKind::Exclusive,
            domain: NumericScalar::Fixed(FixedScalar::I32),
        }),
    };
    let root = builder.push_loop_node(header, body_root, aggregate_wrapper, None);

    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "test helper keeps the scalar kind, bounds, step, and body root as separate inputs"
)]
fn build_float_range_loop_template(
    store: &mut TemplateIrStore,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    scalar: FixedScalar,
    start: f64,
    end: f64,
    step: f64,
    body_root: crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId,
) -> crate::compiler_frontend::ast::templates::tir::ids::TemplateIrId {
    let mut builder = TemplateIrBuilder::new(store);
    let header = TemplateLoopHeader::Range {
        bindings: Box::new(LoopBindings {
            item: None,
            index: None,
        }),
        range: Box::new(RangeLoopSpec {
            start: float_expression_at(expressions, constants, scalar, start),
            end: float_expression_at(expressions, constants, scalar, end),
            step: Some(float_expression_at(expressions, constants, scalar, step)),
            end_kind: RangeEndKind::Exclusive,
            domain: NumericScalar::Fixed(scalar),
        }),
    };
    let root = builder.push_loop_node(header, body_root, None, None);
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        None,
    )
}

#[test]
fn final_view_fold_loop_body_concatenates_iterations() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let dot_text = string_table.intern(".");
            let dot_node = builder.push_text_node(dot_text, 1, TemplateSegmentOrigin::Body, None);
            build_range_loop_template(
                string_table,
                store,
                expressions,
                constants,
                0,
                3,
                dot_node,
                None,
            )
        },
    );

    let emission =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
            .expect("final view fold should succeed");

    assert_eq!(
        emission_to_string(emission, &string_table),
        "...",
        "range loop body should be repeated through the final view"
    );
}

#[test]
fn final_view_fold_loop_binding_and_body_provenance_reach_exact_result() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let header_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "range",
    );
    let body_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "loop_body_value",
    );
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, const_values| {
            let item_path = path_fork
                .try_intern_portable_path("item", string_table)
                .expect("test path fits");
            let mut builder = TemplateIrBuilder::new(store);
            let item_expression = expressions
                .reference_with_type_id(
                    item_path,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    None,
                    ValueMode::ImmutableReference,
                    ConstRecordState::RuntimeValue,
                )
                .expect("loop binding reference should construct")
                .id();
            let item_value = builder.push_dynamic_expression_node(
                item_expression,
                TemplateSegmentOrigin::Body,
                None,
            );
            let body_text = string_table.intern("body");
            let body_expression = expressions
                .string_slice(const_values, body_text, None, ValueMode::ImmutableOwned)
                .expect("loop body string should construct")
                .id();
            let body_expression = expressions
                .with_synthetic_interface_provenance(
                    expressions.own(body_expression),
                    SyntheticInterfaceProvenance::single(body_member.clone()),
                )
                .expect("loop body provenance should attach")
                .id();
            let body_value = builder.push_dynamic_expression_node(
                body_expression,
                TemplateSegmentOrigin::Body,
                None,
            );
            let body = builder.push_sequence_node(vec![item_value, body_value], None);
            let range_provenance = SyntheticInterfaceProvenance::single(header_member.clone());
            let header = TemplateLoopHeader::Range {
                bindings: Box::new(LoopBindings {
                    item: Some({
                        let value = i32_expression(expressions, const_values, 0);
                        make_test_variable(item_path, value, expressions)
                    }),
                    index: None,
                }),
                range: Box::new(RangeLoopSpec {
                    start: {
                        let start = i32_expression(expressions, const_values, 0);
                        expressions
                            .with_synthetic_interface_provenance(
                                expressions.own(start),
                                range_provenance.clone(),
                            )
                            .expect("range start provenance should attach")
                            .id()
                    },
                    end: {
                        let end = i32_expression(expressions, const_values, 2);
                        expressions
                            .with_synthetic_interface_provenance(
                                expressions.own(end),
                                range_provenance,
                            )
                            .expect("range end provenance should attach")
                            .id()
                    },
                    step: None,
                    end_kind: RangeEndKind::Exclusive,
                    domain: NumericScalar::Fixed(FixedScalar::I32),
                }),
            };
            let root = builder.push_loop_node(header, body, None, None);
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );

    let store = fixture.store.borrow();
    let view = TirView::new(
        &store,
        fixture.template_id,
        TemplateTirPhase::Composed,
        fixture.context,
    )
    .expect("range loop view should construct");
    let prepared = prepare_tir_view(
        &view,
        &fixture.expressions,
        &fixture.const_values,
        TemplatePreparationMode::Value,
    )
    .expect("range loop view should prepare");
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Foldable
    ));
    let mut fold_context = build_test_fold_context(
        &mut string_table,
        &mut fixture.expressions,
        &mut fixture.const_values,
    );
    let result = fold_prepared_template(&prepared, view, &mut fold_context)
        .expect("range loop exact fold should succeed");

    assert_eq!(
        emission_to_string(result.emission, &string_table),
        "0body1body",
        "the selected range loop should render its bound values and folded body payload"
    );
    assert!(
        result.provenance.members().contains(&header_member),
        "range provenance must reach the exact folded result through the resolved binding"
    );
    assert!(
        result.provenance.members().contains(&body_member),
        "folded body provenance must reach the exact result through loop aggregation"
    );
    assert_eq!(
        result.provenance.members().len(),
        2,
        "the exact result should contain the distinct header and body dependencies"
    );
}

#[test]
fn final_view_fold_loop_preserves_visited_no_output_selector_provenance() {
    let mut string_table = StringTable::new();
    let body_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "hidden_body_selector",
    );
    let build_loop_template = |string_table: &mut StringTable,
                               store: &mut TemplateIrStore,
                               expressions: &mut AstExpressionBuilder,
                               constants: &mut ConstValueStore,
                               end: i64| {
        let mut builder = TemplateIrBuilder::new(store);
        let hidden_text = string_table.intern("hidden");
        let hidden_body = builder.push_text_node(
            hidden_text,
            "hidden".len(),
            TemplateSegmentOrigin::Body,
            None,
        );
        let selector = bool_expression(expressions, constants, false);
        let selector = expressions
            .with_synthetic_interface_provenance(
                expressions.own(selector),
                SyntheticInterfaceProvenance::single(body_member.clone()),
            )
            .expect("selector provenance should attach")
            .id();
        let no_output_body = builder.push_conditional_node(
            TemplateBranchSelector::Bool(selector),
            hidden_body,
            None,
        );
        let header = TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: i32_expression(expressions, constants, 0),
                end: i32_expression(expressions, constants, end),
                step: None,
                end_kind: RangeEndKind::Exclusive,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        };
        let root = builder.push_loop_node(header, no_output_body, None, None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };
    let fold_exact_view = |fixture: &mut FinalViewFoldFixture, string_table: &mut StringTable| {
        let store = fixture.store.borrow();
        let view = TirView::new(
            &store,
            fixture.template_id,
            TemplateTirPhase::Composed,
            fixture.context,
        )
        .expect("range loop view should construct");
        let prepared = prepare_tir_view(
            &view,
            &fixture.expressions,
            &fixture.const_values,
            TemplatePreparationMode::Value,
        )
        .expect("range loop view should prepare");
        assert!(matches!(
            prepared.outcome,
            TemplatePreparationOutcome::Foldable
        ));
        let mut fold_context = build_test_fold_context(
            string_table,
            &mut fixture.expressions,
            &mut fixture.const_values,
        );
        fold_prepared_template(&prepared, view, &mut fold_context)
            .expect("range loop exact fold should succeed")
    };

    let mut visited_fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            build_loop_template(string_table, store, expressions, constants, 2)
        },
    );
    let visited_result = fold_exact_view(&mut visited_fixture, &mut string_table);
    assert_eq!(
        visited_result.emission,
        TemplateEmission::NoOutput,
        "a false conditional should leave the visited loop body at structural no-output"
    );
    assert_eq!(
        visited_result.provenance.members(),
        std::slice::from_ref(&body_member),
        "a consumed selector dependency should survive visited no-output iterations"
    );

    let mut zero_iteration_fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            build_loop_template(string_table, store, expressions, constants, 0)
        },
    );
    let zero_iteration_result = fold_exact_view(&mut zero_iteration_fixture, &mut string_table);
    assert_eq!(
        zero_iteration_result.emission,
        TemplateEmission::NoOutput,
        "an empty range should remain structural no-output"
    );
    assert!(
        zero_iteration_result.provenance.members().is_empty(),
        "a selector dependency must not reach the result when the loop body is never visited"
    );
}

#[test]
fn final_view_fold_zero_iteration_loop_is_no_output() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let dot_text = string_table.intern(".");
            let dot_node = builder.push_text_node(dot_text, 1, TemplateSegmentOrigin::Body, None);
            build_range_loop_template(
                string_table,
                store,
                expressions,
                constants,
                0,
                0,
                dot_node,
                None,
            )
        },
    );

    let emission =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
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
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            build_range_loop_template(
                string_table,
                store,
                expressions,
                constants,
                0,
                0,
                crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId::new(999),
                None,
            )
        },
    );

    let error =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
            .expect_err("zero-iteration loops must not hide malformed body authority");

    let TemplateError::Infrastructure(error) = error else {
        panic!("missing loop-body authority should remain on the infrastructure lane");
    };
    let error = &error.primary;
    assert!(
        error.msg.contains("TIR preparation: node"),
        "expected a stable preparation node error, got: {}",
        error.msg
    );
}

// -------------------------
//  Range cursor precision
// -------------------------

/// Collects exact float counters from a range cursor to completion.
///
/// WHAT: drives `ConstRangeCursor` directly so precision behaviour is pinned without
///       template rendering in the way.
/// WHY: every counter keeps its range's selected precision, and the header owns that
///      selection, so the caller supplies the domain its operand fixtures were built at
///      and this helper is the single place these tests advance a float cursor to completion.
fn collect_float_counters(
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    start: ExprId,
    end: ExprId,
    step: Option<ExprId>,
    scalar: FixedScalar,
) -> Vec<f64> {
    let range = RangeLoopSpec {
        start,
        end,
        end_kind: RangeEndKind::Exclusive,
        step,
        domain: NumericScalar::Fixed(scalar),
    };
    let mut cursor = ConstRangeCursor::new(
        &range,
        expressions,
        constants,
        crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
    .expect("range cursor should construct");
    let mut counters = Vec::new();
    while let Some(counter) = cursor.next_counter().expect("range cursor should advance") {
        counters.push(
            counter
                .as_f64()
                .expect("float ranges must emit binary-float counters"),
        );
    }
    counters
}

#[test]
fn range_cursor_rounds_float32_step_accumulation() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let start = float32_expression(&mut expressions, &mut constants, 0.0);
    let end = float32_expression(&mut expressions, &mut constants, 1.0);
    let step = float32_expression(&mut expressions, &mut constants, 0.1);
    let counters = collect_float_counters(
        &expressions,
        &constants,
        start,
        end,
        Some(step),
        FixedScalar::F32,
    );

    assert_eq!(
        counters.len(),
        10,
        "F32 0.0 to 1.0 by 0.1 should still take ten const iterations"
    );
    assert_eq!(
        counters[3],
        f64::from(0.1_f32 + 0.1_f32 + 0.1_f32),
        "F32 counters must accumulate with f32 rounding, not f64 drift"
    );
}

#[test]
fn range_cursor_rounds_float16_step_accumulation() {
    // F16(0.1) is 0.099_975_585_937_5; every accumulation rounds back to F16, and the
    // third F16 accumulation is 0.299_804_687_5.
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let start = float_expression_at(&mut expressions, &mut constants, FixedScalar::F16, 0.0);
    let end = float_expression_at(&mut expressions, &mut constants, FixedScalar::F16, 1.0);
    let step = float_expression_at(
        &mut expressions,
        &mut constants,
        FixedScalar::F16,
        0.099_975_585_937_5,
    );
    let counters = collect_float_counters(
        &expressions,
        &constants,
        start,
        end,
        Some(step),
        FixedScalar::F16,
    );

    assert_eq!(
        counters.len(),
        10,
        "F16 0.0 to 1.0 by 0.1 should take ten const iterations, unlike F64"
    );
    assert_eq!(
        counters[3], 0.299_804_687_5,
        "F16 counters must accumulate with F16 rounding, not an F32 or f64 carrier"
    );
}

#[test]
fn range_cursor_rejects_non_progressing_float32_range() {
    // 16777217 is not representable at F32 precision, so a step of 1.0 rounds back to
    // 16777216. The source-level bounds diagnostic may occur during construction or
    // cursor advancement, so consume up to the normal template iteration limit without
    // pinning its timing.
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let range = RangeLoopSpec {
        start: float_expression_at(
            &mut expressions,
            &mut constants,
            FixedScalar::F32,
            16_777_216.0,
        ),
        end: float_expression_at(
            &mut expressions,
            &mut constants,
            FixedScalar::F32,
            16_777_218.0,
        ),
        end_kind: RangeEndKind::Exclusive,
        step: Some(float_expression_at(
            &mut expressions,
            &mut constants,
            FixedScalar::F32,
            1.0,
        )),
        domain: NumericScalar::Fixed(FixedScalar::F32),
    };
    let error =
        expect_non_progressing_range(&expressions, &constants, range, "a stalled F32 update");
    assert_non_progressing_bounds_diagnostic(error, "a stalled F32 update");
}

#[test]
fn range_cursor_rejects_non_progressing_float16_range() {
    // 2049 is not representable at F16 precision, so a step of 1.0 from 2048 rounds back
    // to 2048. An F32 carrier would advance, which is exactly the removed floor.
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let range = RangeLoopSpec {
        start: float_expression_at(&mut expressions, &mut constants, FixedScalar::F16, 2048.0),
        end: float_expression_at(&mut expressions, &mut constants, FixedScalar::F16, 2052.0),
        end_kind: RangeEndKind::Exclusive,
        step: Some(float_expression_at(
            &mut expressions,
            &mut constants,
            FixedScalar::F16,
            1.0,
        )),
        domain: NumericScalar::Fixed(FixedScalar::F16),
    };
    let error =
        expect_non_progressing_range(&expressions, &constants, range, "a stalled F16 update");
    assert_non_progressing_bounds_diagnostic(error, "a stalled F16 update");
}

/// Advances one range cursor to its non-progressing source diagnostic.
///
/// WHAT: accepts either construction or advancement failure because both carry the
///       same source-level bounds reason.
/// WHY: the range owns its endpoint and no-progress rules, so the test must not pin
///      which stage detects the stall.
fn expect_non_progressing_range(
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    range: RangeLoopSpec,
    context: &str,
) -> TemplateError {
    let mut cursor = match ConstRangeCursor::new(
        &range,
        expressions,
        constants,
        crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    ) {
        Err(error) => return error,
        Ok(cursor) => cursor,
    };
    for _ in 0..crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS
    {
        match cursor.next_counter() {
            Err(error) => return error,
            Ok(Some(_)) => {}
            Ok(None) => panic!("{context} must not exhaust the range normally"),
        }
    }
    panic!("{context} must fail within the template iteration limit")
}

fn assert_non_progressing_bounds_diagnostic(error: TemplateError, context: &str) {
    let TemplateError::Diagnostic(diagnostic) = error else {
        panic!("{context} must stay on the source diagnostic lane");
    };
    match &diagnostic.payload {
        DiagnosticPayload::InvalidTemplateStructure { reason } => assert_eq!(
            *reason,
            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
            "{context} must report non-progressing bounds"
        ),
        payload => panic!("expected invalid template structure payload, found {payload:?}"),
    }
}

#[test]
fn range_cursor_keeps_float64_step_accumulation() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let start = float_expression(&mut expressions, &mut constants, 0.0);
    let end = float_expression(&mut expressions, &mut constants, 1.0);
    let step = float_expression(&mut expressions, &mut constants, 0.1);
    let counters = collect_float_counters(
        &expressions,
        &constants,
        start,
        end,
        Some(step),
        FixedScalar::F64,
    );

    assert_eq!(
        counters.len(),
        11,
        "F64 0.0 to 1.0 by 0.1 keeps eleven const iterations"
    );
    assert_eq!(
        counters[3],
        0.1 + 0.1 + 0.1,
        "F64 range counters must keep f64 accumulation unchanged"
    );
}

/// Collects `U64` counters from a const range cursor to completion.
///
/// WHAT: drives `ConstRangeCursor` directly so unsigned iteration is pinned
///       without template rendering in the way.
/// WHY: inclusive-maximum and descending-to-zero endpoints must terminate
///      through the in-range exit without ever materialising an out-of-range
///      `u64` successor.
fn collect_uint_counters(
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    start: ExprId,
    end: ExprId,
    step: Option<ExprId>,
    end_kind: RangeEndKind,
) -> Vec<u64> {
    let range = RangeLoopSpec {
        start,
        end,
        end_kind,
        step,
        domain: NumericScalar::Fixed(FixedScalar::U64),
    };
    let mut cursor = ConstRangeCursor::new(
        &range,
        expressions,
        constants,
        crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
    .expect("range cursor should construct");
    let mut counters = Vec::new();
    while let Some(counter) = cursor.next_counter().expect("range cursor should advance") {
        counters.push(
            counter
                .as_u64()
                .expect("unsigned ranges must emit unsigned counters"),
        );
    }
    counters
}

#[test]
fn range_cursor_terminates_at_inclusive_uint_maximum() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let start = u64_expression(&mut expressions, &mut constants, u64::MAX - 2);
    let end = u64_expression(&mut expressions, &mut constants, u64::MAX);
    let counters = collect_uint_counters(
        &expressions,
        &constants,
        start,
        end,
        None,
        RangeEndKind::Inclusive,
    );

    assert_eq!(
        counters,
        vec![u64::MAX - 2, u64::MAX - 1, u64::MAX],
        "an inclusive U64 range must emit the maximum and stop without a successor"
    );
}

#[test]
fn range_cursor_terminates_descending_at_uint_zero() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let start = u64_expression(&mut expressions, &mut constants, 2);
    let end = u64_expression(&mut expressions, &mut constants, 0);
    let counters = collect_uint_counters(
        &expressions,
        &constants,
        start,
        end,
        None,
        RangeEndKind::Inclusive,
    );

    assert_eq!(
        counters,
        vec![2, 1, 0],
        "a descending U64 range must reach zero and stop without underflowing"
    );
}

#[test]
fn range_cursor_rejects_zero_uint_step() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let range = RangeLoopSpec {
        start: u64_expression(&mut expressions, &mut constants, 0),
        end: u64_expression(&mut expressions, &mut constants, 10),
        end_kind: RangeEndKind::Inclusive,
        step: Some(u64_expression(&mut expressions, &mut constants, 0)),
        domain: NumericScalar::Fixed(FixedScalar::U64),
    };
    let error = match ConstRangeCursor::new(
        &range,
        &expressions,
        &constants,
        crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    ) {
        Err(error) => error,
        Ok(_) => panic!("a zero U64 step must not construct a const cursor"),
    };
    assert_non_progressing_bounds_diagnostic(error, "a zero U64 step");
}

#[test]
fn final_view_fold_float_range_loop_follows_operand_precision() {
    // F32 accumulation through the production `fold_tir_loop` path reaches 1.0 in
    // ten const iterations; an f64-ignoring cursor would emit eleven dots.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut narrow_fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let dot_text = string_table.intern(".");
            let dot_node = builder.push_text_node(dot_text, 1, TemplateSegmentOrigin::Body, None);
            build_float_range_loop_template(
                store,
                expressions,
                constants,
                FixedScalar::F32,
                0.0,
                f64::from(1.0_f32),
                f64::from(0.1_f32),
                dot_node,
            )
        },
    );
    let narrow_emission = fold_final_view_fixture(
        &mut narrow_fixture,
        &mut string_table,
        TemplateTirPhase::Composed,
    )
    .expect("F32 range loop should fold through the TIR fold");
    assert_eq!(
        emission_to_string(narrow_emission, &string_table),
        "..........",
        "F32 0.0 to 1.0 by 0.1 must fold to ten iterations through the TIR fold"
    );

    // The F64 loop keeps eleven f64 iterations; a cursor stuck at F32 precision
    // would emit ten dots here instead.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut standard_fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let dot_text = string_table.intern(".");
            let dot_node = builder.push_text_node(dot_text, 1, TemplateSegmentOrigin::Body, None);
            build_float_range_loop_template(
                store,
                expressions,
                constants,
                FixedScalar::F64,
                0.0,
                1.0,
                0.1,
                dot_node,
            )
        },
    );
    let standard_emission = fold_final_view_fixture(
        &mut standard_fixture,
        &mut string_table,
        TemplateTirPhase::Composed,
    )
    .expect("F64 range loop should fold through the TIR fold");
    assert_eq!(
        emission_to_string(standard_emission, &string_table),
        "...........",
        "F64 0.0 to 1.0 by 0.1 must keep eleven f64 iterations through the TIR fold"
    );
}

#[test]
fn final_view_fold_float_interpolation_uses_value_precision() {
    // An F32 value renders its shortest f32 text, while the same numeric carrier at
    // F64 keeps the long expansion. The value's own precision decides.
    let carrier = f64::from(0.1f32);

    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut narrow_fixture = build_final_view_fixture(
        &mut string_table,
        |_string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let float_node = builder.push_dynamic_expression_node(
                float_expression_at(expressions, constants, FixedScalar::F32, carrier),
                TemplateSegmentOrigin::Body,
                None,
            );
            builder.finish_template(
                float_node,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );
    let narrow_emission = fold_final_view_fixture(
        &mut narrow_fixture,
        &mut string_table,
        TemplateTirPhase::Composed,
    )
    .expect("F32 interpolation should fold through the TIR fold");
    assert_eq!(
        emission_to_string(narrow_emission, &string_table),
        "0.1",
        "an F32 0.1 must render its shortest f32 text through the TIR fold"
    );

    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut standard_fixture = build_final_view_fixture(
        &mut string_table,
        |_string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let float_node = builder.push_dynamic_expression_node(
                float_expression(expressions, constants, carrier),
                TemplateSegmentOrigin::Body,
                None,
            );
            builder.finish_template(
                float_node,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );
    let standard_emission = fold_final_view_fixture(
        &mut standard_fixture,
        &mut string_table,
        TemplateTirPhase::Composed,
    )
    .expect("F64 interpolation should fold through the TIR fold");
    assert_ne!(
        emission_to_string(standard_emission, &string_table),
        "0.1",
        "the same carrier at F64 must keep its full f64 expansion"
    );
}

// -------------------------
//  Aggregate wrapper root
// -------------------------

#[test]
fn final_view_fold_aggregate_wrapper_preserves_aggregate_output_position() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            let aggregate_node = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::AggregateOutput,
                None,
            ));

            let mut builder = TemplateIrBuilder::new(store);
            let open_text = string_table.intern("[");
            let close_text = string_table.intern("]");
            let x_text = string_table.intern("x");

            let open_node = builder.push_text_node(open_text, 1, TemplateSegmentOrigin::Body, None);
            let close_node =
                builder.push_text_node(close_text, 1, TemplateSegmentOrigin::Body, None);
            let wrapper_root =
                builder.push_sequence_node(vec![open_node, aggregate_node, close_node], None);

            let body_node = builder.push_text_node(x_text, 1, TemplateSegmentOrigin::Body, None);
            build_range_loop_template(
                string_table,
                store,
                expressions,
                constants,
                0,
                3,
                body_node,
                Some(wrapper_root),
            )
        },
    );

    let emission =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
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
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_final_view_fixture(
        &mut string_table,
        |string_table, store, expressions, constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let empty_body = builder.push_sequence_node(vec![], None);
            build_range_loop_template(
                string_table,
                store,
                expressions,
                constants,
                0,
                1,
                empty_body,
                Some(
                    crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId::new(999),
                ),
            )
        },
    );

    let error =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Composed)
            .expect_err(
                "a present aggregate wrapper must be validated even when the body is empty",
            );

    let TemplateError::Infrastructure(error) = error else {
        panic!("missing aggregate-wrapper authority should remain on the infrastructure lane");
    };
    let error = &error.primary;
    assert!(
        error.msg.contains("TIR preparation: node"),
        "expected a stable aggregate-wrapper authority error, got: {}",
        error.msg
    );
}

#[test]
fn final_view_aggregate_output_outside_wrapper_classifies_as_runtime() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let fixture = build_final_view_fixture(
        &mut string_table,
        |_string_table, store, _expressions, _constants| {
            let aggregate_node = store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::AggregateOutput,
                None,
            ));

            let mut builder = TemplateIrBuilder::new(store);
            builder.finish_template(
                aggregate_node,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );

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
    let prepared = prepare_tir_view(
        &view,
        &fixture.expressions,
        &fixture.const_values,
        TemplatePreparationMode::Value,
    )
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
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
            None,
        );
        builder.finish_template(
            root,
            style.clone(),
            TemplateType::String,
            TemplateIrSummary::empty(),
            None,
        )
    };

    let formatted_root = {
        let mut store_borrow = store.borrow_mut();
        crate::compiler_frontend::ast::templates::tir::formatter_view::format_tir_template(
            &mut store_borrow,
            &mut expressions,
            &mut const_values,
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
        expressions,
        const_values,
    }
}

#[test]
fn final_view_fold_formatted_markdown_text() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut fixture = build_formatted_markdown_fixture(&mut string_table);
    let emission =
        fold_final_view_fixture(&mut fixture, &mut string_table, TemplateTirPhase::Formatted)
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
    let _path_fork = PathInternerFork::empty();
    let fixture = build_final_view_fixture(
        &mut string_table,
        |_string_table, store, expressions, _constants| {
            let mut builder = TemplateIrBuilder::new(store);
            let handoff = OwnedRuntimeSlotApplicationHandoff {
                wrapper: OwnedRuntimeTemplateNode::Text {
                    text: OwnedFoldedString::Text("<shell>".to_owned()),
                },
                contribution_sources: Vec::new(),
                slot_sites: Vec::new(),
            };
            let expression = runtime_slot_application_handoff_expr(
                expressions,
                handoff,
                None,
                ValueMode::ImmutableOwned,
            );
            let dynamic_node =
                builder.push_dynamic_expression_node(expression, TemplateSegmentOrigin::Body, None);

            builder.finish_template(
                dynamic_node,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                None,
            )
        },
    );

    let store = fixture.store.borrow();
    let view = TirView::new(
        &store,
        fixture.template_id,
        TemplateTirPhase::Finalized,
        fixture.context,
    )
    .expect("final view should construct");
    let first = prepare_tir_view(
        &view,
        &fixture.expressions,
        &fixture.const_values,
        TemplatePreparationMode::Value,
    )
    .expect("runtime slot application should prepare as runtime");
    assert!(matches!(
        first.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));

    let second = prepare_tir_view(
        &view,
        &fixture.expressions,
        &fixture.const_values,
        TemplatePreparationMode::Value,
    )
    .expect("runtime slot application should remain a runtime result");
    assert!(matches!(
        second.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));
}
