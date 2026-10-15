//! Canonical runtime-import candidates for source modules and generated sidecars.
//!
//! Source ownership supplies conservative availability, while final executable call facts add
//! packages reached through re-exports. One checked projection reconciles provider and builder
//! runtime contracts before cloning the published records. Entry assembly alone selects liveness.

use crate::builder_surface::external_import_providers::provider::{
    BuilderRuntimePackageMetadata, RequiredRuntimeImport, RuntimeAssetIdentity,
};
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::hir::reachability::{HirModuleLinkFacts, ReachableExternalCall};
use crate::compiler_frontend::module_compilation::artefact::ModuleExternalImport;

use rustc_hash::{FxHashMap, FxHashSet};

/// Join source-owned availability with demand from final, converged function facts.
pub(crate) fn collect_external_import_candidates(
    source_logical_paths: &[String],
    function_facts: &HirModuleLinkFacts,
    registry: &ExternalPackageRegistry,
    resolution_table: &ExternalImportResolutionTable,
    builder_runtime_packages: &[BuilderRuntimePackageMetadata],
) -> Result<Vec<ModuleExternalImport>, CompilerError> {
    let mut package_ids =
        collect_external_import_package_ids(function_facts.direct_external_calls(), registry)?;
    package_ids.extend(
        resolution_table
            .resolved_imports_for_source_files(source_logical_paths)
            .map(|resolved| resolved.package_id),
    );
    package_ids.extend(
        builder_runtime_packages
            .iter()
            .map(|runtime| runtime.package_id),
    );
    collect_external_import_candidates_for_packages(
        &package_ids,
        resolution_table,
        builder_runtime_packages,
        registry,
    )
}

/// Resolve retained calls once in their issuing registry before deduplicating packages.
pub(crate) fn collect_external_import_package_ids<'a>(
    calls: impl Iterator<Item = &'a ReachableExternalCall>,
    registry: &ExternalPackageRegistry,
) -> Result<FxHashSet<ExternalPackageId>, CompilerError> {
    let mut package_ids = FxHashSet::default();
    for call in calls {
        if registry.get_function_by_id(call.function_id).is_none() {
            return Err(CompilerError::new(
                format!(
                    "Runtime-import inventory: finalized external function {:?} is not registered",
                    call.function_id
                ),
                call.span,
                ErrorType::Compiler,
            ));
        }
        let package_id = registry.resolve_function_package_id(call.function_id).ok_or_else(|| {
            CompilerError::new(
                format!("Runtime-import inventory: finalized external function {:?} has no owning package", call.function_id),
                call.span,
                ErrorType::Compiler,
            )
        })?;
        if registry.get_package_by_id(package_id).is_none() {
            return Err(CompilerError::new(
                format!(
                    "Runtime-import inventory: finalized external function {:?} refers to unknown package {package_id:?}",
                    call.function_id
                ),
                call.span,
                ErrorType::Compiler,
            ));
        }
        package_ids.insert(package_id);
    }
    Ok(package_ids)
}

/// Project one canonical runtime record per requested, registry-owned package.
///
/// No provider record is required for an inline/Core helper. A selected backend module export
/// owns the stricter asset requirement. Provider records retain their canonical provenance when
/// compatible builder references use another authored span or import ordering.
pub(crate) fn collect_external_import_candidates_for_packages(
    package_ids: &FxHashSet<ExternalPackageId>,
    resolution_table: &ExternalImportResolutionTable,
    builder_runtime_packages: &[BuilderRuntimePackageMetadata],
    registry: &ExternalPackageRegistry,
) -> Result<Vec<ModuleExternalImport>, CompilerError> {
    let mut package_ids = package_ids.iter().copied().collect::<Vec<_>>();
    package_ids.sort_by_key(|package_id| package_id.0);
    let mut candidates = Vec::with_capacity(package_ids.len());
    for package_id in package_ids {
        let package = registry.get_package_by_id(package_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Runtime-import inventory: candidate package {package_id:?} is not registered"
            ))
        })?;
        let provider = resolution_table.get_by_package_id(package_id);
        let mut canonical = provider.map(|resolved| {
            (
                resolved.runtime_asset.as_ref(),
                resolved.required_runtime_imports.as_slice(),
            )
        });
        for builder in builder_runtime_packages
            .iter()
            .filter(|runtime| runtime.package_id == package_id)
        {
            let incoming = (
                builder.runtime_asset.as_ref(),
                builder.required_runtime_imports.as_slice(),
            );
            if let Some(retained) = canonical {
                check_runtime_metadata_compatibility(
                    &package.path,
                    package_id,
                    retained,
                    incoming,
                )?;
            } else {
                canonical = Some(incoming);
            }
        }
        let Some((runtime_asset, required_runtime_imports)) = canonical else {
            continue;
        };
        candidates.push(ModuleExternalImport {
            package_id,
            runtime_asset: runtime_asset.cloned(),
            required_runtime_imports: required_runtime_imports.to_vec(),
        });
    }
    Ok(candidates)
}

/// Compare runtime identity, never the authored locations that merely requested it.
fn check_runtime_metadata_compatibility(
    package_path: &str,
    package_id: ExternalPackageId,
    retained: (Option<&RuntimeAssetIdentity>, &[RequiredRuntimeImport]),
    incoming: (Option<&RuntimeAssetIdentity>, &[RequiredRuntimeImport]),
) -> Result<(), CompilerError> {
    let conflicting_fact = match (retained.0, incoming.0) {
        (Some(_), None) | (None, Some(_)) => Some("runtime asset presence"),
        (Some(left), Some(right)) if left.origin != right.origin => Some("runtime asset origin"),
        (Some(left), Some(right)) if left.asset_kind != right.asset_kind => {
            Some("runtime asset kind")
        }
        (Some(left), Some(right)) if left.canonical_source_path != right.canonical_source_path => {
            Some("runtime asset canonical byte-source path")
        }
        _ => None,
    };
    let conflicting_fact = conflicting_fact.or_else(|| {
        (runtime_import_contract(retained.1) != runtime_import_contract(incoming.1))
            .then_some("required runtime module/import contract")
    });
    if let Some(fact) = conflicting_fact {
        return Err(CompilerError::new(
            format!(
                "Runtime-import inventory: incompatible {fact} for package '{package_path}' ({package_id:?}). Canonical asset/provenance: {:?}; conflicting asset/provenance: {:?}. Canonical runtime imports: {:?}; conflicting runtime imports: {:?}",
                retained.0, incoming.0, retained.1, incoming.1,
            ),
            retained
                .0
                .and_then(|asset| asset.source_span)
                .or_else(|| incoming.0.and_then(|asset| asset.source_span)),
            ErrorType::Compiler,
        ));
    }
    Ok(())
}

/// Borrow names into temporary sets only at a repeated-record compatibility boundary.
fn runtime_import_contract(imports: &[RequiredRuntimeImport]) -> FxHashMap<&str, FxHashSet<&str>> {
    let mut modules = FxHashMap::<&str, FxHashSet<&str>>::default();
    for import in imports {
        modules
            .entry(&import.module_name)
            .or_default()
            .extend(import.imported_names.iter().map(String::as_str));
    }
    modules
}

#[cfg(test)]
#[path = "tests/external_imports_tests.rs"]
mod tests;
