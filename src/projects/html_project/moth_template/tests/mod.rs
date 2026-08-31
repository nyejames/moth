//! Tests for the direct HTML-project Moth template API.
//!
//! WHAT: covers input normalization, AST-only compilation, ordering, duplicate diagnostics, and
//! the deferred caller-supplied scope boundary.
//! WHY: this API is intentionally not wired into project builds yet, so module-local tests protect
//! the tooling-facing boundary without adding integration artifacts.

use crate::build_system::create_project_modules::resource_inputs::{
    ResourceContentState, ResourceInputRegistry,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, ImportDiagnosticKind, InvalidConfigReason,
};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::moth_template::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput, MothTemplateCompileRequest,
    MothTemplateInput, MothTemplatePathScope, MothTemplateScopeConstant, MothTemplateSource,
    compile_moth_template,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn request(input: MothTemplateInput) -> MothTemplateCompileRequest {
    MothTemplateCompileRequest {
        input,
        default_module_constants: Vec::new(),
        module_constants_by_path: Vec::new(),
    }
}

fn temp_project(files: &[(&str, &str)]) -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp project should be created");
    for (relative_path, source) in files {
        let path = temp_dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("source parent should be created");
        }
        fs::write(path, source).expect("source should be written");
    }

    temp_dir
}

fn compile_ok(input: MothTemplateInput) -> MothTemplateCompileOutput {
    let mut string_table = StringTable::new();
    compile_moth_template(request(input), &mut string_table)
        .expect("Moth template input should compile")
}

#[test]
fn files_input_template_with_sibling_resource_renders_relative_url() {
    let temp_dir = temp_project(&[
        ("page.mtf", "# Page\n\n[@logo.svg]"),
        ("logo.svg", "<svg/>"),
    ]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("page.mtf"),
    ]));

    assert_eq!(output.documents.len(), 1);
    assert!(
        !output.resources.is_empty(),
        "a sibling-resource compile should return deferred resource outputs"
    );
    assert!(
        output.documents[0].content.contains("./logo.svg"),
        "a sibling resource should render a URL relative to the template's directory: {}",
        output.documents[0].content
    );
}

#[test]
fn files_input_deferred_resources_resolve_through_request_registry() {
    let temp_dir = temp_project(&[
        ("page.mtf", "# Page\n\n[@logo.svg]"),
        ("logo.svg", "<svg/>"),
    ]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("page.mtf"),
    ]));

    assert_eq!(output.resources.len(), 1);
    let sibling_origin = test_resource_origin("", "logo.svg");
    let origin_source = output
        .resource_inputs
        .source_for_origin(&sibling_origin)
        .expect("the sibling resource origin should resolve through the request registry");

    assert_eq!(origin_source.index(), output.resources[0].source_id.index());
    assert_eq!(
        output
            .resource_inputs
            .validate()
            .expect("the returned registry should keep its boundary invariants"),
        ()
    );
}

#[test]
fn stage0_direct_template_resource_sources_stay_unhashed() {
    let temp_dir = temp_project(&[
        ("page.mtf", "# Page\n\n[@logo.svg]"),
        ("logo.svg", "<svg/>"),
    ]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("page.mtf"),
    ]));

    assert!(
        !output.resource_inputs.records().is_empty(),
        "a sibling-resource compile registers at least one physical source"
    );
    assert!(
        output
            .resource_inputs
            .records()
            .iter()
            .all(|record| record.content() == ResourceContentState::Unhashed),
        "direct-template compilation must not read or hash resource bytes; emission does"
    );
}

#[test]
fn directory_input_distinct_documents_mint_distinct_resource_origins() {
    let temp_dir = temp_project(&[
        ("guide/page.mtf", "Guide\n\n[@assets/logo.svg]"),
        ("guide/assets/logo.svg", "<svg/>"),
        ("reference/page.mtf", "Reference\n\n[@assets/logo.svg]"),
        ("reference/assets/logo.svg", "<svg/>"),
    ]);

    let output = compile_ok(MothTemplateInput::Directory {
        path: temp_dir.path().to_path_buf(),
        recursive: true,
    });

    let guide_origin = test_resource_origin("guide", "assets/logo.svg");
    let reference_origin = test_resource_origin("reference", "assets/logo.svg");
    let guide_source = output
        .resource_inputs
        .source_for_origin(&guide_origin)
        .expect("the guide document's resource origin should be attached");
    let reference_source = output
        .resource_inputs
        .source_for_origin(&reference_origin)
        .expect("the reference document's resource origin should be attached");

    assert_ne!(
        guide_source.index(),
        reference_source.index(),
        "same-named resources under distinct module directories own distinct physical sources"
    );
    assert_eq!(
        sorted_deferred_paths(&output),
        vec![
            PathBuf::from("guide/assets/logo.svg"),
            PathBuf::from("reference/assets/logo.svg"),
        ]
    );

    let guide_page = document_with_relative_path(&output, "guide/page.mtf");
    let reference_page = document_with_relative_path(&output, "reference/page.mtf");
    assert!(
        guide_page.content.contains("./assets/logo.svg"),
        "guide/page.mtf should observe the resource from its own directory: {}",
        guide_page.content
    );
    assert!(
        reference_page.content.contains("./assets/logo.svg"),
        "reference/page.mtf should observe the resource from its own directory: {}",
        reference_page.content
    );
    assert!(
        !guide_page.content.contains("./guide/assets/logo.svg")
            && !reference_page
                .content
                .contains("./reference/assets/logo.svg"),
        "nested documents must not prefix their own directory into the resource URL"
    );
}

#[test]
fn compile_time_dead_resource_stays_watchable_and_is_not_emitted() {
    let temp_dir = temp_project(&[
        (
            "page.mtf",
            "[if false:\n    [@dead.svg]\n]\n# Page\n\n[@logo.svg]",
        ),
        ("dead.svg", "<svg id=\"dead\"/>"),
        ("logo.svg", "<svg id=\"live\"/>"),
    ]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("page.mtf"),
    ]));

    assert_eq!(output.documents.len(), 1);
    assert!(
        output.documents[0].content.contains("./logo.svg"),
        "the live resource should render: {}",
        output.documents[0].content
    );
    assert!(
        !output.documents[0].content.contains("dead.svg"),
        "a compile-time-dead resource must not appear in the document: {}",
        output.documents[0].content
    );
    assert_eq!(
        sorted_deferred_paths(&output),
        vec![PathBuf::from("logo.svg")],
        "exact output emission excludes a resource folded out of the final content"
    );
    assert!(
        output.resource_inputs.records().len() >= 2,
        "Stage 0 still registers the dead resource as a watchable source"
    );
}

#[test]
fn distinct_origins_claiming_one_direct_output_path_are_diagnosed() {
    let temp_dir = temp_project(&[
        ("a/page.mtf", "A\n\n[@shared/logo.svg]"),
        ("a/shared/page.mtf", "Shared\n\n[@logo.svg]"),
        ("a/shared/logo.svg", "<svg/>"),
    ]);
    let mut string_table = StringTable::new();

    let messages = compile_moth_template(
        request(MothTemplateInput::Directory {
            path: temp_dir.path().to_path_buf(),
            recursive: true,
        }),
        &mut string_table,
    )
    .expect_err("distinct origins claiming one output path should fail");

    let InvalidConfigReason::ResourceOutputPathCollision {
        output_path,
        existing_origin,
        conflicting_origin,
    } = invalid_config_reason(&messages)
    else {
        panic!("expected a resource output path collision reason");
    };
    assert_eq!(string_table.resolve(*output_path), "a/shared/logo.svg");
    assert!(
        string_table.resolve(*existing_origin).contains("at 'a'/"),
        "the first origin should name module a: {}",
        string_table.resolve(*existing_origin)
    );
    assert!(
        string_table
            .resolve(*conflicting_origin)
            .contains("at 'a/shared'/"),
        "the second origin should name module a/shared: {}",
        string_table.resolve(*conflicting_origin)
    );
}

#[test]
fn same_origin_used_by_multiple_documents_emits_once() {
    let temp_dir = temp_project(&[
        ("docs/page.mtf", "Page\n\n[@logo.svg]"),
        ("docs/about.mtf", "About\n\n[@logo.svg]"),
        ("docs/logo.svg", "<svg/>"),
    ]);

    let output = compile_ok(MothTemplateInput::Directory {
        path: temp_dir.path().to_path_buf(),
        recursive: true,
    });

    assert_eq!(output.documents.len(), 2);
    assert_eq!(
        sorted_deferred_paths(&output),
        vec![PathBuf::from("docs/logo.svg")],
        "one origin used by two documents emits one deferred resource"
    );
    let about = document_with_relative_path(&output, "docs/about.mtf");
    let page = document_with_relative_path(&output, "docs/page.mtf");
    assert!(
        about.content.contains("./logo.svg") && page.content.contains("./logo.svg"),
        "both documents should observe the shared origin: about={} page={}",
        about.content,
        page.content
    );
    assert!(
        output
            .resource_inputs
            .records()
            .iter()
            .all(|record| record.content() == ResourceContentState::Unhashed),
        "request-wide planning must not read or hash resource bytes"
    );
}

#[test]
fn request_wide_conflict_leaves_every_source_unhashed() {
    let temp_dir = temp_project(&[
        ("a/page.mtf", "A\n\n[@shared/logo.svg]"),
        ("a/shared/page.mtf", "Shared\n\n[@logo.svg]"),
        ("a/shared/logo.svg", "<svg/>"),
    ]);
    let mut string_table = StringTable::new();
    let mut resource_inputs = ResourceInputRegistry::new();

    let messages = super::compile::compile_moth_template_with_registry(
        request(MothTemplateInput::Directory {
            path: temp_dir.path().to_path_buf(),
            recursive: true,
        }),
        &mut string_table,
        &mut resource_inputs,
    )
    .expect_err("distinct origins claiming one output path should fail");

    assert!(matches!(
        invalid_config_reason(&messages),
        InvalidConfigReason::ResourceOutputPathCollision { .. }
    ));
    assert!(
        !resource_inputs.records().is_empty(),
        "Stage 0 still registers colliding sources before the plan fails"
    );
    assert!(
        resource_inputs
            .records()
            .iter()
            .all(|record| record.content() == ResourceContentState::Unhashed),
        "a request-wide conflict must not read or hash any registered source"
    );
}

#[test]
fn same_tree_in_two_checkouts_matches_origins_and_output_paths() {
    let files = [
        ("guide/page.mtf", "Guide\n\n[@assets/logo.svg]"),
        ("guide/assets/logo.svg", "<svg/>"),
        ("reference/page.mtf", "Reference\n\n[@assets/logo.svg]"),
        ("reference/assets/logo.svg", "<svg/>"),
    ];
    let first_checkout = temp_project(&files);
    let second_checkout = temp_project(&files);

    let first = compile_ok(MothTemplateInput::Directory {
        path: first_checkout.path().to_path_buf(),
        recursive: true,
    });
    let second = compile_ok(MothTemplateInput::Directory {
        path: second_checkout.path().to_path_buf(),
        recursive: true,
    });

    let expected_origins = [
        test_resource_origin("guide", "assets/logo.svg"),
        test_resource_origin("reference", "assets/logo.svg"),
    ];
    for origin in &expected_origins {
        for output in [&first, &second] {
            assert!(
                output.resource_inputs.source_for_origin(origin).is_some(),
                "origin {origin:?} should resolve identically across checkouts"
            );
        }
    }
    assert_eq!(
        sorted_deferred_paths(&first),
        sorted_deferred_paths(&second)
    );
    assert_ne!(
        first.resource_inputs, second.resource_inputs,
        "identity must match across checkouts while canonical IO paths stay checkout-local"
    );
}

#[test]
fn mixed_relative_and_absolute_in_memory_paths_are_diagnosed() {
    let temp_dir = temp_project(&[]);
    let mut string_table = StringTable::new();

    let messages = compile_moth_template(
        request(MothTemplateInput::Sources(vec![
            MothTemplateSource {
                display_path: PathBuf::from("memory/intro.mtf"),
                source_text: "first".to_owned(),
            },
            MothTemplateSource {
                display_path: temp_dir.path().join("absolute.mtf"),
                source_text: "second".to_owned(),
            },
        ])),
        &mut string_table,
    )
    .expect_err("mixed relative and absolute display paths have one portable basis");

    assert_eq!(messages.error_count(), 1);
    let diagnostic = messages
        .diagnostics()
        .next()
        .expect("mixed-basis diagnostic should be present");
    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Import(ImportDiagnosticKind::MothTemplateInputsShareNoCommonAncestor)
    ));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::MothTemplateInputsShareNoCommonAncestor { .. }
    ));
}

/// Reconstruct the stable module-owned origin a direct compile mints for one resource.
///
/// Module roots derive from the document's portable relative directory under the direct lane's
/// shared project-local package name.
fn test_resource_origin(relative_module_path: &str, resource_path: &str) -> StableResourceOriginId {
    let module_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local(super::bundle::DIRECT_TEMPLATE_PROJECT_NAME),
        Path::new(relative_module_path),
        ModuleRootRole::Normal,
    )
    .expect("test module path should be portable");
    let logical_path = PortableResourcePath::from_relative_logical_path(Path::new(resource_path))
        .expect("test resource path should be portable");

    StableResourceOriginId::module_owned(module_origin, logical_path)
}

fn sorted_deferred_paths(output: &MothTemplateCompileOutput) -> Vec<PathBuf> {
    let mut paths = output
        .resources
        .iter()
        .map(|resource| resource.relative_output_path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn document_with_relative_path<'a>(
    output: &'a MothTemplateCompileOutput,
    relative: &str,
) -> &'a CompiledMothTemplateDocument {
    output
        .documents
        .iter()
        .find(|document| document.relative_path.as_deref() == Some(Path::new(relative)))
        .unwrap_or_else(|| panic!("compiled output should include {relative}"))
}

fn invalid_config_reason(messages: &CompilerMessages) -> &InvalidConfigReason {
    let diagnostic = messages
        .first_error()
        .expect("expected an error-severity diagnostic");
    match &diagnostic.payload {
        DiagnosticPayload::InvalidConfig { reason, .. } => reason,
        _ => panic!("expected an invalid config diagnostic"),
    }
}

#[test]
fn files_input_nested_same_owner_mtf_content_value_is_inlined() {
    let temp_dir = temp_project(&[
        ("page.mtf", "# Page\n\n[@docs/intro.mtf]"),
        ("docs/intro.mtf", "# Nested"),
    ]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("page.mtf"),
    ]));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(
        output.documents[0].content,
        "<h1>Page</h1><p><h1>Nested</h1></p>"
    );
}

#[test]
fn files_input_markdown_content_value_is_inlined() {
    let temp_dir = temp_project(&[
        ("page.mtf", "About\n\n[@docs/legal.md]"),
        ("docs/legal.md", "Plain legal text."),
    ]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("page.mtf"),
    ]));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(
        output.documents[0].content,
        "<p>About</p><p><p>Plain legal text.</p>\n</p>"
    );
}

#[test]
fn file_input_compiles_one_moth_template_file() {
    let temp_dir = temp_project(&[("intro.mtf", "# Intro")]);
    let source_path = temp_dir.path().join("intro.mtf");

    let output = compile_ok(MothTemplateInput::File(source_path.clone()));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "<h1>Intro</h1>");
    assert_eq!(output.documents[0].relative_path, None);
    assert_eq!(
        output.documents[0].source_path,
        fs::canonicalize(source_path).expect("source path should canonicalize")
    );
}

#[test]
fn direct_directory_input_compiles_direct_child_moth_template_files_sorted_by_relative_path() {
    let temp_dir = temp_project(&[
        ("docs/z-last.mtf", "z"),
        ("docs/a-first.mtf", "a"),
        ("docs/nested/ignored.mtf", "nested"),
        ("docs/readme.txt", "ignored"),
    ]);

    let output = compile_ok(MothTemplateInput::Directory {
        path: temp_dir.path().join("docs"),
        recursive: false,
    });

    let relative_paths = output
        .documents
        .iter()
        .map(|document| document.relative_path.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        relative_paths,
        vec![
            Some(Path::new("a-first.mtf")),
            Some(Path::new("z-last.mtf"))
        ]
    );
    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.content.as_str())
            .collect::<Vec<_>>(),
        vec!["<p>a</p>", "<p>z</p>"]
    );
}

#[test]
fn recursive_directory_input_compiles_descendant_moth_template_files() {
    let temp_dir = temp_project(&[
        ("docs/index.mtf", "index"),
        ("docs/nested/detail.mtf", "detail"),
    ]);

    let output = compile_ok(MothTemplateInput::Directory {
        path: temp_dir.path().join("docs"),
        recursive: true,
    });

    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.relative_path.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some(Path::new("index.mtf")),
            Some(Path::new("nested/detail.mtf"))
        ]
    );
}

#[test]
fn explicit_file_list_preserves_caller_order() {
    let temp_dir = temp_project(&[("first.mtf", "first"), ("second.mtf", "second")]);

    let output = compile_ok(MothTemplateInput::Files(vec![
        temp_dir.path().join("second.mtf"),
        temp_dir.path().join("first.mtf"),
    ]));

    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.content.as_str())
            .collect::<Vec<_>>(),
        vec!["<p>second</p>", "<p>first</p>"]
    );
    // Same-directory file-list units share the entry-root empty module path, so their portable
    // identity is the file name itself.
    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.relative_path.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(PathBuf::from("second.mtf")),
            Some(PathBuf::from("first.mtf"))
        ]
    );
}

#[test]
fn in_memory_sources_compile_without_filesystem_output() {
    let output = compile_ok(MothTemplateInput::Sources(vec![MothTemplateSource {
        display_path: PathBuf::from("memory/intro.mtf"),
        source_text: "[:nested]".to_owned(),
    }]));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "<p>nested</p>");
    assert_eq!(
        output.documents[0].source_path,
        PathBuf::from("memory/intro.mtf")
    );
    assert_eq!(
        output.documents[0].relative_path,
        Some(PathBuf::from("memory/intro.mtf"))
    );
}

#[test]
fn duplicate_source_paths_are_diagnostics() {
    let temp_dir = temp_project(&[("intro.mtf", "intro")]);
    let path = temp_dir.path().join("intro.mtf");
    let mut string_table = StringTable::new();

    let messages = compile_moth_template(
        request(MothTemplateInput::Files(vec![path.clone(), path])),
        &mut string_table,
    )
    .expect_err("duplicate input paths should fail");

    assert_eq!(messages.error_count(), 1);
    let diagnostic = messages
        .diagnostics()
        .next()
        .expect("duplicate diagnostic should be present");
    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Import(ImportDiagnosticKind::DuplicateMothTemplateInputPath)
    ));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::DuplicateMothTemplateInputPath { .. }
    ));
}

#[test]
fn compile_api_does_not_write_artifacts() {
    let temp_dir = temp_project(&[("intro.mtf", "intro")]);
    let before = directory_entries(temp_dir.path());

    let _output = compile_ok(MothTemplateInput::File(temp_dir.path().join("intro.mtf")));

    assert_eq!(directory_entries(temp_dir.path()), before);
}

#[test]
fn caller_supplied_scope_constants_are_deferred_without_exposing_internals() {
    let temp_dir = temp_project(&[("intro.mtf", "intro")]);
    let mut string_table = StringTable::new();
    let request = MothTemplateCompileRequest {
        input: MothTemplateInput::File(temp_dir.path().join("intro.mtf")),
        default_module_constants: vec![MothTemplateScopeConstant::test_placeholder()],
        module_constants_by_path: vec![MothTemplatePathScope {
            source_path: temp_dir.path().join("intro.mtf"),
            constants: Vec::new(),
        }],
    };

    let messages = compile_moth_template(request, &mut string_table)
        .expect_err("caller-supplied scope constants are intentionally unsupported in this slice");

    assert_eq!(messages.error_count(), 1);
    let diagnostic = messages
        .diagnostics()
        .next()
        .expect("scope diagnostic should be present");
    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Import(ImportDiagnosticKind::InvalidMothTemplateApiScopeItem)
    ));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMothTemplateApiScopeItem { .. }
    ));
}

fn directory_entries(path: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(path)
        .expect("directory should be readable")
        .map(|entry| {
            entry
                .expect("directory entry should be readable")
                .path()
                .strip_prefix(path)
                .expect("entry should be under directory")
                .to_path_buf()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

// -----------------------------------------------------------------------------
// Moth template direct API TIR-backed behavior tests
// -----------------------------------------------------------------------------

#[test]
fn file_input_compiles_bd_markdown_with_nested_template() {
    let temp_dir = temp_project(&[("intro.mtf", "# [:title]")]);

    let output = compile_ok(MothTemplateInput::File(temp_dir.path().join("intro.mtf")));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "<h1><p>title</p></h1>");
}

#[test]
fn source_input_compiles_bd_nested_authored_template() {
    let output = compile_ok(MothTemplateInput::Sources(vec![MothTemplateSource {
        display_path: PathBuf::from("memory/nested.mtf"),
        source_text: "[:# Nested]".to_owned(),
    }]));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "<h1>Nested</h1>");
}

#[test]
fn source_input_nested_raw_directive_overrides_bd_markdown_default() {
    let output = compile_ok(MothTemplateInput::Sources(vec![MothTemplateSource {
        display_path: PathBuf::from("memory/raw-nested.mtf"),
        source_text: "[$raw:# Nested]".to_owned(),
    }]));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "# Nested");
}

#[test]
fn source_input_nested_non_formatter_directive_overrides_bd_markdown_default() {
    let output = compile_ok(MothTemplateInput::Sources(vec![MothTemplateSource {
        display_path: PathBuf::from("memory/fresh-nested.mtf"),
        source_text: "[$fresh:# Nested]".to_owned(),
    }]));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "# Nested");
}
