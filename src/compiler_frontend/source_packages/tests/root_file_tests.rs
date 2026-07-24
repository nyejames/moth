use super::root_file::{
    HashRootFileDiscovery, PreparedSourcePackageRoots, discover_hash_root_file,
    file_name_is_config_file, file_name_is_hash_root_file, file_name_is_module_root_file,
    file_name_is_support_root_file, hash_root_file_name_from_import_component,
    import_component_is_hash_root_file, import_path_references_config_file,
    import_path_references_hash_root_file,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::PathBuf;

fn path(components: &[&str], string_table: &mut StringTable) -> InternedPath {
    let mut path = InternedPath::new();
    for component in components {
        path.push_str(component, string_table);
    }
    path
}

#[test]
fn classifies_only_hash_prefixed_moth_root_filenames() {
    assert!(file_name_is_hash_root_file("#home.moth"));
    assert!(file_name_is_hash_root_file("#anything.moth"));
    assert!(!file_name_is_hash_root_file("home.moth"));
    assert!(!file_name_is_hash_root_file("#home.js"));
    assert!(!file_name_is_hash_root_file("#.moth"));
    assert!(file_name_is_config_file("config.moth"));
    assert!(!file_name_is_config_file("config"));
}

#[test]
fn import_components_accept_extensionless_hash_roots_only() {
    assert!(import_component_is_hash_root_file("#home"));
    assert!(import_component_is_hash_root_file("#home.moth"));
    assert!(!import_component_is_hash_root_file("#home.js"));
    assert!(!import_component_is_hash_root_file("#home.page"));
    assert_eq!(
        hash_root_file_name_from_import_component("#home"),
        Some("#home.moth".to_owned())
    );
    assert_eq!(
        hash_root_file_name_from_import_component("#home.moth"),
        Some("#home.moth".to_owned())
    );
}

#[test]
fn config_import_classification_uses_the_source_component() {
    let mut string_table = StringTable::new();

    let bare_config = path(&["config"], &mut string_table);
    assert!(import_path_references_config_file(
        &bare_config,
        false,
        &string_table
    ));

    let grouped_config = path(&["config", "project"], &mut string_table);
    assert!(import_path_references_config_file(
        &grouped_config,
        true,
        &string_table
    ));

    let nested_config_folder = path(&["config", "settings"], &mut string_table);
    assert!(!import_path_references_config_file(
        &nested_config_folder,
        false,
        &string_table
    ));

    let grouped_config_folder = path(&["config", "settings", "project"], &mut string_table);
    assert!(!import_path_references_config_file(
        &grouped_config_folder,
        true,
        &string_table
    ));
}

#[test]
fn hash_root_import_classification_uses_the_source_component() {
    let mut string_table = StringTable::new();

    let bare_hash_root = path(&["modules", "#home"], &mut string_table);
    assert!(import_path_references_hash_root_file(
        &bare_hash_root,
        false,
        &string_table
    ));

    let grouped_hash_root = path(&["modules", "#home.moth", "symbol"], &mut string_table);
    assert!(import_path_references_hash_root_file(
        &grouped_hash_root,
        true,
        &string_table
    ));

    let ordinary_hash_extension = path(&["modules", "#home.js"], &mut string_table);
    assert!(!import_path_references_hash_root_file(
        &ordinary_hash_extension,
        false,
        &string_table
    ));
}

#[test]
fn discovers_sorted_direct_child_hash_roots_and_ignores_config_and_nested_files() {
    let directory = tempfile::tempdir().expect("failed to create temp directory");
    fs::create_dir(directory.path().join("nested")).expect("should create nested directory");
    fs::write(directory.path().join("#zeta.moth"), b"").expect("should write root");
    fs::write(directory.path().join("#alpha.moth"), b"").expect("should write root");
    fs::write(directory.path().join("config.moth"), b"").expect("should write config");
    fs::write(directory.path().join("ordinary.moth"), b"").expect("should write ordinary file");
    fs::write(directory.path().join("nested/#inner.moth"), b"").expect("should write nested root");

    let discovery = discover_hash_root_file(directory.path()).expect("discovery should succeed");
    let HashRootFileDiscovery::Multiple(root_files) = discovery else {
        panic!("expected multiple direct-child hash roots");
    };

    assert_eq!(
        root_files,
        vec![
            directory.path().join("#alpha.moth"),
            directory.path().join("#zeta.moth"),
        ]
    );
}

#[test]
fn discovers_mod_and_cosmetic_hash_roots_as_the_same_unique_kind() {
    let directory = tempfile::tempdir().expect("failed to create temp directory");
    let root_file = directory.path().join("#mod.moth");
    fs::write(&root_file, b"").expect("should write root");

    assert_eq!(
        discover_hash_root_file(directory.path()).expect("discovery should succeed"),
        HashRootFileDiscovery::Unique(root_file)
    );

    fs::remove_file(directory.path().join("#mod.moth")).expect("should remove mod root");
    let cosmetic_root = directory.path().join("#package.moth");
    fs::write(&cosmetic_root, b"").expect("should write cosmetic root");

    assert_eq!(
        discover_hash_root_file(directory.path()).expect("discovery should succeed"),
        HashRootFileDiscovery::Unique(cosmetic_root)
    );
}

#[test]
fn prepared_roots_preserve_canonical_prefix_order() {
    let entries = vec![
        (
            "zeta".to_string(),
            PathBuf::from("/lib/zeta"),
            HashRootFileDiscovery::Missing,
        ),
        (
            "alpha".to_string(),
            PathBuf::from("/lib/alpha"),
            HashRootFileDiscovery::Unique(PathBuf::from("/lib/alpha/#mod.moth")),
        ),
        (
            "middle".to_string(),
            PathBuf::from("/lib/middle"),
            HashRootFileDiscovery::Missing,
        ),
    ];

    let prepared = PreparedSourcePackageRoots::from_entries(entries);

    let root_prefixes: Vec<&str> = prepared.roots().keys().map(|k| k.as_str()).collect();
    assert_eq!(root_prefixes, vec!["alpha", "middle", "zeta"]);

    let file_prefixes: Vec<&str> = prepared.root_files().keys().map(|k| k.as_str()).collect();
    assert_eq!(file_prefixes, vec!["alpha", "middle", "zeta"]);
}

// macOS rejects this invalid-byte filename before discovery can inspect it.
#[cfg(target_os = "linux")]
#[test]
fn rejects_non_utf8_direct_child_filename_with_the_offending_path() {
    use super::root_file::HashRootDiscoveryError;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().expect("failed to create temp directory");
    let bad_name = OsString::from_vec(vec![0xC3, 0x28]);
    let bad_file = directory.path().join(bad_name);
    fs::write(&bad_file, b"").expect("should write non-UTF-8 named file");

    let error = discover_hash_root_file(directory.path())
        .expect_err("non-UTF-8 filename should be rejected");
    let HashRootDiscoveryError::InvalidFileName(path) = error else {
        panic!("expected InvalidFileName, got {error:?}");
    };

    assert_eq!(
        path, bad_file,
        "the offending path should be preserved in the typed error"
    );
}

#[test]
fn classifies_only_plus_prefixed_moth_filenames_as_support_roots() {
    assert!(file_name_is_support_root_file("+pkg.moth"));
    assert!(file_name_is_support_root_file("+anything.moth"));
    assert!(!file_name_is_support_root_file("#home.moth"));
    assert!(!file_name_is_support_root_file("pkg.moth"));
    assert!(!file_name_is_support_root_file("+pkg.js"));
    assert!(!file_name_is_support_root_file("+.moth"));
    assert!(file_name_is_module_root_file("#home.moth"));
    assert!(file_name_is_module_root_file("+pkg.moth"));
    assert!(!file_name_is_module_root_file("home.moth"));
}
