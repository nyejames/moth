//! Unit tests for the source-backed package registry.

use crate::builder_surface::package_metadata::PackageBacking;
use crate::builder_surface::{
    PackageMetadata, PackageOrigin, ProvidedSourceRoot, SourcePackageRegistry,
};

#[test]
fn iter_returns_roots_in_canonical_prefix_order() {
    let mut registry = SourcePackageRegistry::new();
    registry.register_filesystem_root(
        "zeta",
        std::path::PathBuf::from("/lib/zeta"),
        PackageOrigin::ProjectLocal,
    );
    registry.register_filesystem_root(
        "alpha",
        std::path::PathBuf::from("/lib/alpha"),
        PackageOrigin::ProjectLocal,
    );
    registry.register_filesystem_root(
        "middle",
        std::path::PathBuf::from("/lib/middle"),
        PackageOrigin::ProjectLocal,
    );

    let prefixes: Vec<&str> = registry
        .iter()
        .map(|root| root.package_prefix.as_str())
        .collect();
    assert_eq!(prefixes, vec!["alpha", "middle", "zeta"]);
}

#[test]
fn merge_reports_collisions_in_canonical_prefix_order() {
    let mut builder = SourcePackageRegistry::new();
    builder.register_filesystem_root(
        "zeta",
        std::path::PathBuf::from("/builder/zeta"),
        PackageOrigin::Builder,
    );
    builder.register_filesystem_root(
        "alpha",
        std::path::PathBuf::from("/builder/alpha"),
        PackageOrigin::Builder,
    );

    let mut project_local = SourcePackageRegistry::new();
    project_local.register_filesystem_root(
        "zeta",
        std::path::PathBuf::from("/local/zeta"),
        PackageOrigin::ProjectLocal,
    );
    project_local.register_filesystem_root(
        "beta",
        std::path::PathBuf::from("/local/beta"),
        PackageOrigin::ProjectLocal,
    );
    project_local.register_filesystem_root(
        "alpha",
        std::path::PathBuf::from("/local/alpha"),
        PackageOrigin::ProjectLocal,
    );

    let mut merged = builder.clone();
    let collisions = merged
        .merge(&project_local)
        .expect_err("overlapping prefixes should collide");

    assert_eq!(collisions, vec!["alpha", "zeta"]);
}

#[test]
fn merge_adds_non_overlapping_roots_in_canonical_order() {
    let mut builder = SourcePackageRegistry::new();
    builder.register_filesystem_root(
        "html",
        std::path::PathBuf::from("/builder/html"),
        PackageOrigin::Builder,
    );

    let mut project_local = SourcePackageRegistry::new();
    project_local.register_filesystem_root(
        "widgets",
        std::path::PathBuf::from("/lib/widgets"),
        PackageOrigin::ProjectLocal,
    );
    project_local.register_filesystem_root(
        "alpha",
        std::path::PathBuf::from("/lib/alpha"),
        PackageOrigin::ProjectLocal,
    );

    let mut merged = builder.clone();
    merged
        .merge(&project_local)
        .expect("non-overlapping prefixes should merge cleanly");

    let prefixes: Vec<&str> = merged
        .iter()
        .map(|root| root.package_prefix.as_str())
        .collect();
    assert_eq!(prefixes, vec!["alpha", "html", "widgets"]);
}

#[test]
fn get_root_and_has_prefix_locate_registered_root() {
    let mut registry = SourcePackageRegistry::new();
    registry.register_filesystem_root(
        "html",
        std::path::PathBuf::from("/builder/html"),
        PackageOrigin::Builder,
    );

    assert!(registry.has_prefix("html"));
    assert!(!registry.has_prefix("missing"));

    let root = registry.get_root("html").expect("html root should exist");
    assert!(
        matches!(&root.root, ProvidedSourceRoot::Filesystem(path) if path == std::path::Path::new("/builder/html"))
    );
}

#[test]
fn source_registry_always_constructs_moth_source_metadata() {
    let mut registry = SourcePackageRegistry::new();
    registry.register_filesystem_root(
        "html",
        std::path::PathBuf::from("/builder/html"),
        PackageOrigin::Builder,
    );
    registry.register_filesystem_root(
        "widgets",
        std::path::PathBuf::from("/lib/widgets"),
        PackageOrigin::ProjectLocal,
    );

    for root in registry.iter() {
        assert_eq!(
            root.metadata,
            PackageMetadata::source(root.metadata.origin),
            "source registry must only carry MothSource backing"
        );
        assert_eq!(
            root.metadata.backing,
            PackageBacking::MothSource,
            "source registry must not construct ExternalBinding backing"
        );
    }
}

#[test]
fn reserved_origins_are_representable_in_source_metadata() {
    let standard = PackageMetadata::source(PackageOrigin::Standard);
    assert_eq!(standard.origin, PackageOrigin::Standard);
    assert_eq!(standard.backing, PackageBacking::MothSource);

    let dependency = PackageMetadata::source(PackageOrigin::Dependency);
    assert_eq!(dependency.origin, PackageOrigin::Dependency);
    assert_eq!(dependency.backing, PackageBacking::MothSource);
}

#[test]
fn reserved_origins_are_representable_in_binding_metadata() {
    let standard = PackageMetadata::binding(PackageOrigin::Standard);
    assert_eq!(standard.origin, PackageOrigin::Standard);
    assert_eq!(standard.backing, PackageBacking::ExternalBinding);

    let dependency = PackageMetadata::binding(PackageOrigin::Dependency);
    assert_eq!(dependency.origin, PackageOrigin::Dependency);
    assert_eq!(dependency.backing, PackageBacking::ExternalBinding);
}
