//! Unit tests for `HeaderStats` aggregation.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::parse_single_file_headers;
use crate::compiler_frontend::headers::parse_file_headers::parse_file_headers_tests::prepare_single_file;
use crate::compiler_frontend::headers::parse_file_headers::{
    bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
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
    let entry_path = PathBuf::from("src/#page.moth");
    let helper_path = PathBuf::from("src/helper.moth");

    let entry_output =
        prepare_single_file("[runtime1]\n", &entry_path, &entry_path, &mut string_table);
    let helper_output = prepare_single_file(
        "helper_func || -> Int:\n    return 1\n;\n",
        &helper_path,
        &entry_path,
        &mut string_table,
    );

    let prepared_syntax =
        prepare_header_syntax(vec![entry_output, helper_output], &mut string_table)
            .expect("header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    )
    .expect("headers should bind");

    assert_eq!(headers.header_stats.functions, 1);
    assert_eq!(headers.header_stats.start_functions, 1);
}
