//! Dependency sorting regression tests.
//!
//! WHAT: validates topological ordering, cycle detection, deterministic order, and start-function
//!       exclusion from the import dependency graph.
//! WHY: dependency sorting is the single producer of sorted declaration placeholders; any drift
//!      here breaks cross-file visibility and AST constant dependency ordering.

use super::*;
use crate::builder_surface::SourceFileKind;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::module_symbols::{PublicExportEntry, PublicExportTarget};
use crate::compiler_frontend::headers::moth_template_prepare::prepare_moth_template_file;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, HeaderKind, HeaderParseOptions, LocalDeclarationOrderingHint,
    bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::headers::plain_markdown_prepare::{
    PlainMarkdownPrepareInput, prepare_plain_markdown_file,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::path::PathBuf;

fn parse_module_headers(
    files: &[(&str, &str)],
    entry_path: &str,
) -> (BoundModuleHeaders, StringTable) {
    let mut string_table = StringTable::new();
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let entry_path_buf = PathBuf::from(entry_path);

    let mut prepared_outputs = Vec::with_capacity(files.len());
    let mut const_template_offset = 0usize;
    let mut runtime_fragment_offset = 0usize;

    for (path, source) in files {
        let path_buf = PathBuf::from(path);
        let interned_path = InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
            .expect("test path should be UTF-8");
        let file_tokens = tokenize(
            source,
            &interned_path,
            TokenizerEntryMode::SourceFile,
            &style_directives,
            &mut string_table,
            Some(FileId(0)),
        )
        .expect("tokenization should succeed");

        let output = prepare_file_from_tokens(
            file_tokens,
            &entry_path_buf,
            &options,
            &mut string_table,
            const_template_offset,
            runtime_fragment_offset,
        )
        .expect("preparation should succeed");

        const_template_offset += output.const_template_count;
        runtime_fragment_offset += output.runtime_fragment_count;
        prepared_outputs.push(output);
    }

    let prepared_syntax = prepare_header_syntax(prepared_outputs, &mut string_table)
        .expect("header syntax preparation should succeed");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        options.project_path_resolver.as_ref(),
        &mut string_table,
    )
    .expect("header binding should succeed");

    (headers, string_table)
}

fn header_name(
    header: &crate::compiler_frontend::headers::parse_file_headers::Header,
    string_table: &StringTable,
) -> String {
    header
        .tokens
        .src_path
        .name_str(string_table)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn sorts_strict_top_level_dependencies_before_dependents_and_appends_start_last() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            ("src/a.moth", "@b Middle\nTop #Middle = Middle\n"),
            ("src/b.moth", "@c Thing\nMiddle #Thing = Thing\n"),
            ("src/c.moth", "Thing #Int = 1\n"),
        ],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("dependency sort should pass");

    let non_start_order = sorted
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header_name(header, &string_table))
        .collect::<Vec<_>>();

    assert_eq!(non_start_order, vec!["Thing", "Middle", "Top"]);
    assert!(
        matches!(
            sorted.headers.last().map(|header| &header.kind),
            Some(HeaderKind::StartFunction)
        ),
        "entry start header must be appended after sorted top-level declarations"
    );

    let start_order = sorted
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header.source_file.to_portable_string(&string_table))
        .collect::<Vec<_>>();

    assert_eq!(start_order, vec!["src/a.moth"]);
}

#[test]
fn dependency_sort_preserves_root_activity_metadata() {
    let (headers, mut string_table) =
        parse_module_headers(&[("src/a.moth", "#[static]\n[runtime]\n")], "src/a.moth");

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("dependency sorting should preserve root activity metadata");

    assert!(sorted.has_non_trivial_root_body);
    assert_eq!(sorted.const_fragment_count, 1);
    assert_eq!(sorted.entry_runtime_fragment_count, 1);
}

#[test]
fn reports_circular_dependencies() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            ("src/a.moth", "@b Middle\nTop #Middle = Middle\n"),
            ("src/b.moth", "@a Top\nMiddle #Top = Top\n"),
        ],
        "src/a.moth",
    );

    let bag =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect_err("cycle should fail dependency sorting");

    let cycle_diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            let DiagnosticPayload::CircularDependency { path } = &diagnostic.payload else {
                return false;
            };

            let path = path.to_portable_string(&string_table);
            path.contains("Top") || path.contains("Middle")
        })
        .unwrap_or_else(|| panic!("expected a cycle diagnostic, got: {bag:?}"));

    assert!(
        cycle_diagnostic
            .primary_location
            .scope
            .to_portable_string(&string_table)
            .contains("src/"),
        "cycle diagnostics should point at a declaration location instead of the default location"
    );
}

#[test]
fn constant_initializer_creates_dependency_sort_edge() {
    // WHY: header-stage constant_dependencies.rs now extracts initializer reference edges.
    // Constant initializers that reference other constants create top-level dependency edges
    // that dependency sorting respects.
    let (headers, mut string_table) = parse_module_headers(
        &[
            // Config's initializer references Value.
            // That reference creates a dependency edge from Config to Value.
            ("src/a.moth", "@b Value\nConfig #= Value\n"),
            ("src/b.moth", "Value #Int = 42\n"),
        ],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed — constant initializer edges are resolved by headers");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|h| !matches!(h.kind, HeaderKind::StartFunction))
        .map(|h| header_name(h, &string_table))
        .collect();

    // Both headers must be present and Value must precede Config.
    assert_eq!(
        non_start_names,
        vec!["Value", "Config"],
        "constant initializer dependency must order Value before Config"
    );
}

#[test]
fn same_file_backward_constant_reference_is_accepted() {
    // WHY: a constant that references an earlier constant in the same file is a backward
    // reference and must be accepted in source order.
    let (headers, mut string_table) = parse_module_headers(
        &[("src/a.moth", "Value #Int = 42\nConfig #= Value\n")],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("same-file backward constant reference should be accepted");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|h| !matches!(h.kind, HeaderKind::StartFunction))
        .map(|h| header_name(h, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["Value", "Config"],
        "same-file backward constant reference should preserve source order"
    );
}

#[test]
fn function_body_references_do_not_influence_header_provided_sort_order() {
    // WHY: function body references are AST/body-phase concerns, not
    // header-provided top-level dependency edges. Sorting should preserve source order
    // for otherwise-independent declarations.
    let (headers, mut string_table) = parse_module_headers(
        &[(
            "src/a.moth",
            "first|| -> Int:\n    return second()\n;\n\nsecond|| -> Int:\n    return 1\n;\n",
        )],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("dependency sort should ignore body-only references");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header_name(header, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["first", "second"],
        "function body call graph must not perturb strict header sorting"
    );
}

#[test]
fn function_error_return_dependency_orders_error_type_before_function() {
    // WHY: `T!` is signature metadata, not a body reference. Header dependency sorting must
    // order imported error payload declarations before functions that expose them.
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/app.moth",
                "@errors AppError\nparse|| -> Int, AppError!:\n    return 1\n;\n",
            ),
            ("src/errors.moth", "AppError = |message String|\n"),
        ],
        "src/app.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("error return dependency should be sortable");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header_name(header, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["AppError", "parse"],
        "error payload type must be sorted before the fallible function signature"
    );
}

#[test]
fn capacity_reference_in_collection_type_orders_constant_before_user() {
    // WHY: bare capacity constants in fixed collection types create value-namespace
    // dependency edges to the referenced constant, even when the declaration is not a constant.
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/a.moth",
                "@b capacity\nmake |items ~{capacity Int}| -> Int:\n    return 1\n;\n",
            ),
            ("src/b.moth", "capacity #Int = 64\n"),
        ],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed — capacity reference edges are resolved by headers");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|h| !matches!(h.kind, HeaderKind::StartFunction))
        .map(|h| header_name(h, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["capacity", "make"],
        "capacity constant must be sorted before the declaration that uses it"
    );
}

#[test]
fn capacity_reference_same_file_forward_reference_is_rejected() {
    // WHY: a capacity constant declared after a typed declaration in the same file is a
    // same-file forward constant reference and must be rejected.
    let mut string_table = StringTable::new();
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let entry_path = PathBuf::from("src/a.moth");
    let file_path = PathBuf::from("src/a.moth");
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        "make |items ~{capacity Int}| -> Int:\n    return 1\n;\ncapacity #Int = 64\n",
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(FileId(0)),
    )
    .expect("tokenization should succeed");

    let output =
        prepare_file_from_tokens(file_tokens, &entry_path, &options, &mut string_table, 0, 0)
            .expect("preparation should succeed");

    let prepared_syntax = prepare_header_syntax(vec![output], &mut string_table)
        .expect("header syntax preparation should succeed");
    let result = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        options.project_path_resolver.as_ref(),
        &mut string_table,
    );

    let bag = match result {
        Err(bag) => bag,
        Ok(_) => panic!("same-file forward capacity reference should fail during header parsing"),
    };

    let found = bag.diagnostics().iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError {
                reason: CompileTimeEvaluationErrorReason::SameFileForwardConstantReference,
                ..
            }
        )
    });

    assert!(
        found,
        "expected a same-file forward constant reference diagnostic"
    );
}

#[test]
fn capacity_reference_in_function_signature_creates_dependency_edge() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/a.moth",
                "@b size\nmake |items ~{size Int}| -> Int:\n    return 1\n;\n",
            ),
            ("src/b.moth", "size #Int = 8\n"),
        ],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|h| !matches!(h.kind, HeaderKind::StartFunction))
        .map(|h| header_name(h, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["size", "make"],
        "capacity reference in function parameter must order constant before function"
    );
}

#[test]
fn capacity_reference_in_type_alias_creates_dependency_edge() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            ("src/a.moth", "@b limit\nItems as {limit Int}\n"),
            ("src/b.moth", "limit #Int = 16\n"),
        ],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|h| !matches!(h.kind, HeaderKind::StartFunction))
        .map(|h| header_name(h, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["limit", "Items"],
        "capacity reference in type alias must order constant before alias"
    );
}

#[test]
fn capacity_references_across_header_type_surfaces_create_dependency_edges() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/a.moth",
                "@b limit\n\
                 Buffer = |\n\
                     items {limit Int},\n\
                 |\n\
                 Status :: Pending |\n\
                     items {limit Int},\n\
                 |;\n\
                 make|| -> {limit Int}:\n\
                     return {}\n\
                 ;\n",
            ),
            ("src/b.moth", "limit #Int = 16\n"),
        ],
        "src/a.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|h| !matches!(h.kind, HeaderKind::StartFunction))
        .map(|h| header_name(h, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["limit", "Buffer", "Status", "make"],
        "capacity references in fields, payloads, and returns must order the constant first"
    );
}

#[test]
fn trait_requirement_type_dependencies_order_required_type_before_trait() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/traits.moth",
                "@types Message\n\
                 DISPLAYABLE must:\n\
                     display |This| -> Message\n\
                 ;\n",
            ),
            ("src/types.moth", "Message = | text String |\n"),
        ],
        "src/traits.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header_name(header, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["Message", "DISPLAYABLE"],
        "trait requirement signatures must order imported type surfaces before the trait"
    );
}

#[test]
fn trait_conformance_references_do_not_create_dependency_sort_edges() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/app.moth",
                "@traits DISPLAYABLE\n\
                 Label = | text String |\n\
                 Label must DISPLAYABLE\n",
            ),
            (
                "src/traits.moth",
                "DISPLAYABLE must:\n\
                     display |This| -> String\n\
                 ;\n",
            ),
        ],
        "src/app.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed");

    let conformance_position = sorted
        .headers
        .iter()
        .position(|header| matches!(header.kind, HeaderKind::TraitConformance { .. }))
        .expect("expected a conformance header");
    let trait_position = sorted
        .headers
        .iter()
        .position(|header| header_name(header, &string_table) == "DISPLAYABLE")
        .expect("expected imported trait header");

    assert!(
        conformance_position < trait_position,
        "conformance references are resolved by AST after trait definitions are registered, so \
         they intentionally do not add dependency-sort edges"
    );
}

#[test]
fn trait_incompatibility_references_do_not_create_dependency_sort_edges() {
    let (headers, mut string_table) = parse_module_headers(
        &[
            (
                "src/app.moth",
                "@traits SERIALIZABLE\n\
                 DISPLAYABLE must:\n\
                 ;\n\
                 DISPLAYABLE must not SERIALIZABLE\n",
            ),
            ("src/traits.moth", "SERIALIZABLE must:\n;\n"),
        ],
        "src/app.moth",
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("sort must succeed");

    let incompatibility_position = sorted
        .headers
        .iter()
        .position(|header| matches!(header.kind, HeaderKind::TraitIncompatibility { .. }))
        .expect("expected an incompatibility header");
    let imported_trait_position = sorted
        .headers
        .iter()
        .position(|header| header_name(header, &string_table) == "SERIALIZABLE")
        .expect("expected imported trait header");

    assert!(
        incompatibility_position < imported_trait_position,
        "trait incompatibility references are resolved by AST after trait definitions are \
         registered, so they intentionally do not add dependency-sort edges"
    );
}

#[test]
fn source_package_public_export_dependency_edges_do_not_require_concrete_header_paths() {
    let (mut headers, mut string_table) = parse_module_headers(
        &[("src/page.moth", "NeedsWidget #String = \"ok\"\n")],
        "src/page.moth",
    );

    let helper_prefix = string_table.intern("helper");
    let widget_name = string_table.intern("Widget");
    let public_export_path = InternedPath::from_components(vec![helper_prefix, widget_name]);
    let concrete_target = InternedPath::try_from_filesystem_path(
        &PathBuf::from("lib/helper/internal/Widget"),
        &mut string_table,
    )
    .expect("test path should be UTF-8");

    let dependent_header = headers
        .headers
        .iter_mut()
        .find(|header| header_name(header, &string_table) == "NeedsWidget")
        .expect("expected dependent header");
    dependent_header
        .local_ordering_hints
        .insert(LocalDeclarationOrderingHint::provider_spelling(
            public_export_path,
        ));

    headers
        .module_symbols
        .source_package_public_exports
        .entry("helper".to_owned())
        .or_default()
        .insert(PublicExportEntry {
            export_name: widget_name,
            target: PublicExportTarget::SourceDeclaration {
                path: concrete_target,
            },
        });

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("public export dependency path should be accepted without a graph header");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header_name(header, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["NeedsWidget"],
        "source-backed package public export paths may differ from concrete source headers"
    );
}

#[test]
fn external_package_dependency_type_hint_does_not_survive_binding_as_graph_participant() {
    // WHY: syntax preparation records the import spelling for every named type reference
    // uniformly, including virtual or provider imports. Binding must drop import-spelled hints
    // that resolve to external symbols so they never become Stage 3 graph participants.
    let (headers, mut string_table) = parse_module_headers(
        &[("src/app.moth", "@core/io print\nwidget #print = \"x\"\n")],
        "src/app.moth",
    );

    let widget_header = headers
        .headers
        .iter()
        .find(|header| header_name(header, &string_table) == "widget")
        .expect("expected widget constant header");

    assert!(
        widget_header.local_ordering_hints.is_empty(),
        "external package import-spelled type hints must be dropped at binding, not retained \
         as Stage 3 graph participants"
    );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("dropped external hints must not perturb Stage 3 sorting");

    let non_start_names: Vec<_> = sorted
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .map(|header| header_name(header, &string_table))
        .collect();

    assert_eq!(
        non_start_names,
        vec!["widget"],
        "external import hints must not introduce graph nodes"
    );
}

// ------------------------
//  Content-source declaration ordering
// ------------------------

/// Parse a module whose content sources (`.mtf` templates and `.md` documents) are prepared with
/// their own source-kind preparation paths, alongside ordinary Moth files.
///
/// WHY: the fixture also simulates Stage 0's content resolution — every content-class path row is
/// paired with the fixture file its authored path names — so the returned `ContentSourceTargets`
/// matches what the compiler service derives from the real resolved-reference table.
fn parse_module_headers_with_content_sources(
    moth_files: &[(&str, &str)],
    templates: &[(&str, &str)],
    markdown_files: &[(&str, &str)],
    entry_path: &str,
) -> (BoundModuleHeaders, ContentSourceTargets, StringTable) {
    let mut string_table = StringTable::new();
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let entry_path_buf = PathBuf::from(entry_path);

    // One deterministic identity table over every fixture file. The fallback logical-path mode
    // keeps each authored spelling, so graph keys match the fixture layout.
    let all_paths = moth_files
        .iter()
        .chain(templates)
        .chain(markdown_files)
        .map(|(path, _)| PathBuf::from(path))
        .collect::<Vec<_>>();
    let source_files =
        SourceFileTable::build(all_paths.iter(), &entry_path_buf, None, &mut string_table)
            .expect("fixture source identities should build");
    let file_id_for = |path: &str| {
        source_files
            .get_by_canonical_path(&PathBuf::from(path))
            .map(|identity| identity.file_id)
            .unwrap_or_else(|| panic!("fixture file {path} should have a source identity"))
    };

    let mut prepared_outputs = Vec::new();

    for (path, source) in moth_files {
        let path_buf = PathBuf::from(path);
        let interned_path = InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
            .expect("test path should be UTF-8");
        let file_tokens = tokenize(
            source,
            &interned_path,
            TokenizerEntryMode::SourceFile,
            &style_directives,
            &mut string_table,
            Some(file_id_for(path)),
        )
        .expect("tokenization should succeed");

        prepared_outputs.push(
            prepare_file_from_tokens(
                file_tokens,
                &entry_path_buf,
                &options,
                &mut string_table,
                0,
                0,
            )
            .expect("preparation should succeed"),
        );
    }

    for (path, source) in templates {
        let path_buf = PathBuf::from(path);
        let interned_path = InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
            .expect("test path should be UTF-8");
        let entry_mode = TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template has a tokenizer entry mode");
        let file_tokens = tokenize(
            source,
            &interned_path,
            entry_mode,
            &style_directives,
            &mut string_table,
            Some(file_id_for(path)),
        )
        .expect("template tokenization should succeed");

        prepared_outputs.push(
            prepare_moth_template_file(file_tokens, &mut string_table)
                .expect("template preparation should succeed"),
        );
    }

    for (path, source) in markdown_files {
        let path_buf = PathBuf::from(path);
        let interned_path = InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
            .expect("test path should be UTF-8");
        prepared_outputs.push(prepare_plain_markdown_file(
            PlainMarkdownPrepareInput {
                source_code: source,
                source_file: interned_path,
                file_id: Some(file_id_for(path)),
                canonical_os_path: None,
            },
            &mut string_table,
        ));
    }

    // Simulate Stage 0: every content-class occurrence resolves to the fixture file its authored
    // path names. An occurrence naming no fixture file has no settled content target — Stage 0
    // retains a diagnostic outcome for it — so it gains no resolved row and ordering defers.
    let mut resolved_references = ResolvedFileReferenceTable::new();
    for output in &prepared_outputs {
        for reference in output.structural_file_references.iter() {
            if reference.class != PreparedFileReferenceClass::ContentSource {
                continue;
            }

            let target_path =
                PathBuf::from(reference.authored_path.to_portable_string(&string_table));
            let Some(target) = source_files
                .get_by_canonical_path(&target_path)
                .map(|identity| identity.file_id)
            else {
                continue;
            };
            resolved_references
                .push(ResolvedFileReference {
                    source_file: reference
                        .source_file
                        .expect("fixture prepared rows carry a FileId"),
                    path_syntax: reference.path_syntax,
                    class: reference.class,
                    outcome: ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::ContentSource { source: target },
                    ),
                })
                .expect("fixture resolved rows should be unique");
        }
    }
    let content_source_targets = ContentSourceTargets::from_resolved_references(
        &resolved_references,
        &source_files,
        &mut string_table,
    );

    let prepared_syntax = prepare_header_syntax(prepared_outputs, &mut string_table)
        .expect("header syntax preparation should succeed");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        options.project_path_resolver.as_ref(),
        &mut string_table,
    )
    .expect("header binding should succeed");

    (headers, content_source_targets, string_table)
}

fn sorted_header_index(
    sorted: &crate::compiler_frontend::module_dependencies::SortedHeaders,
    path_text: &str,
    string_table: &StringTable,
) -> usize {
    sorted
        .headers
        .iter()
        .position(|header| header.tokens.src_path.to_portable_string(string_table) == path_text)
        .unwrap_or_else(|| panic!("expected a sorted header at {path_text}"))
}

#[test]
fn content_value_in_parameter_and_field_defaults_orders_before_consumers() {
    let (headers, content_source_targets, mut string_table) =
        parse_module_headers_with_content_sources(
            &[(
                "@page.moth",
                "render |header String = [: [@docs/intro.mtf] ]| -> String:\n\
                 \x20   return header\n\
                 ;\n\
                 Card = |\n\
                 \x20   icon String = [: [@legal/notice.md] ],\n\
                 |\n",
            )],
            &[("docs/intro.mtf", "intro template body")],
            &[("legal/notice.md", "# Notice")],
            "@page.moth",
        );

    let sorted = resolve_module_dependencies(headers, &content_source_targets, &mut string_table)
        .expect("content ordering should resolve without cycles");

    let render_index = sorted_header_index(&sorted, "@page.moth/render", &string_table);
    let card_index = sorted_header_index(&sorted, "@page.moth/Card", &string_table);
    let intro_content_index = sorted_header_index(&sorted, "docs/intro.mtf/content", &string_table);
    let notice_content_index =
        sorted_header_index(&sorted, "legal/notice.md/content", &string_table);

    assert!(
        intro_content_index < render_index,
        "the parameter default's content constant must sort before the function that folds it"
    );
    assert!(
        notice_content_index < card_index,
        "the field default's content constant must sort before the struct that folds it"
    );
}

#[test]
fn repeated_content_value_occurrences_share_one_resolved_graph_edge() {
    let (parsed, content_source_targets, string_table) = parse_module_headers_with_content_sources(
        &[("@page.moth", "#[: [@docs/intro.mtf] [@docs/intro.mtf]]\n")],
        &[("docs/intro.mtf", "intro template body")],
        &[],
        "@page.moth",
    );

    let BoundModuleHeaders {
        headers,
        module_symbols,
        binding_environment,
        ..
    } = parsed;
    let graph = DependencyGraph::from_headers(
        headers,
        &module_symbols.source_package_public_exports,
        &binding_environment.imported_declarations_by_local_path,
        &content_source_targets,
        &string_table,
    );
    let consumer = graph
        .headers_by_path
        .values()
        .find(|header| header.local_ordering_hints.len() == 2)
        .expect("the repeated content shell should retain both authored hints");

    let edges = graph.sorted_dependency_edges_for_header(consumer, &string_table);
    let content_edges = edges
        .iter()
        .filter(|edge| matches!(edge.kind, DependencyEdgeKind::GraphHeader))
        .collect::<Vec<_>>();

    assert_eq!(
        content_edges.len(),
        1,
        "repeated authored paths should deduplicate at the resolved graph edge"
    );
    assert_eq!(
        content_edges[0]
            .resolved_path
            .as_ref()
            .expect("content edge should resolve to a graph header")
            .to_portable_string(&string_table),
        "docs/intro.mtf/content"
    );
}
#[test]

fn content_dependency_cycle_is_diagnosed_by_the_ordering_authority() {
    // WHY: the smallest content cycle — intro's content constant depends on license's, and
    // license's depends back on intro's. Discovery must not guard recursion; Stage 3's DFS
    // temp-mark diagnoses the cycle.
    let (headers, content_source_targets, mut string_table) =
        parse_module_headers_with_content_sources(
            &[(
                "@page.moth",
                "intro #= @docs/intro.mtf\nlicense #= @legal/license.mtf\n",
            )],
            &[
                ("docs/intro.mtf", "[@legal/license.mtf]"),
                ("legal/license.mtf", "[@docs/intro.mtf]"),
            ],
            &[],
            "@page.moth",
        );

    let bag = resolve_module_dependencies(headers, &content_source_targets, &mut string_table)
        .expect_err("a real content dependency cycle should fail dependency sorting");

    let cycle_diagnostic = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            let DiagnosticPayload::CircularDependency { path } = &diagnostic.payload else {
                return false;
            };

            let path = path.to_portable_string(&string_table);
            path.contains("docs/intro.mtf/content") || path.contains("legal/license.mtf/content")
        })
        .unwrap_or_else(|| panic!("expected a content cycle diagnostic, got: {bag:?}"));

    let diagnostic_scope = cycle_diagnostic
        .primary_location
        .scope
        .to_portable_string(&string_table);
    assert!(
        diagnostic_scope == "docs/intro.mtf" || diagnostic_scope == "legal/license.mtf",
        "the cycle diagnostic should point at a content constant's own location, got {diagnostic_scope}"
    );
}

#[test]
fn nested_module_content_reference_orders_through_resolved_targets() {
    // A nested module root (its own @page.moth below the entry root) compiles with entry-root-
    // relative header keys while its authored @-paths are module-root-relative. Stage 3 resolves
    // the content edge through the Stage 0 resolved target, so the ordering edge still lands on
    // the canonical content constant instead of deferring on a spelling mismatch.
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();

    let project_root = PathBuf::from("project-root");
    let root_file = project_root.join("components/@page.moth");
    let icon_template = project_root.join("components/icon.mtf");
    let source_files = SourceFileTable::build(
        [&root_file, &icon_template],
        &root_file,
        Some(
            &crate::compiler_frontend::paths::path_resolution::ProjectPathResolver::new(
                project_root.clone(),
                project_root,
                crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(),
                &crate::builder_surface::SourceFileKindRegistry::default(),
            )
            .expect("fixture resolver should build"),
        ),
        &mut string_table,
    )
    .expect("nested fixture identities should build");

    let root_file_id = source_files
        .get_by_canonical_path(&root_file)
        .expect("root file identity")
        .file_id;
    let icon_file_id = source_files
        .get_by_canonical_path(&icon_template)
        .expect("icon identity")
        .file_id;
    let root_logical = source_files
        .get(root_file_id)
        .expect("root identity")
        .logical_path
        .clone();
    let icon_logical = source_files
        .get(icon_file_id)
        .expect("icon identity")
        .logical_path
        .clone();

    let entry_path_buf = root_logical.to_path_buf(&string_table);
    let mut prepared_outputs = Vec::new();

    let root_tokens = tokenize(
        "icon #= @icon.mtf\n",
        &root_logical,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(root_file_id),
    )
    .expect("root file should tokenize");
    prepared_outputs.push(
        prepare_file_from_tokens(
            root_tokens,
            &entry_path_buf,
            &options,
            &mut string_table,
            0,
            0,
        )
        .expect("root file should prepare"),
    );

    let icon_tokens = tokenize(
        "[: icon body]",
        &icon_logical,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template has a tokenizer entry mode"),
        &style_directives,
        &mut string_table,
        Some(icon_file_id),
    )
    .expect("icon template should tokenize");
    prepared_outputs.push(
        prepare_moth_template_file(icon_tokens, &mut string_table)
            .expect("icon template should prepare"),
    );

    // Simulate Stage 0 for the nested module: the module-relative authored occurrence inside
    // `components/@page.moth` resolves to the icon template's canonical identity.
    let content_row = prepared_outputs[0]
        .structural_file_references
        .iter()
        .find(|reference| reference.class == PreparedFileReferenceClass::ContentSource)
        .expect("the nested initializer should retain a content-class row");
    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file: content_row
                .source_file
                .expect("fixture prepared rows carry a FileId"),
            path_syntax: content_row.path_syntax,
            class: content_row.class,
            outcome: ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ContentSource {
                    source: icon_file_id,
                },
            ),
        })
        .expect("fixture resolved rows should be unique");
    let content_source_targets = ContentSourceTargets::from_resolved_references(
        &resolved_references,
        &source_files,
        &mut string_table,
    );

    let prepared_syntax = prepare_header_syntax(prepared_outputs, &mut string_table)
        .expect("nested header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        None,
        &mut string_table,
    )
    .expect("nested headers should bind");

    let sorted = resolve_module_dependencies(headers, &content_source_targets, &mut string_table)
        .expect(
            "a nested module-relative content reference must order through the resolved target",
        );

    let icon_content_index =
        sorted_header_index(&sorted, "components/icon.mtf/content", &string_table);
    let icon_declaration_index =
        sorted_header_index(&sorted, "components/@page.moth/icon", &string_table);

    assert!(
        icon_content_index < icon_declaration_index,
        "the nested module's content constant must sort before the declaration that folds it"
    );
}

#[test]
fn missing_content_target_defers_to_the_stage0_diagnostic_lane() {
    // A content occurrence whose Stage 0 outcome is not a settled content target (here: no
    // resolved row at all) defers in ordering instead of raising a second missing-target error,
    // so the resolved-reference lane keeps ownership of the user-facing diagnostic.
    let (headers, _content_source_targets, mut string_table) =
        parse_module_headers_with_content_sources(
            &[("@page.moth", "unused #= @docs/intro.mtf\n")],
            &[],
            &[],
            "@page.moth",
        );

    let sorted =
        resolve_module_dependencies(headers, &ContentSourceTargets::empty(), &mut string_table)
            .expect("a missing content target must defer, not fail dependency sorting");

    assert!(
        sorted
            .headers
            .iter()
            .any(|header| header_name(header, &string_table) == "unused"),
        "the declaring shell must still sort; the missing target is diagnosed by Stage 0's lane"
    );
}
