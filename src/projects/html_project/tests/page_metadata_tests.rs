//! Tests for HTML page metadata extraction.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, InvalidPageMetadataReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::constants::HirModuleConst;
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::{FunctionId, HirConstId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use std::path::PathBuf;

fn test_module(string_table: &mut StringTable) -> HirModule {
    let mut module = HirModule::new();
    let start_path = InternedPath::try_from_filesystem_path(
        PathBuf::from("docs/#page.moth").as_path(),
        string_table,
    )
    .expect("test path should be UTF-8")
    .join_str("start", string_table);
    let mut side_table = HirSideTable::default();
    side_table.bind_function_name(FunctionId(0), start_path);
    module.start_function = FunctionId(0);
    module.side_table = side_table;
    module
}

fn string_constant(name: &str, value: &str) -> HirModuleConst {
    HirModuleConst {
        id: HirConstId(0),
        name: name.to_owned(),
        ty: TypeId(0),
        value: HirConstValue::String(value.to_owned()),
    }
}

#[test]
fn extracts_reserved_entry_metadata() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    module.module_constants = vec![
        string_constant("docs/#page.moth/page_title", "Home"),
        string_constant(
            "docs/#page.moth/page_head",
            "<meta name=\"x\" content=\"y\">",
        ),
        string_constant("page_description", "Landing page"),
    ];

    let metadata =
        extract_html_page_metadata(&module, &mut string_table).expect("metadata should parse");
    assert_eq!(metadata.title, Some(String::from("Home")));
    assert_eq!(
        metadata.extra_head_html,
        Some(String::from("<meta name=\"x\" content=\"y\">"))
    );
    assert_eq!(metadata.description, Some(String::from("Landing page")));
}

#[test]
fn ignores_non_entry_constants() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    module.module_constants = vec![
        string_constant("docs/#page.moth/page_title", "Home"),
        string_constant("docs/shared.moth/page_title", "Shared"),
    ];

    let metadata =
        extract_html_page_metadata(&module, &mut string_table).expect("metadata should parse");
    assert_eq!(metadata.title, Some(String::from("Home")));
}

#[test]
fn rejects_non_string_reserved_values() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    module.module_constants = vec![HirModuleConst {
        id: HirConstId(0),
        name: String::from("page_title"),
        ty: TypeId(0),
        value: HirConstValue::Bool(true),
    }];

    let error = extract_html_page_metadata(&module, &mut string_table)
        .expect_err("non-string metadata should fail");
    assert_eq!(
        error.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidPageMetadata)
    );
    assert_eq!(error.kind.descriptor().code, "MOTH-RULE-0061");
    match &error.payload {
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidPageMetadata {
            reason,
            ..
        } => {
            assert_eq!(*reason, InvalidPageMetadataReason::NotAString);
        }
        other => panic!("expected InvalidPageMetadata payload, got {other:?}"),
    }
}

#[test]
fn rejects_duplicate_reserved_values() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    module.module_constants = vec![
        string_constant("page_title", "Home"),
        string_constant("docs/#page.moth/page_title", "Another"),
    ];

    let error = extract_html_page_metadata(&module, &mut string_table)
        .expect_err("duplicate metadata should fail");
    assert_eq!(
        error.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidPageMetadata)
    );
    match &error.payload {
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidPageMetadata {
            reason,
            ..
        } => {
            assert_eq!(*reason, InvalidPageMetadataReason::DuplicateDeclaration);
        }
        other => panic!("expected InvalidPageMetadata payload, got {other:?}"),
    }
}
