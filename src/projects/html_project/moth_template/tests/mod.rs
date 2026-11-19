//! Tests for the direct HTML-project Moth template API.
//!
//! WHAT: covers input normalization, AST-only compilation, ordering, and duplicate diagnostics.
//! WHY: this API is intentionally not wired into project builds yet, so module-local tests protect
//! the tooling-facing boundary without adding integration artifacts.

use super::bundle::prepare_file_value_bundle;
use crate::build_system::create_project_modules::resource_inputs::{
    ResourceContentState, ResourceInputRegistry,
};
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::DuplicateMothTemplateInputPathView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_resolved_failure_for_test;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, dev_server::render_compilation_result_html,
    terse::format_terse_diagnostic_reports,
};
use crate::compiler_frontend::compiler_messages::{CompilerStage, PreparedFailure};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceOutcome;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::single_source_compilation::{
    MothTemplateCompilationRequest, compile_moth_template_source,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{ForcedExhaustionGuard, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use crate::projects::html_project::moth_template::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput, MothTemplateInput, MothTemplateSource,
    compile_moth_template,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

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

#[cfg(unix)]
fn non_utf8_file_name(bytes: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;

    std::ffi::OsString::from_vec(bytes.to_vec())
}

fn compile_ok(input: MothTemplateInput) -> MothTemplateCompileOutput {
    match compile_moth_template(input, &mut StringTable::new()) {
        Ok(CompilationOutcome::Success { value, .. }) => value,
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            panic!("Moth template input should compile: {reports:?}")
        }
        Err(errors) => panic!("Moth template input failed operationally: {errors:?}"),
    }
}
fn expect_diagnosed<T>(result: CompilationResult<T>, message: &str) -> DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        Ok(CompilationOutcome::Success { .. }) => panic!("{message}"),
        Err(errors) => panic!("{message}: operational failure: {errors:?}"),
    }
}

/// Compile an input that must fail operationally rather than succeed or diagnose.
fn expect_infrastructure_failure(
    input: MothTemplateInput,
    description: &str,
) -> InfrastructureErrors {
    match compile_moth_template(input, &mut StringTable::new()) {
        Ok(CompilationOutcome::Success { .. }) => panic!("{description} must not compile"),
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            panic!("{description} must fail operationally, not diagnostically: {reports:?}")
        }
        Err(errors) => errors,
    }
}

#[test]
fn warnings_only_travel_with_success_and_preserve_document_snapshots() {
    let first_source = "[$html: <script>first_snapshot()</script>]";
    let second_source = "[$html: <script>second_snapshot()</script>]";
    let temp_dir = temp_project(&[
        ("a/page.mtf", first_source),
        ("b/page.mtf", second_source),
        ("c/page.mtf", "]"),
    ]);

    for include_failure in [false, true] {
        let mut inputs = vec![
            temp_dir.path().join("a/page.mtf"),
            temp_dir.path().join("b/page.mtf"),
        ];
        if include_failure {
            inputs.push(temp_dir.path().join("c/page.mtf"));
        }
        let mut string_table = StringTable::new();
        let result = compile_moth_template(MothTemplateInput::Files(inputs), &mut string_table);
        let (reports, succeeded) = if include_failure {
            (
                expect_diagnosed(result, "the third document should fail preparation"),
                false,
            )
        } else {
            match result {
                Ok(CompilationOutcome::Success {
                    value,
                    warnings: Some(reports),
                }) => {
                    assert_eq!(value.documents.len(), 2);
                    (reports, true)
                }
                Ok(CompilationOutcome::Success { warnings: None, .. }) => {
                    panic!("both HTML documents should retain their warnings")
                }
                Ok(CompilationOutcome::Diagnosed(reports)) => {
                    panic!("both HTML documents should compile: {reports:?}")
                }
                Err(errors) => panic!("template compilation failed operationally: {errors:?}"),
            }
        };

        let expected_codes = if include_failure {
            vec!["MOTH-SYNTAX-0032"]
        } else {
            vec!["MOTH-SYNTAX-0005", "MOTH-SYNTAX-0005"]
        };
        assert_eq!(
            reports
                .iter()
                .map(|(_, view)| view.descriptor().code)
                .collect::<Vec<_>>(),
            expected_codes,
        );
        assert_eq!(
            reports.counts().warnings,
            if include_failure { 0 } else { 2 }
        );
        assert_eq!(reports.counts().errors, if include_failure { 1 } else { 0 });

        let expected_source_lines = if include_failure {
            vec!["]"]
        } else {
            vec![first_source, second_source]
        };
        for ((report, view), expected_source_line) in reports.iter().zip(expected_source_lines) {
            let position = DiagnosticRenderContext::for_report(report)
                .site_position(view.primary_site())
                .expect("diagnostic primary source should resolve through its report owner");
            assert_eq!(
                position.line, expected_source_line,
                "each diagnostic should retain its owning document snapshot",
            );
        }

        // Every document has the same logical filename in a separate source domain. A missing
        // range shift or substituted current-document context would show the wrong snapshot.
        let render_result: CompilationResult<()> = if succeeded {
            Ok(CompilationOutcome::Success {
                value: (),
                warnings: Some(reports),
            })
        } else {
            Ok(CompilationOutcome::Diagnosed(reports))
        };
        let rendered = render_compilation_result_html(&render_result, temp_dir.path());
        if include_failure {
            assert!(rendered.contains(r#"<span class="source-line">]</span>"#));
            assert!(!rendered.contains("first_snapshot"));
            assert!(!rendered.contains("second_snapshot"));
        } else {
            assert!(rendered.contains(
                r#"<span class="source-line">[$html: &lt;script&gt;first_snapshot()&lt;/script&gt;]</span>"#
            ));
            assert!(rendered.contains(
                r#"<span class="source-line">[$html: &lt;script&gt;second_snapshot()&lt;/script&gt;]</span>"#
            ));
        }
    }
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

    let messages = expect_diagnosed(
        compile_moth_template(
            MothTemplateInput::Directory {
                path: temp_dir.path().to_path_buf(),
                recursive: true,
            },
            &mut string_table,
        ),
        "distinct origins claiming one output path should fail",
    );

    assert_eq!(
        invalid_config_reason(&messages),
        Some("invalid_config.resource_output_path_collision"),
        "expected a resource output path collision reason",
    );
    let rendered = format_terse_diagnostic_reports(&messages).join("\n");
    assert!(rendered.contains("a/shared/logo.svg"));
    assert!(
        rendered.contains("at 'a'/"),
        "the first origin should name module a: {rendered}"
    );
    assert!(
        rendered.contains("at 'a/shared'/"),
        "the second origin should name module a/shared: {rendered}"
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

    let messages = expect_diagnosed(
        super::compile::compile_moth_template_with_registry(
            MothTemplateInput::Directory {
                path: temp_dir.path().to_path_buf(),
                recursive: true,
            },
            &mut string_table,
            &mut resource_inputs,
        ),
        "distinct origins claiming one output path should fail",
    );

    assert_eq!(
        invalid_config_reason(&messages),
        Some("invalid_config.resource_output_path_collision"),
    );
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
fn mixed_relative_and_rooted_in_memory_paths_are_diagnosed() {
    let temp_dir = temp_project(&[]);
    let absolute_display_path =
        PathBuf::from(portable_path_text(temp_dir.path().join("absolute.mtf")));

    // Absolute and root-relative names share one rooted classification even where a host calls
    // only one of them absolute.
    for (second_path, second_name) in [
        (absolute_display_path, "absolute.mtf"),
        (PathBuf::from("/docs/rooted.mtf"), "rooted.mtf"),
    ] {
        let messages = expect_diagnosed(
            compile_moth_template(
                MothTemplateInput::Sources(vec![
                    MothTemplateSource {
                        display_path: PathBuf::from("memory/intro.mtf"),
                        source_text: "first".to_owned(),
                    },
                    MothTemplateSource {
                        // Rooted in-memory names follow the portable spelling contract too: only
                        // the host prefix and root are native, the remainder uses `/`.
                        display_path: second_path,
                        source_text: "second".to_owned(),
                    },
                ]),
                &mut StringTable::new(),
            ),
            "mixed relative and rooted display paths have one portable basis",
        );

        assert_eq!(messages.counts().errors, 1);
        assert_eq!(
            messages
                .iter()
                .map(|(_, view)| view.descriptor().code)
                .collect::<Vec<_>>(),
            ["MOTH-IMPORT-0029"],
        );
        let rendered = format_terse_diagnostic_reports(&messages).join("\n");
        assert!(
            rendered.contains("memory/intro.mtf"),
            "the diagnostic should render its first actual input path: {rendered}",
        );
        assert!(
            rendered.contains(second_name),
            "the diagnostic should render its second actual input path: {rendered}",
        );
    }
}

// Unix absolute names always share the root directory, so only Windows path parsing can build a
// three-input request whose leading components differ: a drive prefix and the root directory are
// distinct first components.
#[cfg(windows)]
#[test]
fn no_common_ancestor_diagnostic_names_the_conflicting_input() {
    // The third input empties the shared directory prefix. The diagnostic must name the basis
    // and the input that eliminated it, not an earlier pair that still shares a directory.
    let messages = expect_diagnosed(
        compile_moth_template(
            MothTemplateInput::Sources(vec![
                MothTemplateSource {
                    display_path: PathBuf::from("/docs/a.mtf"),
                    source_text: "first".to_owned(),
                },
                MothTemplateSource {
                    display_path: PathBuf::from("/docs/b.mtf"),
                    source_text: "second".to_owned(),
                },
                MothTemplateSource {
                    display_path: PathBuf::from("C:/docs/c.mtf"),
                    source_text: "third".to_owned(),
                },
            ]),
            &mut StringTable::new(),
        ),
        "a request whose third input eliminates the shared ancestor should be diagnosed",
    );

    assert_eq!(messages.counts().errors, 1);
    assert_eq!(
        messages
            .iter()
            .map(|(_, view)| view.descriptor().code)
            .collect::<Vec<_>>(),
        ["MOTH-IMPORT-0029"],
    );
    let rendered = format_terse_diagnostic_reports(&messages).join("\n");
    assert!(
        rendered.contains("a.mtf"),
        "the diagnostic should render the basis input: {rendered}",
    );
    assert!(
        rendered.contains("c.mtf"),
        "the diagnostic should render the conflicting input, not an earlier pair: {rendered}",
    );
    assert!(
        !rendered.contains("b.mtf"),
        "the diagnostic must not pair two inputs that still share a directory: {rendered}",
    );
}

// Windows drive letters are case-insensitive host syntax, so one request may spell the same
// drive differently. The shared ancestor must still be found and each relative identity derived
// from it, rather than a textual prefix strip that would keep the rooted spelling.
#[cfg(windows)]
#[test]
fn mixed_case_drive_letters_share_the_request_ancestor() {
    let output = compile_ok(MothTemplateInput::Sources(vec![
        MothTemplateSource {
            display_path: PathBuf::from("C:/docs/a.mtf"),
            source_text: "# A".to_owned(),
        },
        MothTemplateSource {
            display_path: PathBuf::from("c:/docs/b.mtf"),
            source_text: "# B".to_owned(),
        },
    ]));

    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| (
                document.source_path.clone(),
                document.relative_path.clone(),
                document.content.as_str(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                PathBuf::from("C:/docs/a.mtf"),
                Some(PathBuf::from("a.mtf")),
                "<h1>A</h1>"
            ),
            (
                PathBuf::from("c:/docs/b.mtf"),
                Some(PathBuf::from("b.mtf")),
                "<h1>B</h1>"
            ),
        ],
        "the shared drive root should yield plain file identities with distinct filenames",
    );
}

// Folding the drive letter must not fold ordinary directory names: authored spellings that
// differ in case keep distinct relative identities and module origins.
#[cfg(windows)]
#[test]
fn mixed_case_directory_names_keep_distinct_identities() {
    let output = compile_ok(MothTemplateInput::Sources(vec![
        MothTemplateSource {
            display_path: PathBuf::from("C:/Docs/a.mtf"),
            source_text: "# A".to_owned(),
        },
        MothTemplateSource {
            display_path: PathBuf::from("c:/docs/b.mtf"),
            source_text: "# B".to_owned(),
        },
    ]));

    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.relative_path.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(PathBuf::from("Docs/a.mtf")),
            Some(PathBuf::from("docs/b.mtf")),
        ],
        "only the drive prefix may fold case, so directory spellings stay distinct",
    );
}

// A duplicate identity can be spelled with different drive-letter case even though the supplied
// display paths differ. The duplicate diagnostic owns that identity collision.
#[cfg(windows)]
#[test]
fn mixed_case_drive_duplicate_identities_are_diagnosed() {
    let messages = expect_diagnosed(
        compile_moth_template(
            MothTemplateInput::Sources(vec![
                MothTemplateSource {
                    display_path: PathBuf::from("C:/docs/page.mtf"),
                    source_text: "first".to_owned(),
                },
                MothTemplateSource {
                    display_path: PathBuf::from("c:/docs/page.mtf"),
                    source_text: "second".to_owned(),
                },
            ]),
            &mut StringTable::new(),
        ),
        "two display paths that differ only in drive-letter case share one identity",
    );

    assert_eq!(messages.counts().errors, 1);
    let (report, view) = messages
        .iter()
        .next()
        .expect("duplicate diagnostic should be present");
    assert_eq!(view.descriptor().code, "MOTH-IMPORT-0028");
    let facts = view
        .family_view::<DuplicateMothTemplateInputPathView>()
        .expect("duplicate input diagnostic should retain its typed path fact");
    let context = DiagnosticRenderContext::for_report(report);
    let path_table = context
        .path_table()
        .expect("a retained report should own its path table");
    let mut scratch = Vec::new();
    assert_eq!(
        path_table.render_native_frozen(facts.path, report.context().strings(), &mut scratch),
        PathBuf::from("page.mtf"),
        "the fact should name the shared request-relative identity",
    );
    let rendered = format_terse_diagnostic_reports(&messages).join("\n");
    assert!(
        rendered.contains("page.mtf"),
        "the duplicate should render the shared identity: {rendered}",
    );
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

fn invalid_config_reason(reports: &DiagnosticReportSet) -> Option<&'static str> {
    reports.iter().find_map(|(_, view)| view.reason_key())
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
fn files_input_repeated_markdown_content_value_is_inlined_twice() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let temp_dir = temp_project(&[
        ("page.mtf", "About\n\n[@docs/legal.md]\n\n[@docs/legal.md]"),
        ("docs/legal.md", "Plain legal text."),
    ]);
    let entry_path =
        fs::canonicalize(temp_dir.path().join("page.mtf")).expect("entry should canonicalize");
    let content_path = fs::canonicalize(temp_dir.path().join("docs/legal.md"))
        .expect("content should canonicalize");
    let tracked_root = fs::canonicalize(temp_dir.path()).expect("fixture root should canonicalize");
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &tracked_root,
    );

    let output = compile_ok(MothTemplateInput::Files(vec![entry_path.clone()]));

    // Both references resolve to one source identity, so the source is registered and loaded
    // once and its retained snapshot is reused. Loading it a second time fails the slot's
    // single-write guard and aborts a valid compile.
    assert_eq!(output.documents.len(), 1);
    assert_eq!(
        output.documents[0]
            .content
            .matches("Plain legal text.")
            .count(),
        2,
        "a source referenced twice should inline twice: {}",
        output.documents[0].content
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&entry_path),
        1,
        "the direct entry should be prepared once across bundle construction and folding",
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&content_path),
        1,
        "a repeated content source should be prepared once and reused",
    );
}

/// A lexical `.mtf` spelling can resolve onto a `.md` target, so the entry's authored kind and the
/// kind its own canonical extension names disagree. A content reference that lands back on that
/// same canonical path must reuse the registered identity rather than assert the second kind,
/// which the source database rejects as a conflict.
#[cfg(unix)]
#[test]
fn content_reference_onto_the_entry_reuses_its_registered_kind() {
    let temp_dir = temp_project(&[("page.md", "# Page\n\n[@page.md]")]);
    std::os::unix::fs::symlink(
        temp_dir.path().join("page.md"),
        temp_dir.path().join("entry.mtf"),
    )
    .expect("the template spelling should symlink onto its Markdown target");
    let mut string_table = StringTable::new();

    let result = compile_moth_template(
        MothTemplateInput::Files(vec![temp_dir.path().join("entry.mtf")]),
        &mut string_table,
    );

    if let Err(errors) = result {
        let conflict = errors
            .iter()
            .any(|error| error.message.contains("conflicting kinds"));
        assert!(
            !conflict,
            "the entry's own canonical path must not be registered under a second kind"
        );
    }
}

/// `zeta.mtf` names `gamma` then `alpha`, so BFS/reference order is zeta, gamma, alpha.
/// Canonical logical order is alpha, gamma, zeta. Identities must follow the latter.
#[test]
fn content_source_identities_follow_canonical_logical_order_not_reference_order() {
    let temp_dir = temp_project(&[
        ("zeta.mtf", "# Zeta\n\n[@gamma.mtf]\n\n[@alpha.mtf]"),
        ("gamma.mtf", "# Gamma"),
        ("alpha.mtf", "# Alpha"),
    ]);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut units = MothTemplateInput::Files(vec![temp_dir.path().join("zeta.mtf")])
        .collect_sources(&mut string_table, &mut path_fork)
        .expect("the template unit should collect");
    assert_eq!(units.len(), 1);

    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut resource_inputs = ResourceInputRegistry::new();
    let bundle = prepare_file_value_bundle(
        &mut units[0],
        &style_directives,
        &mut string_table,
        &mut resource_inputs,
    )
    .expect("the content closure should prepare");

    let id_for = |name: &str| {
        let path = fs::canonicalize(temp_dir.path().join(name))
            .unwrap_or_else(|error| panic!("{name} should canonicalize: {error}"));
        bundle
            .source_files
            .sources()
            .get_by_canonical_path(&path)
            .unwrap_or_else(|| panic!("{name} should have a source identity"))
            .id
    };

    let alpha = id_for("alpha.mtf");
    let gamma = id_for("gamma.mtf");
    let zeta = id_for("zeta.mtf");
    assert!(
        alpha < gamma && gamma < zeta,
        "canonical order is alpha, gamma, zeta; got alpha={alpha:?}, gamma={gamma:?}, zeta={zeta:?}"
    );
}

/// A content source is registered before it is prepared, so a failure raised during its
/// preparation must retain both its exact source snapshot and final logical identity. The
/// diagnosed boundary owns the finalized source database after the walk stops.
#[test]
fn content_source_preparation_failures_name_the_logical_source() {
    let temp_dir = temp_project(&[
        ("page.mtf", "# Page\n\n[@docs/broken.mtf]"),
        (
            "docs/broken.mtf",
            "# Broken\n\n[$slot title = \"unterminated]\n",
        ),
    ]);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut units = MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")])
        .collect_sources(&mut string_table, &mut path_fork)
        .expect("the template unit should collect");

    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut resource_inputs = ResourceInputRegistry::new();
    let Err(bundle_failure) = prepare_file_value_bundle(
        &mut units[0],
        &style_directives,
        &mut string_table,
        &mut resource_inputs,
    ) else {
        panic!("an unterminated string in a content source should fail preparation");
    };
    let mut accumulator = BoundaryAccumulator::new();
    bundle_failure.append_to_boundary(&mut accumulator);
    let failure = accumulator
        .finish(InstallMode::Unfrozen)
        .expect("the preparation diagnostic should freeze");

    let (report, view) = failure
        .iter()
        .next()
        .expect("the preparation diagnostic should be present");
    let primary_site = view.primary_site();
    let span = match primary_site {
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
        SiteRef::Absent => panic!("the preparation diagnostic should retain its primary span"),
    };
    assert_eq!(
        report.context().retained_text(span.source()),
        Some("# Broken\n\n[$slot title = \"unterminated]\n"),
        "the diagnosed boundary should retain the exact content snapshot",
    );
    let position = DiagnosticRenderContext::for_report(report)
        .site_position(primary_site)
        .expect("the preparation diagnostic should retain its source span");
    assert_eq!(
        position.path,
        PathBuf::from("docs/broken.mtf"),
        "the failure should name the logical content source, got {:?}",
        position.path,
    );
}

/// Discovery and final registration use the same resolver for logical source paths, while a
/// resolved diagnostic remains separately owned from its reference row. Keep its source scope
/// aligned with the final logical source as the bundle settles its identities.
#[test]
fn retained_resolution_diagnostics_name_the_final_logical_source() {
    const SOURCE: &str = "# Page\n\n[@absent.md]";
    let temp_dir = temp_project(&[("page.mtf", SOURCE)]);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut units = MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")])
        .collect_sources(&mut string_table, &mut path_fork)
        .expect("the template unit should collect");

    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut resource_inputs = ResourceInputRegistry::new();
    let bundle = prepare_file_value_bundle(
        &mut units[0],
        &style_directives,
        &mut string_table,
        &mut resource_inputs,
    )
    .expect("an unresolvable reference is a retained diagnostic, not a bundle failure");

    let entry_source_id = bundle.prepared_entry.file_id;
    let diagnostic = bundle
        .resolved_file_references
        .iter()
        .find_map(|reference| match &reference.outcome {
            ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => Some(diagnostic),
            _ => None,
        })
        .expect("a reference to an absent target should retain a diagnostic");
    let compact = compact_resolved_failure_for_test(*diagnostic);
    let view = compact
        .first()
        .expect("the emitted resolution diagnostic should compact");
    let span = match view.primary_site() {
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
        SiteRef::Absent => panic!("the emitted diagnostic should retain its authored source span"),
    };
    assert_eq!(span.source(), entry_source_id);

    let mut source_builder = bundle.source_files;
    source_builder
        .finish_source(entry_source_id)
        .expect("the template source should finalize after preparation");
    source_builder
        .assemble_finalized_tables()
        .expect("the finalized template source table should assemble");
    let source_files = source_builder
        .finish()
        .expect("the already-finalized source database should validate");
    assert_eq!(source_files.retained_text(entry_source_id), Some(SOURCE));
    let range = span.byte_range(&source_files);
    let expected_start = SOURCE
        .find("@absent.md")
        .expect("fixture should contain the absent target path") as u32;
    assert_eq!(range.start(), expected_start);
    assert_eq!(range.end(), expected_start + "@absent.md".len() as u32);
}

#[test]
fn unreadable_content_value_surfaces_its_own_read_failure() {
    let temp_dir = temp_project(&[("page.mtf", "About\n\n[@docs/legal.md]")]);
    let unreadable = temp_dir.path().join("docs/legal.md");
    fs::create_dir_all(unreadable.parent().expect("content parent should exist"))
        .expect("content parent should be created");
    fs::write(&unreadable, [b'o', b'k', 0xff, b'\n']).expect("invalid UTF-8 should be written");
    let mut string_table = StringTable::new();

    let result = compile_moth_template(
        MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")]),
        &mut string_table,
    );
    let CompilationOutcome::Diagnosed(reports) =
        result.expect("invalid UTF-8 content is diagnosed")
    else {
        panic!("invalid UTF-8 content should diagnose rather than fail operationally");
    };
    assert_eq!(reports.counts().errors, 1);
    let (_, diagnostic) = reports
        .iter()
        .next()
        .expect("invalid UTF-8 content should diagnose once");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0040");
}

/// A failed content producer keeps its own diagnostic when a sibling content source cannot be
/// decoded. The walk pops `docs/broken.mtf` first, so the invalid UTF-8 `docs/legal.md` is only
/// collected. Its read failure must not replace the producer's diagnostic.
#[test]
fn failed_content_producer_keeps_its_diagnostic_beside_an_undecodable_sibling() {
    let temp_dir = temp_project(&[
        (
            "page.mtf",
            "# Page\n\n[@docs/broken.mtf]\n\n[@docs/legal.md]",
        ),
        (
            "docs/broken.mtf",
            "# Broken\n\n[$slot title = \"unterminated]\n",
        ),
    ]);
    fs::write(
        temp_dir.path().join("docs/legal.md"),
        [b'o', b'k', 0xff, b'\n'],
    )
    .expect("invalid UTF-8 should be written");
    let mut string_table = StringTable::new();

    let result = compile_moth_template(
        MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")]),
        &mut string_table,
    );
    let CompilationOutcome::Diagnosed(reports) =
        result.expect("a failed content producer is diagnosed")
    else {
        panic!("a failed content producer should diagnose rather than fail operationally");
    };
    assert_eq!(reports.counts().errors, 1);
    let (report, view) = reports
        .iter()
        .next()
        .expect("the producer diagnostic should be present");
    assert_eq!(
        view.descriptor().code,
        "MOTH-SYNTAX-0006",
        "the undecodable sibling must not replace the producer's unterminated string",
    );
    let position = DiagnosticRenderContext::for_report(report)
        .site_position(view.primary_site())
        .expect("the producer diagnostic should retain its source span");
    assert_eq!(
        position.path,
        PathBuf::from("docs/broken.mtf"),
        "the producer diagnostic should name its own content source, got {:?}",
        position.path,
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

// APFS and HFS+ reject non-UTF-8 file names, so filesystem-input non-UTF-8 coverage needs a
// Unix filesystem that stores arbitrary bytes. The in-memory test below needs only Unix's
// arbitrary-byte `OsString` constructor, not such a filesystem, so it also runs on macOS.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn directory_input_rejects_non_utf8_file_name_before_identity_conversion() {
    let temp_dir = temp_project(&[]);
    let docs = temp_dir.path().join("docs");
    fs::create_dir_all(&docs).expect("docs directory should be created");
    let bad_path = docs.join(non_utf8_file_name(b"bad\xFF.mtf"));
    fs::write(&bad_path, "bad").expect("non-UTF-8 source should be written");
    let canonical_bad_path =
        fs::canonicalize(&bad_path).expect("non-UTF-8 source should canonicalize");

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Directory {
            path: docs,
            recursive: false,
        },
        "a non-UTF-8 Moth template file name",
    );

    assert_eq!(
        errors.primary.kind,
        InfrastructureFailureKind::Filesystem,
        "the non-UTF-8 file name should stay a filesystem failure",
    );
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(canonical_bad_path.as_path()),
        "the failure should retain the native canonical source path",
    );
    let expected_message = format!(
        "Moth template source path {canonical_bad_path:?} contains a non-UTF-8 component; Moth \
         identity requires UTF-8 paths."
    );
    assert_eq!(
        errors.primary.message.as_ref(),
        expected_message.as_str(),
        "a unique non-UTF-8 path should fail the MTF identity gate before duplicate handling",
    );
}

#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn files_input_rejects_distinct_non_utf8_names_without_collapsing_them() {
    let temp_dir = temp_project(&[]);
    let first = temp_dir.path().join(non_utf8_file_name(b"bad\xFF.mtf"));
    let second = temp_dir.path().join(non_utf8_file_name(b"bad\xFE.mtf"));
    fs::write(&first, "first").expect("first non-UTF-8 source should be written");
    fs::write(&second, "second").expect("second non-UTF-8 source should be written");
    let canonical_first = fs::canonicalize(&first).expect("first source should canonicalize");

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Files(vec![first, second]),
        "distinct non-UTF-8 Moth template names",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(canonical_first.as_path()),
        "the first input's native path should be the one rejected",
    );
    assert!(
        errors.primary.message.contains("non-UTF-8"),
        "the rejection should name the non-UTF-8 component: {}",
        errors.primary.message,
    );
}

// A selected source root can sit below a directory whose name is not UTF-8. That ancestor is a
// host detail, not compiler identity: single-file, file-list, and directory requests identify
// their sources below that root, so every lane accepts the tree.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn non_utf8_ancestor_above_the_selected_source_root_is_accepted() {
    let temp_dir = temp_project(&[]);
    let source_root = temp_dir
        .path()
        .join(non_utf8_file_name(b"root\xFF"))
        .join("docs");
    fs::create_dir_all(&source_root).expect("non-UTF-8 ancestor directory should be created");
    let source_path = source_root.join("page.mtf");
    fs::write(&source_path, "# Page").expect("source should be written");
    let second_source_path = source_root.join("second.mtf");
    fs::write(&second_source_path, "# Second").expect("second source should be written");
    let canonical_source_path = fs::canonicalize(&source_path).expect("source should canonicalize");

    // A file-list request selects the same root through its files' common ancestor.
    let files_output = compile_ok(MothTemplateInput::Files(vec![
        source_path.clone(),
        second_source_path,
    ]));
    assert_eq!(files_output.documents.len(), 2);
    assert_eq!(
        files_output
            .documents
            .iter()
            .map(|document| document.relative_path.as_deref())
            .collect::<Vec<_>>(),
        vec![Some(Path::new("page.mtf")), Some(Path::new("second.mtf"))],
        "the shared ancestor, not its non-UTF-8 parent, anchors both relative identities",
    );

    let file_output = compile_ok(MothTemplateInput::File(source_path));
    assert_eq!(file_output.documents.len(), 1);
    assert_eq!(file_output.documents[0].content, "<h1>Page</h1>");
    assert_eq!(
        file_output.documents[0].relative_path, None,
        "a single-file request keeps the entry-root identity",
    );
    assert_eq!(
        file_output.documents[0].source_path, canonical_source_path,
        "the canonical native path should stay the document's host path",
    );

    let directory_output = compile_ok(MothTemplateInput::Directory {
        path: source_root,
        recursive: true,
    });
    assert_eq!(directory_output.documents.len(), 1);
    assert_eq!(directory_output.documents[0].content, "<h1>Page</h1>");
    assert_eq!(
        directory_output.documents[0].relative_path,
        Some(PathBuf::from("page.mtf")),
        "the selected root, not the non-UTF-8 ancestor, anchors the relative identity",
    );
    assert_eq!(
        directory_output.documents[0].source_path, canonical_source_path,
        "the canonical native path should stay the document's host path",
    );
}

// The duplicate fact is the request-relative identity, so a duplicate whose canonical ancestor
// is not UTF-8 still reports MOTH-IMPORT-0028 instead of failing the identity gate.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn duplicate_files_under_a_non_utf8_ancestor_stay_duplicate_diagnostics() {
    let temp_dir = temp_project(&[]);
    let docs = temp_dir
        .path()
        .join(non_utf8_file_name(b"root\xFF"))
        .join("docs");
    fs::create_dir_all(&docs).expect("non-UTF-8 ancestor directory should be created");
    let source_path = docs.join("page.mtf");
    fs::write(&source_path, "# Page").expect("source should be written");
    let canonical_path = fs::canonicalize(&source_path).expect("source should canonicalize");
    let alternate_spelling = docs.join(".").join("page.mtf");

    let messages = expect_diagnosed(
        compile_moth_template(
            MothTemplateInput::Files(vec![canonical_path, alternate_spelling]),
            &mut StringTable::new(),
        ),
        "a duplicate below a non-UTF-8 ancestor should remain a duplicate diagnostic",
    );

    assert_eq!(messages.counts().errors, 1);
    let (report, view) = messages
        .iter()
        .next()
        .expect("duplicate diagnostic should be present");
    assert_eq!(view.descriptor().code, "MOTH-IMPORT-0028");
    let facts = view
        .family_view::<DuplicateMothTemplateInputPathView>()
        .expect("duplicate input diagnostic should retain its typed path fact");
    let context = DiagnosticRenderContext::for_report(report);
    let path_table = context
        .path_table()
        .expect("a retained report should own its path table");
    let mut scratch = Vec::new();
    assert_eq!(
        path_table.render_native_frozen(facts.path, report.context().strings(), &mut scratch),
        PathBuf::from("page.mtf"),
        "the fact should be the UTF-8 identity below the non-UTF-8 ancestor",
    );
    let rendered = format_terse_diagnostic_reports(&messages).join("\n");
    assert!(
        rendered.contains("page.mtf") && !rendered.contains("non-UTF-8"),
        "the duplicate should render its request-relative identity, not an identity failure: \
         {rendered}",
    );
}

#[test]
fn duplicate_files_reuse_the_relative_path_when_the_path_table_is_full() {
    let temp_dir = temp_project(&[("page.mtf", "# Page")]);
    let source_path = temp_dir.path().join("page.mtf");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    path_fork
        .try_intern_filesystem_path(Path::new("page.mtf"), &mut string_table)
        .expect("the request-relative path should fit before exhaustion");

    let input = MothTemplateInput::Files(vec![source_path.clone(), source_path]);
    let failure = {
        let _guard = ForcedExhaustionGuard::new();
        input.collect_sources(&mut string_table, &mut path_fork)
    };
    let Err(PreparedFailure::Diagnosed(batch)) = failure else {
        panic!("a duplicate should remain a diagnosed rejection");
    };
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    let reports = accumulator
        .finish(InstallMode::Unfrozen)
        .expect("the duplicate diagnostic should freeze");
    let (report, view) = reports.iter().next().expect("missing duplicate diagnostic");
    assert_eq!(view.descriptor().code, "MOTH-IMPORT-0028");
    let facts = view
        .family_view::<DuplicateMothTemplateInputPathView>()
        .expect("duplicate diagnostic should carry its path");
    let context = DiagnosticRenderContext::for_report(report);
    assert_eq!(
        context.path_table().unwrap().render_native_frozen(
            facts.path,
            report.context().strings(),
            &mut Vec::new(),
        ),
        PathBuf::from("page.mtf"),
    );
}

#[cfg(unix)]
#[test]
fn literal_backslash_file_names_keep_distinct_identities_and_deterministic_order() {
    let temp_dir = temp_project(&[("docs/a/b.mtf", "nested"), ("docs/a\\b.mtf", "literal")]);
    let docs = temp_dir.path().join("docs");
    let literal_path = docs.join("a\\b.mtf");
    let nested_path = docs.join("a").join("b.mtf");

    let directory_output = compile_ok(MothTemplateInput::Directory {
        path: docs,
        recursive: true,
    });
    assert_eq!(
        directory_output
            .documents
            .iter()
            .map(|document| document.relative_path.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(PathBuf::from("a/b.mtf")),
            Some(PathBuf::from("a\\b.mtf"))
        ],
        "a literal backslash name must not be reinterpreted as a directory separator",
    );
    assert_eq!(
        directory_output
            .documents
            .iter()
            .map(|document| document.content.as_str())
            .collect::<Vec<_>>(),
        vec!["<p>nested</p>", "<p>literal</p>"],
        "directory ordering should follow the native relative paths",
    );

    let files_output = compile_ok(MothTemplateInput::Files(vec![literal_path, nested_path]));
    assert_eq!(
        files_output
            .documents
            .iter()
            .map(|document| document.relative_path.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(PathBuf::from("a\\b.mtf")),
            Some(PathBuf::from("a/b.mtf"))
        ],
        "a file list keeps caller order while preserving both identities",
    );
}

#[cfg(unix)]
#[test]
fn literal_backslash_document_name_referencing_resource_reports_typed_failure() {
    let temp_dir = temp_project(&[
        ("a\\b.mtf", "# Page\n\n[@logo.svg]"),
        ("logo.svg", "<svg/>"),
    ]);
    let source_path = temp_dir.path().join("a\\b.mtf");
    let canonical_source_path =
        fs::canonicalize(&source_path).expect("literal-backslash source should canonicalize");
    let document_path = Path::new("a\\b.mtf");

    let errors = expect_infrastructure_failure(
        MothTemplateInput::File(source_path),
        "a document URL context with a literal backslash",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(canonical_source_path.as_path()),
        "the canonical native source path should be the host path",
    );
    assert!(
        errors
            .primary
            .message
            .contains(&format!("document path {document_path:?}")),
        "the Debug spelling should keep the literal backslash visible: {}",
        errors.primary.message,
    );
}

// Site-root pieces do not depend on the document URL context, so this document must compile even
// though its native name could not anchor a relative resource URL.
#[cfg(unix)]
#[test]
fn literal_backslash_document_name_without_resources_skips_url_context_validation() {
    let temp_dir = temp_project(&[("a\\b.mtf", "Site root: [@/]")]);

    let output = compile_ok(MothTemplateInput::File(temp_dir.path().join("a\\b.mtf")));

    assert_eq!(output.documents[0].content, "<p>Site root: /</p>");
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

#[cfg(unix)]
#[test]
fn in_memory_sources_reject_non_utf8_display_paths() {
    let display_path = PathBuf::from(non_utf8_file_name(b"memory/bad\xFF.mtf"));

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Sources(vec![MothTemplateSource {
            display_path: display_path.clone(),
            source_text: "bad".to_owned(),
        }]),
        "a non-UTF-8 in-memory display path",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(display_path.as_path()),
        "the in-memory logical name should be rejected exactly as supplied",
    );
    assert!(
        errors.primary.message.contains("non-UTF-8"),
        "the rejection should name the non-UTF-8 component: {}",
        errors.primary.message,
    );
}

// The portable-name rule still applies beside a non-UTF-8 ancestor: the UTF-8 identity below a
// rooted name must separate its components with `/`, never a literal backslash.
#[cfg(unix)]
#[test]
fn rooted_in_memory_names_with_non_utf8_ancestors_still_reject_backslashes() {
    let display_path = PathBuf::from(non_utf8_file_name(b"/\xFF/x\\y.mtf"));

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Sources(vec![MothTemplateSource {
            display_path: display_path.clone(),
            source_text: "bad".to_owned(),
        }]),
        "a rooted in-memory name with a non-UTF-8 ancestor and a backslash",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(display_path.as_path()),
        "the in-memory logical name should be rejected exactly as supplied",
    );
    assert!(
        errors.primary.message.contains("contains a backslash"),
        "the rejection should name the portable separator rule: {}",
        errors.primary.message,
    );
}

// Windows parses `\` as a separator while Unix keeps it inside one filename component, so the
// portable-name contract must reject that native spelling under either host parser.
#[test]
fn in_memory_source_names_require_forward_slash_separators() {
    let portable_path = PathBuf::from("docs/intro.mtf");
    let output = compile_ok(MothTemplateInput::Sources(vec![MothTemplateSource {
        display_path: portable_path.clone(),
        source_text: "intro".to_owned(),
    }]));
    assert_eq!(
        output.documents[0].relative_path,
        Some(portable_path),
        "a `/`-separated relative name should pass through unchanged",
    );

    let display_path = PathBuf::from("docs\\intro.mtf");
    let errors = expect_infrastructure_failure(
        MothTemplateInput::Sources(vec![MothTemplateSource {
            display_path: display_path.clone(),
            source_text: "intro".to_owned(),
        }]),
        "a backslash-separated relative in-memory name",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(display_path.as_path()),
        "the rejected portable name should be reported exactly as supplied",
    );
    let offending_name = format!("source name {display_path:?}");
    assert!(
        errors.primary.message.contains(&offending_name),
        "the rejection should show the backslash instead of a portable rewrite: {}",
        errors.primary.message,
    );
    assert!(
        errors.primary.message.contains("contains a backslash"),
        "the rejection should explain the portable separator contract: {}",
        errors.primary.message,
    );
}

#[test]
fn absolute_in_memory_display_paths_keep_their_display_identity() {
    let temp_dir = temp_project(&[]);
    // Absolute in-memory names obey the portable spelling contract: native host prefix and root,
    // then `/` separators.
    let first = PathBuf::from(portable_path_text(temp_dir.path().join("memory/intro.mtf")));
    let second = PathBuf::from(portable_path_text(temp_dir.path().join("memory/guide.mtf")));

    let output = compile_ok(MothTemplateInput::Sources(vec![
        MothTemplateSource {
            display_path: first.clone(),
            source_text: "intro".to_owned(),
        },
        MothTemplateSource {
            display_path: second.clone(),
            source_text: "guide".to_owned(),
        },
    ]));

    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.source_path.clone())
            .collect::<Vec<_>>(),
        vec![first, second],
        "absolute display paths are logical inputs and must not be canonicalized",
    );
    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| document.relative_path.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(PathBuf::from("intro.mtf")),
            Some(PathBuf::from("guide.mtf"))
        ],
        "the common display ancestor should yield request-relative names",
    );
}

#[test]
fn prefixless_rooted_in_memory_display_paths_keep_their_display_identity() {
    // A leading `/` without a drive prefix is rooted on every host, so Windows must derive the
    // same request-relative names Unix already does instead of treating the name as relative.
    let first = PathBuf::from("/docs/intro.mtf");
    let second = PathBuf::from("/docs/guide.mtf");

    let output = compile_ok(MothTemplateInput::Sources(vec![
        MothTemplateSource {
            display_path: first.clone(),
            source_text: "# Intro".to_owned(),
        },
        MothTemplateSource {
            display_path: second.clone(),
            source_text: "# Guide".to_owned(),
        },
    ]));

    assert_eq!(
        output
            .documents
            .iter()
            .map(|document| (
                document.source_path.clone(),
                document.relative_path.clone(),
                document.content.as_str(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (first, Some(PathBuf::from("intro.mtf")), "<h1>Intro</h1>"),
            (second, Some(PathBuf::from("guide.mtf")), "<h1>Guide</h1>"),
        ],
        "rooted display paths should keep their identity and use the common ancestor",
    );
}

#[test]
fn in_memory_names_without_a_file_component_are_rejected_as_input() {
    let display_path = PathBuf::from("/");

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Sources(vec![MothTemplateSource {
            display_path: display_path.clone(),
            source_text: "# Root".to_owned(),
        }]),
        "a bare root in-memory display path",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(errors.primary.stage, CompilerStage::SourceLoading);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(display_path.as_path()),
        "the rejected input name should be reported exactly as supplied",
    );
    assert!(
        errors.primary.message.contains("no source file component"),
        "the rejection should explain the missing source identity: {}",
        errors.primary.message,
    );
}

#[test]
fn in_memory_source_names_reject_dot_and_parent_components() {
    for display_path in [
        ".",
        "..",
        "/.",
        "/..",
        "./nested.mtf",
        "../nested.mtf",
        "a/./b.mtf",
        "a/../b.mtf",
        "a/.",
        "a/..",
    ] {
        let display_path = PathBuf::from(display_path);
        let errors = expect_infrastructure_failure(
            MothTemplateInput::Sources(vec![MothTemplateSource {
                display_path: display_path.clone(),
                source_text: "# Source".to_owned(),
            }]),
            "a dot or parent component in an in-memory display path",
        );

        assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
        assert_eq!(errors.primary.stage, CompilerStage::SourceLoading);
        assert_eq!(
            errors.primary.host_path.as_deref(),
            Some(display_path.as_path()),
            "the rejected input name should be reported exactly as supplied",
        );
        assert!(
            errors.primary.message.contains("path component"),
            "the rejection should explain the component contract: {}",
            errors.primary.message,
        );
    }
}

#[test]
fn in_memory_source_names_reject_directory_like_spellings() {
    for display_path in ["docs/", "/docs/", "C:/docs/", "docs//"] {
        let display_path = PathBuf::from(display_path);
        let errors = expect_infrastructure_failure(
            MothTemplateInput::Sources(vec![MothTemplateSource {
                display_path: display_path.clone(),
                source_text: "# Source".to_owned(),
            }]),
            "a directory-like in-memory display path",
        );

        assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
        assert_eq!(errors.primary.stage, CompilerStage::SourceLoading);
        assert_eq!(
            errors.primary.host_path.as_deref(),
            Some(display_path.as_path()),
            "the rejected input name should be reported exactly as supplied",
        );
        assert!(
            errors.primary.message.contains("directory separator"),
            "the rejection should explain the directory-like spelling: {}",
            errors.primary.message,
        );
    }
}

// Windows alone parses `C:docs/intro.mtf` as a drive prefix without a root. Such a name has no
// portable identity, so it must be rejected as input instead of reaching the relative branch and
// failing later as an internal logical-path error.
#[cfg(windows)]
#[test]
fn drive_relative_in_memory_names_are_rejected_as_input() {
    let display_path = PathBuf::from("C:docs/intro.mtf");

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Sources(vec![MothTemplateSource {
            display_path: display_path.clone(),
            source_text: "# Intro".to_owned(),
        }]),
        "a drive-relative in-memory display path",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(errors.primary.stage, CompilerStage::SourceLoading);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(display_path.as_path()),
        "the rejected input name should be reported exactly as supplied",
    );
    assert!(
        errors
            .primary
            .message
            .contains("drive prefix without a root"),
        "the rejection should explain the missing portable basis: {}",
        errors.primary.message,
    );
}

// Windows parses `\C:guide.mtf` as a root plus a normal component because a drive prefix is only
// recognised at the start of the path. The common-ancestor rule then exposes `C:guide.mtf` as
// the relative identity, which Windows parses as drive-relative.
#[cfg(windows)]
#[test]
fn rooted_in_memory_names_whose_suffix_is_drive_relative_are_rejected_as_input() {
    let display_path = PathBuf::from(r"\C:guide.mtf");

    let errors = expect_infrastructure_failure(
        MothTemplateInput::Sources(vec![MothTemplateSource {
            display_path: display_path.clone(),
            source_text: "# Guide".to_owned(),
        }]),
        "a rooted in-memory name whose relative suffix is drive-relative",
    );

    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(errors.primary.stage, CompilerStage::SourceLoading);
    assert_eq!(
        errors.primary.host_path.as_deref(),
        Some(display_path.as_path()),
        "the rejection should name the original input, not the derived suffix",
    );
    assert!(
        errors.primary.message.contains("drive prefix"),
        "the rejection should explain the drive-relative suffix: {}",
        errors.primary.message,
    );
}

// Windows parses `//server/share` as a UNC prefix with an implicit root and no source file
// component. Its implicit root occupies no bytes of the spelling, so offset arithmetic used to
// panic while locating the portable suffix.
#[cfg(windows)]
#[test]
fn bare_unc_in_memory_names_are_rejected_as_input() {
    for display_path in ["//server/share", r"\\server\share"] {
        let display_path = PathBuf::from(display_path);

        let errors = expect_infrastructure_failure(
            MothTemplateInput::Sources(vec![MothTemplateSource {
                display_path: display_path.clone(),
                source_text: "# Share".to_owned(),
            }]),
            "a bare UNC share in-memory display path",
        );

        assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
        assert_eq!(errors.primary.stage, CompilerStage::SourceLoading);
        assert_eq!(
            errors.primary.host_path.as_deref(),
            Some(display_path.as_path()),
            "the rejected input name should be reported exactly as supplied",
        );
        assert!(
            errors.primary.message.contains("no source file component"),
            "the rejection should explain the missing source identity: {}",
            errors.primary.message,
        );
    }
}

#[test]
fn template_bundle_source_and_header_paths_survive_success_finalization() {
    let temp_dir = temp_project(&[
        ("page.mtf", "About\n\n[@docs/legal.md]"),
        ("docs/legal.md", "# Legal"),
    ]);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut units = MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")])
        .collect_sources(&mut string_table, &mut path_fork)
        .expect("the template unit should collect");
    let source_path = units[0].source_path.clone();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut resource_inputs = ResourceInputRegistry::new();
    let bundle = prepare_file_value_bundle(
        &mut units[0],
        &style_directives,
        &mut string_table,
        &mut resource_inputs,
    )
    .expect("the content closure should prepare");

    let source_table = bundle.source_files.sources();
    let entry_source = source_table
        .get_by_canonical_path(&source_path)
        .expect("the finalized source database should register the entry");
    assert_eq!(
        entry_source.logical_path, bundle.prepared_entry.source_file,
        "the entry source record and prepared output should share one PathId",
    );
    let mut prepared_paths = Vec::new();
    let mut header_count = 0;
    for prepared in
        std::iter::once(&bundle.prepared_entry).chain(bundle.prepared_content_sources.iter())
    {
        prepared_paths.push(prepared.source_file);
        let source_owner = prepared.source_token_stream.as_ref();
        if source_owner.is_none() {
            assert_eq!(
                prepared.token_count, 0,
                "payload-only Markdown sources must not retain token owners",
            );
        }
        for header in &prepared.headers {
            header_count += 1;
            assert_eq!(
                header.tokens.source(),
                prepared.file_id,
                "header token range should use its prepared source identity",
            );
            if let Some(source_owner) = source_owner {
                assert_eq!(
                    source_owner.source(),
                    prepared.file_id,
                    "prepared token owner should use its prepared source identity",
                );
                assert!(
                    source_table
                        .paths()
                        .starts_with(header.declaration_path, prepared.source_file),
                    "header declaration path should remain rooted at its prepared file",
                );
            } else {
                assert!(
                    header.tokens.is_empty() && header.token_sequence.is_none(),
                    "payload-only Markdown headers must not retain token ranges",
                );
            }
            prepared_paths.push(prepared.source_file);
            prepared_paths.push(header.declaration_path);
        }
        prepared_paths.extend(
            prepared
                .path_syntax
                .table()
                .paths()
                .iter()
                .map(|path| path.root),
        );
    }
    assert!(
        header_count > 0,
        "the bundle should retain at least one header"
    );
    prepared_paths.sort();
    prepared_paths.dedup();

    let mut scratch = Vec::new();
    let expected_rendered_paths = prepared_paths
        .iter()
        .map(|path| {
            assert!(
                source_table.paths().contains(*path),
                "prepared path should belong to the finalized source table before folding",
            );
            source_table
                .paths()
                .render_portable(*path, &string_table, &mut scratch)
        })
        .collect::<Vec<_>>();

    let folded = compile_moth_template_source(
        MothTemplateCompilationRequest {
            source_path: &source_path,
            source_code: None,
            style_directives: &style_directives,
            file_value_resolution: Some(bundle),
        },
        &mut string_table,
    )
    .expect("the finalized bundle should compile successfully");

    let final_paths = folded.source_database.paths();
    for (path, expected) in prepared_paths.iter().zip(expected_rendered_paths) {
        assert!(
            final_paths.contains(*path),
            "prepared path should survive the successful source freeze: {expected:?}",
        );
        assert_eq!(
            final_paths.render_portable(*path, &string_table, &mut scratch),
            expected,
            "successful folding should preserve the prepared path spelling",
        );
    }
}

#[test]
fn duplicate_canonical_files_are_diagnostics() {
    let temp_dir = temp_project(&[("intro.mtf", "intro")]);
    let canonical_path =
        fs::canonicalize(temp_dir.path().join("intro.mtf")).expect("source should canonicalize");
    let alternate_spelling = temp_dir.path().join(".").join("intro.mtf");
    let mut string_table = StringTable::new();

    let messages = expect_diagnosed(
        compile_moth_template(
            MothTemplateInput::Files(vec![canonical_path.clone(), alternate_spelling]),
            &mut string_table,
        ),
        "duplicate canonical input paths should fail",
    );

    assert_eq!(messages.counts().errors, 1);
    let (report, diagnostic) = messages
        .iter()
        .next()
        .expect("duplicate diagnostic should be present");
    assert_eq!(diagnostic.descriptor().code, "MOTH-IMPORT-0028");
    assert_eq!(diagnostic.durable_labels().count(), 1);
    let facts = diagnostic
        .family_view::<DuplicateMothTemplateInputPathView>()
        .expect("duplicate input diagnostic should retain its typed path fact");
    let context = DiagnosticRenderContext::for_report(report);
    let path_table = context
        .path_table()
        .expect("a retained report should own its path table");
    let mut scratch = Vec::new();
    assert_eq!(
        path_table.render_native_frozen(facts.path, report.context().strings(), &mut scratch),
        canonical_path,
        "the fact should name the canonical native path once, not either spelling",
    );
    let rendered = format_terse_diagnostic_reports(&messages).join("\n");
    assert!(
        rendered.contains("intro.mtf"),
        "duplicate diagnostics should render the actual input path: {rendered}",
    );
}

#[test]
fn ast_prepared_failure_keeps_type_names_and_source_snapshot() {
    let source = "[:[true + 1]]";
    let temp_dir = temp_project(&[("broken.mtf", source)]);
    let mut string_table = StringTable::new();

    let failure = expect_diagnosed(
        compile_moth_template(
            MothTemplateInput::File(temp_dir.path().join("broken.mtf")),
            &mut string_table,
        ),
        "incompatible interpolation operands should fail AST folding",
    );

    assert_eq!(failure.counts().errors, 1);
    let render_result: CompilationResult<()> = Ok(CompilationOutcome::Diagnosed(failure));
    let rendered = render_compilation_result_html(&render_result, temp_dir.path());
    assert!(
        rendered.contains("Bool") && rendered.contains("I32"),
        "the prepared AST diagnostic should retain its type display names: {rendered}",
    );
    assert!(
        rendered.contains(r#"<span class="source-line">[:[true + 1]]</span>"#),
        "the prepared AST diagnostic should retain its exact source snapshot: {rendered}",
    );
}

#[test]
fn compile_api_does_not_write_artifacts() {
    let temp_dir = temp_project(&[("intro.mtf", "intro")]);
    let before = directory_entries(temp_dir.path());

    let _output = compile_ok(MothTemplateInput::File(temp_dir.path().join("intro.mtf")));

    assert_eq!(directory_entries(temp_dir.path()), before);
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
