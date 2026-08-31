//! Built-in `@web/canvas` package registration.
//!
//! WHAT: parses the embedded `canvas.js` asset and registers `@web/canvas` as a builder-runtime
//!       virtual package with runtime asset metadata.
//! WHY: `@web/canvas` is a JS-only built-in binding package that shares the same parser, registry,
//!      and emission path as project-local `.js` imports.

use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::compiler_frontend::compiler_errors::SourceLocation;
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

/// Registers the built-in `@web/canvas` package in the external package registry.
///
/// WHAT: parses the authored `canvas.js` file, registers opaque types and functions with
///       `PackageOrigin::Builder`, and returns metadata so the build system
///       can emit the JS asset and generated glue through the existing `ModuleExternalImport`
///       path.
/// WHY: built-in JS-backed packages and project-local `.js` imports share the same runtime
///      asset/glue emission path.
pub fn register_web_canvas_package(
    registry: &mut ExternalPackageRegistry,
) -> BuilderRuntimePackageMetadata {
    let source = include_str!("canvas.js");
    let parsed = parse_js_module(source, &RuntimeModuleRegistry::v1());

    // Built-in packages should not have parser diagnostics. If they do, it is a compiler bug.
    assert!(
        parsed.diagnostics.is_empty(),
        "Built-in @web/canvas JS module has parser diagnostics: {:?}",
        parsed.diagnostics
    );

    let package_id = registry
        .register_package(
            "@web/canvas",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("builtin package registration should not collide");

    register_parsed_js_module(package_id, &parsed, registry)
        .expect("builtin package registration should not fail");

    let required_runtime_imports = required_runtime_imports_from_parsed(&parsed);

    let runtime_asset = js_runtime_asset_identity(
        StablePackageIdentity::binding(PackageOrigin::Builder, "@web/canvas"),
        &canvas_logical_source_path(),
        canvas_js_path(),
        SourceLocation::default(),
    )
    .expect("built-in canvas asset identity is a proven internal invariant");

    BuilderRuntimePackageMetadata {
        package_id,
        runtime_asset: Some(runtime_asset),
        required_runtime_imports,
    }
}

fn canvas_js_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/projects/html_project/binding_packages/web/canvas/canvas.js")
}

fn canvas_logical_source_path() -> PortableResourcePath {
    PortableResourcePath::from_relative_logical_path(Path::new("canvas.js"))
        .expect("built-in canvas logical source path is a proven internal invariant")
}
