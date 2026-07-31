//! Receiver method catalog and dispatch regression tests.
//!
//! WHAT: validates how receiver methods are indexed, resolved, and dispatched across same-file
//!       nominal types plus compiler-owned builtin scalar receivers.
//! WHY: source-authored receiver methods travel with their declaring receiver type, while builtin
//!      receiver behavior stays compiler-owned; catalog drift breaks both call paths.

use super::environment::TopLevelDeclarationTable;
use super::scope_context::{ContextKind, ScopeContext};
use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_resolution::validate_no_recursive_runtime_structs;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::{
    BuiltinScalarReceiver, DataType, ReceiverKey, builtin_type_ids,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::rc::Rc;
use std::sync::Arc;

fn interned_path(parts: &[&str], string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_components(parts.iter().map(|part| string_table.intern(part)).collect())
}

fn empty_receiver_entry(
    function_path: InternedPath,
    source_file: InternedPath,
    receiver: ReceiverKey,
) -> ReceiverMethodEntry {
    ReceiverMethodEntry {
        function_path,
        receiver,
        source_file,
        receiver_mutable: false,
        signature: FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
    }
}

fn context_for_source_file(
    source_file: InternedPath,
    receiver_methods: ReceiverMethodCatalog,
) -> ScopeContext {
    ScopeContext::new_for_tests(
        ContextKind::Function,
        InternedPath::new(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_source_file_scope(source_file)
    .with_receiver_methods(Rc::new(receiver_methods))
}

#[test]
fn lookup_receiver_method_prefers_exact_source_file_before_catalog_fallback() {
    let mut string_table = StringTable::new();
    let method_name = string_table.intern("reset");
    let receiver = ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Int);
    let key = (receiver.to_owned(), method_name);
    let local_source = interned_path(&["src", "@page.moth"], &mut string_table);
    let package_source = interned_path(&["lib", "shared.moth"], &mut string_table);

    let local_entry = empty_receiver_entry(
        interned_path(&["src", "reset"], &mut string_table),
        local_source.to_owned(),
        receiver.to_owned(),
    );
    let package_entry = empty_receiver_entry(
        interned_path(&["lib", "reset"], &mut string_table),
        package_source,
        receiver.to_owned(),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog
        .by_receiver_and_name
        .insert(key, vec![package_entry, local_entry.to_owned()]);

    let exact_context = context_for_source_file(local_source, catalog);
    let resolved = exact_context
        .lookup_receiver_method(&receiver, method_name)
        .expect("same-file receiver method should be visible");
    assert_eq!(
        resolved.function_path, local_entry.function_path,
        "same-file receiver methods should be preferred when file visibility is omitted"
    );
}

#[test]
fn visible_method_lookup_prefers_same_file_before_catalog_fallback() {
    let mut string_table = StringTable::new();
    let method_name = string_table.intern("render");

    let local_source = interned_path(&["src", "@page.moth"], &mut string_table);
    let exported_source = interned_path(&["lib", "shared.moth"], &mut string_table);

    let local_entry = empty_receiver_entry(
        interned_path(&["src", "render_local"], &mut string_table),
        local_source.to_owned(),
        ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String),
    );
    let package_entry = empty_receiver_entry(
        interned_path(&["lib", "render"], &mut string_table),
        exported_source,
        ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog
        .by_method_name
        .insert(method_name, vec![package_entry, local_entry.to_owned()]);

    let context = context_for_source_file(local_source, catalog);
    let resolved = context
        .lookup_visible_receiver_method_by_name(method_name)
        .expect("same-file receiver method should be visible");
    assert_eq!(
        resolved.function_path, local_entry.function_path,
        "same-file methods must be preferred over exported fallback entries"
    );
}

#[test]
fn visible_method_lookup_uses_stable_catalog_fallback_order() {
    let mut string_table = StringTable::new();
    let method_name = string_table.intern("render");
    let context_source = interned_path(&["src", "@page.moth"], &mut string_table);

    let first_entry = empty_receiver_entry(
        interned_path(&["lib", "a_render"], &mut string_table),
        interned_path(&["lib", "a.moth"], &mut string_table),
        ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String),
    );
    let second_entry = empty_receiver_entry(
        interned_path(&["lib", "z_render"], &mut string_table),
        interned_path(&["lib", "z.moth"], &mut string_table),
        ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog
        .by_method_name
        .insert(method_name, vec![first_entry.to_owned(), second_entry]);

    let context = context_for_source_file(context_source, catalog);
    let resolved = context
        .lookup_visible_receiver_method_by_name(method_name)
        .expect("catalog fallback receiver method should be visible");
    assert_eq!(
        resolved.function_path, first_entry.function_path,
        "catalog fallback lookup should resolve using stable catalog order"
    );
}

#[test]
fn recursive_runtime_struct_cycles_are_rejected() {
    let mut string_table = StringTable::new();
    let struct_a = interned_path(&["A"], &mut string_table);
    let struct_b = interned_path(&["B"], &mut string_table);
    let struct_a_field_b = interned_path(&["A", "b"], &mut string_table);
    let struct_b_field_a = interned_path(&["B", "a"], &mut string_table);

    let mut struct_fields = FxHashMap::default();
    struct_fields.insert(
        struct_a.to_owned(),
        vec![Declaration {
            id: struct_a_field_b,
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::NONE,
                DataType::runtime_struct(struct_b.to_owned(), builtin_type_ids::NONE),
                ValueMode::ImmutableOwned,
            ),
        }],
    );
    struct_fields.insert(
        struct_b.to_owned(),
        vec![Declaration {
            id: struct_b_field_a,
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::NONE,
                DataType::runtime_struct(struct_a, builtin_type_ids::NONE),
                ValueMode::ImmutableOwned,
            ),
        }],
    );

    let diagnostic = validate_no_recursive_runtime_structs(&struct_fields, &string_table)
        .expect_err("recursive runtime struct cycle should be rejected");
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::RecursiveRuntimeStruct { .. },
            ..
        }
    ));
}

#[test]
fn non_recursive_runtime_structs_are_allowed() {
    let mut string_table = StringTable::new();
    let struct_a = interned_path(&["A"], &mut string_table);
    let field_ax = interned_path(&["A", "x"], &mut string_table);

    let mut struct_fields = FxHashMap::default();
    struct_fields.insert(
        struct_a,
        vec![Declaration {
            id: field_ax,
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        }],
    );

    validate_no_recursive_runtime_structs(&struct_fields, &string_table)
        .expect("non-recursive runtime structs should pass validation");
}
