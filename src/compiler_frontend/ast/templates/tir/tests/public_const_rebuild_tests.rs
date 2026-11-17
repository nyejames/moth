//! Reference-time const-template rebuild tests.
//!
//! WHAT: checks that imported public const-template projections rebuild into the consumer's TIR
//!       with slot defaults, structural pieces and synthetic provenance intact.
//! WHY: head composition and slot routing only attach their semantics to rebuilt template
//!      expressions, so the projection-to-TIR conversion is its own invariant boundary.

use std::cell::RefCell;
use std::path::Path;

use super::super::TemplateIrBuilder;
use super::super::ids::TemplateIrId;
use super::super::node::{TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder};
use super::super::overlays::TemplateViewContext;
use super::super::public_const_rebuild::rebuild_const_template_reference;
use super::super::refs::{TemplateTirChildReference, TemplateWrapperReference};
use super::super::slot_composition::TirSlotContributions;
use super::super::slot_composition::schema::{
    attach_conditional_wrapper_set, expand_tir_slot_placeholders_into,
};
use super::super::store::TemplateIrStore;
use super::super::summary::TemplateIrSummary;
use super::super::view::TemplateTirPhase;
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeEndKind, RangeLoopSpec};
use crate::compiler_frontend::ast::const_eval::FoldServices;
use crate::compiler_frontend::ast::const_values::binding::{
    ProjectedConstTemplateValue, project_const_template_value,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateStructureCode, InvalidTemplateStructureView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplateKind,
    PublicConstTemplatePiece, PublicConstTemplateSlot, PublicTemplateSlotKey,
    resolve_const_string_value,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn public_slot_default(default: Option<OwnedFoldedString>) -> PublicConstTemplate {
    public_slot_default_with_provenance(default, SyntheticInterfaceProvenance::empty())
}

fn public_slot_default_with_provenance(
    default: Option<OwnedFoldedString>,
    default_provenance: SyntheticInterfaceProvenance,
) -> PublicConstTemplate {
    PublicConstTemplate {
        kind: PublicConstTemplateKind::Wrapper,
        pieces: vec![PublicConstTemplatePiece::Slot(PublicConstTemplateSlot {
            key: PublicTemplateSlotKey::Default,
            default,
            default_provenance,
            applied_child_wrappers: Vec::new(),
            child_wrappers: Vec::new(),
            skip_parent_child_wrappers: false,
        })],
        conditional_child_wrappers: Vec::new(),
    }
}

fn rebuilt_placeholder(template: &Template, store: &TemplateIrStore) -> TirSlotPlaceholder {
    let template_ir = store
        .get_template(template.tir_reference.root)
        .expect("rebuilt template exists");
    let root = store
        .get_node(template_ir.root)
        .expect("rebuilt root exists");
    let TemplateIrNodeKind::Sequence { children } = &root.kind else {
        panic!("rebuilt const template root must be a sequence");
    };
    let [slot_id] = children.as_slice() else {
        panic!("rebuilt const template must contain exactly the authored slot");
    };
    match &store.get_node(*slot_id).expect("rebuilt slot exists").kind {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.clone(),
        _ => panic!("rebuilt const template child must be a slot"),
    }
}

#[test]
fn rebuilt_owned_default_becomes_local_string_constant() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Template {
                template: public_slot_default(Some(OwnedFoldedString::Text("fallback".to_owned()))),
                folded: None,
            },
            None,
        )
        .expect("public template payload inserts");

    let template = rebuild_const_template_reference(
        value,
        &mut store,
        &mut expression_builder,
        &mut const_values,
        None,
        &mut string_table,
        None,
    )
    .expect("template payload rebuilds")
    .expect("template payload produces a template expression");
    let placeholder = rebuilt_placeholder(&template, &store);
    let default = placeholder.default.expect("rebuilt slot keeps its default");
    let (value, _) = resolve_const_string_value(&const_values, default)
        .expect("rebuilt default resolves through the local constant store");
    assert_eq!(
        value,
        &ConstStringValue::Text(string_table.intern("fallback"))
    );
}

#[test]
fn rebuilt_structural_default_preserves_piece_identity() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let resources = RefCell::new(ModuleResourceTable::new());
    let origin = StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("rebuild-test"),
            "pages".to_owned(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("test resource path is portable"),
    );
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Template {
                template: public_slot_default(Some(OwnedFoldedString::Pieces(vec![
                    OwnedFoldedStringPiece::Text("open".to_owned()),
                    OwnedFoldedStringPiece::Resource(origin.clone()),
                    OwnedFoldedStringPiece::SiteRoot,
                ]))),
                folded: None,
            },
            None,
        )
        .expect("public template payload inserts");

    let template = rebuild_const_template_reference(
        value,
        &mut store,
        &mut expression_builder,
        &mut const_values,
        Some(&resources),
        &mut string_table,
        None,
    )
    .expect("structural template payload rebuilds")
    .expect("template payload produces a template expression");
    let placeholder = rebuilt_placeholder(&template, &store);
    let default = placeholder.default.expect("rebuilt slot keeps its default");
    let (value, _) = resolve_const_string_value(&const_values, default)
        .expect("rebuilt structural default resolves through the local store");
    let ConstStringValue::Pieces(pieces) = value else {
        panic!("structural default must stay piece-bearing");
    };
    assert!(matches!(
        pieces.as_slice(),
        [
            ConstStringPiece::Text(_),
            ConstStringPiece::Resource(_),
            ConstStringPiece::SiteRoot
        ]
    ));
}

#[test]
fn rebuilt_default_retains_synthetic_interface_provenance() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "source-config",
        "site_name",
    );
    let provenance = SyntheticInterfaceProvenance::single(member.clone());
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Template {
                template: public_slot_default_with_provenance(
                    Some(OwnedFoldedString::Text("fallback".to_owned())),
                    provenance,
                ),
                folded: None,
            },
            None,
        )
        .expect("public template payload inserts");

    let template = rebuild_const_template_reference(
        value,
        &mut store,
        &mut expression_builder,
        &mut const_values,
        None,
        &mut string_table,
        None,
    )
    .expect("template payload rebuilds")
    .expect("template payload produces a template expression");
    let placeholder = rebuilt_placeholder(&template, &store);
    let default = placeholder.default.expect("rebuilt slot keeps its default");
    let (_, resolved) = resolve_const_string_value(&const_values, default)
        .expect("rebuilt default resolves through the local constant store");
    assert_eq!(
        resolved.members(),
        &[member],
        "the rebuilt local default must retain the donor constant's stable provenance"
    );
}

#[test]
fn rebuilt_explicit_empty_default_stays_present() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Template {
                template: public_slot_default(Some(OwnedFoldedString::Text(String::new()))),
                folded: None,
            },
            None,
        )
        .expect("public template payload inserts");

    let template = rebuild_const_template_reference(
        value,
        &mut store,
        &mut expression_builder,
        &mut const_values,
        None,
        &mut string_table,
        None,
    )
    .expect("template payload rebuilds")
    .expect("template payload produces a template expression");
    let placeholder = rebuilt_placeholder(&template, &store);
    let default = placeholder
        .default
        .expect("an authored empty default stays present rather than becoming absent");
    let (value, provenance) = resolve_const_string_value(&const_values, default)
        .expect("rebuilt empty default resolves through the local constant store");
    assert_eq!(value, &ConstStringValue::Text(string_table.intern("")));
    assert!(provenance.is_empty());
}

#[test]
fn rebuilt_supplied_guard_resolves_on_application() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let disabled = string_table.intern("disabled");
    let template = PublicConstTemplate {
        kind: PublicConstTemplateKind::Wrapper,
        pieces: vec![
            PublicConstTemplatePiece::Text(OwnedFoldedString::Text("before".to_owned())),
            PublicConstTemplatePiece::SuppliedGuard {
                key: PublicTemplateSlotKey::Named("disabled".to_owned()),
                body: vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Text(
                    " disabled".to_owned(),
                ))],
            },
            PublicConstTemplatePiece::Text(OwnedFoldedString::Text("after".to_owned())),
        ],
        conditional_child_wrappers: Vec::new(),
    };
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Template {
                template,
                folded: None,
            },
            None,
        )
        .expect("public template payload inserts");

    let rebuilt = rebuild_const_template_reference(
        value,
        &mut store,
        &mut expression_builder,
        &mut const_values,
        None,
        &mut string_table,
        None,
    )
    .expect("template payload rebuilds")
    .expect("template payload produces a template expression");
    let root = store
        .get_template(rebuilt.tir_reference.root)
        .expect("rebuilt template exists")
        .root;
    let children = match &store.get_node(root).expect("rebuilt root exists").kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("rebuilt const template root must be a sequence, found {other:?}"),
    };
    let [_, guard_id, _] = children.as_slice() else {
        panic!("rebuilt const template must keep the guard between its text runs");
    };
    let (key, body_id) = match &store.get_node(*guard_id).expect("guard exists").kind {
        TemplateIrNodeKind::SuppliedGuard { key, body } => (key.clone(), *body),
        other => panic!("rebuilt guard node expected, found {other:?}"),
    };
    assert_eq!(key, SlotKey::Named(disabled));
    let body_reference = match &store
        .get_node(body_id)
        .expect("guard body edge exists")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => *reference,
        other => panic!("guard body must be a child-template edge, found {other:?}"),
    };
    let body_root = store
        .get_template(body_reference.root)
        .expect("guard body template exists")
        .root;
    let body_children = match &store
        .get_node(body_root)
        .expect("guard body root exists")
        .kind
    {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("guard body root must be a sequence, found {other:?}"),
    };
    let [body_text] = body_children.as_slice() else {
        panic!("guard body must keep its authored text as its only piece");
    };
    let TemplateIrNodeKind::Text { text, .. } = &store
        .get_node(*body_text)
        .expect("guard body text exists")
        .kind
    else {
        panic!("guard body must keep its authored text");
    };
    assert_eq!(string_table.resolve(*text), " disabled");

    let absent = expand_tir_slot_placeholders_into(
        &mut store,
        &expression_builder,
        &const_values,
        rebuilt.tir_reference.root,
        &TirSlotContributions::default(),
        &string_table,
    )
    .expect("an unrouted application should expand the rebuilt wrapper");
    let absent_children = match &store.get_node(absent).expect("expanded root exists").kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expanded rebuilt wrapper root must be a sequence, found {other:?}"),
    };
    assert_eq!(
        absent_children.len(),
        2,
        "an unrouted guard must contribute no node, leaving only the surrounding text"
    );

    let mut routed = TirSlotContributions::default();
    routed.extend_named_nodes(disabled, Vec::new());
    let present = expand_tir_slot_placeholders_into(
        &mut store,
        &expression_builder,
        &const_values,
        rebuilt.tir_reference.root,
        &routed,
        &string_table,
    )
    .expect("a routed application should expand the rebuilt wrapper");
    let present_children = match &store.get_node(present).expect("expanded root exists").kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        other => panic!("expanded rebuilt wrapper root must be a sequence, found {other:?}"),
    };
    assert_eq!(
        present_children.len(),
        3,
        "a routed guard must select its authored body at the authored position"
    );
    let selected_reference = match &store
        .get_node(present_children[1])
        .expect("selected guard body exists")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => *reference,
        other => panic!("selected guard body must be a child-template edge, found {other:?}"),
    };
    assert_eq!(
        selected_reference.root, body_reference.root,
        "the routed application must replay the rebuilt guard body"
    );
}

/// Builds one wrapper whose ordinary nested child contains only an open guard.
///
/// WHAT: with `nested`, the outer guard's body template contains a second open guard instead of
///       text, exercising the child boundary inside a guard body as well as beside it.
/// WHY: public projection must retain composition structure that renders no output, and the
///      nested guard is the smallest shape where two such boundaries meet.
fn nested_guard_projection_fixture(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    nested: bool,
) -> (TemplateIrId, StringId, StringId) {
    let flag = string_table.intern("flag");
    let inner = string_table.intern("inner");
    let body_text = string_table.intern("X");
    let before = string_table.intern("[");
    let after = string_table.intern("]");

    let mut builder = TemplateIrBuilder::new(store);
    let body_text_node =
        builder.push_text_node(body_text, "X".len(), TemplateSegmentOrigin::Body, None);
    let inner_body = builder.finish_template(
        body_text_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let inner_body_edge = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            inner_body,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let inner_guard =
        builder.push_supplied_guard_node(SlotKey::Named(inner), inner_body_edge, None);

    let outer_body_root = if nested {
        let body_guard_root = builder.push_sequence_node(vec![inner_guard], None);
        builder.finish_template(
            body_guard_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    } else {
        let body_text_node =
            builder.push_text_node(body_text, "X".len(), TemplateSegmentOrigin::Body, None);
        builder.finish_template(
            body_text_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    };
    let outer_body_edge = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            outer_body_root,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let outer_guard = builder.push_supplied_guard_node(SlotKey::Named(flag), outer_body_edge, None);

    // The guard is nested one ordinary child template below the exported wrapper.
    let child_root = builder.push_sequence_node(vec![outer_guard], None);
    let child = builder.finish_template(
        child_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let child_edge = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            child,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let before_node = builder.push_text_node(before, "[".len(), TemplateSegmentOrigin::Body, None);
    let after_node = builder.push_text_node(after, "]".len(), TemplateSegmentOrigin::Body, None);
    let root = builder.push_sequence_node(vec![before_node, child_edge, after_node], None);
    let wrapper = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    (wrapper, flag, inner)
}

#[test]
fn projected_no_output_child_guard_survives_projection_and_rebuild() {
    for nested in [false, true] {
        let mut store = TemplateIrStore::new();
        let mut expression_builder = AstExpressionBuilder::new().unwrap();
        let mut const_values = ConstValueStore::default();
        let mut string_table = StringTable::new();
        let (wrapper, flag, inner) =
            nested_guard_projection_fixture(&mut store, &mut string_table, nested);
        let projected = project_fixture(
            wrapper,
            &store,
            &mut expression_builder,
            &mut const_values,
            &mut string_table,
        )
        .expect("the wrapper should project publicly");
        let public = projected.public.expect(
            "a wrapper template with an unresolved guard should publish a public projection",
        );

        let expected_inner_body = if nested {
            vec![PublicConstTemplatePiece::SuppliedGuard {
                key: PublicTemplateSlotKey::Named("inner".to_owned()),
                body: vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Text(
                    "X".to_owned(),
                ))],
            }]
        } else {
            vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Text(
                "X".to_owned(),
            ))]
        };
        assert_eq!(
            public.pieces,
            vec![
                PublicConstTemplatePiece::Text(OwnedFoldedString::Text("[".to_owned())),
                PublicConstTemplatePiece::SuppliedGuard {
                    key: PublicTemplateSlotKey::Named("flag".to_owned()),
                    body: expected_inner_body,
                },
                PublicConstTemplatePiece::Text(OwnedFoldedString::Text("]".to_owned())),
            ],
            "a guard behind a no-output child boundary must keep its key and body (nested={nested})"
        );

        let value = const_values
            .insert_resolved(
                builtin_type_ids::STRING,
                ConstValuePayload::Template {
                    template: public,
                    folded: projected.folded,
                },
                None,
            )
            .expect("public template payload inserts");
        let rebuilt = rebuild_const_template_reference(
            value,
            &mut store,
            &mut expression_builder,
            &mut const_values,
            None,
            &mut string_table,
            None,
        )
        .expect("template payload rebuilds")
        .expect("template payload produces a template expression");
        let root = store
            .get_template(rebuilt.tir_reference.root)
            .expect("rebuilt template exists")
            .root;
        let children = match &store.get_node(root).expect("rebuilt root exists").kind {
            TemplateIrNodeKind::Sequence { children } => children.clone(),
            other => panic!("rebuilt wrapper root must be a sequence, found {other:?}"),
        };
        let [_, guard_id, _] = children.as_slice() else {
            panic!("the rebuilt wrapper must keep the guard between its text runs");
        };
        let (key, body_id) = match &store.get_node(*guard_id).expect("guard exists").kind {
            TemplateIrNodeKind::SuppliedGuard { key, body } => (key.clone(), *body),
            other => panic!("rebuilt guard node expected, found {other:?}"),
        };
        assert_eq!(key, SlotKey::Named(flag));
        let body_reference = match &store
            .get_node(body_id)
            .expect("guard body edge exists")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { reference, .. } => *reference,
            other => panic!("guard body must be a child-template edge, found {other:?}"),
        };
        let body_root = store
            .get_template(body_reference.root)
            .expect("guard body template exists")
            .root;
        let body_children = match &store
            .get_node(body_root)
            .expect("guard body root exists")
            .kind
        {
            TemplateIrNodeKind::Sequence { children } => children.clone(),
            other => panic!("guard body root must be a sequence, found {other:?}"),
        };
        if nested {
            let [inner_guard_id] = body_children.as_slice() else {
                panic!("the nested guard body must contain exactly the inner guard");
            };
            let (inner_key, inner_body_id) = match &store
                .get_node(*inner_guard_id)
                .expect("inner guard exists")
                .kind
            {
                TemplateIrNodeKind::SuppliedGuard { key, body } => (key.clone(), *body),
                other => panic!("rebuilt inner guard node expected, found {other:?}"),
            };
            assert_eq!(inner_key, SlotKey::Named(inner));
            assert!(matches!(
                &store
                    .get_node(inner_body_id)
                    .expect("inner guard body exists")
                    .kind,
                TemplateIrNodeKind::ChildTemplate { .. }
            ));
        } else {
            let [text_id] = body_children.as_slice() else {
                panic!("the guard body must keep its authored text as its only piece");
            };
            let TemplateIrNodeKind::Text { text, .. } = &store
                .get_node(*text_id)
                .expect("guard body text exists")
                .kind
            else {
                panic!("guard body must keep its authored text");
            };
            assert_eq!(string_table.resolve(*text), "X");
        }
    }
}

/// Builds a wrapper whose only content is an unresolved guard around one provenance-bearing
/// dynamic expression.
///
/// WHAT: the guard body template is exactly a dynamic constant expression tagged with a
///       project-context member; the guard renders nothing at compile time.
/// WHY: the expression is the projection's only provenance source, and it can reach the
///      export-boundary checks only through the retained guard body.
fn guarded_expression_projection_fixture(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    member: SyntheticInterfaceMemberIdentity,
) -> TemplateIrId {
    let flag = string_table.intern("flag");
    let guarded = string_table.intern("guarded");
    let mut builder = TemplateIrBuilder::new(store);
    let expression = expression_builder
        .string_slice(const_values, guarded, None, ValueMode::ImmutableOwned)
        .expect("the guarded expression constructs")
        .id();
    let expression = expression_builder
        .with_synthetic_interface_provenance(
            expression_builder.own(expression),
            SyntheticInterfaceProvenance::single(member),
        )
        .expect("the guarded expression accepts synthetic provenance")
        .id();
    let expression_node =
        builder.push_dynamic_expression_node(expression, TemplateSegmentOrigin::Body, None);
    let guard_body = builder.finish_template(
        expression_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let guard_body_edge = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            guard_body,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let guard = builder.push_supplied_guard_node(SlotKey::Named(flag), guard_body_edge, None);
    let root = builder.push_sequence_node(vec![guard], None);
    builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    )
}

#[test]
fn projected_guard_body_expression_provenance_reaches_projection() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "source-config",
        "guarded_site_name",
    );
    let wrapper = guarded_expression_projection_fixture(
        &mut store,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
        member.clone(),
    );
    let projected = project_fixture(
        wrapper,
        &store,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
    )
    .expect("the guard-only wrapper should project publicly");
    assert_eq!(
        projected.provenance.members(),
        &[member],
        "an expression used only inside an unresolved guard body must still contribute its \
         synthetic-interface provenance to the projection"
    );
    let public = projected
        .public
        .expect("the wrapper should publish a public projection");
    assert_eq!(
        public.pieces,
        vec![PublicConstTemplatePiece::SuppliedGuard {
            key: PublicTemplateSlotKey::Named("flag".to_owned()),
            body: vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Text(
                "guarded".to_owned()
            ))],
        }],
        "the guard keeps its key and its folded expression body"
    );
}

/// Creates a synthetic, source-qualified span for a hand-built TIR guard.
fn synthetic_guard_span(start: u32, length: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(start, length, &mut span_builder).expect("synthetic test span should fit"),
    )
}

/// Builds a wrapper whose conditional child owns an `IfChildEmits` wrapper set and holds only
/// an unresolved guard.
///
/// WHAT: the exported wrapper is `[ [if true: <guarded child>] ]`, and the child template carries
///       a conditional child-wrapper set that renders `<i>` around anything it emits.
/// WHY: the guard renders nothing at compile time, so the conditional child wrappers are skipped;
///      the public projection must refuse to promote the guard out of that skipped wrapper.
fn conditioned_child_guard_projection_fixture(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> (TemplateIrId, SourceSpan) {
    let flag = string_table.intern("flag");
    let open_bracket = string_table.intern("[");
    let close_bracket = string_table.intern("]");
    let open_tag = string_table.intern("<i>");
    let close_tag = string_table.intern("</i>");
    let x = string_table.intern("X");
    let guard_span = synthetic_guard_span(10, 6);

    let mut builder = TemplateIrBuilder::new(store);

    // The wrapper set that would apply around the child whenever it emits output.
    let wrapper_slot = builder.push_slot_node(SlotKey::Default, None);
    let wrapper_open = builder.push_text_node(open_tag, 3, TemplateSegmentOrigin::Body, None);
    let wrapper_close = builder.push_text_node(close_tag, 4, TemplateSegmentOrigin::Body, None);
    let wrapper_root =
        builder.push_sequence_node(vec![wrapper_open, wrapper_slot, wrapper_close], None);
    let wrapper_template = builder.finish_template(
        wrapper_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let wrapper_set = builder
        .store
        .push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
            wrapper_template,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        )]);

    // The guarded child renders nothing unless `flag` is supplied; its body is `X`.
    let x_node = builder.push_text_node(x, 1, TemplateSegmentOrigin::Body, None);
    let guard_body = builder.finish_template(
        x_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let guard_body_edge = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            guard_body,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let guard =
        builder.push_supplied_guard_node(SlotKey::Named(flag), guard_body_edge, Some(guard_span));
    let guard_template = builder.finish_template(
        guard,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let guard_child = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            guard_template,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let conditioned_child = attach_conditional_wrapper_set(builder.store, guard_child, wrapper_set)
        .expect("the guard child accepts a conditional wrapper set");

    let condition = expression_builder
        .bool(const_values, true, None, ValueMode::ImmutableOwned)
        .expect("the test condition constructs")
        .id();
    let conditional = builder.push_conditional_node(
        TemplateBranchSelector::Bool(condition),
        conditioned_child,
        None,
    );

    let open_node = builder.push_text_node(open_bracket, 1, TemplateSegmentOrigin::Body, None);
    let close_node = builder.push_text_node(close_bracket, 1, TemplateSegmentOrigin::Body, None);
    let root = builder.push_sequence_node(vec![open_node, conditional, close_node], None);
    let wrapper = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    (wrapper, guard_span)
}

/// Builds a wrapper whose loop body is only an unresolved guard and whose aggregate wrapper
/// carries the head prefix.
///
/// WHAT: models `["prefix", loop 0 to 1 |i|: [$supplied flag: X]]`: the loop emits nothing at
///       compile time, so its aggregate wrapper (`prefix` + aggregate output) is skipped.
/// WHY: the guard still reaches the projection, and the public pieces cannot carry the skipped
///      aggregate wrapper.
fn loop_guard_projection_fixture(
    store: &mut TemplateIrStore,
    string_table: &mut StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> (TemplateIrId, SourceSpan) {
    let flag = string_table.intern("flag");
    let prefix = string_table.intern("prefix");
    let x = string_table.intern("X");
    let guard_span = synthetic_guard_span(20, 6);

    let mut builder = TemplateIrBuilder::new(store);

    let x_node = builder.push_text_node(x, 1, TemplateSegmentOrigin::Body, None);
    let guard_body = builder.finish_template(
        x_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let guard_body_edge = builder.push_child_template_node_with_reference(
        TemplateTirChildReference::new(
            guard_body,
            TemplateTirPhase::Composed,
            TemplateViewContext::default(),
        ),
        None,
    );
    let guard =
        builder.push_supplied_guard_node(SlotKey::Named(flag), guard_body_edge, Some(guard_span));

    let aggregate_output = builder.store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::AggregateOutput,
        None,
    ));
    let prefix_node = builder.push_text_node(prefix, 6, TemplateSegmentOrigin::Body, None);
    let aggregate_wrapper = builder.push_sequence_node(vec![prefix_node, aggregate_output], None);
    let header = TemplateLoopHeader::Range {
        bindings: Box::new(LoopBindings {
            item: None,
            index: None,
        }),
        range: Box::new(RangeLoopSpec {
            start: i32_expression(expression_builder, const_values, 0),
            end: i32_expression(expression_builder, const_values, 1),
            step: None,
            end_kind: RangeEndKind::Exclusive,
            domain: NumericScalar::Fixed(FixedScalar::I32),
            fast_math: false,
        }),
    };
    let loop_node = builder.push_loop_node(header, guard, Some(aggregate_wrapper), None);
    let wrapper = builder.finish_template(
        loop_node,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    (wrapper, guard_span)
}

fn i32_expression(
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: i64,
) -> crate::compiler_frontend::ast::expressions::expression_store::ExprId {
    let value =
        FixedScalarValue::signed(FixedScalar::I32, value).expect("the test loop bound fits I32");
    expression_builder
        .fixed_scalar(const_values, value, None, ValueMode::ImmutableOwned)
        .expect("the test loop bound constructs")
        .id()
}

fn project_fixture(
    wrapper: TemplateIrId,
    store: &TemplateIrStore,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<ProjectedConstTemplateValue, ConstValueStoreError> {
    let template = Template {
        tir_reference: super::super::refs::TemplateTirReference {
            root: wrapper,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        span: None,
        body_only_directive_span: None,
    };
    project_const_template_value(
        &template,
        store,
        expression_builder,
        const_values,
        FoldServices::new(string_table, None, None),
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
}

/// Asserts that projection refused a guard that would cross a skipped output-conditioned wrapper.
///
/// WHAT: requires the template-structure diagnostic, its reason code and the guard's authored span.
/// WHY: the refusal must stay a typed source diagnostic; a generic infrastructure error or a
///      silent projection would both rebuild a template with different output.
fn assert_guard_projection_refusal(
    result: Result<ProjectedConstTemplateValue, ConstValueStoreError>,
    expected_span: SourceSpan,
    shape: &str,
) {
    let Err(error) = result else {
        panic!("{shape}: a guard promoted across a skipped wrapper must not project");
    };
    let ConstValueStoreError::Diagnostic(diagnostic) = error else {
        panic!("{shape}: the refusal must use the source-diagnostic lane, got {error:?}");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let view = compact
        .first()
        .expect("the projection refusal should compact into one diagnostic");
    let facts = view
        .family_view::<InvalidTemplateStructureView>()
        .expect("the refusal should use the template-structure family");
    assert_eq!(
        facts.reason,
        InvalidTemplateStructureCode::SuppliedGuardAcrossSkippedWrapper,
        "{shape}: the refusal should name the skipped-wrapper guard rule"
    );
    assert!(
        matches!(view.primary_site(), SiteRef::Local(span) if span == expected_span),
        "{shape}: the refusal should point at the guard's authored span"
    );
}

#[test]
fn suppressed_child_wrapper_guard_projection_is_rejected() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let (wrapper, guard_span) = conditioned_child_guard_projection_fixture(
        &mut store,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    let result = project_fixture(
        wrapper,
        &store,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
    );
    assert_guard_projection_refusal(result, guard_span, "conditional child wrapper");
}

#[test]
fn skipped_loop_aggregate_guard_projection_is_rejected() {
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let (wrapper, guard_span) = loop_guard_projection_fixture(
        &mut store,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    let result = project_fixture(
        wrapper,
        &store,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
    );
    assert_guard_projection_refusal(result, guard_span, "loop aggregate wrapper");
}

#[test]
fn root_conditional_wrapper_guard_projection_keeps_published_wrappers() {
    // The projection root republishes its own conditional wrapper set, so a guard it would have
    // wrapped may still project: rebuild restores the wrapper set and the application applies it
    // around whatever the guard emits. Only wrappers that exist inside the fold are refused.
    let mut store = TemplateIrStore::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let flag = string_table.intern("flag");
    let open_tag = string_table.intern("<i>");
    let close_tag = string_table.intern("</i>");
    let x = string_table.intern("X");

    let wrapper = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let slot = builder.push_slot_node(SlotKey::Default, None);
        let open = builder.push_text_node(open_tag, 3, TemplateSegmentOrigin::Body, None);
        let close = builder.push_text_node(close_tag, 4, TemplateSegmentOrigin::Body, None);
        let wrapper_root = builder.push_sequence_node(vec![open, slot, close], None);
        let wrapper_template = builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );
        let wrapper_set =
            builder
                .store
                .push_or_reuse_wrapper_set(&[TemplateWrapperReference::new(
                    wrapper_template,
                    TemplateTirPhase::Composed,
                    TemplateViewContext::default(),
                )]);

        let x_node = builder.push_text_node(x, 1, TemplateSegmentOrigin::Body, None);
        let guard_body = builder.finish_template(
            x_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );
        let guard_body_edge = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                guard_body,
                TemplateTirPhase::Composed,
                TemplateViewContext::default(),
            ),
            None,
        );
        let guard = builder.push_supplied_guard_node(SlotKey::Named(flag), guard_body_edge, None);
        // The guard is the root's only content, so the root emits no output and its
        // conditional wrapper set is skipped by the fold.
        let root = builder.push_sequence_node(vec![guard], None);
        let wrapper = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );
        builder
            .store
            .set_conditional_child_wrapper_set(wrapper, wrapper_set)
            .expect("the projection root accepts a conditional wrapper set");
        wrapper
    };

    let projected = project_fixture(
        wrapper,
        &store,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
    )
    .expect("a root-level conditional wrapper does not lose structure");
    let public = projected
        .public
        .expect("the wrapper should publish a public projection");
    assert_eq!(
        public.pieces,
        vec![PublicConstTemplatePiece::SuppliedGuard {
            key: PublicTemplateSlotKey::Named("flag".to_owned()),
            body: vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Text(
                "X".to_owned()
            ))],
        }],
        "the guard keeps its authored position at the projection root"
    );
    assert_eq!(
        public.conditional_child_wrappers.len(),
        1,
        "the root's skipped conditional wrapper set must stay in the public projection"
    );

    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::Template {
                template: public,
                folded: projected.folded,
            },
            None,
        )
        .expect("public template payload inserts");
    let rebuilt = rebuild_const_template_reference(
        value,
        &mut store,
        &mut expression_builder,
        &mut const_values,
        None,
        &mut string_table,
        None,
    )
    .expect("template payload rebuilds")
    .expect("template payload produces a template expression");
    let rebuilt_template = store
        .get_template(rebuilt.tir_reference.root)
        .expect("rebuilt template exists");
    let rebuilt_wrapper_set = rebuilt_template
        .conditional_child_wrapper_set
        .expect("rebuild restores the published conditional wrapper set");
    assert_eq!(
        store
            .get_wrapper_set(rebuilt_wrapper_set)
            .expect("the rebuilt wrapper set exists")
            .wrappers
            .len(),
        1,
        "the rebuilt template keeps exactly the authored conditional wrapper"
    );
}
