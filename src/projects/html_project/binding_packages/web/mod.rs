//! Built-in `@web/*` binding package registration.
//!
//! WHAT: each `@web/*` package is one annotated JavaScript asset in its own directory. This module
//!       parses every asset, registers it as a Builder-origin binding package and returns runtime
//!       asset metadata so reachable calls emit the asset and generated glue.
//! WHY: builder-owned packages share the parser, registry and emission path used by project-local
//!      `.js` imports. Only their package paths and asset locations are fixed here.
//!
//! Each package directory also holds a `README.md`. It owns the package's living design, development
//! plan, binding prerequisites and future ideas. Accepted public API and semantics live under
//! `docs/src/docs/packages/builder/`, and current status lives in the packages and builders progress
//! matrix.

use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::projects::html_project::external_js::package_registration::{
    register_parsed_js_module, required_runtime_imports_from_parsed,
};
use crate::projects::html_project::external_js::parser::parse_js_module;
use crate::projects::html_project::external_js::runtime_assets::js_runtime_asset_identity;
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;
use std::path::{Path, PathBuf};

/// One built-in `@web/*` package and its embedded JavaScript asset.
struct WebBindingPackage {
    package_path: &'static str,
    source: &'static str,
    /// Asset path relative to this directory. The file name doubles as the logical source path.
    asset_path: &'static str,
}

const WEB_BINDING_PACKAGES: [WebBindingPackage; 2] = [
    WebBindingPackage {
        package_path: "@web/canvas",
        source: include_str!("canvas/canvas.js"),
        asset_path: "canvas/canvas.js",
    },
    // WebGL2. Registered with no public symbols until its first accepted API slice lands. See its
    // README for the binding prerequisites that gate that slice.
    WebBindingPackage {
        package_path: "@web/graphics",
        source: include_str!("graphics/graphics.js"),
        asset_path: "graphics/graphics.js",
    },
];

/// Registers every built-in `@web/*` package and returns their runtime asset metadata.
pub(crate) fn register_web_binding_packages(
    registry: &mut ExternalPackageRegistry,
) -> Vec<BuilderRuntimePackageMetadata> {
    WEB_BINDING_PACKAGES
        .iter()
        .map(|package| register_web_binding_package(package, registry))
        .collect()
}

fn register_web_binding_package(
    package: &WebBindingPackage,
    registry: &mut ExternalPackageRegistry,
) -> BuilderRuntimePackageMetadata {
    let parsed = parse_js_module(package.source, &RuntimeModuleRegistry::v1());

    // Built-in assets ship with the compiler, so a parser diagnostic is a compiler bug.
    assert!(
        parsed.diagnostics.is_empty(),
        "Built-in {} JS module has parser diagnostics: {:?}",
        package.package_path,
        parsed.diagnostics
    );

    let package_id = registry
        .register_package(package.package_path, PackageOrigin::Builder)
        .expect("built-in @web package paths are distinct");

    register_parsed_js_module(package_id, &parsed, registry)
        .expect("built-in @web package registration should not fail");

    let required_runtime_imports = required_runtime_imports_from_parsed(&parsed);
    let runtime_asset = js_runtime_asset_identity(
        StablePackageIdentity::binding(PackageOrigin::Builder, package.package_path),
        &logical_source_path(package),
        canonical_source_path(package),
        None,
    )
    .expect("built-in @web asset identity is a proven internal invariant");

    BuilderRuntimePackageMetadata {
        package_id,
        runtime_asset: Some(runtime_asset),
        required_runtime_imports,
    }
}

fn canonical_source_path(package: &WebBindingPackage) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/projects/html_project/binding_packages/web")
        .join(package.asset_path)
}

fn logical_source_path(package: &WebBindingPackage) -> PortableResourcePath {
    let file_name = Path::new(package.asset_path)
        .file_name()
        .expect("built-in @web asset paths end in a file name");

    PortableResourcePath::from_relative_logical_path(Path::new(file_name))
        .expect("built-in @web logical source path is a proven internal invariant")
}
