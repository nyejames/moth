use super::*;

#[test]
fn imported_module_root_discards_root_body_but_keeps_exportable_headers() {
    let headers = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\n[ Button(\"ignored\") ]\n",
        "src/@components.moth",
        "src/@page.moth",
    )
    .expect("imported module roots should parse their declaration surface");

    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        0,
        "imported roots must not produce an implicit start header"
    );
    assert_eq!(headers.entry_runtime_fragment_count, 0);
    assert!(!headers.has_non_trivial_root_body);
    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

#[test]
fn imported_module_root_discards_const_root_fragments() {
    let headers = parse_single_file_headers_with_entry(
        "#[html.head: [\"ignored\"]]\n",
        "src/@components.moth",
        "src/@page.moth",
    )
    .expect("imported roots should skip const root fragments");

    assert!(headers.top_level_const_fragments.is_empty());
    assert!(
        headers
            .headers
            .iter()
            .all(|header| !matches!(header.kind, HeaderKind::ConstTemplate { .. }))
    );
}

#[test]
fn typed_constant_retains_local_ordering_hint_for_declared_type() {
    // WHY: the declared type creates a structural ordering constraint so that the type
    // is sorted before any constant that references it. Initializer-expression references
    // are collected later during binding; this check owns the declared type annotation.
    let (headers, string_table) =
        parse_single_file_headers_with_table("struct NavBar {}\ntheme #NavBar = default_navbar\n");

    let constant_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("expected constant header");

    assert!(
        !constant_header.local_ordering_hints.is_empty(),
        "typed constant must retain a local ordering hint for its declared type"
    );
    assert!(
        constant_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path().name_str(&string_table) == Some("NavBar")),
        "local ordering hint must reference the declared type name 'NavBar'"
    );
}

#[test]
fn struct_fields_retain_local_ordering_hints_for_named_field_types() {
    // WHY: struct fields whose types are user-defined names retain conservative hints that Stage 3
    // resolves so the named type is sorted before the struct that depends on it.
    let (headers, string_table) = parse_single_file_headers_with_table(
        "Point = |x Int, y Int|\nSpan = |start Point, end Point|\n",
    );

    let span_header = headers
        .headers
        .iter()
        .find(|header| {
            matches!(header.kind, HeaderKind::Struct { .. })
                && header.tokens.src_path.name_str(&string_table) == Some("Span")
        })
        .expect("expected Span struct header");

    assert!(
        span_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path().name_str(&string_table) == Some("Point")),
        "Span must retain a local ordering hint for Point via its field type annotations"
    );
}

#[test]
fn function_error_return_retains_local_ordering_hint_for_named_type() {
    // WHY: final `T!` error slots are part of the declaration surface. Their named types must
    // participate in local declaration ordering before AST resolves function signatures.
    let (headers, string_table) = parse_single_file_headers_with_table(
        "AppError = |message String|\nparse || -> Int, AppError!:\n    return 1\n;\n",
    );

    let parse_header = headers
        .headers
        .iter()
        .find(|header| {
            matches!(header.kind, HeaderKind::Function { .. })
                && header.tokens.src_path.name_str(&string_table) == Some("parse")
        })
        .expect("expected parse function header");

    assert!(
        parse_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path().name_str(&string_table) == Some("AppError")),
        "function error return slot must retain a local ordering hint for AppError"
    );
}

#[test]
fn constant_header_with_declared_type_captures_type_in_declaration() {
    // Confirms the header-stage contract: declared type annotation is present in the
    // Constant header's declaration, proving initializer resolution is deferred to AST.
    let headers = parse_single_file_headers("threshold #Int = 42\n");

    let constant_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("expected constant header");

    let HeaderKind::Constant { declaration, .. } = &constant_header.kind else {
        panic!("expected Constant header kind");
    };

    assert!(
        !matches!(declaration.type_annotation, ParsedTypeRef::Inferred),
        "declared type annotation on a typed constant must be resolved at the header stage, not left as Inferred"
    );
}
