//! Unit tests for `HeaderStats` aggregation.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::parse_single_file_headers;
use crate::compiler_frontend::headers::parse_file_headers::{
    HeaderParseOptions, bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceDatabase};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::path::PathBuf;

#[test]
fn single_function_header_counts_function() {
    let headers = parse_single_file_headers("identity |value Int| -> Int:\n    return value\n;\n");

    assert_eq!(headers.header_stats.functions, 1);
    assert_eq!(headers.header_stats.structs, 0);
    assert_eq!(headers.header_stats.choices, 0);
}

#[test]
fn struct_and_choice_count_members() {
    let source = r#"
Point = | x Int, y Int |
Color :: Red, Green, Blue, ;
"#;
    let headers = parse_single_file_headers(source);

    assert_eq!(headers.header_stats.structs, 1);
    assert_eq!(headers.header_stats.signature_members, 2);
    assert_eq!(headers.header_stats.choices, 1);
    assert_eq!(headers.header_stats.choice_variants, 3);
}

#[test]
fn trait_requirements_count_signature_members() {
    let source = r#"
DISPLAYABLE must:
    display |This, prefix String| -> String
    reset |~This|
;
"#;
    let headers = parse_single_file_headers(source);

    assert_eq!(headers.header_stats.traits, 1);
    assert_eq!(headers.header_stats.signature_members, 4);
}

#[test]
fn multi_file_declarations_are_aggregated() {
    let mut string_table = StringTable::new();
    let entry_path = PathBuf::from("src/@page.moth");
    let helper_path = PathBuf::from("src/helper.moth");

    let source_files = SourceDatabase::build(
        [&entry_path, &helper_path],
        &entry_path,
        None,
        &mut string_table,
    )
    .expect("fixture source identities should build");
    let entry_id = source_files
        .get_by_canonical_path(&entry_path)
        .expect("entry source identity")
        .id;
    let helper_id = source_files
        .get_by_canonical_path(&helper_path)
        .expect("helper source identity")
        .id;
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut prepare_file = |source: &str, path: &PathBuf, source_id| {
        let interned_path = InternedPath::try_from_filesystem_path(path, &mut string_table)
            .expect("test path should be UTF-8");
        let mut span_builder = ExtendedSpanBuilder::new();
        let tokens = tokenize(
            source,
            &interned_path,
            TokenizerEntryMode::SourceFile,
            &style_directives,
            &mut string_table,
            source_id,
            &mut span_builder,
        )
        .expect("source should tokenize");
        let output = prepare_file_from_tokens(
            tokens,
            &entry_path,
            &HeaderParseOptions::default(),
            &mut string_table,
            0,
            0,
            &mut span_builder,
        )
        .expect("source should prepare");
        (output, span_builder)
    };
    let (entry_output, entry_span_builder) = prepare_file("[runtime1]\n", &entry_path, entry_id);
    let (helper_output, helper_span_builder) = prepare_file(
        "helper_func || -> Int:\n    return 1\n;\n",
        &helper_path,
        helper_id,
    );

    let mut retained_span_builders = [
        (entry_output.file_id, entry_span_builder),
        (helper_output.file_id, helper_span_builder),
    ];
    let prepared_syntax = prepare_header_syntax(
        &mut [entry_output, helper_output],
        &mut string_table,
        &mut |source, diagnostic| {
            let (_, builder) = retained_span_builders
                .iter_mut()
                .find(|(file_id, _)| *file_id == source)
                .expect("prepared source retains its original span builder");
            diagnostic.capture_preparation_span(source, builder)
        },
    )
    .expect("header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        None,
        &source_files,
        &mut string_table,
    )
    .expect("headers should bind");

    assert_eq!(headers.header_stats.functions, 1);
    assert_eq!(headers.header_stats.start_functions, 1);
}
