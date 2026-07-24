//! Const fact collection tests for AST finalization.
//!
//! WHAT: verifies that `AstFinalizer` collects const facts correctly for
//!       explicit module constants, private top-level declarations, and
//!       body-local declarations.
//! WHY: fact collection is the bridge between the resolver (Phase 1) and
//!      later config/HIR consumers (Phases 3–5); it must be tested directly.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::const_values::facts::{
    AstConstDeclarationFact, ConstBindingScope, ConstBindingSource,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

fn assert_has_fact(ast: &Ast, string_table: &StringTable, name: &str) {
    assert!(
        find_fact_by_name(ast, string_table, name).is_some(),
        "expected const fact for '{name}'"
    );
}

fn assert_no_fact(ast: &Ast, string_table: &StringTable, name: &str) {
    assert!(
        find_fact_by_name(ast, string_table, name).is_none(),
        "expected no const fact for '{name}'"
    );
}

fn fact_for<'a>(
    ast: &'a Ast,
    string_table: &StringTable,
    name: &str,
) -> &'a AstConstDeclarationFact {
    find_fact_by_name(ast, string_table, name)
        .unwrap_or_else(|| panic!("expected const fact for '{name}'"))
}

fn find_fact_by_name<'a>(
    ast: &'a Ast,
    string_table: &StringTable,
    name: &str,
) -> Option<&'a AstConstDeclarationFact> {
    ast.const_facts
        .declarations
        .values()
        .find(|fact| fact.declaration_path.name_str(string_table) == Some(name))
}

// ------------------------------
//  Explicit module constant fact
// ------------------------------

#[test]
fn explicit_module_constant_is_collected_as_fact() {
    let source = r#"site_name #= "Moth""#;
    let (ast, string_table) = parse_single_file_ast(source);

    let fact = fact_for(&ast, &string_table, "site_name");
    assert_eq!(fact.scope, ConstBindingScope::ExplicitTopLevel);
    assert_eq!(fact.source, ConstBindingSource::ExplicitHash);
}

// ------------------------------
//  Private top-level literal fact
// ------------------------------

#[test]
fn private_top_level_literal_is_collected_as_fact() {
    let source = r#"entry_root = "src""#;
    let (ast, string_table) = parse_single_file_ast(source);

    let fact = fact_for(&ast, &string_table, "entry_root");
    assert_eq!(fact.scope, ConstBindingScope::PrivateTopLevel);
    assert_eq!(fact.source, ConstBindingSource::InferredImmutable);
}

// ------------------------------
//  Private top-level reference to earlier private fact
// ------------------------------

#[test]
fn private_top_level_reference_to_earlier_private_fact_is_collected() {
    let source = r#"
output_folder = "release"
dev_folder = output_folder
"#;
    let (ast, string_table) = parse_single_file_ast(source);

    assert_has_fact(&ast, &string_table, "output_folder");
    let fact = fact_for(&ast, &string_table, "dev_folder");
    assert_eq!(fact.scope, ConstBindingScope::PrivateTopLevel);
    assert_eq!(fact.source, ConstBindingSource::InferredImmutable);
}

// ------------------------------
//  Private top-level forward reference not fact
// ------------------------------

// NOTE: The current parser rejects forward references in the start body with
// `UnknownValueName` before AST finalization runs. This means the fact
// collector never sees a forward reference in practice. The const resolver unit
// tests in `src/compiler_frontend/ast/const_values/tests/mod.rs` verify that unresolved
// references fail const resolution with `ConstResolutionError::UnresolvedReference`.

// ------------------------------
//  Body-local literal fact
// ------------------------------

#[test]
fn body_local_literal_is_collected_as_fact() {
    let source = r#"
greet || -> String:
    message = "hello"
    return message
;
"#;
    let (ast, string_table) = parse_single_file_ast(source);

    let fact = fact_for(&ast, &string_table, "message");
    assert_eq!(fact.scope, ConstBindingScope::BodyLocal);
    assert_eq!(fact.source, ConstBindingSource::InferredImmutable);
}

// ------------------------------
//  Body-local reference to earlier fact
// ------------------------------

#[test]
fn body_local_reference_to_earlier_fact_is_collected() {
    let source = r#"
greet || -> String:
    prefix = "hello"
    message = prefix
    return message
;
"#;
    let (ast, string_table) = parse_single_file_ast(source);

    assert_has_fact(&ast, &string_table, "prefix");
    let fact = fact_for(&ast, &string_table, "message");
    assert_eq!(fact.scope, ConstBindingScope::BodyLocal);
    assert_eq!(fact.source, ConstBindingSource::InferredImmutable);
}

#[test]
fn catch_handler_body_local_literal_is_collected_as_fact() {
    let source = r#"
can_error || -> String, Error!:
    return! Error("boom")
;

recover || -> String:
    output = can_error() catch |err|:
        fallback = "fallback"
        then fallback
    ;
    return output
;
"#;
    let (ast, string_table) = parse_single_file_ast(source);

    let fact = fact_for(&ast, &string_table, "fallback");
    assert_eq!(fact.scope, ConstBindingScope::BodyLocal);
    assert_eq!(fact.source, ConstBindingSource::InferredImmutable);
}

// ------------------------------
//  Mutable declaration not fact
// ------------------------------

#[test]
fn mutable_declaration_is_not_a_fact() {
    let source = r#"value ~= 1"#;
    let (ast, string_table) = parse_single_file_ast(source);

    assert_no_fact(&ast, &string_table, "value");
}

#[test]
fn body_local_mutable_declaration_is_not_a_fact() {
    let source = r#"
greet || -> Int:
    value ~= 1
    return value
;
"#;
    let (ast, string_table) = parse_single_file_ast(source);

    assert_no_fact(&ast, &string_table, "value");
}
