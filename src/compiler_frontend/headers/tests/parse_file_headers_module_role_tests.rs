use super::*;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateDirectiveCode, InvalidTemplateDirectiveView, InvalidTemplateStructureCode,
    InvalidTemplateStructureView,
};
use crate::compiler_frontend::semantic_identity::ModuleRootFiles;

#[test]
fn api_only_normal_root_keeps_exportable_headers_and_rejects_runtime_body() {
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(
            "$export\nButton = | label String |\n~[ Button(\"ignored\") ]\n",
            "src/@mod.moth",
            "src/@entry.moth",
        ),
        "an API-only normal root must reject its top-level runtime body",
    );
    assert_eq!(errors.diagnostics.len(), 1);
    assert_eq!(
        errors
            .diagnostics
            .first()
            .expect("expected one diagnostic")
            .descriptor()
            .code,
        "MOTH-RULE-0023"
    );

    let headers = expect_header_success(
        parse_single_file_headers_with_entry(
            "$export\nButton = | label String |\n",
            "src/@mod.moth",
            "src/@entry.moth",
        ),
        "API-only normal roots should parse their declaration surface",
    );
    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        0,
        "API-only roots must not produce an entry header"
    );
    assert_eq!(headers.entry_runtime_fragment_count, 0);
    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));

    // API-only roots have no start-body AST consumer, so preparation must diagnose an
    // unclosed runtime template while rejecting the body, including inside a collection.
    for file_path in ["src/@mod.moth", "src/@entry.moth"] {
        for source in ["~[: broken", "values = {\n~[: broken"] {
            let errors = expect_header_error(
                parse_single_file_headers_with_entry(source, file_path, "src/@entry.moth"),
                &format!(
                    "unterminated runtime template must fail preparation: {file_path}: {source:?}"
                ),
            );
            assert_eq!(
                errors.diagnostics.len(),
                1,
                "expected one delimiter diagnostic for {file_path}: {source:?}"
            );
            let diagnostic = errors
                .diagnostics
                .first()
                .expect("expected one delimiter diagnostic");
            assert_eq!(
                diagnostic.descriptor().code,
                "MOTH-SYNTAX-0022",
                "the missing template delimiter keeps its stable diagnostic code"
            );
            let facts = diagnostic
                .family_view::<InvalidTemplateStructureView>()
                .expect("expected typed template-structure facts for the missing delimiter");
            assert_eq!(
                facts.reason,
                InvalidTemplateStructureCode::MissingClosingBracket
            );
            assert_eq!(
                diagnostic_primary_span(diagnostic),
                Some(source_span_for(source, "", source.len())),
                "the missing template delimiter must point at EOF for {file_path}: {source:?}"
            );
            assert_eq!(
                diagnostic_label_span(diagnostic, 0),
                Some(source_span_for(source, "[", 0)),
                "the missing delimiter must label the opening bracket for {file_path}: {source:?}"
            );
        }
    }
}

#[test]
fn api_only_normal_root_rejects_const_root_fragments() {
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(
            "[html.head: [\"ignored\"]]\n",
            "src/@mod.moth",
            "src/@entry.moth",
        ),
        "API-only roots must reject top-level const fragments",
    );

    assert_eq!(errors.diagnostics.len(), 1);
    assert_eq!(
        errors
            .diagnostics
            .first()
            .expect("expected one diagnostic")
            .descriptor()
            .code,
        "MOTH-RULE-0023"
    );
}

#[test]
fn reached_root_keeps_export_surface_without_start() {
    let headers = expect_header_success(
        parse_single_file_headers_with_reached_role(
            "$export\nButton = | label String |\n",
            "src/widgets/@mod.moth",
            FileRole::ReachedApiFacadeRoot(ModuleRootRole::Normal),
        ),
        "a reached facade should keep its export surface",
    );

    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        0,
        "a reached facade must not produce an entry header"
    );
    assert_eq!(headers.entry_runtime_fragment_count, 0);
    assert!(
        headers.headers.iter().any(|header| {
            matches!(header.kind, HeaderKind::Struct { .. })
                && header.export_mode == HeaderExportMode::Public
        }),
        "a reached facade still parses its export surface"
    );
}

#[test]
fn reached_root_rejects_direct_runtime_statement() {
    let errors = expect_header_error(
        parse_single_file_headers_with_reached_role(
            "value = 1\n",
            "src/widgets/@mod.moth",
            FileRole::ReachedApiFacadeRoot(ModuleRootRole::Normal),
        ),
        "a reached facade must reject its top-level runtime body",
    );

    assert_eq!(errors.diagnostics.len(), 1);
    assert_eq!(
        errors
            .diagnostics
            .first()
            .expect("expected one diagnostic")
            .descriptor()
            .code,
        "MOTH-RULE-0023"
    );
}

#[test]
fn reached_root_rejects_runtime_fragments() {
    let errors = expect_header_error(
        parse_single_file_headers_with_reached_role(
            "~[: [\"discarded\"]]\n",
            "src/widgets/@mod.moth",
            FileRole::ReachedApiFacadeRoot(ModuleRootRole::Normal),
        ),
        "a reached facade must reject runtime fragments",
    );

    assert_eq!(errors.diagnostics.len(), 1);
    assert_eq!(
        errors
            .diagnostics
            .first()
            .expect("expected one diagnostic")
            .descriptor()
            .code,
        "MOTH-RULE-0023"
    );
}

#[test]
fn reached_root_rejects_const_fragments() {
    let errors = expect_header_error(
        parse_single_file_headers_with_reached_role(
            "[html.head: [\"discarded\"]]\n",
            "src/widgets/@mod.moth",
            FileRole::ReachedApiFacadeRoot(ModuleRootRole::Normal),
        ),
        "a reached facade must reject top-level const fragments",
    );

    assert_eq!(errors.diagnostics.len(), 1);
    assert_eq!(
        errors
            .diagnostics
            .first()
            .expect("expected one diagnostic")
            .descriptor()
            .code,
        "MOTH-RULE-0023"
    );
}

#[test]
fn entry_root_synthesizes_start_without_export_surface() {
    let headers = expect_header_success(
        parse_single_file_headers_with_options(
            "greeting #= \"hello\"\n~[greeting]\n",
            Path::new("src/@entry.moth"),
            entry_root_options(),
        ),
        "an entry root should synthesize its start body",
    );

    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        1,
        "an entry root owns exactly one start function"
    );
    assert_eq!(headers.entry_runtime_fragment_count, 1);
    assert!(
        headers.headers.iter().all(|header| {
            !matches!(header.kind, HeaderKind::StartFunction)
                || header.export_mode == HeaderExportMode::Private
        }),
        "the synthesized start is never a public surface declaration"
    );
}

#[test]
fn empty_entry_root_keeps_one_start_and_no_fragments() {
    let headers = expect_header_success(
        parse_single_file_headers_with_options(
            "",
            Path::new("src/@entry.moth"),
            entry_root_options(),
        ),
        "an empty entry root is still a valid entry declaration",
    );

    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        1,
        "an empty entry root still owns exactly one start function"
    );
    assert_eq!(headers.entry_runtime_fragment_count, 0);
    assert_eq!(headers.const_fragment_count, 0);
}

#[test]
fn metadata_only_entry_root_keeps_start_and_const_fragment() {
    let headers = expect_header_success(
        parse_single_file_headers_with_options(
            "[$in title: \"metadata only\"]\n",
            Path::new("src/@entry.moth"),
            entry_root_options(),
        ),
        "a metadata-only entry root is a valid const-fragment entry",
    );

    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        1,
        "a metadata-only entry root still owns exactly one start function"
    );
    assert_eq!(
        headers.const_fragment_count, 1,
        "the direct builder slot is the entry's only const fragment"
    );
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "metadata alone must not fabricate runtime body facts"
    );
}

#[test]
fn entry_root_rejects_export_directive() {
    let errors = expect_header_error(
        parse_single_file_headers_with_options(
            "$export\ntheme #= \"dark\"\n",
            Path::new("src/@entry.moth"),
            entry_root_options(),
        ),
        "entry roots must reject export surfaces",
    );

    let diagnostic = errors
        .diagnostics
        .first()
        .expect("entry export rejection holds one diagnostic");
    let view = diagnostic
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("entry export rejection keeps its typed reason");
    assert_eq!(
        view.reason,
        InvalidTemplateDirectiveCode::DirectiveNotAllowedHere,
        "an entry root rejects the source directive at its own span"
    );
}

/// Parse and bind a multi-file fixture whose root-local read must be rejected.
fn expect_single_root_local_diagnostic(
    sources: &[(String, String)],
) -> CompactDiagnosticBagForTest {
    let (result, _warnings, _string_table) =
        parse_multi_file_headers_with_result(sources, "src/@entry.moth");
    let bag = match result {
        Ok(_) => panic!("a cross-file root-local read must be rejected"),
        Err(bag) => bag,
    };
    let diagnostics = compact_bag_for_test(bag, None);
    assert_eq!(
        diagnostics.len(),
        1,
        "root-local isolation reports exactly one diagnostic"
    );
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0030",
        "the failure keeps the dedicated root-local import code"
    );
    diagnostics
}

#[test]
fn helper_file_cannot_resolve_a_normal_root_local_declaration() {
    let sources = [
        ("theme #= \"dark\"\n".to_owned(), "src/@mod.moth".to_owned()),
        (
            "@\"@mod\" theme\n\nuse_theme || -> String:\n    return theme\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the helper read keeps the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::NormalFacade,
        "the diagnostic names the normal facade that authored the declaration"
    );
    assert!(
        diagnostic_label_span(diagnostic, 0).is_some(),
        "the authored declaration span is the related root-local site"
    );
}

#[test]
fn normal_root_cannot_resolve_a_support_root_local_declaration() {
    let sources = [
        (
            "@\"@package\" support_value\n\nnormal_value #= support_value\n".to_owned(),
            "src/@mod.moth".to_owned(),
        ),
        (
            "support_value #= \"support\"\n".to_owned(),
            "src/@package.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the sibling-root read keeps the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::SupportFacade,
        "the diagnostic names the support facade that authored the declaration"
    );
}

#[test]
fn entry_root_cannot_resolve_a_normal_root_local_declaration() {
    let sources = [
        ("theme #= \"dark\"\n".to_owned(), "src/@mod.moth".to_owned()),
        (
            "@\"@mod\" theme\n\n~[: [theme]]\n".to_owned(),
            "src/@entry.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the entry reads reject through the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::NormalFacade,
        "the entry root names the normal facade it tried to read"
    );
}

#[test]
fn helper_file_cannot_resolve_an_entry_root_local_declaration() {
    let sources = [
        (
            "greeting #= \"hello\"\n~[greeting]\n".to_owned(),
            "src/@entry.moth".to_owned(),
        ),
        (
            "@\"@entry\" greeting\ngreeting_copy #= greeting\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the helper read keeps the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::EntryRoot,
        "the diagnostic names the entry root that authored the declaration"
    );
    assert!(
        diagnostic_label_span(diagnostic, 0).is_some(),
        "the authored entry declaration span is the related root-local site"
    );
}

#[test]
fn helper_cannot_bind_an_entry_root_namespace() {
    let sources = [
        (
            "greeting #= \"hello\"\n~[greeting]\n".to_owned(),
            "src/@entry.moth".to_owned(),
        ),
        (
            "@\"@entry\" as entry_root\nhelpers_copy #= 1\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the namespace bind keeps the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::EntryRoot,
        "the namespace gate names the entry root it tried to bind"
    );
}

#[test]
fn normal_root_export_cannot_reexport_an_entry_root_declaration() {
    let sources = [
        (
            "greeting #= \"hello\"\n~[greeting]\n".to_owned(),
            "src/@entry.moth".to_owned(),
        ),
        (
            "$export\n@\"@entry\" greeting\n".to_owned(),
            "src/@mod.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the re-export gate keeps the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::EntryRoot,
        "the exporting normal root names the entry root it tried to re-export"
    );
}

#[test]
fn normal_root_export_cannot_reexport_a_support_facade_declaration() {
    let sources = [
        (
            "support_value #= \"support\"\n".to_owned(),
            "src/@package.moth".to_owned(),
        ),
        (
            "$export\n@\"@package\" support_value\n".to_owned(),
            "src/@mod.moth".to_owned(),
        ),
    ];

    let diagnostics = expect_single_root_local_diagnostic(&sources);
    let diagnostic = diagnostics.first().expect("the bag holds its diagnostic");
    let view = diagnostic
        .family_view::<imports::RootLocalDeclarationNotVisibleView>()
        .expect("the re-export gate keeps the root-local family");
    assert_eq!(
        view.root_role,
        crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole::SupportFacade,
        "the exporting normal root names the support facade it tried to re-export"
    );
}

#[test]
fn one_helper_declaration_binds_from_all_three_root_roles() {
    let sources = [
        (
            "shared #= \"s\"\n\nhelper || -> String:\n    return shared\n;\n".to_owned(),
            "src/shared.moth".to_owned(),
        ),
        (
            "@shared helper\n\nnormal_value #= 1\n".to_owned(),
            "src/@mod.moth".to_owned(),
        ),
        (
            "@shared helper\n\nsupport_value #= 2\n".to_owned(),
            "src/@package.moth".to_owned(),
        ),
        (
            "@shared helper\n\n~[: [\"ok\"]]\n".to_owned(),
            "src/@entry.moth".to_owned(),
        ),
    ];

    let (result, _warnings, mut string_table, mut path_fork) =
        parse_multi_file_headers_with_path(&sources, "src/@entry.moth");
    let headers = result.expect("ordinary helper declarations stay bindable from every root role");
    let helper_name = string_table.intern("helper");

    for root_file in ["src/@mod.moth", "src/@package.moth", "src/@entry.moth"] {
        let root_path = path_fork
            .try_intern_filesystem_path(&PathBuf::from(root_file), &mut string_table)
            .expect("fixture root path is UTF-8");
        let visibility = headers
            .binding_environment
            .visibility_for(&root_path)
            .expect("every root owns a visibility map");
        assert!(
            visibility.visible_source_names.contains_key(&helper_name),
            "{root_file} must bind the shared helper declaration"
        );
    }
}

#[test]
fn same_spelled_root_local_declarations_in_two_roots_stay_independent() {
    let sources = [
        ("theme #= \"dark\"\n".to_owned(), "src/@mod.moth".to_owned()),
        (
            "theme #= \"light\"\n".to_owned(),
            "src/@package.moth".to_owned(),
        ),
    ];

    let (result, _warnings, string_table, path_fork) =
        parse_multi_file_headers_with_path(&sources, "src/@entry.moth");
    let headers = result.expect("disjoint root-local scopes accept equal spellings");

    let theme_paths: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| {
            matches!(header.kind, HeaderKind::Constant { .. })
                && path_fork
                    .component(header.declaration_path)
                    .is_some_and(|name| string_table.resolve(name) == "theme")
        })
        .map(|header| header.declaration_path)
        .collect();

    assert_eq!(
        theme_paths.len(),
        2,
        "each root keeps its own same-spelled declaration"
    );
    assert_ne!(
        theme_paths[0], theme_paths[1],
        "root-local declarations keep distinct authored identities"
    );
}

#[test]
fn normal_root_colocated_with_entry_root_is_api_only() {
    let errors = expect_header_error(
        parse_single_file_headers_with_options(
            "~[greeting]\n",
            Path::new("src/@mod.moth"),
            normal_root_colocated_with_entry_options(),
        ),
        "a normal root co-located with an entry root must reject top-level runtime",
    );

    assert_eq!(errors.diagnostics.len(), 1);
    assert_eq!(
        errors
            .diagnostics
            .first()
            .expect("expected one diagnostic")
            .descriptor()
            .code,
        "MOTH-RULE-0023"
    );
}

fn entry_root_options() -> HeaderParseOptions<'static> {
    HeaderParseOptions {
        root_file_ids: ModuleRootFiles {
            entry: Some(SourceId::COMPILATION_ROOT),
            ..Default::default()
        },
        selected_file: None,
        reached_root_roles: &[],
    }
}

fn normal_root_colocated_with_entry_options() -> HeaderParseOptions<'static> {
    HeaderParseOptions {
        root_file_ids: ModuleRootFiles {
            normal: Some(SourceId::COMPILATION_ROOT),
            entry: Some(SourceId::from_index(1)),
            ..Default::default()
        },
        selected_file: None,
        reached_root_roles: &[],
    }
}

#[test]
fn typed_constant_retains_local_ordering_hint_for_declared_type() {
    // WHY: the declared type creates a structural ordering constraint so that the type
    // is sorted before any constant that references it. Initializer-expression references
    // are collected later during binding; this check owns the declared type annotation.
    let (headers, _string_table) =
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
            .any(|dep| dep.path() != PathId::ROOT),
        "local ordering hint must reference the declared type name"
    );
}

#[test]
fn struct_fields_retain_local_ordering_hints_for_named_field_types() {
    // WHY: struct fields whose types are user-defined names retain conservative hints that Stage 3
    // resolves so the named type is sorted before the struct that depends on it.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/@entry.moth");
    // WHY: `parse_single_file_headers_with_table` discards the `PathInternerFork` that owns the
    // retained `PathId`s, so this fixture prepares through the fork-owning helper instead. The
    // fixture selects the file as its own legacy combined root, whose headers keep the root
    // `PathId`, so struct identity must be resolved structurally rather than by comparing against
    // `PathId::ROOT`.
    let (output, _span_builder) = prepare_single_file_with_fork(
        "Point = |x I32, y I32|\nSpan = |start Point, end Point|\n",
        &file_path,
        &file_path,
        &mut string_table,
        &mut path_fork,
    );

    // Select the Span struct by its authored field-type spelling, and the Point struct by its
    // declaration-name component, so both identities come from the same fork's path table.
    let span_header = output
        .headers
        .iter()
        .find(|header| {
            let HeaderKind::Struct { fields, .. } = &header.kind else {
                return false;
            };
            fields.iter().any(|field| {
                matches!(&field.type_annotation, ParsedTypeRef::Named { name, .. }
                    if string_table.resolve(*name) == "Point")
            })
        })
        .expect("expected Span struct header");
    let point_path = output
        .headers
        .iter()
        .find_map(|header| {
            if !matches!(header.kind, HeaderKind::Struct { .. }) {
                return None;
            }
            let name = path_fork.component(header.declaration_path)?;
            (string_table.resolve(name) == "Point").then_some(header.declaration_path)
        })
        .expect("expected Point struct header");

    assert!(
        span_header.local_ordering_hints.iter().any(|dep| {
            dep.origin() == LocalDeclarationOrderingHintOrigin::SourceOwned
                && dep.path() == point_path
        }),
        "Span must retain a same-file ordering hint targeting the Point struct declaration path"
    );
}

#[test]
fn function_error_return_retains_local_ordering_hint_for_named_type() {
    // WHY: final `T!` error slots are part of the declaration surface. Their named types must
    // participate in local declaration ordering before AST resolves function signatures.
    let (headers, _string_table) = parse_single_file_headers_with_table(
        "AppError = |message String|\nparse || -> I32, AppError!:\n    return 1\n;\n",
    );

    let parse_header = headers
        .headers
        .iter()
        .find(|header| {
            matches!(header.kind, HeaderKind::Function { .. })
                && header.declaration_path != PathId::ROOT
        })
        .expect("expected parse function header");

    assert!(
        parse_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path() != PathId::ROOT),
        "function error return slot must retain a local ordering hint"
    );
}

#[test]
fn constant_header_with_declared_type_captures_type_in_declaration() {
    // Confirms the header-stage contract: declared type annotation is present in the
    // Constant header's declaration, proving initializer resolution is deferred to AST.
    let headers = parse_single_file_headers("threshold #I32 = 42\n");

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
