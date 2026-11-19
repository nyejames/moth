//! Provider JS runtime asset identity and registry attachment.
//!
//! WHAT: declares the stable provider-owned resource identity of one external JS runtime
//!       asset and attaches its canonical byte source to the shared resource registry.
//! WHY: external JS files are backend artifacts, not frontend source files. Their identity
//!      and declared output paths join the shared resource plan and conflict authority, and
//!      the central writer alone reads their bytes, so this module owns no filesystem reads.

use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::builder_surface::package_metadata::{PackageOrigin, PackageSetMembership};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::SourceSpan;
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
    source_span: Option<SourceSpan>,
    package_set_membership: PackageSetMembership,
) -> Result<RuntimeAssetIdentity, InfrastructureFailure> {
    let declared_output_path = js_runtime_asset_output_path(logical_source_path);
    let logical_path = PortableResourcePath::from_relative_logical_path(&declared_output_path)?;

    let owner = StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
        JS_RUNTIME_PROVIDER_KIND,
        package,
    ));

    Ok(RuntimeAssetIdentity {
        origin: StableResourceOriginId::new(owner, logical_path),
        canonical_source_path,
        package_set_membership,
        asset_kind: String::from("js"),
        source_span,
    })
}
/// Register every planned JS runtime asset as a byte source of the shared registry.
///
/// WHAT: registers each supplied byte-source spelling, then attaches each provider-owned origin to
///       its physical source through one preflighted association batch.
/// WHY: JS assets are deferred resource outputs. The central writer reads their bytes only
///      after the complete destination preflight has passed, exactly like module-owned
///      resources. Installed alias spellings stay unresolved here so registration performs no
///      filesystem work; the registry resolves them to one canonical read record at its first
///      content-read boundary.
pub(crate) fn register_js_runtime_asset_sources(
    plan: &HtmlExternalRuntimeEmissionPlan,
    resource_inputs: &mut ResourceInputRegistry,
) -> Result<(), InfrastructureFailure> {
    let mut associations = Vec::with_capacity(plan.js_assets().len());

    for asset in plan.js_assets().values() {
        let source = if uses_deferred_installed_alias(asset) {
            resource_inputs.register_deferred_shipped_source(asset.canonical_source_path.clone())
        } else {
            resource_inputs.register_source_with_membership(
                asset.canonical_source_path.clone(),
                asset.package_set_membership,
            )
        };
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

/// Whether one shipped runtime asset entered registration without a Stage 0 canonical path.
///
/// Direct `@builder/*` binding packages build their runtime asset path from
/// `BUILTIN_PACKAGES_ROOT` and never traverse a provider request. A provider-resolved JS unit
/// carries `ProjectLocal` package identity and the canonical path resolved by Stage 0. Only the
/// direct built-in units therefore wait for the first content read; ordinary and provider plans
/// keep their direct canonical registration lane.
fn uses_deferred_installed_alias(asset: &RuntimeAssetIdentity) -> bool {
    if !asset.package_set_membership.is_compiler_shipped() {
        return false;
    }
    match asset.origin.owner() {
        StableResourceOwnerId::Provider(owner) => {
            owner.package().origin() == PackageOrigin::Builder
        }
        StableResourceOwnerId::Module(_) => false,
    }
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
