use super::*;

// ------------------------------
//  Export modifier parsing tests
// ------------------------------

#[test]
fn unmarked_authored_declarations_in_module_root_remain_private() {
    let source = "Button = | label String |\nrender |button Button| -> String:\n    return button.label\n;\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "headers should parse",
    );

    let non_start_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .collect();

    assert!(
        non_start_headers
            .iter()
            .all(|header| header.export_mode == HeaderExportMode::Private),
        "unmarked declarations in a module root should remain private"
    );
}

#[test]
fn duplicate_declaration_detection_works_with_exported_declarations() {
    let result = parse_single_file_headers_with_entry(
        "$export\nButton = | label String |\nButton = | title String |\n",
        "src/@mod.moth",
        "src/@entry.moth",
    );
    let errors = expect_header_error(
        result,
        "duplicate declaration with export should still be rejected",
    );

    assert!(errors.diagnostics.views().any(|diagnostic| {
        diagnostic
            .family_view::<names_declarations::DuplicateDeclarationView>()
            .is_some()
    }));
}

#[test]
fn public_dependency_and_private_dependency_keep_distinct_retained_shells() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        "@button Button\n$export\n@button Button\n",
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("an API-only normal root should accept its `$export` declarations");

    assert_eq!(
        output.file_dependency_clauses.len(),
        2,
        "the private dependency and the public re-export each retain their own authored clause"
    );
    assert_eq!(
        output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Private
    );
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .ordinal,
        0
    );
    assert_eq!(
        output.file_dependency_clauses[1].export_mode,
        HeaderExportMode::Public
    );
    assert_eq!(
        output.file_dependency_clauses[1]
            .dependency
            .dependency_shell_id
            .ordinal,
        1
    );
}
#[test]
fn capacity_references_extract_value_refs_without_treating_element_type_as_value_ref() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/test.moth");
    let source = "make |items ~{capacity MyType}| -> I32:\n    return 1\n;\n";
    let (output, mut span_builder) = prepare_single_file_with_fork(
        source,
        &file_path,
        &file_path,
        &mut string_table,
        &mut path_fork,
    );

    let headers = prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
        &mut path_fork,
    )
    .expect("headers should parse");

    let make_header = headers
        .headers
        .iter()
        .find(|h| {
            matches!(h.kind, HeaderKind::Function { .. }) && h.declaration_path != PathId::ROOT
        })
        .expect("make header should exist");

    let capacity_names: Vec<_> = make_header
        .capacity_references
        .iter()
        .map(|r| string_table.resolve(r.name))
        .collect();

    assert!(
        capacity_names.contains(&"capacity"),
        "bare capacity syntax should reference the capacity constant"
    );
    assert!(
        !capacity_names.contains(&"MyType"),
        "element type name must not be treated as a capacity value reference"
    );
}
