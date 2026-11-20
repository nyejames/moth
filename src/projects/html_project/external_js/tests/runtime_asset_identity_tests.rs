//! Portability tests for provider JS runtime asset identity.
//!
//! WHAT: proves the JS provider's registered package path, resource origin, and declared
//!       output path derive only from the portable logical source path, never from the
//!       canonical checkout location a file happened to be found under.
//! WHY: `@html-js/...` package identities and `_moth/js/...` output paths enter the shared
//!      resource conflict authority, so two checkout roots for one logical source must agree
//!      and two logical sources under one root must disagree.

use super::{js_runtime_asset_identity, register_js_runtime_asset_sources};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache;
#[cfg(unix)]
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::builder_surface::external_import_providers::provider::{
    ExternalImportProvider, ExternalImportProviderContext, ExternalImportRequest,
};
use crate::builder_surface::package_metadata::PackageSetMembership;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::module_compilation::ModuleExternalImport;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::js_import_provider::{
    JsExternalImportProvider, js_provider_package_path,
};
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use std::path::{Path, PathBuf};

/// A parseable project-local JS module with one export, as Stage 0 would hand it over.
const WIDGET_JS_SOURCE: &str =
    "/**\n * @moth.sig draw || -> I32\n */\nexport function draw() { return 7; }\n";

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
) -> Result<ResolvedWidgetIdentity, PreparedFailure> {
    let logical_source_path =
        PortableResourcePath::from_relative_logical_path(Path::new(logical_source_path))
            .expect("fixture logical JS path should be portable");
    let request = ExternalImportRequest {
        import_path: format!("@{}", logical_source_path.as_str()),
        logical_source_path,
        canonical_source_path,
        source_span: None,
    };

    let mut registry = ExternalPackageRegistry::new();
    let mut cache = ExternalImportProviderCache::new();
    let mut path_fork = PathInternerFork::empty();
    let mut context = ExternalImportProviderContext {
        package_registry: &mut registry,
        cache: &mut cache,
        path_fork: &mut path_fork,
        string_table,
        provider_sources: None,
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
        None,
        PackageSetMembership::Ordinary,
    )
    .expect("identity should build from the fixture logical path");

    let identity_two = js_runtime_asset_identity(
        package,
        &logical,
        checkout_two.path().join("widget.js"),
        None,
        PackageSetMembership::Ordinary,
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

#[cfg(unix)]
#[test]
fn shipped_runtime_symlink_merges_before_authored_first_read_failure() {
    let fixtures = shipped_symlink_fixture();
    let (ordinary, shipped) = fixtures.identities();
    let plan = runtime_plan([ordinary.clone(), shipped.clone()]);
    let mut resources = ResourceInputRegistry::new();
    register_js_runtime_asset_sources(&plan, &mut resources)
        .expect("runtime asset registration should succeed");

    let ordinary_source = resources
        .source_for_origin(&ordinary.origin)
        .expect("ordinary origin should attach");
    let shipped_source = resources
        .source_for_origin(&shipped.origin)
        .expect("shipped origin should attach");
    assert_ne!(
        ordinary_source, shipped_source,
        "registration must keep provisional logical IDs instead of resolving the symlink"
    );
    assert_eq!(
        resources.records().len(),
        2,
        "registration must not collapse the two supplied spellings into one record"
    );

    // The first content read after complete preflight settles every pending shipped alias before
    // reading bytes. Reading an unrelated resource proves the authored target has already inherited
    // shipped provenance before its own read.
    let unrelated_path = fixtures.temp.path().join("unrelated.bin");
    std::fs::write(&unrelated_path, [1_u8, 2, 3]).expect("should write unrelated resource");
    let unrelated_source = resources.register_source(unrelated_path);
    resources
        .hash_source(unrelated_source)
        .expect("an unrelated first read should settle pending aliases");
    resources
        .validate()
        .expect("settled aliases must keep the path and materialization indexes consistent");

    std::fs::remove_file(&fixtures.real_path).expect("the runtime asset should be removable");
    let failure = resources
        .hash_source(ordinary_source)
        .expect_err("the authored canonical read should fail after the target is removed");
    assert!(
        failure.message.contains("incomplete"),
        "shipped provenance must survive the ordinary registration: {}",
        failure.message
    );
}

#[cfg(unix)]
#[test]
fn shipped_runtime_symlink_redirects_to_one_cached_read() {
    let fixtures = shipped_symlink_fixture();
    let (ordinary, shipped) = fixtures.identities();
    let expected =
        std::fs::read(&fixtures.real_path).expect("fixture runtime JS should be readable");
    let plan = runtime_plan([ordinary.clone(), shipped.clone()]);
    let mut resources = ResourceInputRegistry::new();
    register_js_runtime_asset_sources(&plan, &mut resources)
        .expect("runtime asset registration should succeed");

    let ordinary_source = resources
        .source_for_origin(&ordinary.origin)
        .expect("ordinary origin should attach");
    let shipped_source = resources
        .source_for_origin(&shipped.origin)
        .expect("shipped origin should attach");
    assert_ne!(
        ordinary_source, shipped_source,
        "registration must keep provisional logical IDs instead of resolving the symlink"
    );

    let unrelated_path = fixtures.temp.path().join("unrelated.bin");
    std::fs::write(&unrelated_path, [4_u8, 5, 6]).expect("should write unrelated resource");
    let unrelated_source = resources.register_source(unrelated_path);
    resources
        .hash_source(unrelated_source)
        .expect("an unrelated first read should settle pending aliases");

    assert_eq!(
        resources
            .read_source(ordinary_source)
            .expect("the canonical target should read once"),
        expected
    );
    std::fs::remove_file(&fixtures.real_path).expect("the runtime asset should be removable");
    assert_eq!(
        resources
            .read_source(shipped_source)
            .expect("the shipped alias should reuse the canonical cached bytes"),
        expected
    );
    resources
        .validate()
        .expect("a redirected read must leave one owner and no alias content state");
}

#[cfg(unix)]
#[test]
fn shipped_runtime_symlink_merges_membership_when_target_vanishes_before_first_read() {
    let fixtures = shipped_symlink_fixture();
    let (ordinary, shipped) = fixtures.identities();
    let canonical_real_path =
        std::fs::canonicalize(&fixtures.real_path).expect("runtime JS should canonicalize");
    let plan = runtime_plan([ordinary.clone(), shipped.clone()]);
    let mut resources = ResourceInputRegistry::new();
    register_js_runtime_asset_sources(&plan, &mut resources)
        .expect("runtime asset registration should succeed");

    let ordinary_source = resources
        .source_for_origin(&ordinary.origin)
        .expect("ordinary origin should attach");
    let shipped_source = resources
        .source_for_origin(&shipped.origin)
        .expect("shipped origin should attach");
    assert_ne!(
        ordinary_source, shipped_source,
        "registration must keep provisional logical IDs instead of resolving the symlink"
    );

    std::fs::remove_file(&fixtures.real_path)
        .expect("the runtime target should vanish before metadata resolution");
    let failure = resources
        .hash_source(ordinary_source)
        .expect_err("the authored canonical read should fail after the target is removed");
    assert!(
        failure.message.contains("incomplete"),
        "the dangling shipped alias must still join the authored canonical key: {}",
        failure.message
    );
    assert_eq!(
        failure.host_path.as_deref(),
        Some(canonical_real_path.as_path()),
        "the authored source read reports its own canonical host path"
    );
    assert_eq!(
        resources.records()[shipped_source.index()].canonical_source_path(),
        fixtures.symlink_path.as_path(),
        "a missing alias retains its registered spelling instead of being rewritten"
    );
    resources
        .validate()
        .expect("a metadata-only alias merge must keep the sparse indexes consistent");
}

#[cfg(unix)]
#[test]
fn shipped_runtime_missing_leaf_under_symlink_directory_merges_membership() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("should create package fixture root");
    let real_directory = temp.path().join("packages/builder/web/canvas");
    std::fs::create_dir_all(&real_directory).expect("should create real package directory");
    let real_path = real_directory.join("canvas.js");
    std::fs::write(&real_path, "export function draw() {}\n").expect("should write runtime JS");
    let canonical_real_path =
        std::fs::canonicalize(&real_path).expect("runtime JS should canonicalize");
    let installation_parent = temp.path().join("installation/builder/web");
    std::fs::create_dir_all(&installation_parent)
        .expect("should create installation package parent");
    let symlinked_directory = installation_parent.join("canvas");
    symlink(&real_directory, &symlinked_directory)
        .expect("should create package directory symlink");
    let alias_path = symlinked_directory.join("canvas.js");

    let ordinary = js_runtime_asset_identity(
        StablePackageIdentity::binding(PackageOrigin::ProjectLocal, "@test/ordinary-asset"),
        &PortableResourcePath::from_relative_logical_path(Path::new("ordinary.js"))
            .expect("fixture ordinary logical path should be portable"),
        canonical_real_path.clone(),
        None,
        PackageSetMembership::Ordinary,
    )
    .expect("ordinary runtime asset identity should build");
    let shipped = js_runtime_asset_identity(
        StablePackageIdentity::binding(PackageOrigin::Builder, "@builder/web/canvas"),
        &PortableResourcePath::from_relative_logical_path(Path::new("canvas.js"))
            .expect("fixture shipped logical path should be portable"),
        alias_path.clone(),
        None,
        PackageSetMembership::CompilerShipped,
    )
    .expect("shipped runtime asset identity should build");
    let plan = runtime_plan([ordinary.clone(), shipped.clone()]);
    let mut resources = ResourceInputRegistry::new();
    register_js_runtime_asset_sources(&plan, &mut resources)
        .expect("runtime asset registration should succeed");

    let ordinary_source = resources
        .source_for_origin(&ordinary.origin)
        .expect("ordinary origin should attach");
    let shipped_source = resources
        .source_for_origin(&shipped.origin)
        .expect("shipped origin should attach");
    assert_ne!(
        ordinary_source, shipped_source,
        "registration must keep provisional logical IDs instead of resolving the symlinked directory"
    );

    std::fs::remove_file(&real_path).expect("the runtime target should vanish before first read");
    let failure = resources
        .hash_source(ordinary_source)
        .expect_err("the authored canonical read should fail after the target is removed");
    assert!(
        failure.message.contains("incomplete"),
        "a missing leaf below a symlinked directory must join the authored key: {}",
        failure.message
    );
    assert_eq!(
        failure.host_path.as_deref(),
        Some(canonical_real_path.as_path()),
        "the authored source read reports its own canonical host path"
    );
    assert_eq!(
        resources.records()[shipped_source.index()].canonical_source_path(),
        alias_path.as_path(),
        "a missing alias retains its registered spelling instead of being rewritten"
    );
    resources
        .validate()
        .expect("a metadata-only alias merge must keep the sparse indexes consistent");
}

#[cfg(unix)]
#[test]
fn dangling_shipped_runtime_symlink_reports_damage_on_its_own_read() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("should create package fixture root");
    let real_path = temp.path().join("packages/builder/web/canvas/canvas.js");
    std::fs::create_dir_all(real_path.parent().expect("parent should exist"))
        .expect("should create real package directory");
    std::fs::write(&real_path, "export function draw() {}\n").expect("should write runtime JS");
    let symlink_path = temp
        .path()
        .join("installation/builder/web/canvas/canvas.js");
    std::fs::create_dir_all(symlink_path.parent().expect("parent should exist"))
        .expect("should create installation package directory");
    symlink(&real_path, &symlink_path).expect("should create runtime asset symlink");
    std::fs::remove_file(&real_path).expect("fixture symlink should be dangling");

    let shipped = js_runtime_asset_identity(
        StablePackageIdentity::binding(PackageOrigin::Builder, "@builder/web/canvas"),
        &PortableResourcePath::from_relative_logical_path(Path::new("canvas.js"))
            .expect("fixture shipped logical path should be portable"),
        symlink_path.clone(),
        None,
        PackageSetMembership::CompilerShipped,
    )
    .expect("shipped runtime asset identity should build");
    let plan = runtime_plan([shipped.clone()]);
    let mut resources = ResourceInputRegistry::new();
    register_js_runtime_asset_sources(&plan, &mut resources)
        .expect("registration must defer missing-byte failures to the write phase");
    let source = resources
        .source_for_origin(&shipped.origin)
        .expect("dangling shipped origin should attach");

    let failure = resources
        .hash_source(source)
        .expect_err("a dangling shipped symlink should fail its deferred read");
    assert_eq!(
        failure.host_path.as_deref(),
        Some(symlink_path.as_path()),
        "a failed alias resolution must retain the supplied byte-source spelling"
    );
    assert!(
        failure.message.contains("incomplete"),
        "the shipped ID itself reports the damaged package set: {}",
        failure.message
    );
}

#[cfg(unix)]
struct ShippedSymlinkFixture {
    temp: tempfile::TempDir,
    real_path: PathBuf,
    symlink_path: PathBuf,
}

#[cfg(unix)]
impl ShippedSymlinkFixture {
    fn identities(&self) -> (RuntimeAssetIdentity, RuntimeAssetIdentity) {
        let canonical_real_path =
            std::fs::canonicalize(&self.real_path).expect("runtime JS should canonicalize");
        let ordinary = js_runtime_asset_identity(
            StablePackageIdentity::binding(PackageOrigin::ProjectLocal, "@test/ordinary-asset"),
            &PortableResourcePath::from_relative_logical_path(Path::new("ordinary.js"))
                .expect("fixture ordinary logical path should be portable"),
            canonical_real_path,
            None,
            PackageSetMembership::Ordinary,
        )
        .expect("ordinary runtime asset identity should build");
        let shipped = js_runtime_asset_identity(
            StablePackageIdentity::binding(PackageOrigin::Builder, "@builder/web/canvas"),
            &PortableResourcePath::from_relative_logical_path(Path::new("canvas.js"))
                .expect("fixture shipped logical path should be portable"),
            self.symlink_path.clone(),
            None,
            PackageSetMembership::CompilerShipped,
        )
        .expect("shipped runtime asset identity should build");
        (ordinary, shipped)
    }
}

#[cfg(unix)]
fn shipped_symlink_fixture() -> ShippedSymlinkFixture {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("should create package fixture root");
    let real_path = temp.path().join("packages/builder/web/canvas/canvas.js");
    std::fs::create_dir_all(real_path.parent().expect("parent should exist"))
        .expect("should create real package directory");
    std::fs::write(&real_path, "export function draw() {}\n").expect("should write runtime JS");
    let symlink_path = temp
        .path()
        .join("installation/builder/web/canvas/canvas.js");
    std::fs::create_dir_all(symlink_path.parent().expect("parent should exist"))
        .expect("should create installation package directory");
    symlink(&real_path, &symlink_path).expect("should create runtime asset symlink");
    ShippedSymlinkFixture {
        temp,
        real_path,
        symlink_path,
    }
}

#[cfg(unix)]
fn runtime_plan(
    assets: impl IntoIterator<Item = RuntimeAssetIdentity>,
) -> HtmlExternalRuntimeEmissionPlan {
    let imports = assets
        .into_iter()
        .enumerate()
        .map(|(index, runtime_asset)| ModuleExternalImport {
            package_id: ExternalPackageId(index as u32 + 1),
            runtime_asset: Some(runtime_asset),
            required_runtime_imports: Vec::new(),
        })
        .collect::<Vec<_>>();
    HtmlExternalRuntimeEmissionPlan::from_import_sets([imports.as_slice()])
}

#[test]
fn missing_shipped_runtime_asset_keeps_its_deferred_read_failure() {
    let temp = tempfile::tempdir().expect("should create runtime asset fixture root");
    let missing_path = temp.path().join("missing-canvas.js");
    let shipped = js_runtime_asset_identity(
        StablePackageIdentity::binding(PackageOrigin::Builder, "@builder/web/canvas"),
        &PortableResourcePath::from_relative_logical_path(Path::new("missing-canvas.js"))
            .expect("fixture shipped logical path should be portable"),
        missing_path.clone(),
        None,
        PackageSetMembership::CompilerShipped,
    )
    .expect("missing shipped runtime asset identity should build");
    let imports = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(shipped.clone()),
        required_runtime_imports: Vec::new(),
    }];
    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([imports.as_slice()]);
    let mut resources = ResourceInputRegistry::new();
    register_js_runtime_asset_sources(&plan, &mut resources)
        .expect("registration must defer missing-byte failures to the write phase");
    let source = resources
        .source_for_origin(&shipped.origin)
        .expect("missing shipped origin should attach");

    let failure = resources
        .hash_source(source)
        .expect_err("the missing shipped source should fail at its deferred read");
    assert_eq!(
        failure.host_path.as_deref(),
        Some(missing_path.as_path()),
        "a failed canonicalize must retain the supplied byte-source path"
    );
    assert!(
        failure.message.contains("incomplete"),
        "a missing shipped runtime asset reports its damaged package set: {}",
        failure.message
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
