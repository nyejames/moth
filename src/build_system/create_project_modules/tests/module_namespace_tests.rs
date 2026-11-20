use super::*;
use crate::build_system::create_project_modules::module_identity::{
    ModuleIdentityRecord, ModuleIdentityTable,
};
use crate::build_system::create_project_modules::source_tree_index::SourceTreeProjectContext;
use crate::builder_surface::SourcePackageRegistry;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootFiles, ModuleRootRole, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
use std::path::PathBuf;

fn record(
    root: &str,
    logical: &str,
    normal: bool,
    support: bool,
    package: &StablePackageIdentity,
) -> ModuleIdentityRecord {
    ModuleIdentityRecord::new(
        PathBuf::from(root),
        ModuleRootFiles::new(
            normal.then(|| PathBuf::from(format!("{root}/@root.moth"))),
            support.then(|| PathBuf::from(format!("{root}/@package.moth"))),
            None,
        ),
        PathBuf::from(logical),
        package,
    )
    .expect("synthetic structural record should be valid")
}

fn cross_module_surface(namespace: &ModuleNamespace, key: &str) -> Option<ModuleRootRole> {
    match namespace.entries.get(key) {
        Some(NamespaceEntry::CrossModule { surface, .. }) => Some(*surface),
        _ => None,
    }
}

fn cross_module_entry(module_index: usize) -> NamespaceEntry {
    NamespaceEntry::CrossModule {
        target_module_id: ModuleId::from_index(module_index),
        surface: ModuleRootRole::Normal,
    }
}

#[test]
fn paired_root_surface_is_selected_by_consumer_position() {
    let package = StablePackageIdentity::project_local("docs");
    let records = vec![
        record("/app", "", true, false, &package),
        record("/app/engine", "engine", true, true, &package),
        record("/app/ui", "ui", true, false, &package),
    ];
    let identities = ModuleIdentityTable::from_records(records);

    let parent = identities
        .module_id_for_directory(std::path::Path::new("/app"))
        .expect("parent module should exist");
    let engine = identities
        .module_id_for_directory(std::path::Path::new("/app/engine"))
        .expect("engine module should exist");
    let sibling = identities
        .module_id_for_directory(std::path::Path::new("/app/ui"))
        .expect("sibling module should exist");

    let mut parent_namespace = ModuleNamespace::new();
    populate_package_cross_module_targets(&mut parent_namespace, &identities, parent);
    assert_eq!(
        cross_module_surface(&parent_namespace, "engine"),
        Some(ModuleRootRole::Normal),
        "the parent must select the paired normal surface"
    );

    let mut sibling_namespace = ModuleNamespace::new();
    populate_package_cross_module_targets(&mut sibling_namespace, &identities, sibling);
    assert_eq!(
        cross_module_surface(&sibling_namespace, "engine"),
        Some(ModuleRootRole::Support),
        "a sibling branch must select the paired support surface"
    );

    let mut engine_namespace = ModuleNamespace::new();
    populate_package_cross_module_targets(&mut engine_namespace, &identities, engine);
    assert_eq!(
        cross_module_surface(&engine_namespace, "engine"),
        None,
        "a paired module must not import its own support surface"
    );
}

#[test]
fn paired_support_is_suppressed_for_parent_even_through_an_unrooted_directory() {
    let package = StablePackageIdentity::project_local("docs");
    let records = vec![
        record("/app", "", true, false, &package),
        record("/app/group/engine", "group/engine", true, true, &package),
        record("/app/ui", "ui", true, false, &package),
    ];
    let identities = ModuleIdentityTable::from_records(records);
    let parent = identities
        .module_id_for_directory(std::path::Path::new("/app"))
        .expect("parent module should exist");

    let mut parent_namespace = ModuleNamespace::new();
    populate_package_cross_module_targets(&mut parent_namespace, &identities, parent);
    assert_eq!(
        cross_module_surface(&parent_namespace, "group/engine"),
        Some(ModuleRootRole::Normal),
        "the parent still imports the paired normal surface through the unrooted directory"
    );
    assert_eq!(
        cross_module_surface(&parent_namespace, "engine"),
        None,
        "the support surface must not be added under a basename for the parent"
    );
}

#[test]
fn entry_root_file_is_not_a_same_module_namespace_entry() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = std::fs::canonicalize(temp.path()).expect("project root should canonicalize");
    for root_file in ["@entry.moth", "@mod.moth", "@package.moth", "shared.moth"] {
        std::fs::write(root.join(root_file), "").expect("should write fixture source");
    }

    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();
    let index = SourceTreeIndex::discover(
        root.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &SourcePackageRegistry::default(),
        &SourceFileKindRegistry::default(),
        &ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");
    let module_id = index
        .module_identities()
        .module_ids()
        .next()
        .expect("the root directory should be one structural module");
    assert_eq!(
        index.owned_source_indices(module_id).len(),
        4,
        "all three roots and the helper must be owned sources before namespace filtering"
    );

    let mut namespace = ModuleNamespace::new();
    populate_same_module_entries(&mut namespace, &index, module_id);

    for root_key in ["@entry", "@mod", "@package"] {
        assert!(
            !namespace.entries.contains_key(root_key),
            "a special root file must not become a same-module namespace entry: {root_key}"
        );
    }
    assert!(
        matches!(
            namespace.entries.get("shared"),
            Some(NamespaceEntry::SameModuleSource { .. })
        ),
        "ordinary helper sources remain same-module namespace entries"
    );
    assert_eq!(
        namespace.entries.len(),
        1,
        "only the helper source should enter the same-module namespace"
    );
}

#[test]
fn config_named_template_publishes_reserved_key_as_same_module_source() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = std::fs::canonicalize(temp.path()).expect("project root should canonicalize");
    std::fs::write(root.join("@entry.moth"), "").expect("should write entry root");
    std::fs::write(root.join("@config.moth.mtf"), "reserved key fixture")
        .expect("should write config-named template");

    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();
    let index = SourceTreeIndex::discover(
        root.clone(),
        SourceTreeProjectContext {
            validated_output_settings: None,
            config_source: None,
        },
        &config,
        &SourcePackageRegistry::default(),
        &SourceFileKindRegistry::default(),
        &ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should build");
    let module_id = index
        .module_identities()
        .module_ids()
        .next()
        .expect("the root directory should be one structural module");

    let mut namespace = ModuleNamespace::new();
    populate_same_module_entries(&mut namespace, &index, module_id);

    assert!(
        matches!(
            namespace.entries.get("@config.moth"),
            Some(NamespaceEntry::SameModuleSource {
                source_kind: SourceFileKind::MothTemplate,
                ..
            })
        ),
        "the ordinary template must publish the reserved spelling as a same-module source key"
    );
    assert!(
        !matches!(
            namespace.entries.get("@config.moth"),
            Some(NamespaceEntry::CrossModule { .. })
        ),
        "a same-module template must never masquerade as a cross-module facade"
    );
}

#[test]
fn extensionless_portable_path_strips_only_the_final_component_extension() {
    assert_eq!(extensionless_portable_path("src/a.moth"), "src/a");
    assert_eq!(extensionless_portable_path("src/a.b.moth"), "src/a.b");
    assert_eq!(extensionless_portable_path("src/a"), "src/a");
    assert_eq!(extensionless_portable_path("src.d/a.moth"), "src.d/a");
    assert_eq!(extensionless_portable_path(".moth"), ".moth");
    assert_eq!(extensionless_portable_path("src/.moth"), "src/.moth");
}

/// A literal backslash is an ordinary Unix filename byte, not a module separator.
///
/// WHY: the key is derived from an already-portable logical spelling without a native-path or
/// display round trip, so `a\b` stays distinct from a real nested `a/b`. A display conversion
/// here collapsed both under `src/a/b` and falsely marked their namespace ambiguous.
#[cfg(unix)]
#[test]
fn literal_backslash_source_names_keep_distinct_namespace_keys() {
    let backslash_key = extensionless_portable_path("src/a\\b.moth");
    let nested_key = extensionless_portable_path("src/a/b.moth");

    assert_eq!(backslash_key, "src/a\\b");
    assert_eq!(nested_key, "src/a/b");

    let mut namespace = ModuleNamespace::new();
    namespace.insert(backslash_key, cross_module_entry(0));
    namespace.insert(nested_key, cross_module_entry(1));

    assert_eq!(
        namespace.entries.len(),
        2,
        "distinct native names must not collapse into one namespace key"
    );
    assert!(!namespace.is_ambiguous("src/a\\b"));
    assert!(!namespace.is_ambiguous("src/a/b"));
}

/// Same-stem `.moth` and `.mtf` sources still share one extensionless lookup key.
#[test]
fn same_stem_moth_and_mtf_sources_remain_ambiguous() {
    let moth_key = extensionless_portable_path("src/part.moth");
    let mtf_key = extensionless_portable_path("src/part.mtf");
    assert_eq!(moth_key, "src/part");
    assert_eq!(mtf_key, "src/part");

    let mut namespace = ModuleNamespace::new();
    namespace.insert(moth_key, cross_module_entry(0));
    namespace.insert(mtf_key, cross_module_entry(1));

    assert_eq!(namespace.entries.len(), 1);
    assert!(namespace.is_ambiguous("src/part"));
}
