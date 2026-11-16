use super::create_project_modules_tests::{diagnosed_reports, successful_compilation_value};
use super::project_structure_diagnostics::{
    entry_root_package_prefix_collision_error, invalid_module_root_file_name_error,
    reserved_package_namespace_root_claim_error,
};
use super::*;

use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    ExplicitMothExtensionView, ExplicitSourceExtensionView, UnsupportedExternalExtensionView,
    UnsupportedSourceFileKindView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidConfigCode, InvalidConfigView, InvalidDependencyClauseView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{DiagnosticSeverity, PreparedFailure};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;

fn prepared_failure_reports(
    failure: PreparedFailure,
) -> crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    let PreparedFailure::Diagnosed(batch) = failure else {
        panic!("expected diagnosed reports, got {failure:?}");
    };
    assert!(
        batch.has_errors(),
        "expected a user-facing error diagnostic"
    );
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    accumulator
        .finish(InstallMode::Unfrozen)
        .expect("diagnosed reports should freeze")
}

fn assert_config_failure(failure: PreparedFailure, expected_reason: InvalidConfigCode) {
    let reports = prepared_failure_reports(failure);
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, diagnostic)| {
                diagnostic.descriptor().code == "MOTH-CONFIG-0001"
                    && diagnostic
                        .family_view::<InvalidConfigView>()
                        .is_some_and(|facts| facts.reason == expected_reason)
            }),
        "expected the typed config failure {expected_reason:?}",
    );
}

fn assert_reserved_project_dependency_failure(failure: PreparedFailure) {
    let reports = prepared_failure_reports(failure);
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, diagnostic)| {
                diagnostic
                    .family_view::<InvalidDependencyClauseView>()
                    .is_some_and(|facts| {
                        facts.clause_kind == DependencyClauseKind::Namespace
                            && facts.reason
                                == InvalidDependencyClauseReason::ProjectGlobalsPathReserved
                    })
            }),
        "expected the typed reserved project dependency failure",
    );
}

/// Assert one structure failure carries the exact typed reason and literal string facts.
///
/// Expected facts stay literal instead of being derived through the formatter under test: a raw
/// `Path::display()` producer would still match on Windows but exposes `\` on Unix.
fn assert_literal_config_facts(
    failure: PreparedFailure,
    expected_reason: InvalidConfigCode,
    expected_reason_key: &str,
    expected_facts: &[&str],
) {
    let reports = prepared_failure_reports(failure);
    let (report, diagnostic) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("structure failure should retain one diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-CONFIG-0001");
    assert_eq!(diagnostic.reason_key(), Some(expected_reason_key));

    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("structure failure should retain typed config facts");
    assert_eq!(facts.reason, expected_reason);

    let strings = report.context().strings();
    let resolved_facts: Vec<&str> = diagnostic
        .string_ids()
        .iter()
        .map(|id| strings.resolve(*id))
        .collect();
    assert_eq!(resolved_facts.as_slice(), expected_facts);
}

// ── Project-structure collision tests ─────────────────────────────────────────

/// Build the project resolver after `claim` adds one reserved-name candidate and require the typed
/// `@project` reservation failure. Each call gets a fresh project, so case variants never meet on
/// case-insensitive filesystems.
fn assert_claims_reserved_project_name(claim: impl FnOnce(&Path), reason: &str) {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::write(root.join("@entry.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    claim(&root);

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let failure = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect_err(reason);

    assert_config_failure(failure, InvalidConfigCode::ProjectGlobalsNameReserved);
}

#[test]
fn rejects_reserved_project_globals_module_folder_at_entry_root() {
    for folder in ["project", "Project"] {
        assert_claims_reserved_project_name(
            |root| {
                fs::create_dir(root.join(folder)).expect("should create project module folder");
            },
            "entry-root project folder must not claim @project",
        );
    }
}

#[test]
fn rejects_reserved_project_globals_source_file_at_entry_root() {
    for file in ["project.moth", "Project.moth", "PROJECT.mtf"] {
        assert_claims_reserved_project_name(
            |root| {
                fs::write(root.join(file), "x ~= 1\n").expect("should write reserved source file");
            },
            "entry-root project source file must not claim @project",
        );
    }
}

#[test]
fn rejects_nested_reserved_project_globals_dependency() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let resolver = configured_resolver(&config);
    for (source, expected_kind) in [
        (
            "@project/details\nx ~= 1\n",
            DependencyClauseKind::Namespace,
        ),
        (
            "@project/details value\nx ~= 1\n",
            DependencyClauseKind::DirectSelection,
        ),
        (
            "@project/details as project_details\nx ~= 1\n",
            DependencyClauseKind::NamespaceAlias,
        ),
    ] {
        fs::write(root.join("src/@entry.moth"), source).expect("should write entry");
        let failure = discover_modules_for_test(&config, &resolver, &style_directives);
        let diagnostic = first_error_diagnostic(&failure);
        let facts = diagnostic
            .family_view::<InvalidDependencyClauseView>()
            .expect("reserved project dependency should retain typed clause facts");
        assert_eq!(facts.clause_kind, expected_kind);
        assert_eq!(
            facts.reason,
            InvalidDependencyClauseReason::ProjectGlobalsPathReserved,
            "unexpected reserved project dependency reason"
        );
    }
}

#[test]
fn source_package_rejects_exact_reserved_project_globals_dependency() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let src = root.join("src");
    let package_root = root.join("packages/builder/helper");
    fs::create_dir_all(&src).expect("should create src");
    fs::create_dir_all(&package_root).expect("should create helper package");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@entry.moth"), "[:entry]\n").expect("should write project entry");
    fs::write(package_root.join("@package.moth"), "value #= 1\n")
        .expect("should write helper package root");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages
        .register_filesystem_root("builder/helper", package_root, PackageOrigin::Builder)
        .expect("test source root should register");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);
    let declaring_source = fs::canonicalize(root.join("packages/builder/helper/@package.moth"))
        .expect("package source should canonicalize");

    with_namespace_resolution(
        &config,
        &resolver,
        &source_packages,
        Some("builder/helper"),
        |namespace_set,
         _source_tree_index,
         package_prefix,
         string_table,
         provider_paths,
         path_fork| {
            let package_source_tree_index = namespace_set
                .source_package_boundaries()
                .find(|(prefix, _)| *prefix == package_prefix.expect("package boundary prefix"))
                .map(|(_, index)| index)
                .expect("requested source package boundary should be indexed");
            // The resolution's fork is rebound to the caller-owned discovery fork inside
            // `handle_provider_capable_dependency`, so a throwaway interner is sufficient here.
            let throwaway_fork = PathInternerFork::empty();
            let resolution = DirectoryDependencyResolution::package(
                namespace_set,
                package_prefix.expect("package boundary prefix"),
                package_source_tree_index,
                &throwaway_fork,
            );
            let provider = provider_root(&["project"], provider_paths);
            let mut external_packages = ExternalPackageRegistry::new();
            let providers = ExternalImportProviderRegistry::empty();
            let mut cache =
                crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new();
            let mut resolution_table =
                crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
            let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
                external_packages: &mut external_packages,
                providers: &providers,
                cache: &mut cache,
                resolution_table: &mut resolution_table,
            };

            let mut provider_sources = super::source_discovery::ProviderOwnedSourceStaging::new();
            let Err(error) = super::source_discovery::resolve_structural_provider_reference(
                &provider,
                DependencyClauseKind::Namespace,
                &declaring_source,
                &resolver,
                path_fork,
                &mut provider_sources,
                &mut external_imports,
                resolution,
                string_table,
            ) else {
                panic!("source-package @project dependency must be rejected");
            };
            let failure = error.into_failure(
                string_table,
                std::sync::Arc::new(path_fork.snapshot_table()),
            );
            assert_reserved_project_dependency_failure(failure);
        },
    );
}

#[test]
fn rejects_moth_file_and_folder_collision_in_same_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/UI")).expect("should create src/UI");
    fs::write(root.join("src/UI/@entry.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("src/ui.moth"), "y ~= 2\n").expect("should write colliding file");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let result = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "ui.moth + UI/ collision should be rejected"
    );
    let failure = result.expect_err("checked above");
    assert_config_failure(failure, InvalidConfigCode::SourceFileFolderCollision);
}

#[test]
fn rejects_template_file_and_folder_collision_in_same_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/ui")).expect("should create src/ui");
    fs::write(root.join("src/ui/@entry.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(root.join("src/ui.mtf"), "template\n").expect("should write colliding file");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let failure = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect_err("ui.mtf + ui/ collision should be rejected");

    assert_config_failure(failure, InvalidConfigCode::SourceFileFolderCollision);
}

#[test]
fn allows_same_stem_in_different_directories() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/components")).expect("should create src/components");
    fs::create_dir_all(root.join("src/pages")).expect("should create src/pages");
    fs::write(root.join("src/components/card.moth"), "x ~= 1\n").expect("should write card");
    fs::write(root.join("src/pages/card.moth"), "y ~= 2\n").expect("should write another card");
    fs::write(root.join("src/@entry.moth"), "z ~= 3\n").expect("should write entry");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect("same stem in different directories should be allowed");
}

#[test]
fn rejects_collision_with_empty_folder() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::write(root.join("src/helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(root.join("src/@entry.moth"), "y ~= 2\n").expect("should write entry");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    let result = super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "collision with an empty folder should be rejected"
    );
    let failure = result.expect_err("checked above");
    assert_config_failure(failure, InvalidConfigCode::SourceFileFolderCollision);
}

#[test]
fn js_file_with_same_stem_as_folder_does_not_trigger_collision() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/helper")).expect("should create src/helper");
    fs::write(root.join("src/helper.js"), "// js\n").expect("should write js file");
    fs::write(root.join("src/@entry.moth"), "x ~= 1\n").expect("should write entry");
    fs::write(
        root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut string_table = StringTable::new();
    super::project_roots::build_project_path_resolver(
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &mut string_table,
    )
    .expect(".js file with same stem as folder should not trigger collision");
}

#[test]
fn unsupported_js_import_without_provider_reports_moth_import_0021() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    // Entry file imports a .js file explicitly.
    fs::write(
        src.join("@entry.moth"),
        "-- é🦋\n@drawing.js as drawing\n[:ok]\n",
    )
    .expect("should write entry");

    fs::write(src.join("drawing.js"), "export function draw() {}\n").expect("should write js file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0021",
        "expected unsupported external extension diagnostic, got {}",
        diagnostic.descriptor().code
    );
    let facts = diagnostic
        .family_view::<UnsupportedExternalExtensionView>()
        .expect("unsupported external extension should retain typed path facts");
    let (report, _) = diagnosed_reports(&failure)
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("unsupported external extension should retain its error report");
    let context = DiagnosticRenderContext::for_report(report);
    assert_eq!(
        context.render_path(facts.path),
        "drawing.js",
        "unexpected path in diagnostic"
    );
    assert_eq!(
        context.string_table.resolve(facts.extension),
        "js",
        "unexpected extension in diagnostic"
    );
    assert_primary_span_text(&failure, &src.join("@entry.moth"), "@drawing.js");

    let mut source_string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root = project_root.clone();
    let empty_providers = ExternalImportProviderRegistry::empty();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        resolver.source_file_kinds(),
        &empty_providers,
        &mut source_string_table,
    )
    .expect("source tree index should rebuild for span assertions");
    let mut source_files =
        source_database_for_test(&source_tree_index, &resolver, &mut source_string_table);
    let entry_path =
        fs::canonicalize(src.join("@entry.moth")).expect("entry source path should canonicalize");
    let mut selected_source_texts = super::source_loading::SelectedSourceTextMap::default();
    selected_source_texts
        .load(&entry_path)
        .expect("entry source snapshot should load");
    let mut path_fork = source_files.fork_path_interner();
    selected_source_texts
        .retain_into(&mut source_files, &mut path_fork)
        .expect("entry source snapshot should retain");
    let source_id = source_files
        .get_by_canonical_path(&entry_path)
        .expect("entry source should remain in the source database")
        .id;
    let span = match diagnostic.primary_site() {
        crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Local(span)
        | crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Foreign {
            span,
            ..
        } => span,
        crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent => {
            panic!("provider diagnostic should retain its authored path span")
        }
    };
    assert_eq!(span.source(), source_id);
    let range = span.byte_range(&source_files);
    let source_text = source_files
        .retained_text(source_id)
        .expect("entry source snapshot should remain available");
    let expected_start = source_text
        .find("@drawing.js")
        .expect("fixture should contain the provider path") as u32;
    assert_eq!(range.start(), expected_start);
    assert_eq!(
        range.end(),
        expected_start + "@drawing.js".len() as u32,
        "provider span should cover the full UTF-8 path token"
    );
}

#[test]
fn explicit_moth_extension_still_reports_moth_import_0020() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@helper.moth\n[:ok]\n").expect("should write entry");

    fs::write(
        src.join("helper.moth"),
        "greet || -> String:\n    return \"hi\"\n;\n",
    )
    .expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0020",
        "expected explicit .moth extension diagnostic, got {}",
        diagnostic.descriptor().code
    );
    diagnostic
        .family_view::<ExplicitMothExtensionView>()
        .expect("explicit .moth extension should retain typed path facts");
}

#[test]
fn direct_moth_template_extension_dependency_reports_moth_import_0024() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@intro.mtf\n[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "hello\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0024",
        "expected explicit source extension diagnostic, got {}",
        diagnostic.descriptor().code
    );
    diagnostic
        .family_view::<ExplicitSourceExtensionView>()
        .expect("explicit source extension should retain typed path facts");
}

#[test]
fn moth_template_files_are_reachable_without_dependency_scanning() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@intro\n[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "@missing\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();
    assert!(input_paths.contains("@entry.moth"));
    assert!(input_paths.contains("intro.mtf"));
    assert!(modules[0].prepared.contains_moth_template);
}

#[test]
fn unreferenced_moth_template_file_under_entry_root_is_ignored() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.mtf"), "hello\n").expect("should write moth template file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(
        module_prepared_source_names(modules[0]),
        vec!["@entry.moth"]
    );
    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn extensionless_moth_dependency_and_virtual_package_dependency_still_work() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    // Normal extensionless dependencies still resolve as Moth source files, while virtual package
    // dependencies continue to stay out of Stage 0 filesystem traversal.
    fs::write(src.join("@entry.moth"), "@helper\n@core/io line\n[:ok]\n")
        .expect("should write entry");

    fs::write(
        src.join("helper.moth"),
        "greet || -> String:\n    return \"hi\"\n;\n",
    )
    .expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();
    assert_eq!(modules.len(), 1);

    let discovered: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();

    assert!(discovered.contains("@entry.moth"));
    assert!(discovered.contains("helper.moth"));
}

#[test]
fn indexed_module_inventory_includes_referenced_markdown_without_scanning_its_body() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@intro\n[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "@missing\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register(crate::builder_surface::OptionalSourceKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();
    assert!(input_paths.contains("@entry.moth"));
    assert!(input_paths.contains("intro.md"));

    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn indexed_module_inventory_excludes_unrelated_module_root_from_markdown_owner() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("other")).expect("should create other module dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@intro\n[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");
    fs::write(src.join("other/@mod.moth"), "$export\nx #= 1\n")
        .expect("should write other module root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register(crate::builder_surface::OptionalSourceKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    let input_paths: HashSet<_> = module_prepared_source_names(modules[0])
        .into_iter()
        .collect();
    assert!(input_paths.contains("@entry.moth"));
    assert!(input_paths.contains("intro.md"));
    assert!(!input_paths.contains("other/@mod.moth"));

    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn indexed_module_inventory_ignores_unreferenced_markdown_file() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register(crate::builder_surface::OptionalSourceKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let modules = successful_compilation_value(discover_modules_for_test(
        &config,
        &resolver,
        &style_directives,
    ));
    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    assert_eq!(
        module_prepared_source_names(modules[0]),
        vec!["@entry.moth"]
    );
    assert!(!modules[0].prepared.contains_moth_template);
}

#[test]
fn indexed_module_inventory_rejects_direct_markdown_extension_dependency() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@intro.md\n[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");

    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register(crate::builder_surface::OptionalSourceKind::PlainMarkdown);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0024",
        "expected explicit source extension diagnostic, got {}",
        diagnostic.descriptor().code
    );
    let facts = diagnostic
        .family_view::<ExplicitSourceExtensionView>()
        .expect("explicit source extension should retain typed path facts");
    let (report, _) = diagnosed_reports(&failure)
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("explicit source extension should retain its error report");
    let context = DiagnosticRenderContext::for_report(report);
    assert_eq!(
        context.render_path(facts.path),
        "intro.md",
        "unexpected dependency path in explicit source extension diagnostic"
    );
    assert_eq!(
        context.string_table.resolve(facts.extension),
        "md",
        "unexpected extension in explicit source extension diagnostic"
    );
}

#[test]
fn indexed_module_inventory_rejects_unsupported_markdown_dependency() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src dir");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    fs::write(src.join("@entry.moth"), "@intro\n[:ok]\n").expect("should write entry");
    fs::write(src.join("intro.md"), "hello\n").expect("should write markdown file");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let failure = discover_modules_for_test(&config, &resolver, &style_directives);

    let diagnostic = first_error_diagnostic(&failure);
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-IMPORT-0025",
        "expected unsupported source file kind diagnostic, got {}",
        diagnostic.descriptor().code
    );
    let facts = diagnostic
        .family_view::<UnsupportedSourceFileKindView>()
        .expect("unsupported source file kind should retain typed path facts");
    let (report, _) = diagnosed_reports(&failure)
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("unsupported source file kind should retain its error report");
    let context = DiagnosticRenderContext::for_report(report);
    assert_eq!(
        context.render_path(facts.path),
        "intro",
        "unexpected dependency path in unsupported source file kind diagnostic"
    );
    assert_eq!(
        context.string_table.resolve(facts.extension),
        "md",
        "unexpected extension in unsupported source file kind diagnostic"
    );
    assert_primary_span_text(&failure, &src.join("@entry.moth"), "@intro");
}

/// Build the Stage 0 namespace set for one parsed project and its registered source packages.
///
/// The namespace owner consumes the same project index, graph and retained package boundary
/// indexes as production, so reserved-claim validation is exercised without a second traversal.
fn build_namespace_set_for_test(
    config: &Config,
    resolver: &ProjectPathResolver,
    source_packages: &crate::builder_surface::SourcePackageRegistry,
) -> Result<ModuleNamespaceSet, PreparedFailure> {
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root = fs::canonicalize(config.entry_dir.clone()).expect("entry root should resolve");
    let external_import_providers = ExternalImportProviderRegistry::default();
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        config,
        source_packages,
        resolver.source_file_kinds(),
        &external_import_providers,
        &mut string_table,
    )
    .expect("project source tree index should build");
    let project_module_graph =
        super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&source_tree_index);
    let package_boundary_indexes =
        super::source_package_discovery::build_source_package_boundary_indexes(
            source_packages,
            resolver.source_file_kinds(),
            &external_import_providers,
            &mut string_table,
        )
        .expect("source-package boundary indexes should build");
    ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        package_boundary_indexes,
        &ExternalPackageRegistry::new(),
        &mut string_table,
    )
}

fn assert_reserved_package_claim_failure(
    failure: PreparedFailure,
    expected_root: &str,
    claim_suffix: &str,
) {
    let reports = prepared_failure_reports(failure);
    let (report, diagnostic) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("expected one reserved package namespace diagnostic");
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-CONFIG-0001",
        "unexpected diagnostic code"
    );
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("reserved package namespace failure should retain typed config facts");
    assert_eq!(
        facts.reason,
        InvalidConfigCode::ReservedPackageNamespaceRootClaim
    );
    let strings = report.context().strings();
    assert_eq!(
        strings.resolve(diagnostic.string_ids()[0]),
        expected_root,
        "the diagnostic must name the claimed reserved root as authored"
    );
    assert!(
        strings
            .resolve(diagnostic.string_ids()[1])
            .ends_with(claim_suffix),
        "the diagnostic must name the local source claim: {}",
        strings.resolve(diagnostic.string_ids()[1])
    );
}

#[test]
fn registered_source_package_rejects_importable_claims_below_reserved_package_roots() {
    // A registered package boundary owns its own module namespaces. An importable helper or
    // facade below `@core`/`@builder` must fail there exactly as it does in project namespaces.
    for (case, claim_path, claim_source, expected_root, expected_source_suffix) in [
        (
            "package-root helper",
            "core.moth",
            "value #= 1\n",
            "core",
            "core.moth",
        ),
        (
            "nested normal facade",
            "Builder/@mod.moth",
            "$export\nvalue #= 1\n",
            "Builder",
            "Builder",
        ),
        (
            "nested support facade",
            "core/@package.moth",
            "$export\nvalue #= 1\n",
            "core",
            "core",
        ),
    ] {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let src = root.join("src");
        let package_root = root.join("packages/local/widgets");
        fs::create_dir_all(&src).expect("should create src dir");
        fs::create_dir_all(&package_root).expect("should create package dir");
        fs::write(
            root.join(settings::CONFIG_FILE_NAME),
            "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
        )
        .expect("should write config");
        fs::write(src.join("@entry.moth"), "[:entry]\n").expect("should write entry");
        fs::write(package_root.join("@package.moth"), "$export\nvalue #= 1\n")
            .expect("should write package facade");
        // A paired internal normal root makes the package root a normal-position consumer, so a
        // nested support module is visible to it. A support-only root would publish no such
        // importable entry and correctly stay legal.
        fs::write(package_root.join("@mod.moth"), "$export\nvalue #= 1\n")
            .expect("should write internal normal root");
        let claim = package_root.join(claim_path);
        fs::create_dir_all(claim.parent().expect("claim has a parent"))
            .expect("should create claim parent");
        fs::write(&claim, claim_source).expect("should write claim source");

        let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
        source_packages
            .register_filesystem_root("widgets", package_root, PackageOrigin::ProjectLocal)
            .expect("test source root should register");
        let mut config = Config::new(root.clone());
        let style_directives = test_style_directives();
        parse_project_config_for_test(
            &mut config,
            &root.join(settings::CONFIG_FILE_NAME),
            &style_directives,
        )
        .expect("config should parse");
        let resolver = configured_resolver(&config);

        let failure = build_namespace_set_for_test(&config, &resolver, &source_packages)
            .err()
            .unwrap_or_else(|| panic!("{case}: package reserved-root claim must be rejected"));
        assert_reserved_package_claim_failure(failure, expected_root, expected_source_suffix);
    }
}

#[test]
fn registered_source_package_keeps_entry_only_reserved_directory_legal() {
    // An entry-only module publishes no importable namespace entry, so its reserved-name
    // directory, private helper and asset stay legal inside a registered package boundary.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let package_root = root.join("packages/local/widgets");
    fs::create_dir_all(&src).expect("should create src dir");
    fs::create_dir_all(package_root.join("core")).expect("should create package core directory");
    fs::create_dir_all(package_root.join("builder/assets"))
        .expect("should create package asset directory");
    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@entry.moth"), "[:entry]\n").expect("should write entry");
    fs::write(package_root.join("@package.moth"), "$export\nvalue #= 1\n")
        .expect("should write package facade");
    fs::write(package_root.join("core/@entry.moth"), "[:core entry]\n")
        .expect("should write entry-only core module");
    fs::write(package_root.join("core/helper.moth"), "value #= 1\n")
        .expect("should write private helper");
    fs::write(package_root.join("builder/assets/logo.png"), "not source")
        .expect("should write inert asset");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages
        .register_filesystem_root("widgets", package_root, PackageOrigin::ProjectLocal)
        .expect("test source root should register");
    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    build_namespace_set_for_test(&config, &resolver, &source_packages)
        .expect("entry-only reserved-name modules publish no importable claim");
}

#[test]
fn prefix_collision_fact_renders_the_authored_folder_portably() {
    // A synthetic backslash-separated path is one authored folder on Unix and is already native
    // spelling on Windows. The diagnostic fact must render `/` on every host, so a raw
    // `Path::display()` producer is detectably wrong on Unix.
    let colliding_folder = Path::new(r"src\builder\web");
    let mut string_table = StringTable::new();
    let failure = entry_root_package_prefix_collision_error(
        "builder/web",
        colliding_folder,
        &mut string_table,
    );

    assert_literal_config_facts(
        failure,
        InvalidConfigCode::EntryRootPackagePrefixCollision,
        "invalid_config.entry_root_package_prefix_collision",
        &["builder/web", "src/builder/web"],
    );
}

#[test]
fn reserved_namespace_claim_fact_renders_the_authored_source_portably() {
    let claiming_source = Path::new(r"src\core\widget.moth");
    let mut string_table = StringTable::new();
    let failure =
        reserved_package_namespace_root_claim_error("core", claiming_source, &mut string_table);

    assert_literal_config_facts(
        failure,
        InvalidConfigCode::ReservedPackageNamespaceRootClaim,
        "invalid_config.reserved_package_namespace_root_claim",
        &["core", "src/core/widget.moth"],
    );
}

#[test]
fn invalid_root_file_name_fact_renders_the_authored_directory_portably() {
    let directory = Path::new(r"src\docs");
    let mut string_table = StringTable::new();
    let failure = invalid_module_root_file_name_error(directory, "@page.moth", &mut string_table);

    assert_literal_config_facts(
        failure,
        InvalidConfigCode::InvalidModuleRootFileName,
        "invalid_config.invalid_module_root_file_name",
        &["@page.moth", "src/docs"],
    );
}
