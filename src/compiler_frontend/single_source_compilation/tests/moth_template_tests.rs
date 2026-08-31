//! Direct Moth template compilation service tests.
//!
//! WHAT: the service's standalone contract — one in-memory template source in, one folded `content`
//!       value plus that source's warnings out.
//! WHY:  input normalization, ordering and duplicate-path diagnostics are owned by the HTML
//!       project's direct-API tests, which drive real files through the whole request shape. What
//!       those cannot show is that folding a template is a compiler entry point needing no project
//!       request, no builder and no filesystem.

use super::{FoldedMothTemplate, MothTemplateCompilationRequest, compile_moth_template_source};
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

fn resource(
    resources: &mut ModuleResourceTable,
    relative_path: &str,
) -> (ResourceId, StableResourceOriginId) {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let logical_path = PortableResourcePath::from_relative_logical_path(Path::new(relative_path))
        .expect("the test resource path should be portable");
    let origin = StableResourceOriginId::module_owned(module_origin, logical_path);
    let resource = resources.intern_origin(origin.clone(), SourceLocation::default());
    (resource, origin)
}

#[test]
fn plain_text_content_uses_the_text_fast_path_and_moves_owned_text() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let folded = compile_moth_template_source(
        MothTemplateCompilationRequest {
            source_path: Path::new("/templates/intro.mtf"),
            source_code: String::from("# Intro"),
            style_directives: &style_directives,
        },
        &mut string_table,
    )
    .expect("an in-memory template source should fold");

    let FoldedMothTemplate { content, warnings } = folded;
    assert_eq!(
        content,
        OwnedFoldedString::Text(String::from("<h1>Intro</h1>"))
    );
    assert_eq!(
        content.into_text(),
        Some(String::from("<h1>Intro</h1>")),
        "plain text extraction should move the owned text"
    );
    assert!(warnings.is_empty());
}

#[test]
fn structural_content_preserves_piece_order_and_refuses_text_extraction() {
    let mut string_table = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let prefix = string_table.intern("before/");
    let suffix = string_table.intern("/after");
    let (resource_id, resource_origin) = resource(&mut resources, "assets/logo.svg");

    let store_value = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(resource_id),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::Text(suffix),
    ]);
    let content = owned_folded_string_from_const_string(&store_value, &resources, &string_table)
        .expect("the resource table should resolve every structural resource piece");
    let expected = OwnedFoldedString::Pieces(vec![
        OwnedFoldedStringPiece::Text(String::from("before/")),
        OwnedFoldedStringPiece::Resource(resource_origin),
        OwnedFoldedStringPiece::SiteRoot,
        OwnedFoldedStringPiece::Text(String::from("/after")),
    ]);

    // The direct lane has no file-value resolution services yet, so this structural result is
    // exercised through the same owned conversion used by the service until Phase 5 service parity
    // supplies resolved content dependencies.
    let folded = FoldedMothTemplate {
        content,
        warnings: Vec::new(),
    };
    assert_eq!(folded.content, expected);
    assert_eq!(
        folded.content.into_text(),
        None,
        "structural content must not flatten unresolved pieces"
    );
}
