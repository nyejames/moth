//! External import candidates for one compiled module or generated sidecar.
//!
//! WHAT: the two ways a candidate set is resolved — from a module's prepared source files, and
//!       from the external packages one generated sidecar's reachable functions call.
//! WHY: the frontend resolves which provider and builder runtime packages a module's executable
//!      functions can reach. Backends emit runtime assets and glue from these candidates without
//!      needing the per-source-file resolution table, so the compiler records them once here.
//!      Both collections end in the same deterministic package-ID order, so entry assembly never
//!      has to re-sort a module lane against a sidecar lane.

use crate::builder_surface::external_import_providers::provider::BuilderRuntimePackageMetadata;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::external_packages::ExternalPackageId;
use crate::compiler_frontend::module_compilation::artefact::ModuleExternalImport;

use rustc_hash::FxHashSet;

/// Collect the candidates one compiled module's prepared source files resolve to.
///
/// WHAT: every provider import resolved for the module's own source files, plus every builder
///       runtime package offered to this build.
/// WHY: builder runtime packages share the module's candidate store rather than a second lane,
///      because entry assembly selects from one list using its reachable function union. The
///      resolution table can return the same package for several source files, so the result is
///      deduplicated after sorting.
pub(crate) fn collect_external_import_candidates_for_source_files(
    source_logical_paths: &[String],
    resolution_table: &ExternalImportResolutionTable,
    builder_runtime_packages: &[BuilderRuntimePackageMetadata],
) -> Vec<ModuleExternalImport> {
    let resolved_imports =
        resolution_table.collect_unique_resolved_imports_for_source_files(source_logical_paths);

    let mut candidates =
        Vec::with_capacity(resolved_imports.len() + builder_runtime_packages.len());
    candidates.extend(
        resolved_imports
            .into_iter()
            .map(|resolved| ModuleExternalImport {
                package_id: resolved.package_id,
                runtime_asset: resolved.runtime_asset,
                required_runtime_imports: resolved.required_runtime_imports,
            }),
    );
    candidates.extend(builder_runtime_packages.iter().map(|builder_runtime| {
        ModuleExternalImport {
            package_id: builder_runtime.package_id,
            runtime_asset: builder_runtime.runtime_asset.clone(),
            required_runtime_imports: builder_runtime.required_runtime_imports.clone(),
        }
    }));

    candidates.sort_by_key(|candidate| candidate.package_id.0);
    candidates.dedup_by_key(|candidate| candidate.package_id);
    candidates
}

/// Collect the candidates one generated sidecar's reachable external functions resolve to.
///
/// WHY: a sidecar reaches external packages through its own reachability set rather than through
///      source files, so its candidates are resolved by package identity. Builder runtime packages
///      are consulted as a fallback for packages the resolution table does not own.
pub(crate) fn collect_external_import_candidates_for_packages(
    package_ids: &FxHashSet<ExternalPackageId>,
    resolution_table: &ExternalImportResolutionTable,
    builder_runtime_packages: &[BuilderRuntimePackageMetadata],
) -> Vec<ModuleExternalImport> {
    let mut candidates = Vec::with_capacity(package_ids.len());

    for package_id in package_ids {
        if let Some(resolved) = resolution_table.get_by_package_id(*package_id) {
            candidates.push(ModuleExternalImport {
                package_id: *package_id,
                runtime_asset: resolved.runtime_asset.clone(),
                required_runtime_imports: resolved.required_runtime_imports.clone(),
            });
            continue;
        }

        if let Some(builder_runtime) = builder_runtime_packages
            .iter()
            .find(|runtime| runtime.package_id == *package_id)
        {
            candidates.push(ModuleExternalImport {
                package_id: *package_id,
                runtime_asset: builder_runtime.runtime_asset.clone(),
                required_runtime_imports: builder_runtime.required_runtime_imports.clone(),
            });
        }
    }

    candidates.sort_by_key(|candidate| candidate.package_id.0);
    candidates
}
