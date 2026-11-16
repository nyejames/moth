//! Source-tree discovery boundary tests.
//!
//! WHAT: covers one Stage 0 traversal's skip policy, hidden and fixed-skip aliases,
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
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::collections::BTreeSet;

fn single_file_dependency_keys(root: &Path) -> Result<BTreeSet<String>, PreparedFailure> {
    // Keys are independent of module topology; the moduleless scope also exercises the reserved-root
    // path walk that an ordinary selected file uses.
    super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(root, false, &[])
        .map(|inventory| inventory.keys)
}

fn bounded_roots_for_single_file(
    entry_file: &Path,
    config: &Config,
    string_table: &mut StringTable,
) -> Result<crate::compiler_frontend::paths::module_roots::ModuleRootTable, PreparedFailure> {
    super::source_tree_index::SourceTreeIndex::bounded_single_file_index(
        entry_file,
        config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        string_table,
    )
    .map(|index| {
        index
            .map(|index| index.module_identities().derive_compilation_root_table())
            .unwrap_or_default()
    })
}

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
        fs::write(directory.join("@entry.moth"), "").expect("should write skipped root");
        fs::write(directory.join("page.mtf"), "").expect("should write skipped mtf");
        fs::write(directory.join("note.md"), "").expect("should write skipped markdown");
        fs::write(directory.join("data.mon"), "").expect("should write skipped mon");
        fs::write(directory.join("provider.js"), "").expect("should write skipped provider");
    }

    // Directory names the old fixed policy reserved are ordinary source now; only hidden
    // directories and configured output roots stay excluded.
    let visible = entry_root.join("dev");
    fs::create_dir_all(&visible).expect("should create visible module directory");
    fs::write(visible.join("@entry.moth"), "").expect("should write visible root");

    // A visible `release` directory is ordinary source, not a generated-output skip.
    let visible_release = entry_root.join("release");
    fs::create_dir_all(&visible_release).expect("should create visible release source directory");
    fs::write(visible_release.join("@entry.moth"), "").expect("should write visible release root");

    fs::write(entry_root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(entry_root.join("ordinary.moth"), "").expect("should write ordinary source");
    fs::write(nested.join("@entry.moth"), "").expect("should write nested root");
    fs::write(nested.join("@config.moth"), "not source")
        .expect("should write nested reserved config");
    // A directory sharing the reserved config filename is inert too: it is neither traversed nor
    // claimed as an ordinary folder. Keep it under a different parent so the file and directory
    // cases can coexist.
    let reserved_directory_parent = entry_root.join("reserved_directory_parent");
    fs::create_dir_all(&reserved_directory_parent)
        .expect("should create reserved directory parent");
    let nested_config_directory = reserved_directory_parent.join("@config.moth");
    fs::create_dir_all(&nested_config_directory).expect("should create reserved config directory");
    fs::write(nested_config_directory.join("@entry.moth"), "")
        .expect("should write unreachable root");

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
    assert!(entry_root_files[0].ends_with("@entry.moth"));
    assert!(entry_root_files[1].ends_with("dev/@entry.moth"));
    assert!(entry_root_files[2].ends_with("nested/@entry.moth"));
    assert!(entry_root_files[3].ends_with("release/@entry.moth"));
    assert_eq!(index.stats().dirs_visited, 5);
    assert_eq!(index.stats().dirs_skipped, 6);
    assert_eq!(index.stats().files_seen, 6);
    assert_eq!(index.stats().normal_root_files_seen, 0);
    assert_eq!(index.stats().entry_root_files_seen, 4);
    assert_eq!(index.stats().module_roots_found, 4);
    assert!(
        index.sources().iter().all(|source| {
            source
                .canonical_path()
                .file_name()
                .is_some_and(|name| name != "@config.moth")
                && !source
                    .canonical_path()
                    .starts_with(&nested_config_directory)
        }),
        "a nested @config.moth file or directory is reserved and must not enter source inventory"
    );

    let module_roots = index.module_roots();
    let root_directories = module_roots
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
    fs::write(entry_root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(nested.join("@entry.moth"), "").expect("should write nested root");
    fs::write(nested.join("@config.moth"), "not source")
        .expect("should write nested reserved config");

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

    // Fixed-skipped directory whose name matches the leaf of a source-backed package prefix.
    // Under the skip policy this folder is not dependency-bindable, so no prefix collision.
    fs::create_dir_all(entry_root.join("builder/target")).expect("should create target folder");
    fs::write(entry_root.join("@entry.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages
        .register_filesystem_root(
            "builder/target",
            fs::canonicalize(entry_root.join("builder/target")).unwrap(),
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
    fs::create_dir_all(entry.join("builder")).expect("should create ancestor folder");
    fs::write(entry.join("@entry.moth"), "").expect("should write entry root");
    let config = Config::new(root.clone());
    let mut packages = crate::builder_surface::SourcePackageRegistry::default();
    packages
        .register_filesystem_root(
            "builder/web/site",
            root.join("package"),
            PackageOrigin::Builder,
        )
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
            portable_path_text(
                folder
                    .split('/')
                    .fold(entry.clone(), |path, component| path.join(component)),
            ),
            "{reason}"
        );
    };

    discover().expect("ancestor folder alone must be allowed");
    fs::create_dir_all(entry.join("builder/web/site")).expect("should create exact nested folder");
    assert_collision("builder/web/site", "exact nested path must collide");

    // Module ownership of the folder doesn't change its entry-root-relative identity.
    fs::write(entry.join("builder/web/@mod.moth"), "").expect("should write nested module root");
    assert_collision(
        "builder/web/site",
        "exact nested path inside a nested module must still collide",
    );

    // Package identities reject ASCII case-only overlaps, so case variants claim the same path
    // on case-sensitive and case-insensitive hosts alike.
    fs::remove_dir(entry.join("builder/web/site")).expect("should remove exact nested folder");
    fs::create_dir(entry.join("builder/web/Site")).expect("should create case-variant leaf folder");
    assert_collision(
        "builder/web/Site",
        "case-variant leaf inside a nested module must collide",
    );

    fs::remove_dir_all(entry.join("builder")).expect("should remove lowercase ancestor");
    fs::create_dir(entry.join("BUILDER")).expect("should create case-variant ancestor");
    discover().expect("case-variant ancestor alone must be allowed");
    fs::create_dir_all(entry.join("BUILDER/web/site"))
        .expect("should create case-variant nested folder");
    assert_collision(
        "BUILDER/web/site",
        "case-variant ancestor path must collide",
    );
}

#[test]
fn source_tree_index_rejects_project_folders_that_claim_official_core_package_prefixes() {
    // `@core/html` and `@core/css` are registered Core roots on every builder, so a project path
    // at the same entry-root-relative location would claim an official package namespace.
    // Discovery reports that collision instead of giving the local path precedence over the Core
    // package, and identity compares ASCII-case-insensitively like every package path.
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = fs::canonicalize(temp.path()).expect("temp dir should canonicalize");
    let entry = root.join("src");
    fs::create_dir_all(&entry).expect("should create entry root");
    fs::write(entry.join("@entry.moth"), "").expect("should write entry root");
    let config = Config::new(root.clone());
    let packages = crate::builder_surface::BuilderSurface::with_mandatory_core().source_packages;
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
    let assert_collision = |folder: &str, expected_prefix: &str, reason: &str| {
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
                .resolve(diagnostic.string_ids()[0]),
            expected_prefix,
            "{reason}: the collision must name the registered Core prefix"
        );
        assert_eq!(
            report
                .context()
                .strings()
                .resolve(diagnostic.string_ids()[1]),
            portable_path_text(entry.join(folder)),
            "{reason}"
        );
    };

    // Ordinary local style-related names stay legal beside the grouped Core registrations.
    for folder in ["html", "styles", "css"] {
        fs::create_dir(entry.join(folder)).expect("should create ordinary project folder");
        discover().expect("an ordinary local folder must not claim a Core package");
        fs::remove_dir(entry.join(folder)).expect("should remove ordinary project folder");
    }

    // An ancestor folder alone does not claim a complete registered prefix.
    fs::create_dir(entry.join("core")).expect("should create namespace ancestor folder");
    discover().expect("the Core namespace ancestor alone must not collide");

    for (folder, expected_prefix, reason) in [
        (
            "core/html",
            "core/html",
            "a project path claiming the Core @core/html prefix must collide",
        ),
        (
            "core/HTML",
            "core/html",
            "a case-only leaf variant of the Core @core/html prefix must collide",
        ),
        (
            "core/css",
            "core/css",
            "a project path claiming the Core @core/css prefix must collide",
        ),
        (
            "Core/HTML",
            "core/html",
            "a case-only ancestor and leaf variant must collide",
        ),
    ] {
        if folder.starts_with("Core/") {
            // Remove the lowercase ancestor so a case-insensitive host preserves the authored
            // casing this case asserts in the diagnostic path.
            fs::remove_dir(entry.join("core")).expect("should remove previous ancestor folder");
        }
        fs::create_dir_all(entry.join(folder)).expect("should create colliding project path");
        assert_collision(folder, expected_prefix, reason);
        fs::remove_dir_all(entry.join(folder)).expect("should remove colliding project path");
    }

    // Only the complete registered prefix is reserved; a folder that merely starts with it stays.
    fs::create_dir_all(entry.join("core/html_local"))
        .expect("should create unrelated project folder");
    discover().expect("a folder outside the official prefix must not collide");
}

/// Discover one project tree whose only root is an empty `@entry.moth`.
///
/// WHY: reserved-root ownership is decision-point specific, so cases that mix legal directory
/// paths with source claims need a complete traversal rather than a single-directory probe.
fn discover_project_with(files: &[(&str, &str)]) -> Result<(), PreparedFailure> {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = fs::canonicalize(temp.path()).expect("temp dir should canonicalize");
    let entry = root.join("src");
    fs::create_dir_all(&entry).expect("should create entry root");
    fs::write(entry.join("@entry.moth"), "").expect("should write entry root");
    for (relative, contents) in files {
        let path = entry.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("should create fixture directory");
        }
        fs::write(&path, contents).expect("should write fixture file");
    }
    let config = Config::new(root.clone());
    let packages = crate::builder_surface::BuilderSurface::with_mandatory_core().source_packages;
    super::source_tree_index::SourceTreeIndex::discover(
        entry,
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
    .map(|_| ())
}

#[test]
fn source_tree_index_does_not_reserve_reserved_root_directory_names() {
    // `@core` and `@builder` reserve importable package namespace roots, not directory names.
    // Stage 0 owns the filesystem traversal and collision facts; namespace ownership is checked
    // where the importable namespaces exist. These cases must therefore index without a
    // reserved-root diagnostic even though their paths pass through `core` or `builder`.
    for (case, files) in [
        (
            "entry-only core module",
            vec![("core/@entry.moth", "[:entry]\n")],
        ),
        (
            "core directory containing only entry modules",
            vec![
                ("core/@entry.moth", "[:entry]\n"),
                ("core/nested/@entry.moth", "[:nested]\n"),
            ],
        ),
        (
            "entry-only module with a private helper",
            vec![
                ("core/@entry.moth", "[:entry]\n"),
                ("core/helper.moth", "value #= 1\n"),
            ],
        ),
        // Only source claims are reserved. An ordinary asset below `core/` remains a resource.
        (
            "asset-only core folder",
            vec![("core/logo.png", "not source")],
        ),
        (
            "asset-only builder folder",
            vec![("builder/theme.css", "body { color: red; }")],
        ),
        // A module below `src/` is addressed as `src/core`, so it claims no reserved root.
        ("nested ordinary module", vec![("src/core/@mod.moth", "")]),
        // A support facade below a reserved directory is imported by its leaf name.
        (
            "support facade below a reserved directory",
            vec![("core/tools/@package.moth", "")],
        ),
        // Unrelated local names stay legal, including names used by registered prefixes.
        (
            "local web support facade",
            vec![(
                "web/@package.moth",
                "-- local copy of the shipped site facade\n$export\nroot #String = @/\n",
            )],
        ),
        ("local html module", vec![("html/@mod.moth", "")]),
        ("local css module", vec![("css/@mod.moth", "")]),
        (
            "local styles support package",
            vec![("styles/@package.moth", "")],
        ),
    ] {
        discover_project_with(&files).expect(case);
    }
}

#[test]
fn source_tree_index_detects_collision_in_non_skipped_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(entry_root.join("helper")).expect("should create helper folder");
    fs::write(entry_root.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(entry_root.join("@entry.moth"), "").expect("should write entry root");

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
fn bounded_single_file_index_indexes_nested_roots_with_ignored_directories() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    let nested = module_dir.join("nested");
    fs::create_dir_all(&nested).expect("should create nested module");

    // Ignored directory with collision-shaped contents.
    let target_dir = module_dir.join("target");
    fs::create_dir_all(target_dir.join("helper")).expect("should create target/helper");
    fs::write(target_dir.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");

    fs::write(module_dir.join("@entry.moth"), "").expect("should write entry root");
    fs::write(nested.join("@entry.moth"), "").expect("should write nested root");
    fs::write(nested.join("@config.moth"), "not source")
        .expect("should write nested reserved config");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("@entry.moth")).unwrap();
    let mut string_table = StringTable::new();

    let module_roots = bounded_roots_for_single_file(&entry_file, &config, &mut string_table)
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
fn bounded_single_file_index_accepts_each_fixed_root_role() {
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;

    for (root_file_name, expected_role) in [
        ("@mod.moth", ModuleRootRole::Normal),
        ("@package.moth", ModuleRootRole::Support),
        ("@entry.moth", ModuleRootRole::Entry),
    ] {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let module_dir = root.join("module");
        fs::create_dir_all(&module_dir).expect("should create module directory");
        fs::write(module_dir.join(root_file_name), "").expect("should write selected root");
        fs::write(module_dir.join("helper.moth"), "").expect("should write ordinary source");

        let config = Config::new(root.clone());
        let canonical_module_dir =
            fs::canonicalize(&module_dir).expect("module directory should canonicalize");
        let entry_file = canonical_module_dir.join(root_file_name);
        let mut string_table = StringTable::new();

        let module_roots = bounded_roots_for_single_file(&entry_file, &config, &mut string_table)
            .unwrap_or_else(|failure| {
                panic!("{root_file_name} selection should index its bounded tree: {failure:?}")
            });

        let expected_root = canonical_module_dir.join(root_file_name);
        let roles = module_roots.root_file_roles().collect::<Vec<_>>();
        assert_eq!(
            roles,
            vec![(expected_role, expected_root.as_path())],
            "{root_file_name} selection should keep its authored root role"
        );

        // The synthetic namespace owner reads local keys for every prepared root directory, so
        // the same selection must not disappear from that inventory.
        let local_keys = single_file_dependency_keys(&canonical_module_dir)
            .expect("synthetic dependency keys should index the selected root directory");
        assert!(
            local_keys.contains("helper"),
            "{root_file_name}: {local_keys:?}"
        );
    }
}

#[test]
fn bounded_single_file_index_keeps_every_triple_root_role() {
    use crate::compiler_frontend::semantic_identity::ModuleRootRole;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    fs::create_dir_all(&module_dir).expect("should create module directory");
    for root_file_name in ["@mod.moth", "@package.moth", "@entry.moth"] {
        fs::write(module_dir.join(root_file_name), "").expect("should write authored root");
    }
    fs::write(module_dir.join("helper.moth"), "").expect("should write ordinary source");

    let config = Config::new(root.clone());
    let canonical_module_dir =
        fs::canonicalize(&module_dir).expect("module directory should canonicalize");
    let entry_file = canonical_module_dir.join("@entry.moth");
    let mut string_table = StringTable::new();

    // An explicitly selected entry root must not collapse its bounded directory's sibling API
    // facades. Preparation consumes this role table for every reached root file, so dropping a
    // sibling here would silently lose its facade role.
    let module_roots = bounded_roots_for_single_file(&entry_file, &config, &mut string_table)
        .expect("triple-root single-file selection should index its bounded tree");

    let roles = module_roots.root_file_roles().collect::<Vec<_>>();
    assert_eq!(
        roles,
        vec![
            (
                ModuleRootRole::Normal,
                canonical_module_dir.join("@mod.moth").as_path()
            ),
            (
                ModuleRootRole::Support,
                canonical_module_dir.join("@package.moth").as_path()
            ),
            (
                ModuleRootRole::Entry,
                canonical_module_dir.join("@entry.moth").as_path()
            ),
        ],
        "the bounded inventory keeps all three authored roles for an entry selection"
    );
}

#[test]
fn bounded_single_file_index_rejects_dependency_name_collisions() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let module_dir = root.join("module");
    fs::create_dir_all(module_dir.join("helper")).expect("should create helper directory");
    fs::write(module_dir.join("helper.moth"), "helper #= 1\n")
        .expect("should write colliding source file");
    fs::write(module_dir.join("@entry.moth"), "").expect("should write entry root");

    let config = Config::new(root.clone());
    let entry_file = fs::canonicalize(module_dir.join("@entry.moth")).unwrap();
    let mut string_table = StringTable::new();

    let failure = bounded_roots_for_single_file(&entry_file, &config, &mut string_table)
        .expect_err(
            "single-file normal module roots should reject real dependency-name collisions",
        );
    assert_source_file_folder_collision(failure);
}

#[test]
fn invalid_root_beside_canonical_facade_is_rejected() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(&entry_root).expect("should create entry root");
    fs::write(entry_root.join("@mod.moth"), "").expect("should write canonical facade root");
    fs::write(entry_root.join("@page.moth"), "").expect("should write invalid root filename");

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
    .expect_err("an invalid @page.moth root is rejected by the closed vocabulary");
    let boundary = diagnosed_reports(failure);
    let (_report, diagnostic) = first_invalid_config(&boundary);
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("config failure should retain typed facts");
    assert_eq!(facts.reason, InvalidConfigCode::InvalidModuleRootFileName);
}

#[test]
fn marked_project_root_follows_the_invalid_root_lane() {
    for invalid_root in ["@project.moth", "+project.moth", "#project.moth"] {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let entry_root = root.join("src");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::write(entry_root.join(invalid_root), "").expect("should write invalid root marker");

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
        .expect_err("a marked project root name is an invalid root filename");
        let boundary = diagnosed_reports(failure);
        let (_report, diagnostic) = first_invalid_config(&boundary);
        let facts = diagnostic
            .family_view::<InvalidConfigView>()
            .expect("invalid root failure should retain typed facts");
        assert_eq!(
            facts.reason,
            InvalidConfigCode::InvalidModuleRootFileName,
            "{invalid_root} must use the invalid-root lane, not the project-globals reservation"
        );
    }
}

#[test]
fn explicitly_selected_hidden_root_compiles_but_nested_hidden_descendants_do_not() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let project_root = _tmp_root.path().to_path_buf();
    let selected = project_root.join("_prototype");
    fs::create_dir_all(selected.join("_private")).expect("should create hidden descendant");
    fs::create_dir_all(selected.join("visible")).expect("should create visible descendant");
    fs::write(selected.join("@entry.moth"), "").expect("should write selected root");
    fs::write(selected.join("helper.moth"), "").expect("should write selected helper");
    fs::write(selected.join("_private/@entry.moth"), "").expect("should write hidden descendant");
    fs::write(selected.join("visible/@entry.moth"), "").expect("should write visible descendant");

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
            .any(|path| path.ends_with("_prototype/@entry.moth"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("_prototype/helper.moth"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with("_prototype/visible/@entry.moth"))
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
fn directory_alias_into_hidden_tree_creates_no_namespace_or_collision() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let hidden = root.join("_generated");
    fs::create_dir_all(&hidden).expect("should create hidden directory");
    fs::write(hidden.join("@entry.moth"), "").expect("should write hidden root");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(root.join("helper.moth"), "").expect("should write colliding file");
    symlink(&hidden, root.join("helper")).expect("should create hidden directory alias");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
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
    .expect("a hidden directory alias must not create a file/folder collision");

    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    assert_eq!(
        graph.entry_modules().len(),
        1,
        "only the visible entry root may become a module"
    );
    assert_eq!(index.stats().dirs_skipped, 2);
    let source_paths = index
        .sources()
        .iter()
        .map(|source| source.canonical_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(
        source_paths.iter().all(|path| !path.starts_with(&hidden)),
        "the alias target must contribute no source records"
    );
    assert!(
        source_paths
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == "helper.moth")),
        "the ordinary helper file remains eligible"
    );
}

#[cfg(unix)]
#[test]
fn directory_alias_into_fixed_skipped_descendant_is_excluded() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::create_dir_all(root.join("target/nested")).expect("should create skipped descendant");
    fs::write(root.join("target/nested/@entry.moth"), "").expect("should write skipped root");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    symlink(root.join("target/nested"), root.join("visible"))
        .expect("should create fixed-skipped descendant alias");

    let config = Config::new(root.clone());
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
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
    .expect("a fixed-skipped descendant alias must be excluded by physical ancestry");

    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    assert_eq!(graph.entry_modules().len(), 1);
    assert_eq!(index.stats().dirs_skipped, 2);
    assert!(
        index
            .sources()
            .iter()
            .all(|source| !source.canonical_path().starts_with(root.join("target")))
    );
}

#[cfg(unix)]
#[test]
fn directory_alias_outside_entry_root_does_not_create_a_folder_claim() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let project_root = _tmp_root.path().to_path_buf();
    let entry_root = project_root.join("src");
    let outside = project_root.join("outside");
    fs::create_dir_all(&entry_root).expect("should create selected source root");
    fs::create_dir_all(&outside).expect("should create external directory");
    fs::write(outside.join("@entry.moth"), "").expect("should write external root");
    fs::write(entry_root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(entry_root.join("helper.moth"), "").expect("should write colliding file");
    symlink(&outside, entry_root.join("helper")).expect("should create external directory alias");

    let config = Config::new(project_root.clone());
    let canonical_project_root =
        fs::canonicalize(&project_root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let index = super::source_tree_index::SourceTreeIndex::discover(
        canonical_entry_root,
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
    .expect("an external directory alias must not create a file/folder collision");

    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    assert_eq!(graph.entry_modules().len(), 1);
    assert_eq!(index.stats().dirs_skipped, 1);
    assert!(
        index
            .sources()
            .iter()
            .all(|source| !source.canonical_path().starts_with(&outside))
    );
}

#[test]
fn single_file_dependency_keys_include_ordinary_plus_sources_and_reject_invalid_roots() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(root.join("ordinary.moth"), "").expect("should write ordinary source");
    fs::write(root.join("+notes.mtf"), "").expect("should write ordinary plus content source");
    fs::write(root.join("+bad.moth"), "").expect("should write invalid plus root marker");
    fs::write(root.join("#bad.moth"), "").expect("should write invalid hash root marker");
    fs::write(root.join("@bad.moth"), "").expect("should write invalid at root marker");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let keys = single_file_dependency_keys(&canonical_root)
        .expect("synthetic dependency keys should classify ordinary and invalid names");

    assert!(keys.contains("+notes"), "{keys:?}");
    assert!(keys.contains("ordinary"), "{keys:?}");
    assert!(!keys.contains("+bad"), "{keys:?}");
    assert!(!keys.contains("#bad"), "{keys:?}");
    assert!(!keys.contains("@bad"), "{keys:?}");
    assert!(!keys.contains("@entry"), "{keys:?}");
}

#[test]
fn single_file_dependency_keys_exclude_hidden_and_fixed_directories() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(root.join("helper.moth"), "").expect("should write ordinary source");
    fs::write(root.join("_helper.moth"), "").expect("should write underscore source");
    fs::create_dir_all(root.join("_hidden")).expect("should create hidden directory");
    fs::write(root.join("_hidden/@entry.moth"), "").expect("should write hidden root");
    fs::create_dir_all(root.join("target")).expect("should create fixed-skipped directory");
    fs::write(root.join("target/@entry.moth"), "").expect("should write fixed-skipped root");
    fs::create_dir_all(root.join("visible")).expect("should create visible directory");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let keys = single_file_dependency_keys(&canonical_root)
        .expect("synthetic dependency keys should respect the directory skip policy");

    assert!(keys.contains("helper"));
    assert!(keys.contains("_helper"));
    assert!(keys.contains("visible"));
    assert!(!keys.contains("_hidden"));
    assert!(!keys.contains("target"));
}

#[cfg(unix)]
#[test]
fn single_file_dependency_keys_exclude_alias_targets_by_physical_ancestry() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let project_root = _tmp_root.path().to_path_buf();
    let source_root = project_root.join("src");
    let outside = project_root.join("outside");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::create_dir_all(source_root.join("_hidden")).expect("should create hidden directory");
    fs::create_dir_all(source_root.join("target/nested"))
        .expect("should create fixed-skipped descendant");
    fs::create_dir_all(&outside).expect("should create external directory");
    fs::write(source_root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(source_root.join("helper.moth"), "").expect("should write ordinary source");
    fs::write(source_root.join("_helper.moth"), "").expect("should write underscore source");
    fs::write(source_root.join("_hidden/helper.moth"), "").expect("should write hidden source");
    fs::write(source_root.join("target/nested/helper.moth"), "")
        .expect("should write fixed-skipped source");
    fs::write(outside.join("helper.moth"), "").expect("should write external source");
    symlink(
        source_root.join("helper.moth"),
        source_root.join("alias.moth"),
    )
    .expect("should create eligible source alias");
    symlink(
        source_root.join("_hidden"),
        source_root.join("hidden_alias"),
    )
    .expect("should create hidden directory alias");
    symlink(
        source_root.join("target/nested"),
        source_root.join("fixed_alias"),
    )
    .expect("should create fixed-skipped descendant alias");
    symlink(&outside, source_root.join("outside_alias"))
        .expect("should create external directory alias");
    symlink(
        source_root.join("_hidden/helper.moth"),
        source_root.join("hidden_file.moth"),
    )
    .expect("should create hidden file alias");
    symlink(
        outside.join("helper.moth"),
        source_root.join("outside_file.moth"),
    )
    .expect("should create external file alias");

    let canonical_root = fs::canonicalize(&source_root).expect("source root should canonicalize");
    let keys = single_file_dependency_keys(&canonical_root)
        .expect("synthetic dependency keys should resolve alias destinations");

    assert_eq!(
        keys,
        ["_helper", "alias", "helper"]
            .into_iter()
            .map(String::from)
            .collect()
    );
}

#[cfg(unix)]
#[test]
fn single_file_inventory_claims_eligible_sources_below_reserved_directory_aliases() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let core = root.join("core");
    fs::create_dir(&core).expect("should create core directory");
    symlink(&core, core.join("loop")).expect("should create a reserved subtree loop alias");
    fs::write(root.join("linked.moth"), "value #= 1\n").expect("should write linked source");
    symlink(root.join("linked.moth"), core.join("alias.moth"))
        .expect("should create eligible file alias below core");
    let real_directory = root.join("real");
    fs::create_dir(&real_directory).expect("should create real directory");
    fs::write(real_directory.join("helper.moth"), "value #= 1\n")
        .expect("should write aliased helper");
    symlink(&real_directory, core.join("alias_directory"))
        .expect("should create eligible directory alias below core");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &[],
    )
    .expect("reserved directory inventory should resolve aliases");
    let (claimed_root, claim_path) = inventory
        .reserved_root_claim
        .expect("an eligible source below core should claim the reserved root");
    assert_eq!(claimed_root, "core");
    assert!(
        claim_path.ends_with("core/alias.moth")
            || claim_path.ends_with("core/alias_directory/helper.moth"),
        "claim must name an eligible alias target: {claim_path:?}"
    );

    // A reserved directory whose only source aliases leave the boundary contributes no claim.
    let _outside_tmp = tempfile::tempdir().expect("should create outside temp dir");
    let outside = _outside_tmp.path().join("outside.moth");
    fs::write(&outside, "value #= 1\n").expect("should write outside source");
    let isolated = _tmp_root.path().join("isolated");
    fs::create_dir(&isolated).expect("should create isolated root");
    let isolated_core = isolated.join("core");
    fs::create_dir(&isolated_core).expect("should create isolated core directory");
    symlink(&outside, isolated_core.join("outside_alias.moth"))
        .expect("should create external source alias");
    let canonical_isolated =
        fs::canonicalize(&isolated).expect("isolated source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_isolated,
        false,
        &[],
    )
    .expect("external aliases should be skipped");
    assert_eq!(
        inventory.reserved_root_claim, None,
        "an alias leaving the boundary must not claim the reserved root"
    );
}

#[test]
fn moduleless_single_file_scope_treats_entry_only_reserved_subtree_as_private() {
    // An ordinary selected file owns no module identities, so the inventory approximates
    // module privacy structurally: an entry-only reserved subtree publishes no name to its owner
    // and its helpers stay legal. Directory compilation validates the same shape through module
    // namespaces instead.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::create_dir_all(root.join("core/nested")).expect("should create nested directory");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(root.join("core/@entry.moth"), "[:core]\n").expect("should write entry marker");
    fs::write(root.join("core/helper.moth"), "value #= 1\n").expect("should write helper");
    fs::write(root.join("core/nested/@mod.moth"), "$export\nvalue #= 1\n")
        .expect("should write child facade");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &[],
    )
    .expect("flat inventory should resolve");
    assert_eq!(
        inventory.reserved_root_claim, None,
        "an entry-only reserved subtree is private content, not an importable claim"
    );
    assert!(
        !inventory.keys.contains("core"),
        "a reserved directory is not a package-collision key"
    );
}

#[test]
fn moduleless_single_file_scope_ignores_root_markers_and_asset_directories() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::create_dir_all(root.join("core")).expect("should create core directory");
    fs::create_dir_all(root.join("builder/tools")).expect("should create nested support directory");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    fs::write(root.join("core/@entry.moth"), "[:core]\n").expect("should write entry marker");
    fs::write(
        root.join("builder/tools/@package.moth"),
        "$export\nroot #String = @/\n",
    )
    .expect("should write nested support marker");
    fs::write(root.join("core/logo.png"), "not source").expect("should write asset");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &[],
    )
    .expect("asset-only reserved directories should resolve");
    assert_eq!(
        inventory.reserved_root_claim, None,
        "root markers and provider assets are not importable dependency names"
    );
}

/// Whether the test filesystem distinguishes names that differ only by ASCII case.
fn filesystem_is_case_sensitive(root: &Path) -> bool {
    let lower = root.join("case_probe");
    fs::create_dir(&lower).expect("should create case probe");
    let upper = root.join("CASE_PROBE");
    let case_sensitive = fs::create_dir(&upper).is_ok();
    if case_sensitive {
        fs::remove_dir(&upper).expect("should remove upper case probe");
    }
    fs::remove_dir(&lower).expect("should remove case probe");
    case_sensitive
}

#[test]
fn moduleless_single_file_inventory_reports_exact_registered_package_prefix_collisions() {
    // Directory compilation reports a local path that equals a registered package prefix even
    // when the folder publishes no interface. An ordinary selected file owns no Stage 0
    // traversal, so its inventory probes only the registered prefixes' relative paths.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::create_dir_all(root.join("core/html")).expect("should create colliding directory");
    fs::write(root.join("core/html/logo.png"), "not source").expect("should write colliding asset");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["core/html", "core/css"],
    )
    .expect("registered prefix inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision,
        Some(("core/html".to_owned(), canonical_root.join("core/html"))),
        "an exact registered prefix directory must collide even without source"
    );

    // The reserved ancestor alone stays legal, and a folder outside the complete registered path
    // is not a claim on the package identity.
    fs::remove_dir_all(root.join("core/html")).expect("should remove colliding directory");
    fs::create_dir(root.join("core/html_local")).expect("should create unrelated directory");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["core/html"],
    )
    .expect("unrelated directory inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision, None,
        "only the complete registered prefix collides"
    );

    // Package identities reject ASCII case-only overlaps on every host, so a case variant claims
    // the same registered path.
    fs::remove_dir_all(root.join("core")).expect("should remove namespace ancestor");
    fs::create_dir_all(root.join("Core/HTML")).expect("should create case-variant directory");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["core/html"],
    )
    .expect("case-variant inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision,
        Some(("core/html".to_owned(), canonical_root.join("Core/HTML"))),
        "a case-variant registered prefix path must collide"
    );
}

#[test]
fn moduleless_single_file_inventory_probes_every_case_equivalent_prefix_component() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    if !filesystem_is_case_sensitive(&root) {
        return;
    }

    // `Core` sorts before `core` but leads nowhere deeper, and `HTML` sorts before `html` beneath
    // `core`. The inventory must probe every case-equivalent directory at each parent component
    // instead of keeping only the first spelling.
    fs::create_dir(root.join("Core")).expect("should create empty case variant");
    fs::create_dir(root.join("core")).expect("should create lowercase parent");
    fs::create_dir(root.join("core/HTML")).expect("should create empty nested variant");
    fs::create_dir_all(root.join("core/html/deep")).expect("should create deeper path");
    fs::write(root.join("core/html/deep/logo.png"), "not source").expect("should write asset");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["core/html/deep"],
    )
    .expect("case-variant prefix inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision,
        Some((
            "core/html/deep".to_owned(),
            canonical_root.join("core").join("html").join("deep")
        )),
        "every case-equivalent parent candidate must be probed"
    );
}

#[cfg(unix)]
#[test]
fn moduleless_single_file_inventory_uses_canonical_parent_identity_for_prefix_collisions() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::create_dir_all(root.join("other/html")).expect("should create asset directory");
    fs::write(root.join("other/html/logo.png"), "not source").expect("should write asset");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    symlink(root.join("other"), root.join("core")).expect("should create parent alias");

    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["core/html"],
    )
    .expect("aliased prefix inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision, None,
        "a symlink alias must not claim its destination's parent identity"
    );

    // The same tree is still a collision under the destination's canonical registered path, as it
    // is for directory compilation.
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["other/html"],
    )
    .expect("canonical prefix inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision,
        Some(("other/html".to_owned(), canonical_root.join("other/html"))),
        "the canonical destination keeps its own registered prefix identity"
    );
}

#[cfg(unix)]
#[test]
fn moduleless_single_file_inventory_matches_leaf_aliases_by_authored_name() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    fs::create_dir_all(root.join("core/html_target")).expect("should create asset directory");
    fs::write(root.join("core/html_target/logo.png"), "not source").expect("should write asset");
    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    symlink(root.join("core/html_target"), root.join("core/html"))
        .expect("should create leaf alias");

    // The traversal's leaf check reads listed folder names, so the alias claims the prefix even
    // though its destination has a different name.
    let canonical_root = fs::canonicalize(&root).expect("source root should canonicalize");
    let inventory = super::source_tree_index::SourceTreeIndex::single_file_namespace_inventory(
        &canonical_root,
        false,
        &["core/html"],
    )
    .expect("leaf alias inventory should resolve");
    assert_eq!(
        inventory.registered_prefix_collision,
        Some(("core/html".to_owned(), canonical_root.join("core/html"))),
        "the authored leaf alias must claim the registered prefix"
    );
}

#[cfg(unix)]
#[test]
fn source_tree_index_excludes_file_symlinks_into_hidden_directories() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");

    fs::write(root.join("@entry.moth"), "").expect("should write entry root");
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
