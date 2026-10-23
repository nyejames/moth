//! Tests for the direct HTML-project Moth template API.
//!
//! WHAT: covers input normalization, AST-only compilation, ordering, and duplicate diagnostics.
//! WHY: this API is intentionally not wired into project builds yet, so module-local tests protect
//! the tooling-facing boundary without adding integration artifacts.

use super::bundle::prepare_file_value_bundle;
use crate::build_system::create_project_modules::resource_inputs::{
    ResourceContentState, ResourceInputRegistry,
};
use crate::compiler_frontend::compiler_errors::{BoundaryFailure, ErrorType};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, dev_server::render_boundary_messages_html,
    terse::format_terse_boundary_messages,
};
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
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::moth_template::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput, MothTemplateInput, MothTemplateSource,
    compile_moth_template,
};
use crate::projects::html_project::style_directives::html_project_style_directives;
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

fn compile_ok(input: MothTemplateInput) -> MothTemplateCompileOutput {
    let mut string_table = StringTable::new();
    compile_moth_template(input, &mut string_table).expect("Moth template input should compile")
}

#[test]
fn warnings_keep_document_snapshots_on_success_and_later_preparation_failure() {
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
        let failure = if include_failure {
            result.expect_err("the third document should fail preparation")
        } else {
            let output = result.expect("both HTML documents should fold with warnings");
            let mut accumulator = BoundaryAccumulator::with_root_string_table(&string_table);
            for warning in output.warnings {
                accumulator.append_prepared(warning.batch, Some((warning.source_database, None)));
            }
            accumulator.finish(InstallMode::Unfrozen)
        };

        let mut expected_codes = vec!["MOTH-SYNTAX-0005", "MOTH-SYNTAX-0005"];
        if include_failure {
            expected_codes.push("MOTH-SYNTAX-0032");
        }
        assert_eq!(
            failure
                .reports
                .iter()
                .map(|(_, view)| view.descriptor().code)
                .collect::<Vec<_>>(),
            expected_codes,
        );
        assert_eq!(failure.warning_count(), 2);
        assert_eq!(failure.error_count(), if include_failure { 1 } else { 0 });

        let expected_source_lines = [first_source, second_source, "]"];
        for ((report, view), expected_source_line) in
            failure
                .reports
                .iter()
                .zip(
                    expected_source_lines
                        .into_iter()
                        .take(if include_failure { 3 } else { 2 }),
                )
        {
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
        let rendered = render_boundary_messages_html(&failure, temp_dir.path());
        assert!(rendered.contains(
            r#"<span class="source-line">[$html: &lt;script&gt;first_snapshot()&lt;/script&gt;]</span>"#
        ));
        assert!(rendered.contains(
            r#"<span class="source-line">[$html: &lt;script&gt;second_snapshot()&lt;/script&gt;]</span>"#
        ));
        if include_failure {
            assert!(rendered.contains(r#"<span class="source-line">]</span>"#));
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

    let messages = compile_moth_template(
        MothTemplateInput::Directory {
            path: temp_dir.path().to_path_buf(),
            recursive: true,
        },
        &mut string_table,
    )
    .expect_err("distinct origins claiming one output path should fail");

    assert_eq!(
        invalid_config_reason(&messages),
        Some("invalid_config.resource_output_path_collision"),
        "expected a resource output path collision reason",
    );
    let rendered = format_terse_boundary_messages(&messages).join("\n");
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

    let messages = super::compile::compile_moth_template_with_registry(
        MothTemplateInput::Directory {
            path: temp_dir.path().to_path_buf(),
            recursive: true,
        },
        &mut string_table,
        &mut resource_inputs,
    )
    .expect_err("distinct origins claiming one output path should fail");

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
fn mixed_relative_and_absolute_in_memory_paths_are_diagnosed() {
    let temp_dir = temp_project(&[]);
    let mut string_table = StringTable::new();

    let messages = compile_moth_template(
        MothTemplateInput::Sources(vec![
            MothTemplateSource {
                display_path: PathBuf::from("memory/intro.mtf"),
                source_text: "first".to_owned(),
            },
            MothTemplateSource {
                display_path: temp_dir.path().join("absolute.mtf"),
                source_text: "second".to_owned(),
            },
        ]),
        &mut string_table,
    )
    .expect_err("mixed relative and absolute display paths have one portable basis");

    assert_eq!(messages.error_count(), 1);
    assert_eq!(
        messages
            .reports
            .iter()
            .map(|(_, view)| view.descriptor().code)
            .collect::<Vec<_>>(),
        ["MOTH-IMPORT-0029"],
    );
    let rendered = format_terse_boundary_messages(&messages).join("\n");
    assert!(
        rendered.contains("memory/intro.mtf"),
        "the diagnostic should render its first actual input path: {rendered}",
    );
    assert!(
        rendered.contains("absolute.mtf"),
        "the diagnostic should render its second actual input path: {rendered}",
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

fn invalid_config_reason(failure: &BoundaryFailure) -> Option<&'static str> {
    failure
        .reports
        .iter()
        .find_map(|(_, view)| view.reason_key())
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
        "<h1 id=\"page\">Page</h1><p><h1 id=\"nested\">Nested</h1></p>"
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

    if let Err(failure) = result {
        let conflict = failure.infrastructure_errors().is_some_and(|errors| {
            errors
                .iter()
                .any(|error| error.msg.contains("conflicting kinds"))
        });
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

    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("style directives should merge");
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
        ("docs/broken.mtf", "# Broken\n\n[$insert(\"unterminated]\n"),
    ]);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut units = MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")])
        .collect_sources(&mut string_table, &mut path_fork)
        .expect("the template unit should collect");

    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("style directives should merge");
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
    let failure = accumulator.finish(InstallMode::Unfrozen);

    let (report, view) = failure
        .reports
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
        Some("# Broken\n\n[$insert(\"unterminated]\n"),
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

/// A canonical content target can differ from the authored spelling when a symlink is involved.
/// If that target fails before preparation, final registration still adds its logical path; the
/// failure cause owner must retain that path in the same table as the discovery fork.
#[cfg(unix)]
#[test]
fn aborted_content_walk_retains_unvisited_canonical_source_identity() {
    use std::os::unix::fs::symlink;

    let temp_dir = temp_project(&[
        ("page.mtf", "# Page\n\n[@alias.md]"),
        ("real.md", "placeholder"),
    ]);
    let real_path = temp_dir.path().join("real.md");
    fs::write(&real_path, [0xff]).expect("invalid UTF-8 should be written");
    symlink(&real_path, temp_dir.path().join("alias.md"))
        .expect("the authored alias should point at the unreadable target");
    let canonical_real_path =
        fs::canonicalize(&real_path).expect("the canonical target path should resolve");

    let mut string_table = StringTable::new();
    let Err(messages) = compile_moth_template(
        MothTemplateInput::File(temp_dir.path().join("page.mtf")),
        &mut string_table,
    ) else {
        panic!("the unreadable canonical content target should fail discovery");
    };

    let infrastructure = messages
        .infrastructure_errors()
        .expect("the aborted walk should retain its infrastructure causes");
    let identity = infrastructure
        .frozen_identity_for_cause(0)
        .expect("the read failure should retain its finalized source identity");
    // The reserved root is index 0. Final logical order puts page.mtf before real.md.
    let canonical_target_source = SourceId::from_index(2);
    let source_slot = identity
        .get(canonical_target_source)
        .expect("the unreadable canonical target should retain its source slot");
    assert_eq!(
        source_slot.canonical_os_path.as_deref(),
        Some(canonical_real_path.as_path()),
        "the retained identity slot should name the canonical target",
    );
    let logical_path = identity
        .source_logical_path(source_slot.id)
        .expect("the canonical target should retain its logical path");
    assert_eq!(
        identity.render_path(logical_path, &mut Vec::new()),
        "real.md",
    );
}

/// Discovery and final registration use the same resolver for logical source paths, while a
/// resolved diagnostic remains separately owned from its reference row. Keep its source scope
/// aligned with the final logical source as the bundle settles its identities.
#[test]
fn retained_resolution_diagnostics_name_the_final_logical_source() {
    let temp_dir = temp_project(&[("page.mtf", "# Page\n\n[@absent.md]")]);
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut units = MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")])
        .collect_sources(&mut string_table, &mut path_fork)
        .expect("the template unit should collect");

    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("style directives should merge");
    let mut resource_inputs = ResourceInputRegistry::new();
    let bundle = prepare_file_value_bundle(
        &mut units[0],
        &style_directives,
        &mut string_table,
        &mut resource_inputs,
    )
    .expect("an unresolvable reference is a retained diagnostic, not a bundle failure");

    let diagnostic = bundle
        .resolved_file_references
        .iter()
        .find_map(|reference| match &reference.outcome {
            ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => Some(diagnostic),
            _ => None,
        })
        .expect("a reference to an absent target should retain a diagnostic");

    assert!(
        diagnostic.primary_span.is_some(),
        "the retained resolution diagnostic should keep its authored source span",
    );
}

#[test]
fn unreadable_content_value_surfaces_its_own_read_failure() {
    let temp_dir = temp_project(&[("page.mtf", "About\n\n[@docs/legal.md]")]);
    let unreadable = temp_dir.path().join("docs/legal.md");
    fs::create_dir_all(unreadable.parent().expect("content parent should exist"))
        .expect("content parent should be created");
    fs::write(&unreadable, [b'o', b'k', 0xff, b'\n']).expect("invalid UTF-8 should be written");
    let mut string_table = StringTable::new();

    let messages = compile_moth_template(
        MothTemplateInput::Files(vec![temp_dir.path().join("page.mtf")]),
        &mut string_table,
    )
    .expect_err("an unreadable content source should fail the compile");

    // The read now happens before preparation, so the recorded failure must be replayed at the
    // preparation boundary. Reporting the slot's own "no retained text" state instead would
    // replace the user-facing read failure with an internal compiler error.
    let error = messages
        .infrastructure_errors()
        .and_then(|errors| errors.iter().next())
        .expect("the read failure should be reported");
    assert_eq!(error.error_type, ErrorType::File);
}

#[test]
fn file_input_compiles_one_moth_template_file() {
    let temp_dir = temp_project(&[("intro.mtf", "# Intro")]);
    let source_path = temp_dir.path().join("intro.mtf");

    let output = compile_ok(MothTemplateInput::File(source_path.clone()));

    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].content, "<h1 id=\"intro\">Intro</h1>");
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
    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("style directives should merge");
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
fn duplicate_source_paths_are_diagnostics() {
    let temp_dir = temp_project(&[("intro.mtf", "intro")]);
    let path = temp_dir.path().join("intro.mtf");
    let mut string_table = StringTable::new();

    let messages = compile_moth_template(
        MothTemplateInput::Files(vec![path.clone(), path]),
        &mut string_table,
    )
    .expect_err("duplicate input paths should fail");

    assert_eq!(messages.error_count(), 1);
    let (_, diagnostic) = messages
        .reports
        .iter()
        .next()
        .expect("duplicate diagnostic should be present");
    assert_eq!(diagnostic.descriptor().code, "MOTH-IMPORT-0028");
    assert_eq!(diagnostic.durable_labels().count(), 1);
    let rendered = format_terse_boundary_messages(&messages).join("\n");
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

    let failure = compile_moth_template(
        MothTemplateInput::File(temp_dir.path().join("broken.mtf")),
        &mut string_table,
    )
    .expect_err("incompatible interpolation operands should fail AST folding");

    assert_eq!(failure.error_count(), 1);
    let rendered = render_boundary_messages_html(&failure, temp_dir.path());
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
    assert_eq!(output.documents[0].content, "<h1 id=\"nested\">Nested</h1>");
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
