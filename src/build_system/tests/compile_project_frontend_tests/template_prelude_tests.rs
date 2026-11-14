//! Directory, single-file and check-only coverage for the fixed `.mtf` constant prelude.
//!
//! WHAT: proves the official Core prelude packages bind under the HTML and minimal
//!       mandatory-core builder surfaces, and that both providers reach templates in every
//!       compilation mode.
//! WHY: the prelude is compiler policy tied to the mandatory Core roots. It must not depend on a
//!      builder's HTML capability list, on the mode that discovered the template, or on which
//!      package happened to publish first.

use super::*;

/// One `.mtf` body that names both prelude packages and ordinary project data.
///
/// WHAT: `p` comes from `@html`, the `center` slot contribution from `@styles`, and `site_name`
///       from a sibling project module.
/// WHY: one fixture then proves the merged prelude and project-data binding fold together under
///      two different builder surfaces.
const PARITY_FILES: &[(&str, &str)] = &[
    (
        "src/@entry.moth",
        "@docs intro_content\n\n[: [intro_content] ]\n",
    ),
    (
        "src/docs/@mod.moth",
        "$export\n@intro content as intro_content\n",
    ),
    ("src/docs/intro.mtf", "[p, center: [@settings site_name]]"),
    ("src/docs/settings.moth", "site_name #= \"Moth Site\"\n"),
];

/// The same shape as [`PARITY_FILES`] with a runtime value in the template head.
const PARITY_ERROR_FILES: &[(&str, &str)] = &[
    (
        "src/@entry.moth",
        "@docs intro_content\n\n[: [intro_content] ]\n",
    ),
    (
        "src/docs/@mod.moth",
        "$export\n@intro content as intro_content\n",
    ),
    ("src/docs/intro.mtf", "[@settings runtime_name()]"),
    (
        "src/docs/settings.moth",
        "runtime_name || -> String:\n    return \"runtime\"\n;\n",
    ),
];

fn fixture_folded_output(files: &[(&str, &str)], frontend_surface: &mut BuilderSurface) -> String {
    let compiled = successful_compilation_value(compile_fixture_result_with_surface(
        files,
        BuildProfile::Dev,
        frontend_surface,
    ));
    entry_folded_output(&compiled)
}

fn fixture_error_signatures(
    files: &[(&str, &str)],
    frontend_surface: &mut BuilderSurface,
) -> Vec<(String, Option<String>)> {
    match compile_fixture_result_with_surface(files, BuildProfile::Dev, frontend_surface) {
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            let mut signatures = reports
                .iter_severity(DiagnosticSeverity::Error)
                .map(|(_, view)| {
                    (
                        view.descriptor().code.to_owned(),
                        view.reason_key().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            signatures.sort();
            assert!(
                !signatures.is_empty(),
                "the fixture should retain at least one error"
            );
            signatures
        }
        Ok(CompilationOutcome::Success { .. }) => {
            panic!("the fixture should be diagnosed")
        }
        Err(errors) => {
            panic!("user-authored template input must stay in the diagnostic lane: {errors:?}")
        }
    }
}

#[test]
fn moth_template_binding_and_folded_output_match_across_html_and_minimal_surfaces() {
    // A `.mtf` that imports ordinary project data keeps the same binding and folded output when
    // the builder surface supplies no HTML capability beyond the mandatory Core roots.
    let mut html_surface = HtmlProjectBuilder::new().frontend_surface();
    let mut minimal_surface = BuilderSurface::with_mandatory_core();

    let html_output = fixture_folded_output(PARITY_FILES, &mut html_surface);
    let minimal_output = fixture_folded_output(PARITY_FILES, &mut minimal_surface);

    assert_eq!(
        html_output, minimal_output,
        "the HTML and minimal surfaces must fold the same template content"
    );
    assert!(
        html_output.contains("Moth Site"),
        "the folded template should retain the qualified project value: {html_output:?}"
    );
    assert!(
        html_output.contains("text-align: center;"),
        "the folded template should retain the @styles slot contribution: {html_output:?}"
    );
}

#[test]
fn moth_template_errors_match_across_html_and_minimal_surfaces() {
    // The same rejected template head must produce the same diagnostics without an HTML builder:
    // prelude and project-data binding share one contract on every surface.
    let mut html_surface = HtmlProjectBuilder::new().frontend_surface();
    let mut minimal_surface = BuilderSurface::with_mandatory_core();

    let html_errors = fixture_error_signatures(PARITY_ERROR_FILES, &mut html_surface);
    let minimal_errors = fixture_error_signatures(PARITY_ERROR_FILES, &mut minimal_surface);

    assert_eq!(
        html_errors, minimal_errors,
        "both surfaces must diagnose the same rejected template input"
    );
}

#[test]
fn single_file_template_input_materialises_both_prelude_packages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::write(dir.join("intro.mtf"), "[p, center: Hello]").expect("template should be written");
    let entry_path = dir.join("main.moth");
    fs::write(&entry_path, "body #= @intro.mtf\n").expect("entry should be written");

    let mut config = Config::new(entry_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut frontend_surface = BuilderSurface::with_mandatory_core();

    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    ));

    assert!(
        frontend.source_packages.by_prefix("html").is_some(),
        "the single-file template input must materialise @html before its consumer"
    );
    assert!(
        frontend.source_packages.by_prefix("styles").is_some(),
        "the single-file template input must materialise @styles before its consumer"
    );
}

#[test]
fn check_only_template_input_binds_both_prelude_packages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("config should be written");
    fs::write(root.join("@entry.moth"), "value ~= 1\n").expect("entry should be written");
    fs::write(root.join("page.mtf"), "[p, center: Hello]").expect("template should be written");
    fs::write(root.join("orphan.moth"), "body #= @page.mtf\n")
        .expect("check-only source should be written");

    let mut config = Config::new(root);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    let mut project_source_files = None;
    let mut project_source_builder = Some(SourceDatabaseBuilder::new(SourceDatabase::empty()));

    let frontend = successful_compilation_value(compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
        FrontendCompilationMode::Check,
    ));

    assert!(
        frontend.transient_batches.is_empty(),
        "the check-only template consumer should not retain diagnostics"
    );
    assert!(
        frontend.source_packages.by_prefix("html").is_some(),
        "check-only template consumers must order after @html"
    );
    assert!(
        frontend.source_packages.by_prefix("styles").is_some(),
        "check-only template consumers must order after @styles"
    );
}
