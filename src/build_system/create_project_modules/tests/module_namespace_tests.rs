use super::*;
use crate::build_system::create_project_modules::module_identity::{
    ModuleIdentityRecord, ModuleIdentityTable,
};
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StablePackageIdentity};
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
        normal.then(|| PathBuf::from(format!("{root}/@root.moth"))),
        support.then(|| PathBuf::from(format!("{root}/+root.moth"))),
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
