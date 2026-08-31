//! Unit tests for stable resource origin identity.
//!
//! These protect invariants that no compiler output can show: identity must ignore where a
//! declaration was written and must change when the resource itself moves.

use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use std::path::Path;

fn module_origin(logical_module_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        logical_module_path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn portable(relative: &str) -> PortableResourcePath {
    PortableResourcePath::from_relative_logical_path(Path::new(relative))
        .expect("relative resource path should be portable")
}

#[test]
fn portable_resource_path_uses_forward_slash_spelling() {
    let path = portable("assets/images/logo.svg");

    assert_eq!(path.as_str(), "assets/images/logo.svg");
}

#[test]
fn portable_resource_path_rejects_an_empty_owner_relative_path() {
    let result = PortableResourcePath::from_relative_logical_path(Path::new(""));

    assert!(result.is_err());
}

#[test]
fn portable_resource_path_rejects_a_final_component_without_an_extension() {
    for spelling in ["assets/logo", "assets/.hidden", "assets/logo."] {
        let result = PortableResourcePath::from_relative_logical_path(Path::new(spelling));

        assert!(
            result.is_err(),
            "{spelling} carries no explicit extension and must not become an identity"
        );
    }
}

#[test]
fn portable_resource_path_rejects_traversal_components() {
    let result = PortableResourcePath::from_relative_logical_path(Path::new("../shared/logo.svg"));

    assert!(result.is_err());
}

#[test]
fn one_resource_named_from_two_files_in_one_module_has_one_origin() {
    // The declaring source file is not part of identity, so moving a declaration between ordinary
    // files inside one module must not renumber or duplicate the resource.
    let first = StableResourceOriginId::module_owned(module_origin("blog"), portable("logo.svg"));
    let second = StableResourceOriginId::module_owned(module_origin("blog"), portable("logo.svg"));

    assert_eq!(first, second);
}

#[test]
fn moving_the_resource_within_its_owner_changes_origin() {
    let before = StableResourceOriginId::module_owned(module_origin("blog"), portable("logo.svg"));
    let after =
        StableResourceOriginId::module_owned(module_origin("blog"), portable("assets/logo.svg"));

    assert_ne!(before, after);
}

#[test]
fn renaming_the_resource_changes_origin() {
    let before = StableResourceOriginId::module_owned(module_origin("blog"), portable("logo.svg"));
    let after = StableResourceOriginId::module_owned(module_origin("blog"), portable("mark.svg"));

    assert_ne!(before, after);
}

#[test]
fn two_modules_naming_the_same_relative_path_own_distinct_origins() {
    let blog = StableResourceOriginId::module_owned(module_origin("blog"), portable("logo.svg"));
    let shop = StableResourceOriginId::module_owned(module_origin("shop"), portable("logo.svg"));

    assert_ne!(blog, shop);
}

#[test]
fn a_provider_owner_is_distinct_from_the_module_owner_of_the_same_path() {
    let module_owned =
        StableResourceOriginId::module_owned(module_origin("blog"), portable("logo.svg"));

    let provider_owned = StableResourceOriginId::new(
        StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
            "js",
            StablePackageIdentity::project_local("site"),
        )),
        portable("logo.svg"),
    );

    assert_ne!(module_owned, provider_owned);
}

#[test]
fn two_providers_generating_the_same_path_own_distinct_origins() {
    let package = StablePackageIdentity::project_local("site");

    let from_js = StableProviderResourceOwnerId::new("js", package.clone());
    let from_wit = StableProviderResourceOwnerId::new("wit", package);

    assert_ne!(from_js, from_wit);
    assert_eq!(from_js.provider_kind(), "js");
}
