use super::root_file::{
    PreparedSourcePackageRoots, dependency_component_is_support_root_file,
    dependency_path_references_config_file, dependency_path_references_support_root_file,
    file_name_is_config_file, file_name_is_legacy_hash_root_file, file_name_is_module_root_file,
    file_name_is_normal_module_root_file, file_name_is_support_root_file,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;

fn path(components: &[&str], string_table: &mut StringTable) -> InternedPath {
    let mut path = InternedPath::new();
    for component in components {
        path.push_str(component, string_table);
    }
    path
}

#[test]
fn classifies_only_at_prefixed_moth_root_filenames() {
    assert!(file_name_is_normal_module_root_file("@home.moth"));
    assert!(file_name_is_normal_module_root_file("@anything.moth"));
    assert!(!file_name_is_normal_module_root_file("home.moth"));
    assert!(!file_name_is_normal_module_root_file("@home.js"));
    assert!(!file_name_is_normal_module_root_file("@.moth"));
    assert!(file_name_is_config_file("config.moth"));
    assert!(!file_name_is_config_file("config"));
}

#[test]
fn classifies_legacy_hash_prefixed_moth_root_filenames() {
    assert!(file_name_is_legacy_hash_root_file("#home.moth"));
    assert!(file_name_is_legacy_hash_root_file("#anything.moth"));
    assert!(!file_name_is_legacy_hash_root_file("home.moth"));
    assert!(!file_name_is_legacy_hash_root_file("#home.js"));
    assert!(!file_name_is_legacy_hash_root_file("#.moth"));
}

#[test]
fn dependency_components_identify_support_roots() {
    assert!(dependency_component_is_support_root_file("+pkg"));
    assert!(dependency_component_is_support_root_file("+pkg.moth"));
    assert!(!dependency_component_is_support_root_file("pkg"));
    assert!(!dependency_component_is_support_root_file("+pkg.js"));
}

#[test]
fn config_dependency_classification_uses_the_source_component() {
    let mut string_table = StringTable::new();

    let bare_config = path(&["config"], &mut string_table);
    assert!(dependency_path_references_config_file(
        &bare_config,
        &string_table
    ));

    let nested_config_folder = path(&["config", "settings"], &mut string_table);
    assert!(!dependency_path_references_config_file(
        &nested_config_folder,
        &string_table
    ));

    let ordinary_config_folder = path(&["config", "settings", "project"], &mut string_table);
    assert!(!dependency_path_references_config_file(
        &ordinary_config_folder,
        &string_table
    ));
}

#[test]
fn support_root_dependency_classification_uses_the_source_component() {
    let mut string_table = StringTable::new();

    let bare_support_root = path(&["modules", "+pkg"], &mut string_table);
    assert!(dependency_path_references_support_root_file(
        &bare_support_root,
        &string_table
    ));

    let ordinary_module = path(&["modules", "+pkg.moth", "symbol"], &mut string_table);
    assert!(!dependency_path_references_support_root_file(
        &ordinary_module,
        &string_table
    ));

    let ordinary_plus_extension = path(&["modules", "+pkg.js"], &mut string_table);
    assert!(!dependency_path_references_support_root_file(
        &ordinary_plus_extension,
        &string_table
    ));
}

#[test]
fn prepared_roots_preserve_canonical_prefix_order() {
    let entries = vec![
        (
            "zeta".to_string(),
            PathBuf::from("/lib/zeta"),
            PathBuf::from("/lib/zeta/@mod.moth"),
        ),
        (
            "alpha".to_string(),
            PathBuf::from("/lib/alpha"),
            PathBuf::from("/lib/alpha/@mod.moth"),
        ),
        (
            "middle".to_string(),
            PathBuf::from("/lib/middle"),
            PathBuf::from("/lib/middle/@mod.moth"),
        ),
    ];

    let prepared = PreparedSourcePackageRoots::from_entries(entries);

    let root_prefixes: Vec<&str> = prepared.roots().keys().map(|k| k.as_str()).collect();
    assert_eq!(root_prefixes, vec!["alpha", "middle", "zeta"]);

    let file_prefixes: Vec<&str> = prepared.root_files().keys().map(|k| k.as_str()).collect();
    assert_eq!(file_prefixes, vec!["alpha", "middle", "zeta"]);
}

#[test]
fn classifies_only_plus_prefixed_moth_filenames_as_support_roots() {
    assert!(file_name_is_support_root_file("+pkg.moth"));
    assert!(file_name_is_support_root_file("+anything.moth"));
    assert!(!file_name_is_support_root_file("@home.moth"));
    assert!(!file_name_is_support_root_file("pkg.moth"));
    assert!(!file_name_is_support_root_file("+pkg.js"));
    assert!(!file_name_is_support_root_file("+.moth"));
    assert!(file_name_is_module_root_file("@home.moth"));
    assert!(file_name_is_module_root_file("+pkg.moth"));
    assert!(!file_name_is_module_root_file("home.moth"));
}

#[test]
fn config_file_distinct_from_module_and_legacy_roots() {
    assert!(file_name_is_config_file("config.moth"));
    assert!(!file_name_is_normal_module_root_file("config.moth"));
    assert!(!file_name_is_legacy_hash_root_file("config.moth"));

    assert!(file_name_is_normal_module_root_file("@config.moth"));
    assert!(!file_name_is_config_file("@config.moth"));
    assert!(!file_name_is_legacy_hash_root_file("@config.moth"));

    assert!(file_name_is_legacy_hash_root_file("#config.moth"));
    assert!(!file_name_is_config_file("#config.moth"));
    assert!(!file_name_is_normal_module_root_file("#config.moth"));
}
