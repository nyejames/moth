//! Reference-time const-template rebuild tests.
//!
//! WHAT: checks that imported public const-template projections rebuild into the consumer's TIR
//!       with slot defaults, structural pieces and synthetic provenance intact.
//! WHY: head composition and slot routing only attach their semantics to rebuilt template
//!      expressions, so the projection-to-TIR conversion is its own invariant boundary.

use std::cell::RefCell;
use std::path::Path;

use super::super::node::{TemplateIrNodeKind, TirSlotPlaceholder};
use super::super::public_const_rebuild::rebuild_const_template_reference;
use super::super::store::TemplateIrStore;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplateKind,
    PublicConstTemplatePiece, PublicConstTemplateSlot, PublicTemplateSlotKey,
    resolve_const_string_value,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};

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
