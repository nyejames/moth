//! Unit tests for the store's structural string and record payloads.
//!
//! These protect folded-string invariants that external output cannot inspect: plain text keeps
//! the compact fast path, piece-bearing strings survive `fold_value` in authored piece order,
//! and the text-only accessor refuses to flatten structure. Record tests protect authored field
//! order, field locations, and the duplicate-name construction invariant.

use super::{
    ConstStringPiece, ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
    ConstValueVisit,
};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::path::Path;

// ------------------------------------
//  Fixtures
// ------------------------------------

fn text_declaration(path: &str, text: StringId, string_table: &mut StringTable) -> Declaration {
    Declaration {
        id: InternedPath::from_single_str(path, string_table),
        value: Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned),
    }
}

/// Mint one real resource handle through the issuing table, as production callers do.
fn resource_id(table: &mut ModuleResourceTable, relative: &str) -> ResourceId {
    let module = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let logical_path = PortableResourcePath::from_relative_logical_path(Path::new(relative))
        .expect("relative resource path should be portable");

    table.intern_origin(
        StableResourceOriginId::module_owned(module, logical_path),
        SourceLocation::default(),
    )
}

fn structural_string_declaration(
    path: &str,
    pieces: Vec<ConstStringPiece>,
    string_table: &mut StringTable,
) -> Declaration {
    Declaration {
        id: InternedPath::from_single_str(path, string_table),
        value: Expression::structural_string(pieces, SourceLocation::default()),
    }
}

fn template_declaration(path: &str, string_table: &mut StringTable) -> Declaration {
    let template = Template {
        tir_reference: TemplateTirReference {
            root: TemplateIrId::new(0),
            phase: TemplateTirPhase::Finalized,
            context: TemplateViewContext::default(),
        },
        location: SourceLocation::default(),
    };
    Declaration {
        id: InternedPath::from_single_str(path, string_table),
        value: Expression::template(template, ValueMode::ImmutableOwned),
    }
}

fn visit_string_value(
    store: &ConstValueStore,
    id: ConstValueId,
) -> Result<ConstStringValue, CompilerError> {
    let mut visited = None;
    store.fold_value(id, &mut |_, visit| {
        match visit {
            ConstValueVisit::String(value) => visited = Some(value.clone()),

            // A folded string row must never visit as any other value shape.
            _ => {
                return Err(CompilerError::compiler_error(
                    "a folded string row visited as a non-string value",
                ));
            }
        }
        Ok(())
    })?;
    visited.ok_or_else(|| {
        CompilerError::compiler_error("a folded string row visited without a string value")
    })
}

// ------------------------------------
//  Fast path
// ------------------------------------

#[test]
fn plain_text_stays_on_the_fast_path() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let text = string_table.intern("plain");
    let declaration = text_declaration("greeting", text, &mut string_table);

    let store = ConstValueStore::from_test_declarations(vec![declaration], &type_environment)
        .expect("a plain text constant is representable in the store");
    let id = store
        .value_for_path(&InternedPath::from_single_str(
            "greeting",
            &mut string_table,
        ))
        .expect("the defining path indexes the store");

    assert!(matches!(
        store.payload(id),
        Some(ConstValuePayload::String(ConstStringValue::Text(stored))) if *stored == text
    ));
    assert_eq!(
        visit_string_value(&store, id).expect("text visits as a string"),
        ConstStringValue::Text(text)
    );
    assert_eq!(store.string_value(id), Some(text));
}

// ------------------------------------
//  Piece round trip
// ------------------------------------

#[test]
fn pieces_round_trip_through_the_visitor_in_order() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let mut resources = ModuleResourceTable::new();
    let prefix = string_table.intern("docs/");
    let logo = resource_id(&mut resources, "assets/logo.svg");

    let declaration = structural_string_declaration(
        "logo",
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::SiteRoot,
        ],
        &mut string_table,
    );
    let store = ConstValueStore::from_test_declarations(vec![declaration], &type_environment)
        .expect("a piece-bearing constant is representable in the store");
    let id = store.value_for_path(&InternedPath::from_single_str("logo", &mut string_table));

    let Some(id) = id else {
        panic!("the defining path indexes the store");
    };

    assert_eq!(
        visit_string_value(&store, id).expect("pieces visit as a string"),
        ConstStringValue::Pieces(vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::SiteRoot,
        ])
    );
}

#[test]
fn mixed_pieces_keep_authored_order() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let mut resources = ModuleResourceTable::new();
    let site_root_suffix = string_table.intern("docs/");
    let stylesheet = resource_id(&mut resources, "assets/site.css");
    let extension = string_table.intern(".html");

    let declaration = structural_string_declaration(
        "docs_url",
        vec![
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(site_root_suffix),
            ConstStringPiece::Resource(stylesheet),
            ConstStringPiece::Text(extension),
        ],
        &mut string_table,
    );
    let store = ConstValueStore::from_test_declarations(vec![declaration], &type_environment)
        .expect("a piece-bearing constant is representable in the store");
    let id = store
        .value_for_path(&InternedPath::from_single_str(
            "docs_url",
            &mut string_table,
        ))
        .expect("the defining path indexes the store");

    assert_eq!(
        visit_string_value(&store, id).expect("pieces visit as a string"),
        ConstStringValue::Pieces(vec![
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(site_root_suffix),
            ConstStringPiece::Resource(stylesheet),
            ConstStringPiece::Text(extension),
        ])
    );
}

// ------------------------------------
//  Template folds
// ------------------------------------

#[test]
fn a_plain_text_template_fold_stays_on_the_text_fast_path() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let folded = string_table.intern("rendered body");
    let declaration = template_declaration("page", &mut string_table);

    let store = ConstValueStore::from_test_template_folds(
        vec![declaration],
        ConstStringValue::Text(folded),
        &type_environment,
    )
    .expect("a folded template result is representable in the store");
    let id = store
        .value_for_path(&InternedPath::from_single_str("page", &mut string_table))
        .expect("the defining path indexes the store");

    // The fold must land on the compact text fast path, not a one-element piece vector.
    assert!(matches!(
        store.payload(id),
        Some(ConstValuePayload::String(ConstStringValue::Text(stored))) if *stored == folded
    ));
    assert_eq!(
        visit_string_value(&store, id).expect("text visits as a string"),
        ConstStringValue::Text(folded)
    );
    assert_eq!(store.string_value(id), Some(folded));
}

#[test]
fn a_piece_bearing_template_fold_round_trips_its_pieces_in_order() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let mut resources = ModuleResourceTable::new();
    let prefix = string_table.intern("docs/");
    let logo = resource_id(&mut resources, "assets/logo.svg");

    let declaration = template_declaration("docs_url", &mut string_table);
    let store = ConstValueStore::from_test_template_folds(
        vec![declaration],
        ConstStringValue::Pieces(vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::SiteRoot,
        ]),
        &type_environment,
    )
    .expect("a piece-bearing template fold is representable in the store");
    let id = store
        .value_for_path(&InternedPath::from_single_str(
            "docs_url",
            &mut string_table,
        ))
        .expect("the defining path indexes the store");

    // A template fold is a structural string like any other: the pieces must survive the
    // store row and the shared visitor in authored order.
    assert_eq!(
        visit_string_value(&store, id).expect("pieces visit as a string"),
        ConstStringValue::Pieces(vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
            ConstStringPiece::SiteRoot,
        ])
    );

    // A piece-bearing fold has no final text while URL context is unresolved, exactly like
    // a piece-bearing file value.
    assert_eq!(store.string_value(id), None);
}

// ------------------------------------
//  Text-only accessor
// ------------------------------------

#[test]
fn the_text_only_accessor_refuses_pieces() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let mut resources = ModuleResourceTable::new();
    let prefix = string_table.intern("docs/");
    let logo = resource_id(&mut resources, "assets/logo.svg");

    let declaration = structural_string_declaration(
        "logo",
        vec![
            ConstStringPiece::Text(prefix),
            ConstStringPiece::Resource(logo),
        ],
        &mut string_table,
    );
    let store = ConstValueStore::from_test_declarations(vec![declaration], &type_environment)
        .expect("a piece-bearing constant is representable in the store");
    let id = store
        .value_for_path(&InternedPath::from_single_str("logo", &mut string_table))
        .expect("the defining path indexes the store");

    // No piece may flatten to text through the accessor while the URL context is unresolved.
    assert_eq!(store.string_value(id), None);
}

// ------------------------------------
//  Record fields
// ------------------------------------

/// Builds one record-typed declaration whose fields carry the given authored locations.
fn record_declaration(
    path: &str,
    fields: Vec<Declaration>,
    string_table: &mut StringTable,
) -> Declaration {
    Declaration {
        id: InternedPath::from_single_str(path, string_table),
        value: Expression::struct_instance(
            InternedPath::from_single_str("Record", string_table),
            fields,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
            true,
            None,
            TypeEnvironment::default().builtins().none,
        ),
    }
}

fn int_field(name: &str, location: SourceLocation, string_table: &mut StringTable) -> Declaration {
    Declaration {
        id: InternedPath::from_single_str(name, string_table),
        value: Expression::int(7, location, ValueMode::ImmutableOwned),
    }
}

#[test]
fn record_fields_keep_authored_order_and_locations() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();
    let alpha_location = SourceLocation::default();

    let declaration = record_declaration(
        "meta",
        vec![
            int_field("alpha", alpha_location.clone(), &mut string_table),
            int_field("beta", SourceLocation::default(), &mut string_table),
        ],
        &mut string_table,
    );

    let store = ConstValueStore::from_test_declarations(vec![declaration], &type_environment)
        .expect("a two-field record is representable in the store");
    let id = store
        .value_for_path(&InternedPath::from_single_str("meta", &mut string_table))
        .expect("the defining path indexes the store");

    let Some(ConstValuePayload::Record(fields)) = store.payload(id) else {
        panic!("expected a stored record payload");
    };

    // Authored field order survives the store, and each field's location is the location of
    // its field expression.
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name.name(), Some(string_table.intern("alpha")));
    assert_eq!(fields[0].location, alpha_location);
    assert_eq!(fields[1].name.name(), Some(string_table.intern("beta")));
}

#[test]
fn duplicate_record_field_name_is_a_construction_error() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::default();

    let declaration = record_declaration(
        "meta",
        vec![
            int_field("name", SourceLocation::default(), &mut string_table),
            int_field("name", SourceLocation::default(), &mut string_table),
        ],
        &mut string_table,
    );

    let mut store = ConstValueStore::default();
    let error = store
        .try_insert_test_declaration(declaration, &type_environment)
        .expect_err("duplicate record field names must fail store construction");
    assert!(
        error.msg.contains("duplicate field names"),
        "unexpected error: {}",
        error.msg
    );
}
