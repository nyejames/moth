//! Provider JS runtime asset identity and registry attachment.
//!
//! WHAT: declares the stable provider-owned resource identity of one external JS runtime
//!       asset and attaches its canonical byte source to the shared resource registry.
//! WHY: external JS files are backend artifacts, not frontend source files. Their identity
//!      and declared output paths join the shared resource plan and conflict authority, and
//!      the central writer alone reads their bytes, so this module owns no filesystem reads.

use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use std::path::PathBuf;

/// Provider kind of the HTML JS lane that owns emitted JS runtime assets.
pub(crate) const JS_RUNTIME_PROVIDER_KIND: &str = "html-js";

/// Build the stable provider-owned identity of one JS runtime asset.
///
/// WHAT: interns a provider resource origin whose logical path is the declared stable output
///       path under `_moth/js/`, spelled out component-wise from the portable logical source
///       path. The canonical source path stays a byte-source fact.
/// WHY: JS runtime assets join the shared resource identity and conflict authority rather
///      than keying emission on a filesystem `PathBuf`, and they must never emit beside pages.
pub(crate) fn js_runtime_asset_identity(
    package: StablePackageIdentity,
    logical_source_path: &PortableResourcePath,
    canonical_source_path: PathBuf,
    authored_import_location: SourceLocation,
) -> Result<RuntimeAssetIdentity, CompilerError> {
    let declared_output_path = js_runtime_asset_output_path(logical_source_path);
    let logical_path = PortableResourcePath::from_relative_logical_path(&declared_output_path)?;

    let owner = StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
        JS_RUNTIME_PROVIDER_KIND,
        package,
    ));

    Ok(RuntimeAssetIdentity {
        origin: StableResourceOriginId::new(owner, logical_path),
        canonical_source_path,
        asset_kind: String::from("js"),
        authored_import_location,
    })
}

/// Register every planned JS runtime asset as a byte source of the shared registry.
///
/// WHAT: deduplicates the canonical sources, then attaches each provider-owned origin to its
///       physical source through one preflighted association batch.
/// WHY: JS assets are deferred resource outputs. The central writer reads their bytes only
///      after the complete destination preflight has passed, exactly like module-owned
///      resources.
pub(crate) fn register_js_runtime_asset_sources(
    plan: &HtmlExternalRuntimeEmissionPlan,
    resource_inputs: &mut ResourceInputRegistry,
) -> Result<(), CompilerError> {
    let mut associations = Vec::with_capacity(plan.js_assets().len());

    for asset in plan.js_assets().values() {
        let source = resource_inputs.register_source(asset.canonical_source_path.clone());
        associations.push(ResourceSourceAssociation {
            origin: asset.origin.clone(),
            source,
        });
    }

    let publication = resource_inputs.preflight_resource_source_associations(&associations)?;
    resource_inputs.reserve_resource_source_associations(&publication);
    resource_inputs.commit_resource_source_associations(publication);

    Ok(())
}

/// Generate the declared output path for a JS runtime asset.
///
/// WHAT: joins `_moth/js/` with the portable logical source spelling component by component.
/// WHY: the mapping is purely structural, so distinct logical sources always produce distinct
///      output paths without any hash of a machine-local canonical path. `output_path_for_origin`
///      trusts this contract: a provider origin's logical path is its declared output path.
fn js_runtime_asset_output_path(logical_source_path: &PortableResourcePath) -> PathBuf {
    let mut output_path = PathBuf::from("_moth");
    output_path.push("js");

    for component in logical_source_path.as_str().split('/') {
        output_path.push(component);
    }

    output_path
}

#[cfg(test)]
#[path = "tests/runtime_asset_identity_tests.rs"]
mod tests;
