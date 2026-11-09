use super::root_file::{
    PreparedSourcePackageRoots, dependency_path_references_config_file, file_name_is_config_file,
    file_name_is_invalid_module_root_marker, file_name_is_module_root_file,
    module_root_role_for_file_name,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;

fn path(
    components: &[&str],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> PathId {
    let spelling = components.join("/");
    path_fork
        .try_intern_portable_path(&spelling, string_table)
        .expect("test path fits")
}

#[test]
fn classifies_only_the_fixed_root_filenames() {
    assert_eq!(
        module_root_role_for_file_name("@mod.moth"),
        Some(ModuleRootRole::Normal)
    );
    assert_eq!(
        module_root_role_for_file_name("@package.moth"),
        Some(ModuleRootRole::Support)
    );
    assert_eq!(
        module_root_role_for_file_name("@entry.moth"),
        Some(ModuleRootRole::Entry)
    );
    for invalid in [
        "@page.moth",
        "@home.moth",
        "@site.moth",
        "@api.moth",
        "@config.moth",
        "@anything.moth",
        "+pkg.moth",
        "#pkg.moth",
        "package.moth",
        "mod.moth",
        "@mod.js",
        "@package.js",
        "@.moth",
        "@Entry.moth",
        "@entry",
    ] {
        assert_eq!(
            module_root_role_for_file_name(invalid),
            None,
            "{invalid} is not one of the three fixed root filenames"
        );
    }
    assert!(file_name_is_config_file("config.moth"));
    assert!(!file_name_is_config_file("config"));
}

#[test]
fn classifies_the_closed_three_role_root_filenames() {
    assert!(file_name_is_module_root_file("@mod.moth"));
    assert!(file_name_is_module_root_file("@package.moth"));
    assert!(file_name_is_module_root_file("@entry.moth"));
    for invalid in [
        "@page.moth",
        "@home.moth",
        "@site.moth",
        "@api.moth",
        "@config.moth",
        "@anything.moth",
        "+pkg.moth",
        "#pkg.moth",
        "page.moth",
        "config.moth",
    ] {
        assert!(
            !file_name_is_module_root_file(invalid),
            "{invalid} is not one of the three closed root filenames"
        );
    }
}

#[test]
fn invalid_special_moth_names_are_recognized_without_a_role_guess() {
    for invalid in [
        "@page.moth",
        "@home.moth",
        "@site.moth",
        "@api.moth",
        "@config.moth",
        "@anything.moth",
        "@Entry.moth",
        "@MOD.moth",
        "@.moth",
        "+pkg.moth",
        "+shared.moth",
        "+.moth",
        "#pkg.moth",
        "#.moth",
    ] {
        assert!(
            file_name_is_invalid_module_root_marker(invalid),
            "{invalid} is an invalid special .moth filename"
        );
    }

    for accepted in ["@mod.moth", "@package.moth", "@entry.moth"] {
        assert!(
            !file_name_is_invalid_module_root_marker(accepted),
            "{accepted} is an accepted root filename"
        );
    }

    for ordinary in [
        "config.moth",
        "helper.moth",
        "page.moth",
        "+notes.mtf",
        "+runtime.js",
        "@theme.css",
        "@entry",
        ".moth",
    ] {
        assert!(
            !file_name_is_invalid_module_root_marker(ordinary),
            "{ordinary} is not an invalid special .moth filename"
        );
    }
}

#[test]
fn config_dependency_classification_uses_the_source_component() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

    let bare_config = path(&["config"], &mut string_table, &mut path_fork);
    assert!(dependency_path_references_config_file(
        bare_config,
        &path_fork,
        &string_table
    ));

    let nested_config_folder = path(&["config", "settings"], &mut string_table, &mut path_fork);
    assert!(!dependency_path_references_config_file(
        nested_config_folder,
        &path_fork,
        &string_table
    ));

    let ordinary_config_folder = path(
        &["config", "settings", "project"],
        &mut string_table,
        &mut path_fork,
    );
    assert!(!dependency_path_references_config_file(
        ordinary_config_folder,
        &path_fork,
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
fn config_file_is_distinct_from_the_fixed_roots() {
    assert!(file_name_is_config_file("config.moth"));
    assert!(!file_name_is_module_root_file("config.moth"));

    // `@config.moth` is not the build-only config file and is not a root role; Stage 0 rejects it
    // with the closed-vocabulary source-structure diagnostic.
    assert!(!file_name_is_config_file("@config.moth"));
    assert!(!file_name_is_module_root_file("@config.moth"));
}
