//! Source-tree discovery boundary tests.
//!
//! WHAT: covers one Stage 0 traversal's skip policy, symlink and configured-output aliases,
//!      hidden-subtree handling, root claims and collision ordering.
//! WHY: discovery owns filesystem exclusion and inventory facts that later stages cannot
//!      reconstruct. Diagnostic/report fixture helpers live in the sibling source-tree test
//!      module; this file keeps direct filesystem-boundary cases local to the inventory owner.

use super::*;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidConfigCode, InvalidConfigView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReport;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;

fn diagnosed_reports(
    failure: PreparedFailure,
) -> crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    let PreparedFailure::Diagnosed(batch) = failure else {
        panic!("expected diagnosed source-tree reports, got {failure:?}");
    };
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    accumulator
        .finish(InstallMode::Unfrozen)
        .expect("source-tree diagnostics should freeze")
}

fn first_invalid_config(
    reports: &crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet,
) -> (&DiagnosticReport, DiagnosticView<'_>) {
    let (report, diagnostic) = reports
        .iter_severity(crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error)
        .next()
        .expect("source-tree failure should retain its diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-CONFIG-0001");
    diagnostic
        .family_view::<InvalidConfigView>()
        .expect("source-tree failure should retain typed config facts");
    (report, diagnostic)
}

fn assert_source_file_folder_collision(failure: PreparedFailure) {
    let boundary = diagnosed_reports(failure);
    let (_, diagnostic) = first_invalid_config(&boundary);
    assert_eq!(
        diagnostic
            .family_view::<InvalidConfigView>()
            .expect("config failure should retain typed facts")
            .reason,
        InvalidConfigCode::SourceFileFolderCollision
    );
}

fn assert_multiple_module_roots(failure: PreparedFailure, expected_directory: &Path) {
    let boundary = diagnosed_reports(failure);
    let (report, diagnostic) = first_invalid_config(&boundary);
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("config failure should retain typed facts");
    assert_eq!(facts.reason, InvalidConfigCode::MultipleModuleRootFiles);

    let strings = diagnostic.string_ids();
    assert_eq!(
        strings.len(),
        3,
        "directory plus two candidates should be retained"
    );
    assert_eq!(
        report.context().strings().resolve(strings[0]),
        expected_directory.display().to_string()
    );
}

#[test]
fn source_tree_index_collects_one_scan_and_applies_skip_policy() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.clone();
    let nested = entry_root.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module directory");

    for directory_name in [
        ".git",
        "target",
        "node_modules",
        ".cache",
        "_generated",
        "_scratch",
    ] {
        let directory = entry_root.join(directory_name);
        fs::create_dir_all(&directory).expect("should create skipped directory");
        fs::write(directory.join("@skipped.moth"), "").expect("should write skipped root");
        fs::write(directory.join("page.mtf"), "").expect("should write skipped mtf");
        fs::write(directory.join("note.md"), "").expect("should write skipped markdown");
        fs::write(directory.join("data.mon"), "").expect("should write skipped mon");
        fs::write(directory.join("provider.js"), "").expect("should write skipped provider");
    }

    // Directory names the old fixed policy reserved are ordinary source now; only hidden
    // directories and configured output roots stay excluded.
    let visible = entry_root.join("dev");
    fs::create_dir_all(&visible).expect("should create visible module directory");
    fs::write(visible.join("@dev.moth"), "").expect("should write visible root");

    // A visible `release` directory is ordinary source, not a generated-output skip.
    let visible_release = entry_root.join("release");
    fs::create_dir_all(&visible_release).expect("should create visible release source directory");
    fs::write(visible_release.join("@release.moth"), "")
        .expect("should write visible release root");

    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");
    fs::write(entry_root.join("ordinary.moth"), "").expect("should write ordinary source");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");
    fs::write(nested.join("config.moth"), "not source").expect("should write nested config");

    let mut config = Config::new(root.clone());
    config.project_name = String::from("docs");
    config.output_roots.dev_output = Some(String::from("scratch"));
    config.output_roots.release_output = Some(String::from("generated"));
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let validated_output_settings = {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
        crate::build_system::project_config::validate_directory_output_settings(
            &config,
            &mut diagnostic_strings,
        )
    }
    .expect("configured output folders should validate");

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root.clone(),
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: Some(&validated_output_settings),
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");

    assert_eq!(index.entry_root(), canonical_entry_root);
    for hidden_output in ["_generated", "_scratch"] {
        let hidden_output = root.join(hidden_output);
        assert!(
            index
                .sources()
                .iter()
                .all(|source| { !source.canonical_path().starts_with(&hidden_output) }),
            "generated files under hidden output roots must not enter source inventory"
        );
    }
    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    let entry_root_files: Vec<PathBuf> = graph
        .entry_modules()
        .iter()
        .map(|module_id| graph.node(*module_id).root_file().to_path_buf())
        .collect();
    assert_eq!(entry_root_files.len(), 4);
    assert!(entry_root_files[0].ends_with("@home.moth"));
    assert!(entry_root_files[1].ends_with("dev/@dev.moth"));
    assert!(entry_root_files[2].ends_with("nested/@nested.moth"));
    assert!(entry_root_files[3].ends_with("release/@release.moth"));
    assert_eq!(index.stats().dirs_visited, 4);
    assert_eq!(index.stats().dirs_skipped, 6);
    assert_eq!(index.stats().files_seen, 6);
    assert_eq!(index.stats().normal_root_files_seen, 4);
    assert_eq!(index.stats().module_roots_found, 4);
    assert!(
        index.sources().iter().all(|source| {
            source
                .canonical_path()
                .file_name()
                .is_some_and(|name| name != "config.moth")
        }),
        "a nested config.moth is reserved and must not enter source inventory"
    );

    let root_directories = index
        .module_roots()
        .root_directories()
        .map(|path| root_directory_name(path.as_path()))
        .collect::<Vec<_>>();
    assert_eq!(
        root_directories[0],
        root_directory_name(&canonical_entry_root)
    );
    assert_eq!(root_directories[1], "dev");
    assert_eq!(root_directories[2], "nested");
}

#[test]
fn source_tree_index_ignores_collision_in_fixed_skipped_directory() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.clone();

    // Fixed-skipped directory with collision-shaped contents. Configured output directories
    // remain outside the source-entry tree and do not receive a compatibility exception.
    let target_dir = entry_root.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    // Real module root that should be discovered.
    let nested = entry_root.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");
    fs::write(nested.join("config.moth"), "not source").expect("should write nested config");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("fixed-skipped collision-shaped inputs must not trigger collision diagnostics");

    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    assert_eq!(graph.entry_modules().len(), 2);
    assert_eq!(index.stats().dirs_skipped, 1);
}

#[test]
fn source_tree_index_ignores_package_prefix_collision_in_skipped_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");

    // Fixed-skipped directory whose name matches a source-backed package prefix.
    // Under the skip policy this folder is not dependency-bindable, so no prefix collision.
    fs::create_dir_all(entry_root.join("target")).expect("should create target folder");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages
        .register_filesystem_root(
            "target",
            fs::canonicalize(entry_root.join("target")).unwrap(),
            PackageOrigin::Builder,
        )
        .expect("test source root should register");

    let mut string_table = StringTable::new();
    super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &source_packages,
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("skipped folder matching a package prefix must not trigger prefix collision");
}

#[test]
fn source_tree_index_rejects_exact_nested_package_path_but_allows_ancestor() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = fs::canonicalize(temp.path()).expect("should canonicalize temp dir");
    let entry = root.join("src");
    fs::create_dir_all(entry.join("web")).expect("should create ancestor folder");
    fs::write(entry.join("@home.moth"), "").expect("should write entry root");
    let config = Config::new(root.clone());
    let mut packages = crate::builder_surface::SourcePackageRegistry::default();
    packages
        .register_filesystem_root("web/site", root.join("package"), PackageOrigin::Builder)
        .expect("should register nested package prefix");
    let discover = || {
        super::source_tree_index::SourceTreeIndex::discover(
            entry.clone(),
            super::source_tree_index::SourceTreeProjectContext {
                project_root: &root,
                validated_output_settings: None,
            },
            &config,
            &packages,
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut StringTable::new(),
        )
    };
    // The diagnostic names the offending folder as spelled on disk, not the registered prefix.
    let assert_collision = |folder: &str, reason: &str| {
        let reports = diagnosed_reports(discover().expect_err(reason));
        let (report, diagnostic) = first_invalid_config(&reports);
        let facts = diagnostic
            .family_view::<InvalidConfigView>()
            .expect("config failure should retain typed facts");
        assert_eq!(
            facts.reason,
            InvalidConfigCode::EntryRootPackagePrefixCollision,
            "{reason}"
        );
        assert_eq!(
            report
                .context()
                .strings()
                .resolve(diagnostic.string_ids()[1]),
            folder
                .split('/')
                .fold(entry.clone(), |path, component| path.join(component))
                .display()
                .to_string(),
            "{reason}"
        );
    };

    discover().expect("ancestor folder alone must be allowed");
    fs::create_dir(entry.join("web/site")).expect("should create exact nested folder");
    assert_collision("web/site", "exact nested path must collide");

    // Module ownership of the folder doesn't change its entry-root-relative identity.
    fs::write(entry.join("web/@mod.moth"), "").expect("should write nested module root");
    assert_collision(
        "web/site",
        "exact nested path inside a nested module must still collide",
    );

    // Package identities reject ASCII case-only overlaps, so case variants claim the same path
    // on case-sensitive and case-insensitive hosts alike.
    fs::remove_dir(entry.join("web/site")).expect("should remove exact nested folder");
    fs::create_dir(entry.join("web/Site")).expect("should create case-variant leaf folder");
    assert_collision(
        "web/Site",
        "case-variant leaf inside a nested module must collide",
    );

    fs::remove_dir_all(entry.join("web")).expect("should remove lowercase ancestor");
    fs::create_dir(entry.join("Web")).expect("should create case-variant ancestor");
    discover().expect("case-variant ancestor alone must be allowed");
    fs::create_dir(entry.join("Web/site")).expect("should create case-variant nested folder");
    assert_collision("Web/site", "case-variant ancestor path must collide");
}

#[test]
fn source_tree_index_detects_collision_in_non_skipped_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(entry_root.join("helper")).expect("should create helper folder");
    fs::write(entry_root.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();

    let failure = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("non-skipped bst/folder collision should be rejected");
    assert_source_file_folder_collision(failure);
}

#[test]
fn bounded_module_roots_for_single_file_indexes_nested_roots_with_ignored_directories() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    let nested = module_dir.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");

    // Ignored directory with collision-shaped contents.
    let target_dir = module_dir.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    fs::write(module_dir.join("@home.moth"), "").expect("should write entry root");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");
    fs::write(nested.join("config.moth"), "not source").expect("should write nested config");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("@home.moth")).unwrap();
    let mut string_table = StringTable::new();

    let module_roots =
        super::source_tree_index::SourceTreeIndex::bounded_module_roots_for_single_file(
            &entry_file,
            &config,
            &crate::builder_surface::SourcePackageRegistry::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
            &mut string_table,
        )
        .expect("single-file normal module root should index its tree without collision errors");

    let root_directories = module_roots
        .root_directories()
        .map(|path| root_directory_name(path.as_path()))
        .collect::<Vec<_>>();
    assert_eq!(root_directories.len(), 2);
    assert!(root_directories.contains(&"module"));
    assert!(root_directories.contains(&"nested"));
}

#[test]
fn bounded_module_roots_for_single_file_rejects_dependency_name_collisions() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    fs::create_dir_all(module_dir.join("helper")).expect("should create helper directory");
    fs::write(module_dir.join("helper.moth"), "helper #= 1\n")
        .expect("should write colliding source file");
    fs::write(module_dir.join("@home.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("@home.moth")).unwrap();
    let mut string_table = StringTable::new();

    let failure = super::source_tree_index::SourceTreeIndex::bounded_module_roots_for_single_file(
        &entry_file,
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("single-file normal module roots should reject real dependency-name collisions");
    assert_source_file_folder_collision(failure);
}

#[test]
fn source_tree_index_rejects_duplicate_normal_module_root_files() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");
    fs::write(entry_root.join("@home.moth"), "").expect("should write page root");
    fs::write(entry_root.join("@layout.moth"), "").expect("should write layout root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let failure = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect_err("a module directory may contain only one normal module root");
    assert_multiple_module_roots(failure, &fs::canonicalize(&entry_root).unwrap());
}

#[test]
fn explicitly_selected_hidden_root_compiles_but_nested_hidden_descendants_do_not() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let project_root = _tmp_root.path().to_path_buf();
    let selected = project_root.join("_prototype");
    fs::create_dir_all(selected.join("_private")).expect("should create hidden descendant");
    fs::create_dir_all(selected.join("visible")).expect("should create visible descendant");
    fs::write(selected.join("@site.moth"), "").expect("should write selected root");
    fs::write(selected.join("helper.moth"), "").expect("should write selected helper");
    fs::write(selected.join("_private/@hidden.moth"), "").expect("should write hidden descendant");
    fs::write(selected.join("visible/@visible.moth"), "").expect("should write visible descendant");

    let config = Config::new(selected.clone());
    let canonical_project_root =
        fs::canonicalize(&project_root).expect("project root should canonicalize");
    let canonical_selected =
        fs::canonicalize(&selected).expect("selected root should canonicalize");
    let mut string_table = StringTable::new();
    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_selected,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_project_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("an explicitly selected hidden root should index");

    let paths = index
        .sources()
        .iter()
        .map(|source| source.canonical_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("_prototype/@site.moth"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("_prototype/helper.moth"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("_prototype/visible/@visible.moth"))
    );
    assert!(
        paths
            .iter()
            .all(|path| !path.starts_with(selected.join("_private"))),
        "hidden descendants of an explicitly selected root remain excluded"
    );
}

#[cfg(unix)]
#[test]
fn source_tree_index_excludes_file_symlinks_into_hidden_directories() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");

    fs::write(root.join("@home.moth"), "").expect("should write entry root");
    fs::write(root.join("_helper.moth"), "").expect("underscore file remains eligible");
    let hidden = root.join("_private");
    fs::create_dir_all(&hidden).expect("should create hidden directory");
    fs::write(hidden.join("helper.moth"), "").expect("should write hidden helper");
    symlink(hidden.join("helper.moth"), root.join("visible.moth"))
        .expect("should create file symlink into hidden directory");
    fs::create_dir_all(root.join("docs")).expect("should create visible folder");
    symlink(hidden.join("helper.moth"), root.join("docs.moth"))
        .expect("should create colliding-name symlink into hidden directory");

    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();
    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_root.clone(),
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("a visible symlink into a hidden directory should be skipped, not fail discovery");

    let source_paths = index
        .sources()
        .iter()
        .map(|source| source.canonical_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        source_paths.iter().all(|path| !path.starts_with(&hidden)),
        "hidden physical files must not enter the source inventory"
    );
    assert!(
        source_paths
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == "_helper.moth")),
        "an underscore-prefixed file in a visible directory remains eligible"
    );
}

#[cfg(unix)]
#[test]
fn source_alias_into_configured_output_root_is_excluded() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::write(root.join("@home.moth"), "").expect("should write entry root");
    fs::create_dir_all(root.join("_scratch")).expect("should create configured output root");
    fs::create_dir_all(root.join("helper")).expect("should create colliding folder");
    let output_source = root.join("_scratch/helper.moth");
    fs::write(&output_source, "").expect("should write generated source");
    symlink(&output_source, root.join("helper.moth"))
        .expect("should create alias into configured output root");

    let mut config = Config::new(root.clone());
    config.output_roots.dev_output = Some(String::from("scratch"));
    config.output_roots.release_output = Some(String::from("generated"));
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let mut string_table = StringTable::new();
    let validated_output_settings = {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
        crate::build_system::project_config::validate_directory_output_settings(
            &config,
            &mut diagnostic_strings,
        )
    }
    .expect("configured output folders should validate");

    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_root.clone(),
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &canonical_root,
            validated_output_settings: Some(&validated_output_settings),
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("an alias into a configured output root should be skipped");

    assert_eq!(
        index.sources().len(),
        1,
        "the output alias must not enter candidate inventory or collide with helper/"
    );
    let canonical_output_source =
        fs::canonicalize(&output_source).expect("output source should canonicalize");
    assert!(
        index
            .sources()
            .iter()
            .all(|source| source.canonical_path() != canonical_output_source),
        "a configured-output alias must not contribute a source record"
    );
}
