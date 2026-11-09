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
fn template_prelude_accepts_only_the_official_html_root() {
    use crate::builder_surface::definition::{
        BuilderSurfaceValidationError, TEMPLATE_PRELUDE_SOURCE_PACKAGE_PREFIX,
    };
    use crate::builder_surface::{BuilderSurface, SourceFileKind};

    let mut surface = BuilderSurface::with_mandatory_core();
    surface
        .source_file_kinds
        .register("mtf", SourceFileKind::MothTemplate);
    surface
        .source_packages
        .register_filesystem_root(
            TEMPLATE_PRELUDE_SOURCE_PACKAGE_PREFIX,
            std::path::PathBuf::from("/tmp/custom-html"),
            PackageOrigin::Builder,
        )
        .expect("the registry accepts the reserved prefix; surface validation owns the policy");
    let error = surface
        .validate()
        .expect_err("a custom @html directory cannot supply the template prelude");
    let message = match error {
        BuilderSurfaceValidationError::InvalidDefinition(message) => message,
        other => panic!("expected an invalid-definition capability error, got {other:?}"),
    };
    assert!(
        message.contains("reserved") && message.contains("standard HTML builder package set"),
        "{message}"
    );

    let mut surface = BuilderSurface::with_mandatory_core();
    surface
        .source_file_kinds
        .register("mtf", SourceFileKind::MothTemplate);
    surface
        .source_packages
        .register_filesystem_root(
            TEMPLATE_PRELUDE_SOURCE_PACKAGE_PREFIX,
            BuilderSurface::builtin_source_package_root(TEMPLATE_PRELUDE_SOURCE_PACKAGE_PREFIX),
            PackageOrigin::Builder,
        )
        .expect("the official HTML root registers");
    surface
        .validate()
        .expect("the standard HTML builder package set supplies the template prelude");
}
