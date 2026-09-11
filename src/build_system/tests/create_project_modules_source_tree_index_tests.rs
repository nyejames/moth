use super::*;
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
        "release",
        "dev",
        "dist",
        "build",
        ".cache",
        "generated",
        "scratch",
    ] {
        let directory = entry_root.join(directory_name);
        fs::create_dir_all(&directory).expect("should create skipped directory");
        fs::write(directory.join("@skipped.moth"), "").expect("should write skipped root");
    }

    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");
    fs::write(entry_root.join("ordinary.moth"), "").expect("should write ordinary source");
    fs::write(nested.join("@nested.moth"), "").expect("should write nested root");

    let mut config = Config::new(root.clone());
    config.html_section.dev_output = Some(String::from("scratch"));
    config.html_section.release_output = Some(String::from("generated"));
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");
    let mut string_table = StringTable::new();
    let validated_output_settings =
        crate::build_system::project_config::validate_directory_output_settings(
            &config,
            &mut string_table,
        )
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
    let graph = super::project_module_graph::ProjectModuleGraph::from_source_tree_index(&index);
    let entry_root_files: Vec<PathBuf> = graph
        .entry_modules()
        .iter()
        .map(|module_id| graph.node(*module_id).root_file().to_path_buf())
        .collect();
    assert_eq!(entry_root_files.len(), 2);
    assert!(entry_root_files[0].ends_with("@home.moth"));
    assert_eq!(index.stats().dirs_visited, 2);
    assert_eq!(index.stats().dirs_skipped, 10);
    assert_eq!(index.stats().files_seen, 3);
    assert_eq!(index.stats().normal_root_files_seen, 2);
    assert_eq!(index.stats().module_roots_found, 2);

    let root_directories = index
        .module_roots()
        .root_directories()
        .map(|path| root_directory_name(path.as_path()))
        .collect::<Vec<_>>();
    assert_eq!(
        root_directories[0],
        root_directory_name(&canonical_entry_root)
    );
    assert_eq!(root_directories[1], "nested");
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

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let canonical_root = fs::canonicalize(&root).expect("project root should canonicalize");
    let canonical_entry_root =
        fs::canonicalize(&entry_root).expect("entry root should canonicalize");

    let mut source_packages = crate::builder_surface::SourcePackageRegistry::default();
    source_packages.register_filesystem_root(
        "target",
        fs::canonicalize(entry_root.join("target")).unwrap(),
        PackageOrigin::Builder,
    );

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
fn source_tree_index_detects_collision_in_non_skipped_directory() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let entry_root = root.join("src");
    fs::create_dir_all(entry_root.join("helper")).expect("should create helper folder");
    fs::write(entry_root.join("helper.moth"), "x ~= 1\n").expect("should write colliding file");
    fs::write(entry_root.join("@home.moth"), "").expect("should write entry root");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
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
    let messages = failure.into_messages(&string_table);

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
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
    let messages = failure.into_messages(&string_table);

    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::SourceFileFolderCollision { .. }
    ));
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
    let messages = failure.into_messages(&string_table);

    let reason = first_invalid_config_reason(&messages);
    let InvalidConfigReason::MultipleModuleRootFiles {
        directory,
        candidates,
    } = reason
    else {
        panic!("expected duplicate module root diagnostic, got {reason:?}");
    };
    assert_eq!(
        messages.string_table.resolve(*directory),
        fs::canonicalize(&entry_root).unwrap().display().to_string()
    );
    assert_eq!(candidates.len(), 2);
}
