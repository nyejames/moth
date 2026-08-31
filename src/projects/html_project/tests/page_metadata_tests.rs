//! Tests for HTML page metadata extraction.

use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, InvalidPageMetadataReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::constants::HirModuleConst;
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::{FunctionId, HirConstId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use std::path::{Path, PathBuf};

fn test_module(string_table: &mut StringTable) -> HirModule {
    let mut module = HirModule::new();
    let start_path = InternedPath::try_from_filesystem_path(
        PathBuf::from("docs/@page.moth").as_path(),
        string_table,
    )
    .expect("test path should be UTF-8")
    .join_str("start", string_table);
    let mut side_table = HirSideTable::default();
    side_table.bind_function_name(FunctionId(0), start_path);
    module.start_function = Some(FunctionId(0));
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

/// Interns one fixture resource origin so tests can build realistic `Resource` pieces.
fn fixture_resource_id(resources: &mut ModuleResourceTable, relative_path: &str) -> ResourceId {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("page-metadata-tests"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let origin = StableResourceOriginId::module_owned(
        module_origin,
        PortableResourcePath::from_relative_logical_path(Path::new(relative_path))
            .expect("fixture resource path should be portable"),
    );
    resources.intern_origin(origin, SourceLocation::default())
}

/// Renders the page-metadata payload message through the shared render boundary.
fn metadata_error_message(
    payload: &crate::compiler_frontend::compiler_messages::DiagnosticPayload,
    string_table: &StringTable,
) -> String {
    let render_context = DiagnosticRenderContext::new(string_table);
    terminal::format_payload_guidance(payload, render_context).join("\n")
}

#[test]
fn extracts_reserved_entry_metadata() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    module.module_constants = vec![
        string_constant("docs/@page.moth/page_title", "Home"),
        string_constant(
            "docs/@page.moth/page_head",
            "<meta name=\"x\" content=\"y\">",
        ),
        string_constant("page_description", "Landing page"),
    ];

    let metadata = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &mut string_table,
    )
    .expect("metadata should parse");
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
        string_constant("docs/@page.moth/page_title", "Home"),
        string_constant("docs/shared.moth/page_title", "Shared"),
    ];

    let metadata = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &mut string_table,
    )
    .expect("metadata should parse");
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

    let error = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &mut string_table,
    )
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

    // The rendered voice of NotAString must stay distinct from NotYetRenderable: this value
    // genuinely is not a string, so the message claims it must fold to one.
    let message = metadata_error_message(&error.payload, &string_table);
    assert!(
        message.contains("must fold to a string"),
        "unexpected message: {message}"
    );
}

#[test]
fn rejects_structural_string_reserved_values() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    let mut resources = ModuleResourceTable::new();
    module.module_constants = vec![HirModuleConst {
        id: HirConstId(0),
        name: String::from("page_favicon"),
        ty: TypeId(0),
        value: HirConstValue::StructuralString {
            pieces: vec![ConstStringPiece::Resource(fixture_resource_id(
                &mut resources,
                "static/favicon.png",
            ))],
        },
    }];

    let error = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &mut string_table,
    )
    .expect_err("structural string metadata should fail");
    assert_eq!(
        error.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidPageMetadata)
    );
    match &error.payload {
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidPageMetadata {
            reason,
            ..
        } => {
            assert_eq!(*reason, InvalidPageMetadataReason::NotYetRenderable);
        }
        other => panic!("expected InvalidPageMetadata payload, got {other:?}"),
    }

    // The value is a legitimate string whose resource piece has no final text yet, so the
    // message must not claim it is not a string or borrow the NotAString voice.
    let message = metadata_error_message(&error.payload, &string_table);
    assert!(
        message.contains("is a string"),
        "unexpected message: {message}"
    );
    assert!(
        !message.contains("not a string"),
        "unexpected message: {message}"
    );
    assert!(
        !message.contains("must fold to a string"),
        "unexpected message: {message}"
    );
}

#[test]
fn rejects_duplicate_reserved_values() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    module.module_constants = vec![
        string_constant("page_title", "Home"),
        string_constant("docs/@page.moth/page_title", "Another"),
    ];

    let error = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &mut string_table,
    )
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
