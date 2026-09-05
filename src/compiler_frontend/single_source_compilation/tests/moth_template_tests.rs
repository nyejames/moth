//! Direct Moth template compilation service tests.
//!
//! WHAT: the service's standalone contract — one in-memory template source in, one folded
//!       `content` value plus that module's resource facts and warnings out, and the prepared
//!       file-value bundle path for Stage 0 parity with integrated module folds.
//! WHY:  input normalization, ordering and duplicate-path diagnostics are owned by the HTML
//!       project's direct-API tests, which drive real files through the whole request shape. What
//!       those cannot show is that folding a template is a compiler entry point needing no project
//!       request, no builder and no filesystem.

use super::{FoldedMothTemplate, MothTemplateCompilationRequest, compile_moth_template_source};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareOutput, HeaderParseOptions,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget, ResourceSourceId,
};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::single_source_compilation::MothTemplateFileValueBundle;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use std::path::Path;
use std::sync::Arc;

const TEMPLATE_PATH: &str = "site/intro.mtf";
const MARKDOWN_PATH: &str = "site/docs/intro.md";

#[test]
fn plain_text_content_uses_the_text_fast_path_and_moves_owned_text() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let folded = compile_moth_template_source(
        MothTemplateCompilationRequest {
            source_path: Path::new("/templates/intro.mtf"),
            source_code: Some(String::from("# Intro")),
            style_directives: &style_directives,
            file_value_resolution: None,
        },
        &mut string_table,
    )
    .expect("an in-memory template source should fold");

    let FoldedMothTemplate {
        content,
        module_resources,
        warnings,
    } = folded;
    assert_eq!(
        content,
        OwnedFoldedString::Text(String::from("<h1>Intro</h1>"))
    );
    assert_eq!(
        content.into_text(),
        Some(String::from("<h1>Intro</h1>")),
        "plain text extraction should move the owned text"
    );
    assert!(
        module_resources.origins().is_empty(),
        "a plain request has no file values, so no resource origin is interned"
    );
    assert!(warnings.is_empty());
}

#[test]
fn bundle_request_folds_resource_site_root_and_nested_content_structurally() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();

    let template_path = Path::new(TEMPLATE_PATH);
    let markdown_path = Path::new(MARKDOWN_PATH);
    // The template names a nested Markdown content source, a resource file and the site root.
    let template_source = "# Intro\n\n[@docs/intro.md]\n\n[@assets/logo.svg] [@/]";
    let mut source_files = SourceDatabase::build(
        [template_path, markdown_path],
        template_path,
        None,
        &mut string_table,
    )
    .expect("bundle source identities should build");
    let template_id = source_files
        .get_by_canonical_path(template_path)
        .expect("template source should have a source identity")
        .id;
    source_files
        .retain_text(template_id, template_source.to_owned())
        .expect("template source should retain its text");
    let markdown_id = source_files
        .get_by_canonical_path(markdown_path)
        .expect("markdown source should have a source identity")
        .id;
    source_files
        .retain_text(markdown_id, "Nested intro body.".to_owned())
        .expect("markdown source should retain its text");
    let source_files = Arc::new(source_files);
    let file_id = |source_files: &SourceDatabase, path: &Path| {
        source_files
            .get_by_canonical_path(path)
            .unwrap_or_else(|| panic!("bundle file {path:?} should have a source identity"))
            .id
    };

    let prepared_template = prepare_bundle_source(
        &source_files,
        template_path,
        template_source,
        &style_directives,
        &mut string_table,
    );
    let prepared_markdown = prepare_bundle_source(
        &source_files,
        markdown_path,
        "Nested intro body.",
        &style_directives,
        &mut string_table,
    );

    // This is the Stage 0 fact the caller supplies: one content target, one resource source and
    // the site root's no-target outcome, keyed by the prepared occurrence identities.
    let mut resolved_file_references = ResolvedFileReferenceTable::new();
    for reference in prepared_template.structural_file_references.references() {
        let source_file = reference
            .source_file
            .expect("prepared rows carry a source SourceId");
        let outcome = match reference.class {
            PreparedFileReferenceClass::ContentSource => {
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
                    source: file_id(&source_files, markdown_path),
                })
            }
            PreparedFileReferenceClass::ResourceFile => {
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
                    source: ResourceSourceId::from_index(0),
                    owner_relative_path: portable_resource_path("assets/logo.svg"),
                })
            }
            PreparedFileReferenceClass::SiteRoot => ResolvedFileReferenceOutcome::NoPhysicalTarget,
            PreparedFileReferenceClass::SourceKindNoFileValue
            | PreparedFileReferenceClass::Extensionless => continue,
        };
        resolved_file_references
            .push(ResolvedFileReference {
                source_file,
                path_syntax: reference.path_syntax,
                class: reference.class,
                outcome,
            })
            .expect("bundle resolved rows should be unique");
    }

    let module_origin = direct_test_module_origin();
    let bundle = MothTemplateFileValueBundle {
        prepared_content_sources: vec![prepared_markdown],
        resolved_file_references,
        source_files: Arc::clone(&source_files),
        module_origin: Some(module_origin.clone()),
    };

    let FoldedMothTemplate {
        content,
        module_resources,
        warnings,
    } = compile_moth_template_source(
        MothTemplateCompilationRequest {
            source_path: template_path,
            source_code: None,
            style_directives: &style_directives,
            file_value_resolution: Some(bundle),
        },
        &mut string_table,
    )
    .expect("a bundle-bearing template should fold against the resolved bundle");

    let expected_resource_origin = StableResourceOriginId::module_owned(
        module_origin,
        portable_resource_path("assets/logo.svg"),
    );
    let OwnedFoldedString::Pieces(pieces) = &content else {
        panic!("bundle content with resource and site-root values must stay structural");
    };
    assert!(
        pieces.contains(&OwnedFoldedStringPiece::Resource(
            expected_resource_origin.clone()
        )),
        "resource file value must keep its stable origin as a piece: {pieces:?}"
    );
    assert!(
        pieces.contains(&OwnedFoldedStringPiece::SiteRoot),
        "bare site root must stay a structural piece: {pieces:?}"
    );
    assert!(
        pieces.iter().any(|piece| matches!(
            piece,
            OwnedFoldedStringPiece::Text(text) if text.contains("<h1>Intro</h1>")
        )),
        "the template's own Markdown should fold into a text piece: {pieces:?}"
    );
    assert_eq!(
        content.into_text(),
        None,
        "structural content must not flatten unresolved pieces"
    );
    assert!(
        module_resources
            .origins()
            .iter()
            .any(|origin| origin.origin == expected_resource_origin),
        "the folded module's resource table must report the resolved origin as a source fact"
    );
    assert!(warnings.is_empty());
}

fn prepare_bundle_source(
    source_files: &SourceDatabase,
    source_path: &Path,
    source_code: &str,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> FileFrontendPrepareOutput {
    let source = match source_path.extension() {
        Some(extension) if extension == "md" => FrontendFilePrepareSource::PlainMarkdown {
            source_code,
            source_path: source_path.to_path_buf(),
        },
        _ => FrontendFilePrepareSource::MothTemplate {
            source_code,
            source_path: source_path.to_path_buf(),
        },
    };

    // Bundle construction only needs the shared per-file preparation owner; the service
    // re-prepares its own source from the same identity facts.
    let options = HeaderParseOptions::default();
    let context = FrontendFilePrepareContext {
        source_files,
        style_directives,
        entry_file_path: source_path,
        options: &options,
    };
    let input = FrontendFilePrepareInput {
        source,
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };
    CompilerFrontend::prepare_file_frontend_local(&context, input, string_table)
        .expect("bundle source preparation should succeed")
}

fn direct_test_module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    )
}

fn portable_resource_path(path: &str) -> PortableResourcePath {
    PortableResourcePath::from_relative_logical_path(std::path::Path::new(path))
        .expect("test resource path should be portable")
}
