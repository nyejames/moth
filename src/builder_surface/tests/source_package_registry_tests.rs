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

fn rejection(prefix: &str, origin: PackageOrigin) -> String {
    let mut registry = SourcePackageRegistry::new();
    registry
        .register_filesystem_root(prefix, std::path::PathBuf::from("/builder/x"), origin)
        .expect_err(&format!("source prefix {prefix:?} must be rejected"))
        .to_string()
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
    register(
        &mut registry,
        "builder/ui",
        "/builder/ui",
        PackageOrigin::Builder,
    );

    registry
        .register_filesystem_root(
            "builder/ui",
            std::path::PathBuf::from("/other/ui"),
            PackageOrigin::Builder,
        )
        .expect_err("a second root for one prefix must not replace the first");

    let root = registry
        .get_root("builder/ui")
        .expect("first root should remain");
    assert!(
        matches!(&root.root, ProvidedSourceRoot::Filesystem(path) if path == std::path::Path::new("/builder/ui"))
    );
}

#[test]
fn register_filesystem_root_rejects_case_only_duplicate_prefix() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "builder/ui",
        "/builder/ui",
        PackageOrigin::Builder,
    );

    registry
        .register_filesystem_root(
            "BUILDER/UI",
            std::path::PathBuf::from("/other/ui"),
            PackageOrigin::Builder,
        )
        .expect_err("case-only variants of a registered prefix must be rejected");
    assert!(registry.get_root("BUILDER/UI").is_none());
}

#[test]
fn register_filesystem_root_accepts_nested_prefix_and_rejects_overlaps() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "builder/web/site",
        "/builder/site",
        PackageOrigin::Builder,
    );
    for overlapping in [
        "Builder/WEB/SITE",
        "builder/web",
        "BUILDER/WEB",
        "builder/web/site/extra",
        // A case-only difference in a non-root component is the same package path.
        "builder/Web/site",
        "builder/Web/site/x",
        "builder/web/Site/x",
    ] {
        registry
            .register_filesystem_root(
                overlapping,
                std::path::PathBuf::from("/other/site"),
                PackageOrigin::Builder,
            )
            .expect_err(&format!(
                "{overlapping:?} overlaps registered builder/web/site and must be rejected"
            ));
    }
    register(
        &mut registry,
        "builder/web/sites",
        "/builder/sites",
        PackageOrigin::Builder,
    );

    let prefixes: Vec<&str> = registry
        .iter()
        .map(|root| root.package_prefix.as_str())
        .collect();
    assert_eq!(prefixes, vec!["builder/web/site", "builder/web/sites"]);
}

#[test]
fn register_filesystem_root_enforces_origin_namespace_ownership() {
    // Core packages register only below `core/`; Builder packages only below `builder/`. The
    // other origins may use any other root, so a local support package keeps its flat name.
    let rejected = [
        ("ui", PackageOrigin::Builder),
        ("Builder/ui", PackageOrigin::Builder),
        ("builder", PackageOrigin::Builder),
        ("core/ui", PackageOrigin::Builder),
        ("Core/ui", PackageOrigin::Builder),
        ("ui", PackageOrigin::Core),
        ("builder/ui", PackageOrigin::Core),
        ("Core/ui", PackageOrigin::Core),
        ("core", PackageOrigin::Core),
        ("core/ui", PackageOrigin::ProjectLocal),
        ("Core/ui", PackageOrigin::ProjectLocal),
        ("builder/ui", PackageOrigin::ProjectLocal),
        ("BUILDER/ui", PackageOrigin::Dependency),
        ("builder", PackageOrigin::Dependency),
    ];
    for (prefix, origin) in rejected {
        let message = rejection(prefix, origin);
        assert!(
            message.contains(prefix),
            "{prefix:?} as {origin:?} must be named in its rejection: {message}"
        );
    }

    for (prefix, origin) in [
        ("core/ui", PackageOrigin::Core),
        ("core/io/nested", PackageOrigin::Core),
        ("builder/ui", PackageOrigin::Builder),
        ("builder/game/widgets", PackageOrigin::Builder),
        ("ui", PackageOrigin::ProjectLocal),
        ("acme/widgets", PackageOrigin::Dependency),
        ("builders/ui", PackageOrigin::ProjectLocal),
        ("corelike/ui", PackageOrigin::Dependency),
    ] {
        let mut registry = SourcePackageRegistry::new();
        register(&mut registry, prefix, "/installed/package", origin);
        let root = registry
            .get_root(prefix)
            .expect("accepted prefix should register");
        assert_eq!(root.metadata, PackageMetadata::source(origin));
    }
}

#[test]
fn register_filesystem_root_rejects_reserved_roots_and_malformed_prefixes() {
    for prefix in ["", "/ui", "ui//x", "ui#x", "builder/"] {
        let message = rejection(prefix, PackageOrigin::Builder);
        assert!(!message.is_empty(), "{prefix:?} must be rejected");
    }

    // `@project` stays separately reserved for every origin, including the privileged ones.
    for origin in [
        PackageOrigin::Core,
        PackageOrigin::Builder,
        PackageOrigin::ProjectLocal,
        PackageOrigin::Dependency,
    ] {
        let message = rejection("project", origin);
        assert!(
            message.contains("@project"),
            "{origin:?} must not claim the project globals name: {message}"
        );
        let message = rejection("Project/x", origin);
        assert!(
            message.contains("@project"),
            "{origin:?} must not claim a case variant of the project globals name: {message}"
        );
    }

    // The exact reserved roots are never importable package surfaces.
    for (prefix, origin) in [
        ("core", PackageOrigin::Core),
        ("CORE", PackageOrigin::Core),
        ("builder", PackageOrigin::Builder),
        ("BUILDER", PackageOrigin::Builder),
    ] {
        let message = rejection(prefix, origin);
        assert!(
            message.contains("reserved namespace root"),
            "{prefix:?} must be rejected as a reserved root: {message}"
        );
    }
}

#[test]
fn registration_membership_is_explicit_and_independent_of_origin() {
    use crate::builder_surface::package_metadata::PackageSetMembership;

    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "builder/custom",
        "/tmp/custom",
        PackageOrigin::Builder,
    );
    assert_eq!(
        registry
            .get_root("builder/custom")
            .expect("ordinary root")
            .package_set_membership,
        PackageSetMembership::Ordinary,
        "public builder registration never joins the compiler-shipped set"
    );

    registry
        .register_compiler_shipped_filesystem_root(
            "builder/web/site",
            std::path::PathBuf::from("/tmp/official-site"),
            PackageOrigin::Builder,
        )
        .expect("an internal shipped Builder root should register");
    let shipped = registry.get_root("builder/web/site").expect("shipped root");
    assert_eq!(
        shipped.package_set_membership,
        PackageSetMembership::CompilerShipped,
        "shipped membership comes from the registration call, not the Builder prefix"
    );
    assert_eq!(
        shipped.metadata,
        PackageMetadata::source(PackageOrigin::Builder),
        "membership is orthogonal to origin and backing"
    );
}

#[test]
fn get_root_locates_registered_root() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "builder/html",
        "/builder/html",
        PackageOrigin::Builder,
    );

    assert!(registry.get_root("missing").is_none());

    let root = registry
        .get_root("builder/html")
        .expect("html root should exist");
    assert!(
        matches!(&root.root, ProvidedSourceRoot::Filesystem(path) if path == std::path::Path::new("/builder/html"))
    );
}

#[test]
fn source_registry_always_constructs_moth_source_metadata() {
    let mut registry = SourcePackageRegistry::new();
    register(
        &mut registry,
        "builder/html",
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

    assert_eq!(
        CORE_SOURCE_PACKAGE_PREFIXES,
        ["core/html", "core/css"],
        "the mandatory source namespace is exactly the Core HTML and CSS packages"
    );

    let surface = BuilderSurface::with_mandatory_core();
    let mut roots = Vec::new();
    for prefix in CORE_SOURCE_PACKAGE_PREFIXES {
        let root = surface
            .source_packages
            .get_root(prefix)
            .expect("mandatory Core root");
        assert_eq!(
            root.metadata,
            PackageMetadata {
                origin: PackageOrigin::Core,
                backing: PackageBacking::MothSource,
            },
            "@{prefix} must stay Core/MothSource"
        );
        assert_eq!(
            root.package_set_membership,
            crate::builder_surface::package_metadata::PackageSetMembership::CompilerShipped,
            "@{prefix} is registered by the compiler as part of its shipped package set"
        );
        let ProvidedSourceRoot::Filesystem(path) = &root.root;
        assert_eq!(
            path,
            &BuilderSurface::builtin_source_package_root(prefix),
            "@{prefix} must stay on its official source root"
        );
        assert!(path.join("@package.moth").is_file());
        roots.push(path.clone());
    }
    assert_ne!(
        roots[0], roots[1],
        "the two mandatory Core packages keep independent filesystem roots"
    );
    assert_eq!(
        surface.source_packages.iter().count(),
        CORE_SOURCE_PACKAGE_PREFIXES.len(),
        "the mandatory surface exposes exactly the two Core source packages"
    );
    for removed in ["html", "styles"] {
        assert!(
            surface.source_packages.get_root(removed).is_none(),
            "the removed flat @{removed} package must not stay registered"
        );
    }
    for reserved in ["core", "builder"] {
        assert!(
            surface.source_packages.get_root(reserved).is_none(),
            "the reserved @{reserved} namespace root must never resolve as a package"
        );
    }
    surface.validate().expect("Core surface has official roots");
}

#[test]
fn mandatory_core_roots_reject_replacement_and_nested_overlaps() {
    use crate::builder_surface::BuilderSurface;

    let mut surface = BuilderSurface::with_mandatory_core();
    for prefix in [
        "core/html",
        "Core/HTML",
        "core/html/docs",
        "core/css",
        "Core/CSS",
        "core/css/docs",
    ] {
        let error = surface.source_packages.register_filesystem_root(
            prefix,
            std::path::PathBuf::from("/tmp/custom-package"),
            PackageOrigin::Builder,
        );
        assert!(
            error.is_err(),
            "@{prefix} cannot replace a Core source package as a Builder package"
        );

        // An attempted Core-origin replacement is rejected by the existing overlap rule.
        let error = surface.source_packages.register_filesystem_root(
            prefix,
            std::path::PathBuf::from("/tmp/custom-package"),
            PackageOrigin::Core,
        );
        assert!(
            error.is_err(),
            "@{prefix} cannot replace an official Core root even with Core origin"
        );
    }
    surface
        .validate()
        .expect("rejected writes preserve official roots");
}
