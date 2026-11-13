//! Unit tests for the source-backed package registry.

use crate::builder_surface::package_metadata::PackageBacking;
use crate::builder_surface::{
    PackageMetadata, PackageOrigin, ProvidedSourceRoot, SourcePackageRegistry,
};

fn register(registry: &mut SourcePackageRegistry, prefix: &str, root: &str, origin: PackageOrigin) {
    registry
        .register_filesystem_root(prefix, std::path::PathBuf::from(root), origin)
        .expect("test source root should register");
}

#[test]
fn iter_returns_roots_in_canonical_prefix_order() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "zeta",
        "/lib/zeta",
        PackageOrigin::ProjectLocal,
    );
    register(
        &mut registry,
        "alpha",
        "/lib/alpha",
        PackageOrigin::ProjectLocal,
    );
    register(
        &mut registry,
        "middle",
        "/lib/middle",
        PackageOrigin::ProjectLocal,
    );

    let prefixes: Vec<&str> = registry
        .iter()
        .map(|root| root.package_prefix.as_str())
        .collect();
    assert_eq!(prefixes, vec!["alpha", "middle", "zeta"]);
}

#[test]
fn register_filesystem_root_rejects_duplicate_prefix_without_replacing_root() {
    let mut registry = SourcePackageRegistry::new();
    register(&mut registry, "ui", "/builder/ui", PackageOrigin::Builder);

    registry
        .register_filesystem_root(
            "ui",
            std::path::PathBuf::from("/other/ui"),
            PackageOrigin::Builder,
        )
        .expect_err("a second root for one prefix must not replace the first");

    let root = registry.get_root("ui").expect("first root should remain");
    assert!(
        matches!(&root.root, ProvidedSourceRoot::Filesystem(path) if path == std::path::Path::new("/builder/ui"))
    );
}

#[test]
fn register_filesystem_root_rejects_case_only_duplicate_prefix() {
    let mut registry = SourcePackageRegistry::new();
    register(&mut registry, "ui", "/builder/ui", PackageOrigin::Builder);

    registry
        .register_filesystem_root(
            "UI",
            std::path::PathBuf::from("/other/ui"),
            PackageOrigin::Builder,
        )
        .expect_err("case-only variants of a registered prefix must be rejected");
    assert!(registry.get_root("UI").is_none());
}

#[test]
fn register_filesystem_root_accepts_nested_prefix_and_rejects_overlaps() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "web/site",
        "/builder/site",
        PackageOrigin::Builder,
    );
    for overlapping in ["Web/SITE", "web", "WEB", "web/site/extra"] {
        registry
            .register_filesystem_root(
                overlapping,
                std::path::PathBuf::from("/other/site"),
                PackageOrigin::Builder,
            )
            .expect_err(&format!(
                "{overlapping:?} overlaps registered web/site and must be rejected"
            ));
    }
    register(
        &mut registry,
        "web/sites",
        "/builder/sites",
        PackageOrigin::Builder,
    );

    let prefixes: Vec<&str> = registry
        .iter()
        .map(|root| root.package_prefix.as_str())
        .collect();
    assert_eq!(prefixes, vec!["web/site", "web/sites"]);
}

#[test]
fn register_filesystem_root_rejects_reserved_and_malformed_prefixes() {
    let mut registry = SourcePackageRegistry::new();
    for prefix in [
        "project",
        "Project",
        "project/x",
        "core",
        "CORE",
        "core/ui",
        "",
        "ui//x",
        "ui#x",
    ] {
        registry
            .register_filesystem_root(
                prefix,
                std::path::PathBuf::from("/builder/x"),
                PackageOrigin::Builder,
            )
            .expect_err(&format!(
                "reserved or malformed source prefix {prefix:?} must be rejected"
            ));
    }
    registry
        .register_filesystem_root(
            "project",
            std::path::PathBuf::from("/project/package"),
            PackageOrigin::ProjectLocal,
        )
        .expect_err("project-local source packages cannot claim project globals");
    assert_eq!(registry.iter().count(), 0);
}

#[test]
fn get_root_locates_registered_root() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "html",
        "/builder/html",
        PackageOrigin::Builder,
    );

    assert!(registry.get_root("missing").is_none());

    let root = registry.get_root("html").expect("html root should exist");
    assert!(
        matches!(&root.root, ProvidedSourceRoot::Filesystem(path) if path == std::path::Path::new("/builder/html"))
    );
}

#[test]
fn source_registry_always_constructs_moth_source_metadata() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "html",
        "/builder/html",
        PackageOrigin::Builder,
    );
    register(
        &mut registry,
        "widgets",
        "/lib/widgets",
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
fn dependency_origin_is_representable_in_source_metadata() {
    let dependency = PackageMetadata::source(PackageOrigin::Dependency);
    assert_eq!(dependency.origin, PackageOrigin::Dependency);
    assert_eq!(dependency.backing, PackageBacking::MothSource);
}

#[test]
fn dependency_origin_is_representable_in_binding_metadata() {
    let dependency = PackageMetadata::binding(PackageOrigin::Dependency);
    assert_eq!(dependency.origin, PackageOrigin::Dependency);
    assert_eq!(dependency.backing, PackageBacking::ExternalBinding);
}

#[test]
fn mandatory_core_registers_official_source_package_roots() {
    use crate::builder_surface::BuilderSurface;
    use crate::builder_surface::definition::CORE_SOURCE_PACKAGE_PREFIXES;

    let surface = BuilderSurface::with_mandatory_core();
    for prefix in CORE_SOURCE_PACKAGE_PREFIXES {
        let root = surface
            .source_packages
            .get_root(prefix)
            .expect("mandatory Core root");
        assert_eq!(root.metadata, PackageMetadata::source(PackageOrigin::Core));
        assert!(matches!(
            &root.root,
            ProvidedSourceRoot::Filesystem(path)
                if path == &BuilderSurface::builtin_source_package_root(prefix)
        ));
    }
    surface.validate().expect("Core surface has official roots");
}

#[test]
fn mandatory_core_roots_reject_replacement_and_nested_overlaps() {
    use crate::builder_surface::BuilderSurface;

    let mut surface = BuilderSurface::with_mandatory_core();
    for prefix in [
        "html",
        "HTML",
        "html/docs",
        "styles",
        "Styles",
        "styles/docs",
    ] {
        let error = surface.source_packages.register_filesystem_root(
            prefix,
            std::path::PathBuf::from("/tmp/custom-package"),
            PackageOrigin::Builder,
        );
        assert!(
            error.is_err(),
            "@{prefix} cannot replace a Core source package"
        );
    }
    surface
        .validate()
        .expect("rejected writes preserve official roots");
}
