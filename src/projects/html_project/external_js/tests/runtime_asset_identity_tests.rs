//! Portability tests for provider JS runtime asset identity.
//!
//! WHAT: proves the JS provider's registered package path, resource origin, and declared
//!       output path derive only from the portable logical source path, never from the
//!       canonical checkout location a file happened to be found under.
//! WHY: `@html-js/...` package identities and `_moth/js/...` output paths enter the shared
//!      resource conflict authority, so two checkout roots for one logical source must agree
//!      and two logical sources under one root must disagree.

use super::js_runtime_asset_identity;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
use crate::builder_surface::external_import_providers::provider::{
    ExternalImportProvider, ExternalImportProviderContext, ExternalImportRequest,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::js_import_provider::{
    JsExternalImportProvider, js_provider_package_path,
};
use std::path::{Path, PathBuf};

/// A parseable project-local JS module with one export, as Stage 0 would hand it over.
const WIDGET_JS_SOURCE: &str =
    "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 7; }\n";

struct ResolvedWidgetIdentity {
    registered_package_path: String,
    origin: StableResourceOriginId,
}

/// Resolve one logical JS source through the real JS provider against a fresh registry.
///
/// The caller supplies the logical spelling Stage 0 would derive plus the canonical IO path
/// the request pretends was resolved, mirroring `invoke_provider_and_record_resolution`.
fn resolve_widget_via_provider(
    logical_source_path: &str,
    canonical_source_path: PathBuf,
    string_table: &mut StringTable,
) -> Result<ResolvedWidgetIdentity, CompilerMessages> {
    let logical_source_path =
        PortableResourcePath::from_relative_logical_path(Path::new(logical_source_path))
            .expect("fixture logical JS path should be portable");
    let request = ExternalImportRequest {
        import_path: format!("@{}", logical_source_path.as_str()),
        logical_source_path,
        canonical_source_path,
        source_location: SourceLocation::default(),
    };

    let mut registry = ExternalPackageRegistry::new();
    let mut cache = ExternalImportProviderCache::new();
    let mut context = ExternalImportProviderContext {
        package_registry: &mut registry,
        cache: &mut cache,
        string_table,
    };

    let resolved = JsExternalImportProvider
        .resolve_external_import(request, &mut context)?
        .expect("a parseable JS module always resolves into a package");

    let draw_function_id = resolved.exported_free_functions[0];
    let registered_package_path = registry
        .resolve_function_package(draw_function_id)
        .expect("the provider should register its parsed export")
        .to_owned();
    let runtime_asset = resolved
        .runtime_asset
        .expect("the JS provider always declares a runtime asset");

    Ok(ResolvedWidgetIdentity {
        registered_package_path,
        origin: runtime_asset.origin,
    })
}

fn write_widget_js(checkout_root: &Path) -> PathBuf {
    let source_path = checkout_root.join("widget.js");

    std::fs::write(&source_path, WIDGET_JS_SOURCE)
        .expect("fixture should write the widget JS module");

    source_path
}

#[test]
fn same_relative_js_file_under_two_checkout_roots_shares_provider_identity() {
    let checkout_one = tempfile::tempdir().expect("first checkout root should build");
    let checkout_two = tempfile::tempdir().expect("second checkout root should build");

    let canonical_one =
        std::fs::canonicalize(write_widget_js(checkout_one.path())).expect("root should resolve");
    let canonical_two =
        std::fs::canonicalize(write_widget_js(checkout_two.path())).expect("root should resolve");
    assert_ne!(
        canonical_one, canonical_two,
        "the two checkout roots must produce distinct canonical IO paths"
    );

    let mut string_table = StringTable::new();
    let resolved_one = resolve_widget_via_provider("widget.js", canonical_one, &mut string_table)
        .expect("first checkout should resolve");
    let resolved_two = resolve_widget_via_provider("widget.js", canonical_two, &mut string_table)
        .expect("second checkout should resolve");

    assert_eq!(
        resolved_one.registered_package_path, "@html-js/widget.js",
        "the package path must spell the logical source, not a hash of the checkout path"
    );
    assert_eq!(
        resolved_one.registered_package_path, resolved_two.registered_package_path,
        "the same logical source under two checkouts must register one package path"
    );
    assert_eq!(
        resolved_one.origin, resolved_two.origin,
        "canonical IO paths differ, so they must not enter the resource origin"
    );
    assert_eq!(
        resolved_one.origin.logical_path().as_str(),
        "_moth/js/widget.js",
        "the declared output path must be the portable spelling under _moth/js"
    );
}

#[test]
fn distinct_relative_js_paths_produce_distinct_identities() {
    let checkout = tempfile::tempdir().expect("checkout root should build");
    let canonical_a = write_widget_two_level_js(checkout.path(), "a");
    let canonical_b = write_widget_two_level_js(checkout.path(), "b");

    let mut string_table = StringTable::new();
    let resolved_a = resolve_widget_via_provider("a/widget.js", canonical_a, &mut string_table)
        .expect("a/widget.js should resolve");
    let resolved_b = resolve_widget_via_provider("b/widget.js", canonical_b, &mut string_table)
        .expect("b/widget.js should resolve");

    assert_eq!(resolved_a.registered_package_path, "@html-js/a/widget.js");
    assert_eq!(resolved_b.registered_package_path, "@html-js/b/widget.js");
    assert_ne!(
        resolved_a.origin, resolved_b.origin,
        "two logical widgets must not collapse onto one resource origin"
    );
    assert_eq!(
        resolved_a.origin.logical_path().as_str(),
        "_moth/js/a/widget.js"
    );
    assert_eq!(
        resolved_b.origin.logical_path().as_str(),
        "_moth/js/b/widget.js"
    );
}

#[test]
fn same_logical_path_from_different_canonical_paths_yields_equal_origins() {
    let checkout_one = tempfile::tempdir().expect("first checkout root should build");
    let checkout_two = tempfile::tempdir().expect("second checkout root should build");
    let logical = PortableResourcePath::from_relative_logical_path(Path::new("widget.js"))
        .expect("fixture logical JS path should be portable");
    let package = StablePackageIdentity::binding(
        PackageOrigin::ProjectLocal,
        &js_provider_package_path(&logical),
    );

    let identity_one = js_runtime_asset_identity(
        package.clone(),
        &logical,
        checkout_one.path().join("widget.js"),
        SourceLocation::default(),
    )
    .expect("identity should build from the fixture logical path");

    let identity_two = js_runtime_asset_identity(
        package,
        &logical,
        checkout_two.path().join("widget.js"),
        SourceLocation::default(),
    )
    .expect("identity should build from the fixture logical path");

    assert_ne!(
        identity_one.canonical_source_path, identity_two.canonical_source_path,
        "the identity keeps each build's canonical IO path as an IO fact"
    );
    assert_eq!(
        identity_one.origin, identity_two.origin,
        "one logical source path means one origin regardless of checkout"
    );
}

fn write_widget_two_level_js(checkout_root: &Path, directory: &str) -> PathBuf {
    let source_path = checkout_root.join(directory).join("widget.js");

    std::fs::create_dir_all(source_path.parent().expect("parent should exist"))
        .expect("fixture should create the widget directory");
    std::fs::write(&source_path, WIDGET_JS_SOURCE)
        .expect("fixture should write the nested widget JS module");

    source_path
}
