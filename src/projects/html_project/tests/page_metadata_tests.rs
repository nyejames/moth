//! Tests for HTML page metadata extraction.

use super::*;
use crate::compiler_frontend::ast::const_values::facts::{
    ConstBindingScope, ConstBindingSource, ConstFactValueKind,
};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, InvalidPageMetadataReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::const_facts::HirConstDeclarationFact;
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
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
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

fn add_const_fact(
    module: &mut HirModule,
    name: &str,
    location: SourceLocation,
    string_table: &mut StringTable,
) {
    let declaration_path = InternedPath::from_single_str(name, string_table);
    module.const_facts.declarations.insert(
        declaration_path.clone(),
        HirConstDeclarationFact {
            declaration_path,
            scope: ConstBindingScope::ExplicitTopLevel,
            source: ConstBindingSource::ExplicitHash,
            value_kind: ConstFactValueKind::RenderableTemplate,
            location,
        },
    );
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
        &ModuleResourceTable::new(),
        &mut string_table,
    )
    .expect("metadata should parse");
    assert_eq!(
        metadata.metadata.title,
        Some(OwnedFoldedString::Text(String::from("Home")))
    );
    assert_eq!(
        metadata.metadata.extra_head_html,
        Some(OwnedFoldedString::Text(String::from(
            "<meta name=\"x\" content=\"y\">"
        )))
    );
    assert_eq!(
        metadata.metadata.description,
        Some(OwnedFoldedString::Text(String::from("Landing page")))
    );
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
        &ModuleResourceTable::new(),
        &mut string_table,
    )
    .expect("metadata should parse");
    assert_eq!(
        metadata.metadata.title,
        Some(OwnedFoldedString::Text(String::from("Home")))
    );
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
        &ModuleResourceTable::new(),
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

    let message = metadata_error_message(&error.payload, &string_table);
    assert!(
        message.contains("must fold to a string"),
        "unexpected message: {message}"
    );
}

#[test]
fn renders_structural_string_reserved_values() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    let mut resources = ModuleResourceTable::new();
    let resource_id = fixture_resource_id(&mut resources, "static/favicon.png");
    let origin = resources
        .try_origin(resource_id)
        .expect("fixture resource should be present")
        .origin
        .clone();
    module.module_constants = vec![HirModuleConst {
        id: HirConstId(0),
        name: String::from("page_favicon"),
        ty: TypeId(0),
        value: HirConstValue::StructuralString {
            pieces: vec![ConstStringPiece::Resource(resource_id)],
        },
    }];

    let metadata = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &resources,
        &mut string_table,
    )
    .expect("structural string metadata should render later");
    assert_eq!(
        metadata.metadata.favicon,
        Some(OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Resource(origin.clone())
        ]))
    );

    let mut plan = HtmlResourceOutputPlan::new("page-metadata-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    plan.plan_origin(
        origin,
        Default::default(),
        context.clone(),
        &mut string_table,
        true,
    )
    .expect("resource should be planned");
    let renderer = StructuralUrlRenderer::new(&plan, &context, Some("/"));
    let rendered = renderer
        .render_owned(
            metadata
                .metadata
                .favicon
                .as_ref()
                .expect("favicon should be present"),
        )
        .expect("structural favicon should render");
    assert_eq!(rendered, "./static/favicon.png");
}

#[test]
fn metadata_plan_keeps_authored_resource_and_site_root_uses() {
    let mut string_table = StringTable::new();
    let mut module = test_module(&mut string_table);
    let mut resources = ModuleResourceTable::new();
    let resource_id = fixture_resource_id(&mut resources, "static/favicon.png");
    let origin = resources
        .try_origin(resource_id)
        .expect("fixture resource should be present")
        .origin
        .clone();
    let metadata_location = SourceLocation::new(
        InternedPath::from_single_str("metadata.moth", &mut string_table),
        CharPosition {
            line_number: 11,
            char_column: 3,
        },
        CharPosition {
            line_number: 11,
            char_column: 15,
        },
    );
    module.module_constants = vec![HirModuleConst {
        id: HirConstId(0),
        name: String::from("page_favicon"),
        ty: TypeId(0),
        value: HirConstValue::StructuralString {
            pieces: vec![
                ConstStringPiece::Resource(resource_id),
                ConstStringPiece::SiteRoot,
            ],
        },
    }];
    add_const_fact(
        &mut module,
        "page_favicon",
        metadata_location.clone(),
        &mut string_table,
    );

    let plan = extract_html_page_metadata(
        &module,
        module
            .start_function
            .expect("entry module should have start"),
        &resources,
        &mut string_table,
    )
    .expect("metadata plan should parse");

    assert_eq!(
        plan.resource_uses,
        vec![MetadataResourceUse {
            origin,
            authored_location: metadata_location,
        }]
    );
    assert!(plan.uses_site_root);
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
        &ModuleResourceTable::new(),
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
