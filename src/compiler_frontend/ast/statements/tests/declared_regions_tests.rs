//! Deferred declared-region statement classification tests.
//!
//! WHAT: verifies the reserved `identifier:` header, focused `_:` rejection and ordinary
//!       `block`, `group` and `region` identifier policy.
//! WHY: declared-region headers must be classified before ordinary symbol use or declaration parsing while
//!      the retained compiler-generated lexical-scope node remains unreachable from source.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureReason, DiagnosticPayload, InvalidStatementPositionReason,
};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

fn assert_declared_region_is_deferred(source: &str) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::DeferredFeature {
            reason: DeferredFeatureReason::DeclaredRegion
        }
    ));
}

#[test]
fn reserves_declared_region_header_before_declaration_dispatch() {
    assert_declared_region_is_deferred("request:\n    value = 1\n;\n");
}

#[test]
fn reserves_existing_local_name_as_declared_region_header() {
    assert_declared_region_is_deferred("request = 1\nrequest:\n    value = 2\n;\n");
}

#[test]
fn underscore_prefixed_name_uses_the_declared_region_path() {
    assert_declared_region_is_deferred("_request:\n    value = 1\n;\n");
}

#[test]
fn ordinary_identifier_spellings_use_the_declared_region_path() {
    for name in ["block", "group", "region"] {
        assert_declared_region_is_deferred(&format!("{name}:\n    value = 1\n;\n"));
    }
}

#[test]
fn colon_type_shape_still_uses_the_declared_region_path() {
    assert_declared_region_is_deferred("request: String = \"Priya\"\n");
}

#[test]
fn rejects_exact_anonymous_declared_region_spelling() {
    let diagnostic = parse_single_file_ast_diagnostic("_:\n    value = 1\n;\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::AnonymousDeclaredRegion
        }
    ));
}

#[test]
fn block_group_and_region_are_ordinary_variable_names() {
    let (ast, string_table) = parse_single_file_ast("block = 1\ngroup = 2\nregion = 3\n");
    let body = start_function_body(&ast, &string_table);

    assert_eq!(body.len(), 3);
    assert!(
        body.iter()
            .all(|node| matches!(node.kind, NodeKind::VariableDeclaration(_)))
    );
}

#[test]
fn block_is_an_ordinary_function_name() {
    let (ast, string_table) =
        parse_single_file_ast("block || -> Int:\n    return 1\n;\n\nresult = block()\n");

    assert_eq!(function_body_by_name(&ast, &string_table, "block").len(), 1);
}

#[test]
fn typed_declaration_is_not_a_declared_region_header() {
    let (ast, string_table) = parse_single_file_ast("name String = \"Priya\"\n");
    let body = start_function_body(&ast, &string_table);

    assert!(matches!(body[0].kind, NodeKind::VariableDeclaration(_)));
}

#[test]
fn executable_source_cannot_emit_internal_lexical_scope_node() {
    let (ast, string_table) =
        parse_single_file_ast("condition ~= true\nif condition:\n    value = 1\n;\n\nafter = 2\n");
    let body = start_function_body(&ast, &string_table);

    assert!(
        body.iter()
            .all(|node| !matches!(node.kind, NodeKind::LexicalScope { .. }))
    );
}
