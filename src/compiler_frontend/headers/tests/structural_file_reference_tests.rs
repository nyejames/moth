//! Header-prep classification of graph-active file-value paths.

use crate::compiler_frontend::headers::parse_file_headers::prepare_file_from_tokens;
use crate::compiler_frontend::headers::types::HeaderParseOptions;
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::path::Path;

fn prepare_source(
    source: &str,
) -> (
    crate::compiler_frontend::headers::types::FileFrontendPrepareOutput,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let file_path = Path::new("@page.moth");
    let interned_path = InternedPath::try_from_filesystem_path(file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        Some(SourceId::from_index(0)),
    )
    .expect("tokenization should succeed");

    let output = prepare_file_from_tokens(
        file_tokens,
        file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
    )
    .expect("preparation should succeed");
    (output, string_table)
}

#[test]
fn dependency_clause_paths_are_excluded_from_file_values() {
    let (output, strings) = prepare_source(
        "@core/math sin\n\
         unused #= @assets/logo.svg\n",
    );
    let references = output.structural_file_references.references();
    assert_eq!(
        output
            .path_syntax
            .table()
            .try_path(references[0].path_syntax)
            .expect("prepared reference should point into its path table")
            .root
            .to_portable_string(&strings),
        "assets/logo.svg"
    );
    assert_eq!(
        references[0].class,
        PreparedFileReferenceClass::ResourceFile
    );
}

#[test]
fn unused_content_and_resource_paths_are_graph_active() {
    let (output, _) = prepare_source(
        "unused_mtf #= @docs/old.mtf\n\
         unused_md #= @legal/license.md\n\
         unused_resource #= @assets/large.webp\n",
    );
    let classes: Vec<_> = output
        .structural_file_references
        .references()
        .iter()
        .map(|reference| reference.class)
        .collect();
    assert_eq!(
        classes,
        [
            PreparedFileReferenceClass::ContentSource,
            PreparedFileReferenceClass::ContentSource,
            PreparedFileReferenceClass::ResourceFile,
        ]
    );
}

#[test]
fn site_root_and_quoted_urls_create_no_file_edge() {
    let (output, _) = prepare_source(
        "root #= @/\n\
         external #= \"https://example.com/logo.svg\"\n",
    );
    let references = output.structural_file_references.references();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].class, PreparedFileReferenceClass::SiteRoot);
}

#[test]
fn moth_value_paths_are_classified_without_becoming_dependency_clauses() {
    let (output, _) = prepare_source("helpers = @helpers.moth\n");
    assert!(output.file_dependency_clauses.is_empty());
    let references = output.structural_file_references.references();
    assert_eq!(references.len(), 1);
    assert_eq!(
        references[0].class,
        PreparedFileReferenceClass::SourceKindNoFileValue
    );
}

#[test]
fn a_path_inside_a_broken_expression_is_still_graph_active() {
    let (output, _) = prepare_source("broken #= @assets/logo.svg foo\n");
    let references = output.structural_file_references.references();
    assert_eq!(references.len(), 1);
    assert_eq!(
        references[0].class,
        PreparedFileReferenceClass::ResourceFile
    );
}
